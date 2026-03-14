# Write Ahead Log

## Motivation

In some cases, the daemon may die before an agent task is complete. This may leave tasks in the work database (e.g. `bd`) in the `in_progress` state.

It would be valuable for lelouch to be able to identify which work is already in progress, and which daemons have started. When executing a task.