# Synchronization Behavior

## Problem

Worktrees must start each task from the latest upstream state. Without
explicit synchronization, the remote-tracking refs (`origin/main`, etc.)
grow stale and worktrees diverge from what has been merged upstream.

## Strategy: Fetch → Merge-Base → Reset

Each worktree reset follows a three-step sequence:

1. **Fetch** — `git fetch --quiet` updates remote-tracking refs from the
   default remote so that `origin/main` (or `@{u}`) reflects the true
   upstream state.

2. **Merge-Base** — `git merge-base HEAD <upstream>` computes the most
   recent common ancestor between the worktree's current HEAD and the
   upstream branch. The upstream ref is resolved with a fallback chain:
   `@{u}` → `origin/main` → `origin/master` → `HEAD`.

3. **Reset** — `git reset --hard <merge-base>` followed by `git clean -fd`
   drops all local modifications and moves the worktree to the computed
   merge-base commit.

```
 remote        ──A──B──C──D──E    (origin/main after fetch)
                          \
 worktree HEAD             F──G   (leftover from previous task)
                          │
                    merge-base = D

 after reset:  worktree HEAD = D
```

## Why Merge-Base Instead of Reset to origin/main Directly

Resetting to the merge-base (rather than directly to `origin/main`) is
intentional. The repo's local HEAD may be on a branch that has diverged
from the default branch. The merge-base gives us the latest commit that is
shared between the two, which is always safe to reset to — it avoids pulling
in local-only commits from the main repo's working branch while still
incorporating everything that has been merged upstream.

In practice, when the main repo's HEAD _is_ on the default branch (i.e.
tracking origin/main), the merge-base equals the latest fetched upstream
commit, and the worktree gets a full sync.

## When This Runs

`reset_worktree` is called by `dispatch_task` in the daemon immediately
before handing a worktree to an executor. This means every task starts from
a freshly fetched, clean state.

## Trade-Offs

- **Network dependency**: Every task dispatch triggers a fetch. If the remote
  is unreachable, the fetch fails and blocks the task. This is acceptable
  because the agent will push results upstream anyway — network access is
  already a hard requirement.
- **Fetch frequency**: With N workers, up to N fetches can run concurrently
  against the same remote. Git's lock mechanism handles this safely, though
  it could create minor contention. In practice, task dispatch is infrequent
  enough that this is unlikely to matter.
- **No rebase/merge of in-flight work**: Worktrees are fully reset between
  tasks. There is no attempt to carry forward work-in-progress. This is
  by design — each task should start clean.

## Design Discussions

A few different strategies were considered, before landing upon the current one. These include:

- attempting to minimize the remote repository as a the synchronization point (not a blocker but would be nice to not have as a hard requirement).
- x

### Pushing to the remote head dirctly
