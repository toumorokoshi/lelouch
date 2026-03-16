use crate::executor::{ExecutionResponse, Executor, OutputTx};
use crate::work_db::Task;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
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
impl Executor for CursorAgentExecutor {
    fn name(&self) -> &str {
        "cursor-agent"
    }

    async fn execute(
        &self,
        task: &Task,
        repo_path: &Path,
        pre_prompt: Option<&str>,
        output_tx: OutputTx,
    ) -> Result<ExecutionResponse> {
        let prompt = Self::build_prompt(task, pre_prompt);

        info!(
            task_id = task.id,
            executor = "cursor-agent",
            repo = %repo_path.display(),
            "dispatching task to cursor-agent"
        );

        let mut child = Command::new("agent")
            .arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--workspace")
            .arg(repo_path)
            .arg("--force")
            .arg(&prompt)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("failed to spawn agent (Cursor Agent CLI)")?;

        let stdout_handle = child.stdout.take().context("missing stdout")?;
        let stderr_handle = child.stderr.take().context("missing stderr")?;
        let tx_stdout = output_tx.clone();
        let tx_stderr = output_tx.clone();

        let read_stdout = async {
            let mut out = String::new();
            let mut reader = BufReader::new(stdout_handle).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                out.push_str(&line);
                out.push('\n');
                if let Some(ref tx) = tx_stdout {
                    let _ = tx.send(line).await;
                }
            }
            out
        };
        let read_stderr = async {
            let mut out = String::new();
            let mut reader = BufReader::new(stderr_handle);
            let mut line = String::new();
            while reader.read_line(&mut line).await.is_ok() && !line.is_empty() {
                out.push_str("[stderr] ");
                out.push_str(&line);
                if let Some(ref tx) = tx_stderr {
                    let _ = tx.send(format!("[stderr] {}", line.trim_end())).await;
                }
                line.clear();
            }
            out
        };

        let (stdout_acc, stderr_acc) = tokio::join!(read_stdout, read_stderr);
        let mut accumulated = stdout_acc;
        accumulated.push_str(&stderr_acc);

        let status = child.wait().await.context("waiting for agent")?;
        drop(output_tx);

        if !status.success() {
            error!(
                task_id = task.id,
                exit_code = ?status.code(),
                stderr = %accumulated.trim(),
                "cursor-agent failed"
            );
            anyhow::bail!("cursor-agent failed for task {} (exit {})", task.id, status);
        }

        let response = Self::parse_json_result(accumulated.trim())?;
        info!(task_id = task.id, "cursor-agent completed successfully");
        Ok(response)
    }
}
