fmt:
  cargo +nightly fmt

clippy:
  cargo clippy --all-features --no-deps -- -D warnings
maxperf:
  RUSTFLAGS="-C target-cpu=native" cargo build --profile maxperf --bin ethgas-node

maxperf-symbols:
  RUSTFLAGS="-C target-cpu=native" cargo build --profile maxperf-symbols --bin ethgas-node
  
profiling:
  RUSTFLAGS="-C target-cpu=native" cargo build --profile profiling --bin ethgas-node
