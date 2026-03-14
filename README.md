# lelouch

lelouch is a coding-focused orchestration system for agents.

## Problem Statement

Code agent frameworks and tools (Claude Code, Cursor CLI, Cursor, etc) are advancing rapidly, but still have their papercuts when it comes to more complex agent orchestration.

The goal of lelouch is to be a tool-agnostic framework, primarily focused on getting one or more agents running for a variety of use cases, including:

- enabling long-running tasks (e.g. starting a task, then waiting a few hours for something to finish, and coming back to it).

## Overall approach

lelouch runs as a daemon (or foregrounded process), that checks as new work needs to be done, at a particular time. Once a task reached the desired time, it will execute.

lelouch requires some form of local task management to pull from. I recommend [beads](https://github.com/steveyegge/beads).

## User Guide

lelouch is designed to work with multiple repositories at once. The global configuration is specified in

- `$HOME/.config/lelouch/config.toml`  for Linux
- `$HOME/Library/Application Support/lelouch/config.toml` for macOS

The config file lists each of the repositories it wants to work on, and the agents to run in each repository.

## Example config.toml

```toml
[[my-project]]
path = "~/git/my-project"

[[another-project]]
path = "~/git/another-project"
```