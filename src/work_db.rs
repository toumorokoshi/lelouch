use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::Path;

/// A task retrieved from the work database.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub status: String,
    pub priority: u32,
    pub issue_type: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub defer_until: Option<DateTime<Utc>>,
}

/// Trait abstracting over different work database backends.
///
/// Implementations shell out to the appropriate CLI (e.g. `bd`) to
/// query and mutate issue state.
pub trait WorkDb: Send + Sync {
    /// Poll for tasks that are ready to be executed (not blocked, not deferred).
    fn poll_ready(&self, repo_path: &Path) -> Result<Vec<Task>>;

    /// Perform a full scan of all open tasks (used on startup for recovery).
    fn full_scan(&self, repo_path: &Path) -> Result<Vec<Task>>;

    /// Mark a task as in-progress before dispatching to an executor.
    fn set_in_progress(&self, task_id: &str, repo_path: &Path) -> Result<()>;

    /// Create a new deferred task.
    fn create_deferred(&self, title: &str, defer_until: &str, repo_path: &Path) -> Result<Task>;
}
