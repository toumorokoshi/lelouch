use crate::config::RepoConfig;
use crate::executor::resolve_executor;
use crate::work_db::{Task, WorkDb};
use anyhow::Result;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
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

/// The daemon manages the poll loop across all configured repositories.
/// At most one task runs at a time; each poll dispatches one ready task (if any) and waits for it.
pub struct Daemon {
    repos: Vec<RepoConfig>,
    work_db: Arc<dyn WorkDb>,
    /// When true, do not dispatch tasks (startup scan and poll only).
    dry_run: bool,
    /// Track tasks we've already dispatched (to avoid double-dispatch).
    in_flight: RefCell<HashSet<String>>,
}

impl Daemon {
    pub fn new(repos: Vec<RepoConfig>, work_db: Arc<dyn WorkDb>, dry_run: bool) -> Self {
        Self {
            repos,
            work_db,
            dry_run,
            in_flight: RefCell::new(HashSet::new()),
        }
    }

    /// Run the daemon: startup recovery scan, then poll loop.
    pub async fn run(&self) -> Result<()> {
        info!("lelouch daemon starting");

        // Startup: full scan of all repos for recovery
        self.startup_scan().await?;

        // Main poll loop
        self.poll_loop().await
    }

    /// On startup, read the entire database for each repo to recover
    /// any tasks that were enqueued but not yet processed.
    async fn startup_scan(&self) -> Result<()> {
        info!("performing startup recovery scan");
        for repo in &self.repos {
            let repo_path = repo.resolved_path()?;
            match self.work_db.full_scan(&repo_path) {
                Ok(tasks) => {
                    info!(
                        repo = repo.name,
                        open_tasks = tasks.len(),
                        "startup scan complete"
                    );
                }
                Err(e) => {
                    error!(
                        repo = repo.name,
                        error = %e,
                        "startup scan failed"
                    );
                }
            }
        }
        Ok(())
    }

    /// Poll each repo on its configured interval, dispatching ready tasks.
    async fn poll_loop(&self) -> Result<()> {
        // Create a ticker for each repo based on its poll interval.
        // For simplicity, we use the minimum interval and poll all repos.
        let min_interval = self
            .repos
            .iter()
            .map(|r| r.poll_interval_secs)
            .min()
            .unwrap_or(60);

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(min_interval));

