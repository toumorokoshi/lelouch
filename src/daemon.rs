use crate::config::{self, RepoConfig};
use crate::executor::resolve_executor;
use crate::work_db::{Task, WorkDb};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info};

const STREAM_UPDATE_INTERVAL_SECS: u64 = 5;

/// Future that completes when a shutdown signal (SIGINT or SIGTERM) is received.
async fn shutdown_signal() {
    #[cfg(unix)]
    let sigterm = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        } else {
            std::future::pending::<()>().await
        }
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm => {}
    }
}

async fn run_streaming_updater(
    mut rx: mpsc::Receiver<String>,
    work_db: Arc<dyn WorkDb>,
    task_id: String,
    comment_id: String,
    repo_path: std::path::PathBuf,
    accumulated_tx: oneshot::Sender<String>,
) {
    let mut accumulated = String::new();
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        STREAM_UPDATE_INTERVAL_SECS,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            chunk = rx.recv() => {
                match chunk {
                    Some(s) => {
                        accumulated.push_str(&s);
                        accumulated.push('\n');
                    }
                    None => break,
                }
            }
            _ = interval.tick() => {
                if !accumulated.is_empty() {
                    if let Err(e) = work_db.update_comment(&task_id, &comment_id, &accumulated, &repo_path) {
                        error!(task_id = %task_id, error = %e, "streaming update failed");
                    }
                }
            }
        }
    }
    if !accumulated.is_empty() {
        let _ = work_db.update_comment(&task_id, &comment_id, &accumulated, &repo_path);
    }
    let _ = accumulated_tx.send(accumulated);
}

/// The daemon watches the config file and spins up a worker task for each repository.
pub struct Daemon {
    config_path: std::path::PathBuf,
    work_db: Arc<dyn WorkDb>,
    /// When true, do not dispatch tasks (startup scan and poll only).
    dry_run: bool,
}

impl Daemon {
    pub fn new(config_path: std::path::PathBuf, work_db: Arc<dyn WorkDb>, dry_run: bool) -> Self {
        Self {
            config_path,
            work_db,
            dry_run,
        }
    }

    /// Run the daemon: spawn workers for configured repositories and watch config for changes.
    pub async fn run(&self) -> Result<()> {
        info!("lelouch daemon starting");

        let mut current_repos: HashMap<String, RepoConfig> = HashMap::new();
        let mut workers: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
        let mut last_modified = std::time::SystemTime::UNIX_EPOCH;

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Check config modification
                    if let Ok(metadata) = std::fs::metadata(&self.config_path) {
                        if let Ok(modified) = metadata.modified() {
                            if modified > last_modified {
                                info!("config update detected");
                                last_modified = modified;
                                match config::load_config_from(&self.config_path) {
                                    Ok(cfg) => {
                                        let new_repos: HashMap<String, RepoConfig> = cfg
                                            .repositories
                                            .into_iter()
                                            .map(|r| (r.name.clone(), r))
                                            .collect();

                                        // Stop removed or changed repositories
                                        for (name, old_repo) in current_repos.iter() {
                                            let should_stop = match new_repos.get(name) {
                                                Some(new_repo) => new_repo != old_repo,
                                                None => true,
                                            };
                                            if should_stop {
                                                if let Some(handle) = workers.remove(name) {
                                                    info!(repo = %name, "stopping worker for repository");
                                                    handle.abort();
                                                }
                                            }
                                        }

                                        // Start new or changed repositories
                                        for (name, new_repo) in new_repos.iter() {
                                            if !workers.contains_key(name) {
                                                info!(repo = %name, "starting worker for repository");
                                                // Quick startup scan
                                                if let Ok(repo_path) = new_repo.resolved_path() {
                                                    if let Err(e) = self.work_db.full_scan(&repo_path) {
                                                        error!(repo = %name, error = %e, "startup scan failed");
                                                    }
                                                }
                                                let handle = tokio::spawn(run_worker(
                                                    new_repo.clone(),
                                                    Arc::clone(&self.work_db),
                                                    self.dry_run,
                                                ));
                                                workers.insert(name.clone(), handle);
                                            }
                                        }

                                        current_repos = new_repos;
                                    }
                                    Err(e) => {
                                        error!(error = %e, "failed to reload config");
                                    }
                                }
                            }
                        }
                    }
                }
                _ = shutdown_signal() => {
                    info!("received shutdown signal, exiting");
                    break;
                }
            }
        }
        Ok(())
    }
}

async fn run_worker(repo: RepoConfig, work_db: Arc<dyn WorkDb>, dry_run: bool) {
    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(repo.poll_interval_secs));
    let in_flight = Arc::new(Mutex::new(HashSet::new()));

    info!(repo = %repo.name, interval = repo.poll_interval_secs, "worker started");

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(()) = process_ready_tasks(&repo, &work_db, dry_run, &in_flight).await {
                    break;
                }
            }
            _ = shutdown_signal() => {
                info!(repo = %repo.name, "worker shutdown signal received");
                break;
            }
        }
    }
}

