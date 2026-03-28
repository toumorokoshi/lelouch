use clap::{Parser, Subcommand};

/// lelouch: a coding-focused orchestration system for agents.
#[derive(Parser)]
#[command(name = "lelouch", version, about)]
pub struct Cli {
    /// Enable verbose/debug logging.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Path to config file (overrides default location).
    #[arg(short, long, global = true)]
    pub config: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the daemon/polling loop (foreground).
    Run {
        /// Run without dispatching tasks (startup scan and poll only).
        #[arg(long)]
        dry_run: bool,
    },

    /// Initialize config and add a repository.
    Init {
        /// Path to the repository directory.
        #[arg(default_value = ".")]
        path: String,

        /// Executor to use for this repository (e.g. "gemini").
        #[arg(long)]
        executor: String,

        /// Optional name for the repository (defaults to directory name).
        #[arg(long)]
        name: Option<String>,

        /// Optional prompt fragment injected before the task prompt for the executor.
        #[arg(long)]
        pre_prompt: Option<String>,

        /// Optional model to use for the executor (e.g. "gpt-4").
        #[arg(long)]
        model: Option<String>,

        /// Maximum number of workers to use for this repository.
        #[arg(long)]
        max_workers: Option<usize>,

        /// Optional path to a custom Dockerfile.
        #[arg(long)]
        dockerfile: Option<String>,
    },

    /// Queue management commands.
    Queue {
        #[command(subcommand)]
        command: QueueCommands,
    },

    /// Show current status of watched repositories.
    Status,
}

#[derive(Subcommand)]
pub enum QueueCommands {
    /// Add a deferred task to a repository's work database.
    Add {
        /// Repository name (as defined in config.toml).
        #[arg(short, long)]
        repo: String,

        /// Task title.
        #[arg(short, long)]
        title: String,

        /// Defer until this time. Accepts any format bd supports:
        /// +6h, +1d, +2w, tomorrow, next monday, 2025-01-15, or ISO 8601.
        #[arg(short, long)]
        defer: Option<String>,
    },
}
