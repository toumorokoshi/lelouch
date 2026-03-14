use crate::executor::Executor;
use crate::work_db::Task;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;
use tracing::{error, info};

/// Executor that dispatches tasks to the Cursor Agent CLI.
///
/// Spawns `agent` (Cursor Agent) with the task as prompt, non-interactive
/// and with workspace set to the repository directory.
pub struct CursorAgentExecutor;

impl CursorAgentExecutor {
    pub fn new() -> Self {
        Self
    }

    fn build_prompt(task: &Task) -> String {
        let mut prompt = format!("Work on issue {}: {}", task.id, task.title);
        if let Some(ref desc) = task.description {
            if !desc.is_empty() {
                prompt.push_str(&format!("\n\nDescription:\n{desc}"));
            }
        }
        prompt
    }
}

#[async_trait::async_trait]
impl Executor for CursorAgentExecutor {
    fn name(&self) -> &str {
        "cursor-agent"
    }

    async fn execute(&self, task: &Task, repo_path: &Path) -> Result<()> {
        let prompt = Self::build_prompt(task);

        info!(
            task_id = task.id,
            executor = "cursor-agent",
            repo = %repo_path.display(),
            "dispatching task to cursor-agent"
        );

        let output = Command::new("agent")
            .arg("-p")
            .arg("--workspace")
            .arg(repo_path)
            .arg("--force")
            .arg(&prompt)
            .output()
            .await
            .context("failed to spawn agent (Cursor Agent CLI)")?;

        if output.status.success() {
            info!(task_id = task.id, "cursor-agent completed successfully");
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                task_id = task.id,
                exit_code = ?output.status.code(),
                stderr = %stderr.trim(),
                "cursor-agent failed"
            );
            anyhow::bail!(
                "cursor-agent failed for task {} (exit {})",
                task.id,
                output.status
            );
        }

        Ok(())
    }
}
