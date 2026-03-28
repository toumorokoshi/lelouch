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
}

#[async_trait::async_trait]
impl Executor for CursorAgentExecutor {
    fn name(&self) -> &str {
        "cursor-agent"
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
            executor = "cursor-agent",
            repo = %repo.name,
            worktree = %worktree_path.display(),
            "dispatching task to cursor-agent via docker"
        );

        let dockerfile = repo.dockerfile.as_deref().unwrap_or("Dockerfile");
        let image_name = format!("lelouch-{}-cursor-agent", repo.name.to_lowercase());

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
            let cursor_dir = base_dirs.home_dir().join(".cursor");
            cmd.arg("-v")
                .arg(format!("{}:/root/.cursor", cursor_dir.display()));
        }

        cmd.arg("-v")
            .arg(format!("{}:/workspace", worktree_path.display()));
        cmd.arg("-w").arg("/workspace");
        cmd.arg(&image_name);

        cmd.arg("agent");
        cmd.arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--workspace")
            .arg("/workspace")
            .arg("--force")
            .arg(&prompt);
        if let Some(m) = &repo.model {
            cmd.arg("-m").arg(m);
        }

        let mut child = cmd
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
