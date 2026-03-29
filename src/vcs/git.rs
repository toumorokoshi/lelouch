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

    fn reset_worktree(&self, repo_path: &Path, worktree_path: &Path) -> Result<()> {
        let merge_base = Self::get_merge_base(repo_path)?;

        let reset_status = Command::new("git")
            .current_dir(worktree_path)
            .arg("reset")
            .arg("--hard")
            .arg(&merge_base)
            .status()
            .context("failed to execute git reset")?;

        if !reset_status.success() {
            anyhow::bail!("git reset failed with exit code: {}", reset_status);
        }

        let clean_status = Command::new("git")
            .current_dir(worktree_path)
            .arg("clean")
            .arg("-fdx")
            .status()
            .context("failed to execute git clean")?;

        if !clean_status.success() {
            anyhow::bail!("git clean failed with exit code: {}", clean_status);
        }

        Ok(())
    }
}

impl GitVcs {
    fn get_merge_base(repo_path: &Path) -> Result<String> {
        // Try @{u}
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["merge-base", "HEAD", "@{u}"])
            .output()?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        // Try origin/main
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["merge-base", "HEAD", "origin/main"])
            .output()?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        // Try origin/master
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["merge-base", "HEAD", "origin/master"])
            .output()?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        // Fallback to HEAD
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["rev-parse", "HEAD"])
            .output()?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}
