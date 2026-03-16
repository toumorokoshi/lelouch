use crate::work_db::Task;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

/// Result of executor run: optional response text to post as a comment on the issue.
pub type ExecutionResponse = Option<String>;

/// When present, the executor streams stdout/stderr to this sender (e.g. line by line).
/// Dropping the sender signals end of stream.
pub type OutputTx = Option<mpsc::Sender<String>>;

/// Trait for executing tasks via a coding agent.
///
/// Each executor implementation handles spawning the agent process,
/// passing the task context, and reporting results. When output_tx is Some,
/// the executor streams process output to it; the daemon can update an issue comment periodically.
#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    /// The name of this executor (e.g. "antigravity").
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Execute a task in the given repository directory.
    /// If output_tx is Some, stream stdout/stderr to it (then drop the sender when done).
    async fn execute(
        &self,
        task: &Task,
        repo_path: &Path,
        pre_prompt: Option<&str>,
        output_tx: OutputTx,
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
