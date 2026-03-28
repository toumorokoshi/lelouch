# Daemon Status Table

## Overview
Lelouch supports orchestrating multiple AI agents through different repositories and their associated workers. To improve observability, administrators and developers need a quick glance at the current status of all running workers.

This document describes the design implementation to periodically report in `stdout` what each repository's worker is currently executing.

## Requirements
* Monitor and output the real-time execution condition of every initialized repository worker.
* Print a single consolidated table of all active repositories.
* Do not spam `stdout` unnecessarily; the table should appear when a worker's state actively changes (e.g. going from "Idle" to "Executing" or changing the task it's working on).
* Display truncated issue summaries to prevent word wrapping or destroying table formatting.

## Architecture

### Shared State Management
The workers in Lelouch execute as separate Tokio asynchronous tasks. To enable the main daemon loop to observe them:
1. We introduce a single source of truth for worker state tracking: `Arc<Mutex<HashMap<String, Option<Task>>>>`. 
2. The `String` key identifies the repository name. 
3. The value `Option<Task>` records whether a task is active (`Some(task)`) or the worker is resting (`None`).

### State Transition Notifications
To prevent excessive polling or locking, we utilize an unbounded or bounded `tokio::sync::mpsc::channel`. 
Every time `run_worker` updates its entry in the `HashMap`, it will send a unit `()` on the channel.

### Daemon Output Loop
The main `Daemon::run()` select loop will add a branch handling the notification channel `rx.recv()`. When an event occurs, it locks the shared map, clears formatting if needed, and prints an ASCII-rendered table:

```
+----------------------+-----------+--------------------------------------------------+
| Repository           | Status    | Issue                                            |
+----------------------+-----------+--------------------------------------------------+
| lelouch              | Executing | #123 Implement status table                      |
| another_repo         | Idle      | -                                                |
+----------------------+-----------+--------------------------------------------------+
```

### Edge Cases
1. Config reload: Removed repositories should be properly culled from the shared state hash map, and added repositories should be initialized to `None`. This naturally fires a state change notification.
2. Graceful Shutdown: The status loop breaks concurrently with `shutdown_signal()`.
