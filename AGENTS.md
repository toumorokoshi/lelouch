# Agent instructions

When editing Rust code in this repo:

- **Run `cargo fmt --all` every time** after making changes (or before finishing a task). CI enforces formatting with `cargo fmt --all -- --check`.

## Committing code

- **unless** the prompt contains "don't commit", commit the code.
- **unless** the prompt contains "don't push", push the code.

- Use the conventional commit format for commit messages.
- The commit description must explain the problem first.
- The commit description must a summary of each area modified.
