# Design

## Overall approach

## Integration into frameworks

Lelouch is intended to be flexible to multiple work database systems, as well as agent execution interfaces (e.g. Claude Code, antigravity, or Cursor CLI).

## Implementation Details

### Command line interface

lelouch provides it's own command-line interface, providing a uniform abstraction if desired for workflow management. It supports the following:

- Adding a task to the queue for a given repository, with a given timestamp to pick up the work.
  - For `bd` (beads) natively, this is implemented by using the `bd` issue's `--defer` capability. `lelouch` relies on the database's native state to hide deferred work.
  - When the timestamp expires, `bd` inherently transitions the task back to an open/actionable state, enabling `lelouch` to easily query for ready work (e.g., via `bd ready`) without maintaining a separate "sleeping" queue.

This generally follows the recommendations in https://justin.poehnelt.com/posts/rewrite-your-cli-for-ai-agents/.

### Work Database Support

- Includes native support for beads via the `bd` cli.

### Executor Support

- Supports `antigravity`.

### Polling new tasks

Polling for new tasks is done incrementally, with checkpointing on when it polled last if the work database supports that type of iteration.

### Startup

When lelouch starts, it reads the whole target database (e.g. beads) for all tasks enqueued, for each repository: this ensures that even the proces dies, it will be able to recover by launching again.

### Language

Rust in the chosen development language.