async fn process_ready_tasks(
    repo: &RepoConfig,
    work_db: &Arc<dyn WorkDb>,
    dry_run: bool,
    in_flight: &Arc<Mutex<HashSet<String>>>,
) -> Result<(), ()> {
    let repo_path = match repo.resolved_path() {
        Ok(p) => p,
        Err(e) => {
            error!(repo = %repo.name, error = %e, "failed to resolve repo path");
            return Ok(());
        }
    };

    let tasks = match work_db.poll_ready(&repo_path) {
        Ok(t) => t,
        Err(e) => {
            error!(repo = %repo.name, error = %e, "error polling repo");
            return Ok(());
        }
    };

    let mut in_flight_guard = in_flight.lock().await;
    let candidate = tasks.into_iter().find(|t| !in_flight_guard.contains(&t.id));

    if let Some(task) = candidate {
        if dry_run {
            info!(
                task_id = %task.id,
                title = %task.title,
                repo = %repo.name,
                "dry-run: would dispatch task"
            );
        } else {
            // Drop guard before awaiting dispatch_task
            let task_id = task.id.clone();
            in_flight_guard.insert(task_id.clone());
            drop(in_flight_guard);

            if dispatch_task(repo, &task, work_db, in_flight)
                .await
                .is_err()
            {
                return Err(());
            }
        }
    }
    Ok(())
}

/// Dispatch a single task to the appropriate executor and wait for it to complete.
/// Returns Err(()) if shutdown was requested during execution (issue was moved back to open).
async fn dispatch_task(
    repo: &RepoConfig,
    task: &Task,
    work_db: &Arc<dyn WorkDb>,
    in_flight: &Arc<Mutex<HashSet<String>>>,
) -> Result<(), ()> {
    let task_id = task.id.clone();

    let repo_path = match repo.resolved_path() {
        Ok(p) => p,
        Err(e) => {
            error!(repo = %repo.name, error = %e, "failed to resolve repo path");
            in_flight.lock().await.remove(&task_id);
            return Ok(());
        }
    };

    if let Err(e) = work_db.set_in_progress(&task.id, &repo_path) {
        error!(
            task_id = %task.id,
            error = %e,
            "failed to mark task as in_progress"
        );
    }

    info!(
        task_id = %task.id,
        title = %task.title,
        repo = %repo.name,
        executor = %repo.executor,
        "dispatching task"
    );

    let executor = match resolve_executor(&repo.executor) {
        Ok(e) => e,
        Err(e) => {
            error!(
                executor = %repo.executor,
                error = %e,
                "failed to resolve executor"
            );
            if let Err(e) = work_db.set_open(&task.id, &repo_path) {
                error!(task_id = %task.id, error = %e, "failed to move task back to open");
            }
            in_flight.lock().await.remove(&task_id);
            return Ok(());
        }
    };

    let pre_prompt = repo.pre_prompt.as_deref();
    let comment_id_opt =
        match work_db.add_streaming_comment(&task.id, "Agent output:\n\n", &repo_path) {
            Ok(id) => id,
            Err(e) => {
                error!(task_id = %task.id, error = %e, "failed to create streaming comment");
                None
            }
        };

    let (output_tx, output_rx) = match &comment_id_opt {
        Some(_) => {
            let (tx, rx) = mpsc::channel::<String>(256);
            (Some(tx), Some(rx))
        }
        None => (None, None),
    };

    let (accumulated_tx, accumulated_rx) = match &comment_id_opt {
        Some(_) => {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        }
        None => (None, None),
    };

    if let (Some(comment_id), Some(rx), Some(acc_tx)) =
        (comment_id_opt.as_ref(), output_rx, accumulated_tx)
    {
        let db_clone = Arc::clone(work_db);
        let id_clone = task.id.clone();
        let cid_clone = comment_id.clone();
        let path_clone = repo_path.to_path_buf();
        tokio::spawn(run_streaming_updater(
            rx, db_clone, id_clone, cid_clone, path_clone, acc_tx,
        ));
    }

    let model = repo.model.as_deref();
    let run = executor.execute(task, &repo_path, pre_prompt, model, output_tx);

    let result = tokio::select! {
        res = run => res,
        _ = shutdown_signal() => {
            info!(task_id = %task.id, "shutdown during task, moving issue back to open");
            if let Err(e) = work_db.set_open(&task.id, &repo_path) {
                error!(task_id = %task.id, error = %e, "failed to move task back to open");
            }
            in_flight.lock().await.remove(&task_id);
            return Err(());
        }
    };

    let response = match result {
        Ok(r) => r,
        Err(e) => {
            error!(task_id = %task.id, error = %e, "task execution failed");
            if let Err(e) = work_db.set_open(&task.id, &repo_path) {
                error!(
                    task_id = %task.id,
                    error = %e,
                    "failed to move task back to open"
                );
            }
            in_flight.lock().await.remove(&task_id);
            return Ok(());
        }
    };

    let accumulated_opt = if let Some(rx) = accumulated_rx {
        rx.await.ok()
    } else {
        None
    };

    if let (Some(comment_id), Some(accumulated)) = (comment_id_opt.as_ref(), accumulated_opt) {
        let body = match &response {
            Some(r) if !r.trim().is_empty() => {
                format!("{}\n\n---\nResult:\n{}", accumulated, r)
            }
            _ => accumulated,
        };
        if let Err(e) = work_db.update_comment(&task.id, comment_id, &body, &repo_path) {
            error!(task_id = %task.id, error = %e, "failed to update streaming comment");
        }
    } else if let Some(ref body) = response {
        if !body.trim().is_empty() {
            if let Err(e) = work_db.add_comment(&task.id, body, &repo_path) {
                error!(
                    task_id = %task.id,
                    error = %e,
                    "failed to add executor response as comment"
                );
            }
        }
    }

    if let Err(e) = work_db.set_complete(&task.id, &repo_path) {
        error!(
            task_id = %task.id,
            error = %e,
            "failed to mark task as complete"
        );
    }

    in_flight.lock().await.remove(&task_id);
    Ok(())
}
