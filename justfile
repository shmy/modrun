pre_commit:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo clippy --all-targets --no-default-features -- -D warnings
    cargo test --all-targets --all-features
    cargo test --all-targets --no-default-features
    cargo test --doc --all-features
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
    cargo machete