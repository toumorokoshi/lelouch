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
        worktree_path: &Path,
        repo: &crate::config::RepoConfig,
        output_tx: OutputTx,
    ) -> Result<ExecutionResponse> {
        let prompt = crate::executor::build_prompt(task, repo.pre_prompt.as_deref());

        info!(
            task_id = task.id,
            executor = "gemini",
            repo = %repo.name,
            worktree = %worktree_path.display(),
            "dispatching task to gemini via docker"
        );

        let dockerfile = repo.dockerfile.as_deref().unwrap_or("Dockerfile");
        let image_name = format!("lelouch-{}-gemini", repo.name.to_lowercase());

        let build_status = Command::new("docker")
            .arg("build")
            .arg("-t")
            .arg(&image_name)
            .arg("-f")
            .arg(worktree_path.join(dockerfile))
            .arg(worktree_path)
            .status()
            .await
            .context("failed to execute docker build")?;

        if !build_status.success() {
            anyhow::bail!("docker build failed with exit code: {}", build_status);
        }

        let mut cmd = Command::new("docker");
        cmd.arg("run").arg("--rm");

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let gemini_dir = base_dirs.home_dir().join(".gemini");
            cmd.arg("-v")
                .arg(format!("{}:/root/.gemini", gemini_dir.display()));
        }

        cmd.arg("-v")
            .arg(format!("{}:/workspace", worktree_path.display()));
        cmd.arg("-w").arg("/workspace");
        cmd.arg(&image_name);

        cmd.arg("gemini").arg("--prompt").arg(&prompt).arg("--yolo");
        if let Some(m) = &repo.model {
            cmd.arg("-m").arg(m);
        }

        let mut child = cmd
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
