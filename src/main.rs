mod beads;
mod cli;
mod config;
mod daemon;
mod executor;
mod executors;
mod work_db;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, QueueCommands};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        "lelouch=debug"
    } else {
        "lelouch=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .init();

    // Handle init before loading config (since config may not exist yet)
    if let Commands::Init {
        path,
        executor,
        name,
        pre_prompt,
        model,
    } = &cli.command
    {
        let abs_path = std::fs::canonicalize(path)
            .with_context(|| format!("could not resolve path: {path}"))?;
        let repo_name = name.clone().unwrap_or_else(|| {
            abs_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unnamed".to_string())
        });
        let path_str = abs_path.to_string_lossy().to_string();

        let cfg_path = config::add_repo(
            cli.config.as_deref(),
            &repo_name,
            &path_str,
            executor,
            pre_prompt.as_deref(),
            model.as_deref(),
        )?;
        println!("Added repository '{repo_name}' ({path_str})");
        println!("  executor: {executor}");
        if pre_prompt.is_some() {
            println!("  pre_prompt: set");
        }
        if let Some(m) = model {
            println!("  model: {m}");
        }
        println!("  config: {}", cfg_path.display());
        return Ok(());
    }

    // Load config
    let cfg = if let Some(ref path) = cli.config {
        config::load_config_from(std::path::Path::new(path))?
    } else {
        config::load_config().context(
            "failed to load config; run `lelouch init` first or pass --config. \
             See `lelouch --help` for config location.",
        )?
    };

    let work_db: Arc<dyn work_db::WorkDb> = Arc::new(beads::BeadsDb::new());

    match cli.command {
        Commands::Init { .. } => unreachable!(),
        Commands::Run { dry_run } => {
            let daemon = daemon::Daemon::new(cfg.repositories.clone(), work_db, dry_run);
            daemon.run().await?;
        }
        Commands::Queue { command } => match command {
            QueueCommands::Add { repo, title, defer } => {
                let repo_config = cfg
                    .repositories
                    .iter()
                    .find(|r| r.name == repo)
                    .with_context(|| {
                        format!(
                            "repository '{}' not found in config. Available: {}",
                            repo,
                            cfg.repositories
                                .iter()
                                .map(|r| r.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;

                let repo_path = repo_config.resolved_path()?;
                let defer_str = defer.as_deref().unwrap_or("+0s");

                let task = work_db.create_deferred(&title, defer_str, &repo_path)?;
                println!("Created task: {} ({})", task.id, task.title);
                if let Some(until) = task.defer_until {
                    println!("Deferred until: {until}");
                }
            }
        },
        Commands::Status => {
            println!("Lelouch — Configured Repositories:\n");
            for repo in &cfg.repositories {
                let repo_path = repo.resolved_path()?;
                let ready = work_db.poll_ready(&repo_path);
                let ready_count = ready.map(|t| t.len()).unwrap_or(0);
                println!(
                    "  {} ({})\n    executor: {}\n    ready tasks: {}\n",
                    repo.name, repo.path, repo.executor, ready_count
                );
            }
        }
    }

    Ok(())
}
