use crate::executor::{ExecutionResponse, Executor, OutputTx};
use crate::work_db::Task;
use anyhow::{Context, Result};
use std::path::Path;
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
        vcs: &dyn crate::vcs::Vcs,
        output_tx: OutputTx,
    ) -> Result<ExecutionResponse> {
        let prompt = crate::executor::build_prompt(task, repo.pre_prompt.as_deref());

        let mut args = vec![
            "agent".to_string(),
            "-p".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--workspace".to_string(),
            "/workspace".to_string(),
            "--force".to_string(),
            prompt,
        ];
        if let Some(m) = &repo.model {
            args.push("-m".to_string());
            args.push(m.clone());
        }

        let (accumulated, status) = crate::executor::run_container(
            "cursor-agent",
            Some(".cursor"),
            task,
            worktree_path,
            repo,
            vcs,
            output_tx,
            args,
        )
        .await?;

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
