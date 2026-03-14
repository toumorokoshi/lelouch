use crate::config::RepoConfig;
use crate::executor::resolve_executor;
use crate::work_db::{Task, WorkDb};
use anyhow::Result;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{error, info};

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
                    self.poll_all_repos().await;
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("received shutdown signal, exiting");
                    return Ok(());
                }
            }
        }
    }

    /// Poll all repos; dispatch at most one ready task and wait for it to complete.
    async fn poll_all_repos(&self) {
        if !self.in_flight.borrow().is_empty() {
            return;
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
            } else {
                self.dispatch_task(&repo, &task).await;
            }
        }
    }

    /// Dispatch a single task to the appropriate executor and wait for it to complete.
    async fn dispatch_task(&self, repo: &RepoConfig, task: &Task) {
        let task_id = task.id.clone();

        self.in_flight.borrow_mut().insert(task_id.clone());

        let repo_path = match repo.resolved_path() {
            Ok(p) => p,
            Err(e) => {
                error!(repo = repo.name, error = %e, "failed to resolve repo path");
                return;
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

        // Resolve executor and run
        let executor = match resolve_executor(&repo.executor) {
            Ok(e) => e,
            Err(e) => {
                error!(
                    executor = repo.executor,
                    error = %e,
                    "failed to resolve executor"
                );
                return;
            }
        };

        match executor.execute(task, &repo_path).await {
            Ok(response) => {
                info!(task_id = task.id, "task completed successfully");
                if let Some(ref body) = response {
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
            }
            Err(e) => {
                error!(
                    task_id = task.id,
                    error = %e,
                    "task execution failed"
                );
            }
        }

        self.in_flight.borrow_mut().remove(&task_id);
    }
}
