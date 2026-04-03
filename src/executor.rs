use crate::config::RepoConfig;
use crate::work_db::Task;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::info;

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
        worktree_path: &Path,
        repo: &RepoConfig,
        vcs: &dyn crate::vcs::Vcs,
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
        let desc_str: &str = desc;
        if !desc_str.is_empty() {
            task_prompt.push_str(&format!("\n\nDescription:\n{desc_str}"));
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

/// Helper to build a Docker image and run a container with the given arguments.
/// Mounts the worktree and an optional credential directory.
pub async fn run_container(
    executor_name: &str,
    credential_dir_name: Option<&str>,
    task: &Task,
    worktree_path: &Path,
    repo: &RepoConfig,
    vcs: &dyn crate::vcs::Vcs,
    output_tx: OutputTx,
    args: Vec<String>,
) -> Result<(String, std::process::ExitStatus)> {
    let image_name = &repo.docker_image_name;

    let mut cmd;

    if repo.no_sandbox || repo.in_repo {
        info!(
            task_id = task.id,
            executor = executor_name,
            repo = %repo.name,
            "spawning native process (no sandbox)"
        );
        cmd = Command::new(&args[0]);
        if args.len() > 1 {
            cmd.args(&args[1..]);
        }
        cmd.current_dir(worktree_path);
    } else {
        info!(
            task_id = task.id,
            executor = executor_name,
            repo = %repo.name,
            image = %image_name,
            "spawning docker container"
        );

        cmd = Command::new("docker");
        cmd.arg("run").arg("--rm").arg("-i");

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let home_dir = base_dirs.home_dir();

            if let Some(cred_dir) = credential_dir_name {
                let home_cred_dir = home_dir.join(cred_dir);
                cmd.arg("-v")
                    .arg(format!("{}:/root/{}", home_cred_dir.display(), cred_dir));
            }
        }

        let repo_path = repo
            .resolved_path()
            .context("failed to resolve repo path")?;

        for (host_path, container_path, read_only) in vcs.get_required_mounts(&repo_path)? {
            let ro = if read_only { ":ro" } else { "" };
            cmd.arg("-v").arg(format!(
                "{}:{}{}",
                host_path.display(),
                container_path.display(),
                ro
            ));
        }

        cmd.arg("-v").arg(format!(
            "{}:{}",
            worktree_path.display(),
            worktree_path.display()
        ));
        cmd.arg("-w").arg(worktree_path.display().to_string());
        cmd.arg(image_name);
        cmd.args(args);
    }

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context(format!("failed to spawn {}", executor_name))?;

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

    let status = child.wait().await.context("waiting for container")?;
    Ok((accumulated, status))
}
