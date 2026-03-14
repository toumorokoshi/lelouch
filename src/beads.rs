use crate::work_db::{Task, WorkDb};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use tracing::{debug, info};

/// Work database backend that shells out to the `bd` CLI (beads).
pub struct BeadsDb;

impl BeadsDb {
    pub fn new() -> Self {
        Self
    }

    /// Run a `bd` command in the given repo directory and return stdout.
    fn run_bd(args: &[&str], repo_path: &Path) -> Result<String> {
        debug!(
            command = format!("bd {}", args.join(" ")),
            repo = %repo_path.display(),
            "running bd command"
        );

        let output = Command::new("bd")
            .args(args)
            .current_dir(repo_path)
            .output()
            .with_context(|| {
                format!(
                    "failed to execute bd {} in {}",
                    args.join(" "),
                    repo_path.display()
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "bd {} failed (exit {}): {}",
                args.join(" "),
                output.status,
                stderr.trim()
            );
        }

        let stdout = String::from_utf8(output.stdout).context("bd output was not valid UTF-8")?;
        Ok(stdout)
    }

    /// Parse JSON array output from bd into a Vec<Task>.
    fn parse_tasks(json: &str) -> Result<Vec<Task>> {
        // bd --json returns `[]` for empty results
        let tasks: Vec<Task> =
            serde_json::from_str(json).context("failed to parse bd JSON output")?;
        Ok(tasks)
    }
}

impl WorkDb for BeadsDb {
    fn poll_ready(&self, repo_path: &Path) -> Result<Vec<Task>> {
        let output = Self::run_bd(&["ready", "--json", "--limit", "0"], repo_path)?;
        let tasks = Self::parse_tasks(&output)?;
        info!(
            repo = %repo_path.display(),
            count = tasks.len(),
            "polled ready tasks"
        );
        Ok(tasks)
    }

    fn full_scan(&self, repo_path: &Path) -> Result<Vec<Task>> {
        let output = Self::run_bd(
            &["list", "--json", "--status", "open", "--limit", "0"],
            repo_path,
        )?;
        let tasks = Self::parse_tasks(&output)?;
        info!(
            repo = %repo_path.display(),
            count = tasks.len(),
            "full scan of open tasks"
        );
        Ok(tasks)
    }

    fn set_in_progress(&self, task_id: &str, repo_path: &Path) -> Result<()> {
        Self::run_bd(&["set-state", task_id, "in_progress"], repo_path)?;
        info!(task_id, "marked task as in_progress");
        Ok(())
    }

    fn create_deferred(&self, title: &str, defer_until: &str, repo_path: &Path) -> Result<Task> {
        let output = Self::run_bd(
            &[
                "create",
                "--title",
                title,
                "--type",
                "task",
                "--defer",
                defer_until,
                "--json",
                "--silent",
            ],
            repo_path,
        )?;
        // bd create --json --silent returns a single object, not an array
        let task: Task =
            serde_json::from_str(&output).context("failed to parse bd create output")?;
        info!(
            task_id = task.id,
            title, defer_until, "created deferred task"
        );
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_tasks() {
        let tasks = BeadsDb::parse_tasks("[]").unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_task_json() {
        let json = r#"[
  {
    "id": "lelouch-abc",
    "title": "Test task",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "test@example.com",
    "created_at": "2026-03-14T05:52:27Z",
    "created_by": "Test User",
    "updated_at": "2026-03-14T05:52:27Z",
    "defer_until": "2026-03-14T06:52:27Z"
  }
]"#;
        let tasks = BeadsDb::parse_tasks(json).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "lelouch-abc");
        assert_eq!(tasks[0].title, "Test task");
        assert!(tasks[0].defer_until.is_some());
    }
}
