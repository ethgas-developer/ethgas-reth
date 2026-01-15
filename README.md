RUST_LOG=info,ethgas_flashblocks=debug ./target/release/ethgas-node node --websocket-url ws://localhost:1111 --full --chain hoodi --engine.persistence-threshold 0 --engine.memory-block-buffer-target 0       --authrpc.addr 0.0.0.0       --authrpc.port 8551       --http       --http.api "eth,net,web3,flashbots"

