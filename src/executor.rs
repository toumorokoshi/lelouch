use crate::work_db::Task;
use anyhow::Result;
use std::path::Path;

/// Trait for executing tasks via a coding agent.
///
/// Each executor implementation handles spawning the agent process,
/// passing the task context, and reporting results.
#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    /// The name of this executor (e.g. "antigravity").
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Execute a task in the given repository directory.
    async fn execute(&self, task: &Task, repo_path: &Path) -> Result<()>;
}

/// Resolve an executor by name.
pub fn resolve_executor(name: &str) -> Result<Box<dyn Executor>> {
    match name {
        "antigravity" => Ok(Box::new(
            crate::executors::antigravity::AntigravityExecutor::new(),
        )),
        other => anyhow::bail!("unknown executor: {other}"),
    }
}
