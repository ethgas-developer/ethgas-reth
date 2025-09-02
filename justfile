fmt:
  cargo +nightly fmt

clippy:
  cargo clippy --all-features --no-deps -- -D warnings