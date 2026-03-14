use crate::executor::{ExecutionResponse, Executor};
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

#[derive(serde::Deserialize)]
struct AgentJsonOutput {
    #[serde(default)]
    result: Option<String>,
}

impl CursorAgentExecutor {
    pub fn new() -> Self {
        Self
    }

    fn parse_json_result(stdout: &str) -> Result<ExecutionResponse> {
        let s = stdout.trim();
        let parse_one = |line: &str| -> Result<ExecutionResponse> {
            let out: AgentJsonOutput = serde_json::from_str(line).context("invalid agent JSON")?;
            Ok(out.result.filter(|r| !r.trim().is_empty()))
        };
        if let Ok(r) = parse_one(s) {
            return Ok(r);
        }
        let last_line = s.lines().rev().find(|l| !l.trim().is_empty());
        if let Some(line) = last_line {
            if let Ok(r) = parse_one(line) {
                return Ok(r);
            }
        }
        anyhow::bail!("no valid result object in agent JSON output")
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

    async fn execute(&self, task: &Task, repo_path: &Path) -> Result<ExecutionResponse> {
        let prompt = Self::build_prompt(task);

        info!(
            task_id = task.id,
            executor = "cursor-agent",
            repo = %repo_path.display(),
            "dispatching task to cursor-agent"
        );

        let output = Command::new("agent")
            .arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--workspace")
            .arg(repo_path)
            .arg("--force")
            .arg(&prompt)
            .output()
            .await
            .context("failed to spawn agent (Cursor Agent CLI)")?;

        if !output.status.success() {
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

        let stdout = String::from_utf8(output.stdout).context("agent output was not valid UTF-8")?;
        let response = Self::parse_json_result(stdout.trim())?;
        info!(task_id = task.id, "cursor-agent completed successfully");
        Ok(response)
    }
}
