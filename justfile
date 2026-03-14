# Run all checks that CI runs
test:
    cargo check
    cargo test
    cargo fmt --all -- --check
    cargo clippy -- -D warnings

# Fix formatting and apply automatic lint fixes
fix:
    cargo fmt --all
    # allow no vcs to enable jujutsu
    cargo fix --allow-dirty --allow-staged --allow-no-vcs
    cargo clippy --fix --allow-no-vcs --allow-dirty --allow-staged -- -D warnings
