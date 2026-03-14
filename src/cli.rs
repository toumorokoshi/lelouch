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
    Run,

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