        info!(
            poll_interval_secs = min_interval,
            repo_count = self.repos.len(),
            "entering poll loop"
        );

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if self.poll_all_repos().await.is_err() {
                        return Ok(());
                    }
                }
                _ = shutdown_signal() => {
                    info!("received shutdown signal, exiting");
                    return Ok(());
                }
            }
        }
    }

    /// Poll all repos; dispatch at most one ready task and wait for it to complete.
    /// Returns Err(()) if shutdown was requested during task execution (issue was moved back to open).
    async fn poll_all_repos(&self) -> Result<(), ()> {
        if !self.in_flight.borrow().is_empty() {
            return Ok(());
        }

        let mut candidate: Option<(RepoConfig, Task)> = None;
        for repo in &self.repos {
            let repo_path = match repo.resolved_path() {
                Ok(p) => p,
                Err(e) => {
                    error!(repo = repo.name, error = %e, "failed to resolve repo path");
                    continue;
                }
            };
            let tasks = match self.work_db.poll_ready(&repo_path) {
                Ok(t) => t,
                Err(e) => {
                    error!(repo = repo.name, error = %e, "error polling repo");
                    continue;
                }
            };
            let in_flight = self.in_flight.borrow();
            if let Some(task) = tasks.into_iter().find(|t| !in_flight.contains(&t.id)) {
                candidate = Some((repo.clone(), task));
                break;
            }
        }

        if let Some((repo, task)) = candidate {
            if self.dry_run {
                info!(
                    task_id = task.id,
                    title = task.title,
                    repo = repo.name,
                    "dry-run: would dispatch task"
                );
            } else if self.dispatch_task(&repo, &task).await.is_err() {
                return Err(());
            }
        }
        Ok(())
    }

    /// Dispatch a single task to the appropriate executor and wait for it to complete.
    /// Returns Err(()) if shutdown was requested during execution (issue was moved back to open).
    async fn dispatch_task(&self, repo: &RepoConfig, task: &Task) -> Result<(), ()> {
        let task_id = task.id.clone();

        self.in_flight.borrow_mut().insert(task_id.clone());

        let repo_path = match repo.resolved_path() {
            Ok(p) => p,
            Err(e) => {
                error!(repo = repo.name, error = %e, "failed to resolve repo path");
                self.in_flight.borrow_mut().remove(&task_id);
                return Ok(());
            }
        };

        if let Err(e) = self.work_db.set_in_progress(&task.id, &repo_path) {
            error!(
                task_id = task.id,
                error = %e,
                "failed to mark task as in_progress"
            );
        }

        info!(
            task_id = task.id,
            title = task.title,
            repo = repo.name,
            executor = repo.executor,
            "dispatching task"
        );

        let executor = match resolve_executor(&repo.executor) {
            Ok(e) => e,
            Err(e) => {
                error!(
                    executor = repo.executor,
                    error = %e,
                    "failed to resolve executor"
                );
                if let Err(e) = self.work_db.set_open(&task.id, &repo_path) {
                    error!(task_id = task.id, error = %e, "failed to move task back to open");
                }
                self.in_flight.borrow_mut().remove(&task_id);
                return Ok(());
            }
        };

        let pre_prompt = repo.pre_prompt.as_deref();
        let comment_id_opt =
            match self
                .work_db
                .add_streaming_comment(&task.id, "Agent output:\n\n", &repo_path)
            {
                Ok(id) => id,
                Err(e) => {
                    error!(task_id = task.id, error = %e, "failed to create streaming comment");
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
            let work_db = Arc::clone(&self.work_db);
            let task_id = task.id.clone();
            let comment_id = comment_id.clone();
            let repo_path_buf = repo_path.to_path_buf();
            tokio::spawn(run_streaming_updater(
                rx,
                work_db,
                task_id,
                comment_id,
                repo_path_buf,
                acc_tx,
            ));
        }
        let run = executor.execute(task, &repo_path, pre_prompt, output_tx);

        let result = tokio::select! {
            res = run => res,
            _ = shutdown_signal() => {
                info!(task_id = task.id, "shutdown during task, moving issue back to open");
                if let Err(e) = self.work_db.set_open(&task.id, &repo_path) {
                    error!(task_id = task.id, error = %e, "failed to move task back to open");
                }
                self.in_flight.borrow_mut().remove(&task_id);
                return Err(());
            }
        };

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                error!(task_id = task.id, error = %e, "task execution failed");
                if let Err(e) = self.work_db.set_open(&task.id, &repo_path) {
                    error!(
                        task_id = task.id,
                        error = %e,
                        "failed to move task back to open"
                    );
                }
                self.in_flight.borrow_mut().remove(&task_id);
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
            if let Err(e) = self
                .work_db
                .update_comment(&task.id, comment_id, &body, &repo_path)
            {
                error!(task_id = task.id, error = %e, "failed to update streaming comment");
            }
        } else if let Some(ref body) = response {
            if !body.trim().is_empty() {
                if let Err(e) = self.work_db.add_comment(&task.id, body, &repo_path) {
                    error!(
                        task_id = task.id,
                        error = %e,
                        "failed to add executor response as comment"
                    );
                }
            }
        }

        if let Err(e) = self.work_db.set_complete(&task.id, &repo_path) {
            error!(
                task_id = task.id,
                error = %e,
                "failed to mark task as complete"
            );
        }

        self.in_flight.borrow_mut().remove(&task_id);
        Ok(())
    }
}
