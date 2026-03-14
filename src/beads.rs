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

    /// Parse JSON output from bd into a Vec<Task>.
    /// Accepts: single JSON array; object with issues/items/data/results/tasks;
    /// or NDJSON (one JSON object per line). Skips leading non-JSON (e.g. log prefix).
    /// Treats the specific plain-text "no issues" message from bd as empty list.
    fn parse_tasks(json: &str) -> Result<Vec<Task>> {
        let json = json.trim();
        if json.is_empty() {
            return Ok(Vec::new());
        }
        if json.eq_ignore_ascii_case("no issues found.") {
            return Ok(Vec::new());
        }
        let json = Self::strip_json_preamble(json);
        if let Ok(tasks) = Self::parse_tasks_single(json) {
            return Ok(tasks);
        }
        let mut tasks = Vec::new();
        for line in json.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(t) = serde_json::from_str::<Task>(line) {
                tasks.push(t);
            } else if let Ok(ts) = Self::parse_tasks_single(line) {
                tasks.extend(ts);
            }
        }
        if tasks.is_empty() {
            anyhow::bail!("failed to parse bd JSON output (not array, object with issues/items/data/results/tasks, or NDJSON)");
        }
        Ok(tasks)
    }

    fn strip_json_preamble(s: &str) -> &str {
        let s = s.trim_start();
        if s.starts_with('[') || s.starts_with('{') {
            return s;
        }
        s.find('[')
            .or_else(|| s.find('{'))
            .map(|i| &s[i..])
            .unwrap_or(s)
    }

    fn parse_tasks_single(json: &str) -> Result<Vec<Task>> {
        let value: serde_json::Value =
            serde_json::from_str(json).context("failed to parse bd JSON output")?;
        let arr = match &value {
            serde_json::Value::Array(a) => a.clone(),
            serde_json::Value::Object(o) => o
                .get("issues")
                .or_else(|| o.get("items"))
                .or_else(|| o.get("data"))
                .or_else(|| o.get("results"))
                .or_else(|| o.get("tasks"))
                .and_then(|v| v.as_array())
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "bd JSON object has no 'issues'/'items'/'data'/'results'/'tasks' array"
                    )
                })?,
            _ => anyhow::bail!("bd JSON output is neither array nor object"),
        };
        serde_json::from_value(serde_json::Value::Array(arr))
            .context("failed to parse bd JSON array into tasks")
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
        Self::run_bd(&["update", task_id, "--status", "in_progress"], repo_path)?;
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

    fn add_comment(&self, task_id: &str, body: &str, repo_path: &Path) -> Result<()> {
        Self::run_bd(&["comments", "add", task_id, body], repo_path)?;
        info!(task_id, "added comment to issue");
        Ok(())
    }

    fn set_complete(&self, task_id: &str, repo_path: &Path) -> Result<()> {
        Self::run_bd(&["update", task_id, "--status", "closed"], repo_path)?;
        info!(task_id, "marked task as complete");
        Ok(())
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
    fn test_parse_no_issues_found_plain_text() {
        let tasks = BeadsDb::parse_tasks("No issues found.\n").unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_other_non_json_errors() {
        assert!(BeadsDb::parse_tasks("error: something went wrong").is_err());
        assert!(BeadsDb::parse_tasks("not valid json").is_err());
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

    #[test]
    fn test_parse_tasks_object_with_issues() {
        let json = r#"{"issues":[{"id":"x","title":"T","status":"open","priority":1,"issue_type":"task","created_at":"2026-03-14T05:52:27Z"}]}"#;
        let tasks = BeadsDb::parse_tasks(json).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "x");
        assert_eq!(tasks[0].title, "T");
    }

    #[test]
    fn test_parse_task_with_description() {
        let json = r#"[{"id":"a","title":"T","description":"Do the thing.","status":"open","priority":1,"issue_type":"task","created_at":"2026-03-14T05:52:27Z"}]"#;
        let tasks = BeadsDb::parse_tasks(json).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].description.as_deref(), Some("Do the thing."));
    }

    #[test]
    fn test_parse_task_with_body_alias() {
        let json = r#"[{"id":"b","title":"T","body":"Issue body text.","status":"open","priority":1,"issue_type":"task","created_at":"2026-03-14T05:52:27Z"}]"#;
        let tasks = BeadsDb::parse_tasks(json).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].description.as_deref(), Some("Issue body text."));
    }
}
