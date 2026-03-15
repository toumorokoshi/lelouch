use crate::executor::{ExecutionResponse, Executor};
use crate::work_db::Task;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;
use tracing::{error, info};

/// Executor that dispatches tasks to the antigravity agent.
///
/// Spawns `antigravity` as a subprocess with the task description
/// as a prompt, running in the repository directory.
pub struct AntigravityExecutor;

impl AntigravityExecutor {
    pub fn new() -> Self {
        Self
    }

    fn build_prompt(task: &Task, pre_prompt: Option<&str>) -> String {
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
}

#[async_trait::async_trait]
impl Executor for AntigravityExecutor {
    fn name(&self) -> &str {
        "antigravity"
    }

    async fn execute(
        &self,
        task: &Task,
        repo_path: &Path,
        pre_prompt: Option<&str>,
    ) -> Result<ExecutionResponse> {
        let prompt = Self::build_prompt(task, pre_prompt);

        info!(
            task_id = task.id,
            executor = "antigravity",
            repo = %repo_path.display(),
            "dispatching task to antigravity"
        );

        let output = Command::new("antigravity")
            .arg("--prompt")
            .arg(&prompt)
            .current_dir(repo_path)
            .output()
            .await
            .context("failed to spawn antigravity")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                task_id = task.id,
                exit_code = ?output.status.code(),
                stderr = %stderr.trim(),
                "antigravity failed"
            );
            anyhow::bail!(
                "antigravity failed for task {} (exit {})",
                task.id,
                output.status
            );
        }

        let stdout =
            String::from_utf8(output.stdout).context("antigravity output was not valid UTF-8")?;
        let response = if stdout.trim().is_empty() {
            None
        } else {
            Some(stdout)
        };
        info!(task_id = task.id, "antigravity completed successfully");
        Ok(response)
    }
}
