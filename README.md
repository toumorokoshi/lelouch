# lelouch

lelouch is a coding-focused orchestration system for agents.

## Problem Statement

Code agent frameworks and tools (Claude Code, Cursor CLI, Cursor, etc) are advancing rapidly, but still have their papercuts when it comes to more complex agent orchestration.

The goal of lelouch is to be a tool-agnostic framework, primarily focused on getting one or more agents running for a variety of use cases, including:

- enabling long-running tasks (e.g. starting a task, then waiting a few hours for something to finish, and coming back to it).

## Overall approach

lelouch runs as a daemon (or foregrounded process), that checks as new work needs to be done, at a particular time. Once a task reached the desired time, it will execute it in the configured

lelouch requires some form of local task management to pull from. I recommend [beads](https://github.com/steveyegge/beads).

## User Guide

lelouch is designed to work with multiple repositories at once. The global configuration is specified in

- `$HOME/.config/lelouch/config.toml`  for Linux
- `$HOME/Library/Application Support/lelouch/config.toml` for macOS

The config file lists each of the repositories it wants to work on, and the agents to run in each repository.

## Example config.toml

```toml
[[repositories]]
name = "my-project"
path = "~/git/my-project"
executor = "antigravity"

[[repositories]]
name = "another-project"
path = "~/git/another-project"
executor = "cursor-agent"
pre_prompt = "Always write tests first. Prefer functional style."
```

Supported executors: `antigravity`, `cursor-agent` (Cursor Agent CLI; requires `agent` on PATH).

Optional per-repo settings:

- **`pre_prompt`** — Text injected before the task prompt when dispatching to the executor. Use this to give the agent consistent instructions (e.g. coding style or constraints) for all tasks in that repository.

## Building

```bash
cargo build --release
```

The binary will be at `target/release/lelouch`.

## CLI Usage

### Start the daemon

Run the polling loop in the foreground. Lelouch will continuously check each configured repository for ready tasks and dispatch them to the configured executor.

```bash
lelouch run
```

Use `-v` for verbose/debug logging:

```bash
lelouch -v run
```

### Add your repository to lelouch

Initialize and modify the global config, adding the directory, with:

```bash
lelouch init . --executor=antigravity   # or --executor=cursor-agent
```

To set a pre-prompt that will be injected before every task for this repo:

```bash
lelouch init . --executor=cursor-agent --pre-prompt "Always write tests first."
```

### Queue a deferred task

Add a task to a repository's work database, optionally deferred until a specific time:

```bash
# Create a task deferred by 2 hours
lelouch queue add --repo my-project --title "Migrate database" --defer "+2h"

# Create a task deferred until a specific date
lelouch queue add --repo my-project --title "Review logs" --defer "2026-04-01"

# Create a task with no deferral (immediately ready)
lelouch queue add --repo my-project --title "Fix typo"
```

The `--defer` flag accepts any format supported by `bd`: `+6h`, `+1d`, `+2w`, `tomorrow`, `next monday`, `2025-01-15`, or ISO 8601.

### Check status

Show configured repositories and the number of ready tasks in each:

```bash
lelouch status
```

### Custom config path

Override the default config file location:

```bash
lelouch --config /path/to/config.toml run
```