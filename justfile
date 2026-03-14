# Run all checks that CI runs
test:
    cargo check
    cargo test
    cargo fmt --all -- --check
    cargo clippy -- -D warnings
