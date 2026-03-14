use crate::config::RepoConfig;
use crate::executor::resolve_executor;
use crate::work_db::{Task, WorkDb};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

/// The daemon manages the poll loop across all configured repositories.
pub struct Daemon {
    repos: Vec<RepoConfig>,
    work_db: Arc<dyn WorkDb>,
    /// Track tasks we've already dispatched (to avoid double-dispatch).
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl Daemon {
    pub fn new(repos: Vec<RepoConfig>, work_db: Arc<dyn WorkDb>) -> Self {
        Self {
            repos,
            work_db,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
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

    /// Poll all repos for ready tasks and dispatch them.
    async fn poll_all_repos(&self) {
        for repo in &self.repos {
            if let Err(e) = self.poll_repo(repo).await {
                error!(
                    repo = repo.name,
                    error = %e,
                    "error polling repo"
                );
            }
        }
    }

    /// Poll a single repo and dispatch any ready tasks.
    async fn poll_repo(&self, repo: &RepoConfig) -> Result<()> {
        let repo_path = repo.resolved_path()?;
        let tasks = self.work_db.poll_ready(&repo_path)?;

        for task in tasks {
            // Skip tasks already in flight
            {
                let in_flight = self.in_flight.lock().await;
                if in_flight.contains(&task.id) {
                    continue;
                }
            }

            self.dispatch_task(repo, &task).await;
        }

        Ok(())
    }

    /// Dispatch a single task to the appropriate executor.
    async fn dispatch_task(&self, repo: &RepoConfig, task: &Task) {
        let task_id = task.id.clone();

        // Mark as in-flight
        {
            let mut in_flight = self.in_flight.lock().await;
            in_flight.insert(task_id.clone());
        }

        info!(
            task_id = task.id,
            title = task.title,
            repo = repo.name,
            executor = repo.executor,
            "dispatching task"
        );

        // Mark in-progress in the work database
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
            Ok(()) => {
                info!(task_id = task.id, "task completed successfully");
            }
            Err(e) => {
                error!(
                    task_id = task.id,
                    error = %e,
                    "task execution failed"
                );
            }
        }

        // Remove from in-flight
        {
            let mut in_flight = self.in_flight.lock().await;
            in_flight.remove(&task_id);
        }
    }
}
