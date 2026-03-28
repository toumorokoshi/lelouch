use anyhow::Result;
use std::path::Path;

pub mod git;

pub trait Vcs: Send + Sync {
    /// Creates a persistent worktree at `worktree_path` from the repository at `repo_path`.
    fn create_worktree(&self, repo_path: &Path, worktree_path: &Path) -> Result<()>;
    /// Removes the worktree at `worktree_path` from the repository at `repo_path`.
    fn remove_worktree(&self, repo_path: &Path, worktree_path: &Path) -> Result<()>;
}
