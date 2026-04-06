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
        Self::fetch_upstream(repo_path)?;
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
            .arg("-fd")
            .status()
            .context("failed to execute git clean")?;

        if !clean_status.success() {
            anyhow::bail!("git clean failed with exit code: {}", clean_status);
        }

        Ok(())
    }

    fn get_required_mounts(
        &self,
        repo_path: &Path,
    ) -> Result<Vec<(std::path::PathBuf, std::path::PathBuf, bool)>> {
        let mut mounts = Vec::new();

        let host_git_dir = repo_path.join(".git");
        if host_git_dir.exists() {
            mounts.push((host_git_dir.clone(), host_git_dir, false));
        }

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let home_dir = base_dirs.home_dir();

            let gitconfig_path = home_dir.join(".gitconfig");
            if gitconfig_path.exists() {
                mounts.push((
                    gitconfig_path,
                    std::path::PathBuf::from("/root/.gitconfig"),
                    true,
                ));
            }

            let config_git_path = home_dir.join(".config").join("git");
            if config_git_path.exists() {
                mounts.push((
                    config_git_path,
                    std::path::PathBuf::from("/root/.config/git"),
                    true,
                ));
            }
        }

        Ok(mounts)
    }
}

impl GitVcs {
    fn fetch_upstream(repo_path: &Path) -> Result<()> {
        let status = Command::new("git")
            .current_dir(repo_path)
            .args(["fetch", "--quiet"])
            .status()
            .context("failed to execute git fetch")?;

        if !status.success() {
            anyhow::bail!("git fetch failed with exit code: {}", status);
        }
        Ok(())
    }

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
