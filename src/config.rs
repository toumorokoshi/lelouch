use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level configuration for lelouch.
///
/// The config file maps project names to their repository settings.
/// Example config.toml:
/// ```toml
/// [[repositories]]
/// name = "my-project"
/// path = "~/git/my-project"
/// executor = "antigravity"
/// ```
#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub repositories: Vec<RepoConfig>,
}

/// Configuration for a single repository that lelouch manages.
#[derive(Debug, Deserialize, Clone)]
pub struct RepoConfig {
    /// Human-readable name for this repository.
    pub name: String,
    /// Path to the repository on disk. Supports `~` expansion.
    pub path: String,
    /// Which executor to use (e.g. "antigravity").
    pub executor: String,
    /// Polling interval in seconds (default: 60).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    60
}

impl RepoConfig {
    /// Resolve the repository path, expanding `~` to the home directory.
    pub fn resolved_path(&self) -> Result<PathBuf> {
        let path = if self.path.starts_with("~/") {
            let home = dirs_home().context("could not determine home directory")?;
            home.join(&self.path[2..])
        } else {
            PathBuf::from(&self.path)
        };
        Ok(path)
    }
}

/// Returns the platform-appropriate config file path.
pub fn config_path() -> Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("", "", "lelouch")
        .context("could not determine config directory")?;
    Ok(proj_dirs.config_dir().join("config.toml"))
}

/// Load configuration from the default config path.
pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    load_config_from(&path)
}

/// Load configuration from a specific path.
pub fn load_config_from(path: &Path) -> Result<Config> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let config: Config =
        toml::from_str(&contents).with_context(|| "failed to parse config file")?;
    Ok(config)
}

fn dirs_home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[[repositories]]
name = "my-project"
path = "~/git/my-project"
executor = "antigravity"

[[repositories]]
name = "other"
path = "/tmp/other"
executor = "antigravity"
poll_interval_secs = 120
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.repositories.len(), 2);
        assert_eq!(config.repositories[0].name, "my-project");
        assert_eq!(config.repositories[0].executor, "antigravity");
        assert_eq!(config.repositories[0].poll_interval_secs, 60);
        assert_eq!(config.repositories[1].poll_interval_secs, 120);
    }
}
