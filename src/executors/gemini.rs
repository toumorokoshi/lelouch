use crate::executor::{ExecutionResponse, Executor, OutputTx};
use crate::work_db::Task;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{error, info};

/// Executor that dispatches tasks to the gemini agent.
///
/// Spawns `gemini` as a subprocess with the task description
/// as a prompt, running in the repository directory.
pub struct GeminiExecutor;

impl GeminiExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Executor for GeminiExecutor {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn execute(
        &self,
        task: &Task,
        repo_path: &Path,
        pre_prompt: Option<&str>,
        model: Option<&str>,
        output_tx: OutputTx,
    ) -> Result<ExecutionResponse> {
        let prompt = crate::executor::build_prompt(task, pre_prompt);

        info!(
            task_id = task.id,
            executor = "gemini",
            repo = %repo_path.display(),
            "dispatching task to gemini"
        );

        let mut cmd = Command::new("gemini");
        cmd.arg("--prompt").arg(&prompt).arg("--yolo");
        if let Some(m) = model {
            cmd.arg("-m").arg(m);
        }

        let mut child = cmd
            .current_dir(repo_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("failed to spawn gemini")?;

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

        let status = child.wait().await.context("waiting for gemini")?;
        drop(output_tx);

        if !status.success() {
            error!(
                task_id = task.id,
                exit_code = ?status.code(),
                stderr = %accumulated.trim(),
                "gemini failed"
            );
            anyhow::bail!("gemini failed for task {} (exit {})", task.id, status);
        }

        let response = if accumulated.trim().is_empty() {
            None
        } else {
            Some(accumulated)
        };
        info!(task_id = task.id, "gemini completed successfully");
        Ok(response)
    }
}
