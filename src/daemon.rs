use crate::config::{self, RepoConfig};
use crate::executor::resolve_executor;
use crate::work_db::{Task, WorkDb};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info};

type SharedState = Arc<Mutex<HashMap<String, Option<Task>>>>;

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

        let states: SharedState = Arc::new(Mutex::new(HashMap::new()));
        let (notify_tx, mut notify_rx) = mpsc::channel::<()>(100);

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
                                                    {
                                                        let mut states_guard = states.lock().await;
                                                        states_guard.remove(name);
                                                        // Also remove any suffixed worker entries (e.g. "name-0")
                                                        let prefix = format!("{}-", name);
                                                        states_guard.retain(|k, _| k != name && !k.starts_with(&prefix));
                                                    }
                                                    let _ = notify_tx.try_send(());
                                                    handle.abort();
                                                }
                                            }
                                        }

                                        // Start new or changed repositories
                                        for (name, new_repo) in new_repos.iter() {
                                            if !workers.contains_key(name) {
                                                info!(repo = %name, "starting worker for repository");
                                                if let Ok(repo_path) = new_repo.resolved_path() {
                                                    if let Err(e) = self.work_db.full_scan(&repo_path) {
                                                        error!(repo = %name, error = %e, "startup scan failed");
                                                    }
                                                }
                                                let handle = tokio::spawn(run_worker(
                                                    new_repo.clone(),
                                                    Arc::clone(&self.work_db),
                                                    self.dry_run,
                                                    Arc::clone(&states),
                                                    notify_tx.clone(),
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
                Some(_) = notify_rx.recv() => {
                    print_status_table(&states).await;
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

async fn run_worker(
    repo: RepoConfig,
    work_db: Arc<dyn WorkDb>,
    dry_run: bool,
    states: SharedState,
    notify_tx: mpsc::Sender<()>,
) {
    let repo_path = match repo.resolved_path() {
        Ok(p) => p,
        Err(e) => {
            error!(repo = %repo.name, error = %e, "failed to resolve repo path");
            return;
        }
    };

    let effective_max_workers = if repo.in_repo {
        1
    } else {
        repo.max_worker_count
    };
    let wt_manager = Arc::new(crate::worktree::WorktreeManager::new(
        repo.name.clone(),
        repo_path.clone(),
        effective_max_workers,
        Box::new(crate::vcs::git::GitVcs),
    ));

    if !repo.in_repo {
        if let Err(e) = wt_manager.sync_worktrees() {
            error!(repo = %repo.name, error = %e, "failed to sync worktrees");
            return;
        }
    }

    // Initialize worker states
    {
        let mut states_guard = states.lock().await;
        for i in 0..effective_max_workers {
            let key = if repo.in_repo {
                repo.name.clone()
            } else {
                format!("{}-{}", repo.name, i)
            };
            states_guard.insert(key, None);
        }
    }
    let _ = notify_tx.try_send(());

    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(repo.poll_interval_secs));
    let in_flight = Arc::new(Mutex::new(HashSet::new()));
    let available_worktrees = Arc::new(Mutex::new(
        (0..effective_max_workers).collect::<Vec<usize>>(),
    ));

    info!(repo = %repo.name, interval = repo.poll_interval_secs, workers = effective_max_workers, "worker started");

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(()) = process_ready_tasks(
                    &repo,
                    &repo_path,
                    &wt_manager,
                    &work_db,
                    dry_run,
                    &in_flight,
                    &available_worktrees,
                    &states,
                    &notify_tx,
                ).await {
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

#[allow(clippy::too_many_arguments)]
async fn process_ready_tasks(
    repo: &RepoConfig,
    repo_path: &std::path::Path,
    wt_manager: &Arc<crate::worktree::WorktreeManager>,
    work_db: &Arc<dyn WorkDb>,
    dry_run: bool,
    in_flight: &Arc<Mutex<HashSet<String>>>,
    available_worktrees: &Arc<Mutex<Vec<usize>>>,
    states: &SharedState,
    notify_tx: &mpsc::Sender<()>,
) -> Result<(), ()> {
    let tasks = match work_db.poll_ready(repo_path) {
        Ok(t) => t,
        Err(e) => {
            error!(repo = %repo.name, error = %e, "error polling repo");
            return Ok(());
        }
    };

    let mut in_flight_guard = in_flight.lock().await;
    let mut available_guard = available_worktrees.lock().await;

    for task in tasks {
        if in_flight_guard.contains(&task.id) {
            continue;
        }

        if let Some(wt_index) = available_guard.pop() {
            if dry_run {
                info!(
                    task_id = %task.id,
                    title = %task.title,
                    repo = %repo.name,
                    wt_index = wt_index,
                    "dry-run: would dispatch task"
                );
                available_guard.push(wt_index);
            } else {
                let task_id = task.id.clone();
                in_flight_guard.insert(task_id.clone());

                let state_key = if repo.in_repo {
                    repo.name.clone()
                } else {
                    format!("{}-{}", repo.name, wt_index)
                };
                states.lock().await.insert(state_key, Some(task.clone()));
                let _ = notify_tx.try_send(());

                let repo_clone = repo.clone();
                let task_clone = task.clone();
                let work_db_clone = Arc::clone(work_db);
                let in_flight_clone = Arc::clone(in_flight);
                let available_clone = Arc::clone(available_worktrees);
                let states_clone = Arc::clone(states);
                let notify_tx_clone = notify_tx.clone();
                let wt_manager_clone = Arc::clone(wt_manager);
                let repo_path_clone = repo_path.to_path_buf();

                tokio::spawn(async move {
                    let _ = dispatch_task(
                        &repo_clone,
                        &repo_path_clone,
                        &task_clone,
                        &work_db_clone,
                        &in_flight_clone,
                        wt_index,
                        &wt_manager_clone,
                    )
                    .await;

                    available_clone.lock().await.push(wt_index);
                    let state_key = if repo_clone.in_repo {
                        repo_clone.name.clone()
                    } else {
                        format!("{}-{}", repo_clone.name, wt_index)
                    };
                    states_clone.lock().await.insert(state_key, None);
                    let _ = notify_tx_clone.try_send(());
                });
            }
        } else {
            // No more available worktrees, break out of loop to wait for next tick or completion
            break;
        }
    }

    Ok(())
}

/// Dispatch a single task to the appropriate executor and wait for it to complete.
/// Returns Err(()) if shutdown was requested during execution (issue was moved back to open).
async fn dispatch_task(
    repo: &RepoConfig,
    repo_path: &std::path::Path,
    task: &Task,
    work_db: &Arc<dyn WorkDb>,
    in_flight: &Arc<Mutex<HashSet<String>>>,
    wt_index: usize,
    wt_manager: &Arc<crate::worktree::WorktreeManager>,
) -> Result<(), ()> {
    let task_id = task.id.clone();

    let worktree_path = if repo.in_repo {
        repo_path.to_path_buf()
    } else {
        match wt_manager.worktree_path(wt_index) {
            Ok(p) => p,
            Err(e) => {
                error!(repo = %repo.name, error = %e, wt_index = wt_index, "failed to get worktree path");
                in_flight.lock().await.remove(&task_id);
                return Ok(());
            }
        }
    };

    if repo.in_repo {
        if let Err(e) = wt_manager.vcs().reset_worktree(repo_path, repo_path) {
            error!(repo = %repo.name, error = %e, "failed to reset main repository branch");
            // still proceed but log error
        }
    } else {
        if let Err(e) = wt_manager.reset_worktree(wt_index) {
            error!(repo = %repo.name, error = %e, wt_index = wt_index, "failed to reset worktree");
            // still proceed but log error
        }
    }

    if let Err(e) = work_db.set_in_progress(&task.id, repo_path) {
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
        wt_index = wt_index,
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
            if let Err(e) = work_db.set_open(&task.id, repo_path) {
                error!(task_id = %task.id, error = %e, "failed to move task back to open");
            }
            in_flight.lock().await.remove(&task_id);
            return Ok(());
        }
    };

    let comment_id_opt =
        match work_db.add_streaming_comment(&task.id, "Agent output:\n\n", repo_path) {
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

    let run = executor.execute(task, &worktree_path, repo, wt_manager.vcs(), output_tx);

    let result = tokio::select! {
        res = run => res,
        _ = shutdown_signal() => {
            info!(task_id = %task.id, "shutdown during task, moving issue back to open");
            if let Err(e) = work_db.set_open(&task.id, repo_path) {
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
            if let Err(e) = work_db.set_open(&task.id, repo_path) {
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
        if let Err(e) = work_db.update_comment(&task.id, comment_id, &body, repo_path) {
            error!(task_id = %task.id, error = %e, "failed to update streaming comment");
        }
    } else if let Some(ref body) = response {
        if !body.trim().is_empty() {
            if let Err(e) = work_db.add_comment(&task.id, body, repo_path) {
                error!(
                    task_id = %task.id,
                    error = %e,
                    "failed to add executor response as comment"
                );
            }
        }
    }

    if let Err(e) = work_db.set_complete(&task.id, repo_path) {
        error!(
            task_id = %task.id,
            error = %e,
            "failed to mark task as complete"
        );
    }

    in_flight.lock().await.remove(&task_id);
    Ok(())
}

async fn print_status_table(states: &SharedState) {
    let states = states.lock().await;
    if !states.is_empty() {
        use std::fmt::Write;
        let mut table = String::new();
        let _ = writeln!(
            &mut table,
            "\n{:<20} | {:<10} | Issue",
            "Repository", "Status"
        );
        let _ = writeln!(&mut table, "{:-<20}-+-{:-<10}-+-{:-<50}", "", "", "");

        let mut repos: Vec<_> = states.keys().cloned().collect();
        repos.sort();

        for repo_name in repos {
            let task_opt = states.get(&repo_name).unwrap();
            let trunc_repo = if repo_name.len() > 20 {
                format!("{}...", &repo_name[..17])
            } else {
                repo_name.clone()
            };

            match task_opt {
                Some(task) => {
                    let issue_str = format!("{} {}", task.id, task.title);
                    let trunc_issue = if issue_str.len() > 47 {
                        format!("{}...", &issue_str[..44])
                    } else {
                        issue_str
                    };
                    let _ = writeln!(
                        &mut table,
                        "{:<20} | {:<10} | {}",
                        trunc_repo, "Executing", trunc_issue
                    );
                }
                None => {
                    let _ = writeln!(&mut table, "{:<20} | {:<10} | -", trunc_repo, "Idle");
                }
            }
        }
        info!("{}", table);
    }
}
