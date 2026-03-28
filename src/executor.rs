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
    /// The name of this executor (e.g. "gemini").
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Execute a task in the given repository directory.
    /// If output_tx is Some, stream stdout/stderr to it (then drop the sender when done).
    async fn execute(
        &self,
        task: &Task,
        repo_path: &Path,
        pre_prompt: Option<&str>,
        model: Option<&str>,
        output_tx: OutputTx,
    ) -> Result<ExecutionResponse>;
}

/// Construct the task prompt string, optionally prepending a pre-prompt.
pub fn build_prompt(task: &Task, pre_prompt: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(pre) = pre_prompt {
        if !pre.trim().is_empty() {
            parts.push(pre.trim().to_string());
        }
    }
    let mut task_prompt = format!("Work on issue {}: {}", task.id, task.title);
    if let Some(ref desc) = task.description {
        if !desc.is_empty() {
            task_prompt.push_str(&format!("\n\nDescription:\n{desc}"));
        }
    }
    parts.push(task_prompt);
    parts.join("\n\n")
}

/// Resolve an executor by name.
pub fn resolve_executor(name: &str) -> Result<Box<dyn Executor>> {
    match name {
        "gemini" => Ok(Box::new(crate::executors::gemini::GeminiExecutor::new())),
        "cursor-agent" => Ok(Box::new(
            crate::executors::cursor_agent::CursorAgentExecutor::new(),
        )),
        other => anyhow::bail!("unknown executor: {other}"),
    }
}
