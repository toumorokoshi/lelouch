use crate::vcs::Vcs;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Manages dynamic worktrees for a single repository.
pub struct WorktreeManager {
    repo_name: String,
    repo_base_path: PathBuf,
    max_workers: usize,
    vcs: Box<dyn Vcs>,
}

impl WorktreeManager {
    pub fn new(
        repo_name: String,
        repo_base_path: PathBuf,
        max_workers: usize,
        vcs: Box<dyn Vcs>,
    ) -> Self {
        Self {
            repo_name,
            repo_base_path,
            max_workers,
            vcs,
        }
    }

    /// Returns the directory where worktrees are stored for this repository
    pub fn worktrees_dir(&self) -> Result<PathBuf> {
        let proj_dirs = directories::ProjectDirs::from("", "", "lelouch")
            .context("could not determine app directory")?;
        let dir = proj_dirs.data_dir().join("worktrees");
        if !dir.exists() {
            std::fs::create_dir_all(&dir).with_context(|| {
                format!("failed to create worktrees array at {}", dir.display())
            })?;
        }
        Ok(dir)
    }

    /// Get the path for the nth worker's worktree.
    pub fn worktree_path(&self, index: usize) -> Result<PathBuf> {
        let dir = self.worktrees_dir()?;
        let name = format!("{}-{}", self.repo_name, index);
        Ok(dir.join(name))
    }

    /// Ensure all worktrees up to `max_workers` exist, and remove any excess ones.
    pub fn sync_worktrees(&self) -> Result<()> {
        for i in 0..self.max_workers {
            let p = self.worktree_path(i)?;
            if !p.exists() {
                self.vcs.create_worktree(&self.repo_base_path, &p)?;
            }
        }

        // Remove excess worktrees if max_workers was reduced.
        // We'll just check a reasonable bound to clean up.
        for i in self.max_workers..self.max_workers + 10 {
            let p = self.worktree_path(i)?;
            if p.exists() {
                // Ignore removal errors since maybe someone deleted the files directly
                let _ = self.vcs.remove_worktree(&self.repo_base_path, &p);
            }
        }

        Ok(())
    }
}
