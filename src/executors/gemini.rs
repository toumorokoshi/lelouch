use crate::executor::{ExecutionResponse, Executor, OutputTx};
use crate::work_db::Task;
use anyhow::Result;
use std::path::Path;
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

        let mut args = vec![
            "gemini".to_string(),
            "--prompt".to_string(),
            prompt,
            "--yolo".to_string(),
        ];
        if let Some(m) = &repo.model {
            args.push("-m".to_string());
            args.push(m.clone());
        }

        let (accumulated, status) = crate::executor::run_container(
            "gemini",
            Some(".gemini"),
            task,
            worktree_path,
            repo,
            output_tx,
            args,
        )
        .await?;

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
