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

# A local Amsterdam chain with no consensus client: reth's dev miner seals a block every 2 s, and
# Amsterdam activates 60 s after launch. The genesis is stamped with the launch time, because the
# miner stamps blocks with the wall clock. Pass a flashblocks endpoint to follow a producer.
devnet flashblocks_url="":
  rm -rf target/devnet && mkdir -p target/devnet
  jq --argjson now "$(date +%s)" '.timestamp = $now | .config.amsterdamTime = $now + 60' devnet/amsterdam-genesis.json > target/devnet/genesis.json
  cargo run --bin ethgas-node -- node --dev --dev.block-time 2s --chain target/devnet/genesis.json --datadir target/devnet/data --http --http.api eth,net,web3 {{ if flashblocks_url != "" { "--flashblocks-url " + flashblocks_url } else { "" } }}
