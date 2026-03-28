use super::Vcs;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

pub struct GitVcs;

impl Vcs for GitVcs {
    fn create_worktree(&self, repo_path: &Path, worktree_path: &Path) -> Result<()> {
        let status = Command::new("git")
            .current_dir(repo_path)
            .arg("worktree")
            .arg("add")
            .arg("-f") // Force creation if branch exists or attached
            .arg(worktree_path)
            .status()
            .context("failed to execute git worktree add")?;

        if !status.success() {
            anyhow::bail!("git worktree add failed with exit code: {}", status);
        }
        Ok(())
    }

    fn remove_worktree(&self, repo_path: &Path, worktree_path: &Path) -> Result<()> {
        let status = Command::new("git")
            .current_dir(repo_path)
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(worktree_path)
            .status()
            .context("failed to execute git worktree remove")?;

        if !status.success() {
            anyhow::bail!("git worktree remove failed with exit code: {}", status);
        }
        Ok(())
    }
}
