# Graceful Shutdown

Lelouch supports a two-phase shutdown triggered by `Ctrl-C` (SIGINT).

## Phase 1 — Graceful (first Ctrl-C)

- The daemon stops scheduling new work to workers.
- Workers that are currently idle stop immediately.
- Workers that are executing a task continue until the task completes.
- Once all in-flight tasks finish, the process exits cleanly.

## Phase 2 — Immediate (second Ctrl-C)

- All in-flight executor processes are terminated.
- Any issue that was being worked on is moved back to **open** status so it can be picked up again on the next run.
- The process exits immediately after cleanup.

## Sequence Diagram

```
User            Daemon              Worker            Executor
 |                |                   |                  |
 |-- Ctrl-C ----->|                   |                  |
 |                |-- stop polling -->|                  |
 |                |                   |-- (continues) -->|
 |                |                   |                  |
 |                |   ... task completes naturally ...   |
 |                |                   |<---- done -------|
 |                |<-- worker exits --|                  |
 |                |-- exit ---------->|                  |
 |                |                                      |
 |  (alternatively, if Ctrl-C pressed again)             |
 |                |                                      |
 |-- Ctrl-C ----->|                   |                  |
 |                |-- force stop ---->|-- kill --------->|
 |                |                   |-- set_open() --->|
 |                |<-- worker exits --|                  |
 |                |-- exit ---------->|                  |
```

## SIGTERM

A `SIGTERM` signal (on Unix systems) behaves identically to the first `Ctrl-C` — it triggers a graceful shutdown.
