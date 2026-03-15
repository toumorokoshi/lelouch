use crate::work_db::Task;
use anyhow::Result;
use std::path::Path;

/// Result of executor run: optional response text to post as a comment on the issue.
pub type ExecutionResponse = Option<String>;

/// Trait for executing tasks via a coding agent.
///
/// Each executor implementation handles spawning the agent process,
/// passing the task context, and reporting results. The returned response
/// (if any) is posted as a comment on the work-db issue.
#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    /// The name of this executor (e.g. "antigravity").
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Execute a task in the given repository directory.
    /// pre_prompt, when present, is prepended to the task prompt for the executor.
    /// Returns the response text to post as a comment on the issue, or None to skip.
    async fn execute(
        &self,
        task: &Task,
        repo_path: &Path,
        pre_prompt: Option<&str>,
    ) -> Result<ExecutionResponse>;
}

/// Resolve an executor by name.
pub fn resolve_executor(name: &str) -> Result<Box<dyn Executor>> {
    match name {
        "antigravity" => Ok(Box::new(
            crate::executors::antigravity::AntigravityExecutor::new(),
        )),
        "cursor-agent" => Ok(Box::new(
            crate::executors::cursor_agent::CursorAgentExecutor::new(),
        )),
        other => anyhow::bail!("unknown executor: {other}"),
    }
}
