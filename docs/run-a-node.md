# Run a Node

How to run an `ethgas-node` with flashblocks enabled, so it serves pre-confirmed state at the
`pending` tag.

Flashblocks are off by default. Without a flashblocks feed the node behaves as an ordinary Ethereum
execution client.

## Run it

```bash
git clone https://github.com/ethgas-developer/ethgas-reth.git
cd ethgas-reth
cargo build --release --bin ethgas-node

./target/release/ethgas-node node \
  --chain hoodi \
  --flashblocks-url wss://hoodi.flashblocks.ethgas.com/ws \
  --engine.persistence-threshold 0 \
  --engine.memory-block-buffer-target 0 \
  --http --http.api eth,net,web3 \
  --ws --ws.api eth,net,web3
```

## Run with Docker

Images are published to `ghcr.io/ethgas-developer/ethgas-reth`.

```bash
docker run -d --name ethgas-node \
  -p 8545:8545 -p 8546:8546 -p 30303:30303 -p 30303:30303/udp \
  -v ethgas-data:/data \
  ghcr.io/ethgas-developer/ethgas-reth:latest node \
  --chain hoodi \
  --datadir /data \
  --flashblocks-url wss://hoodi.flashblocks.ethgas.com/ws \
  --engine.persistence-threshold 0 \
  --engine.memory-block-buffer-target 0 \
  --http --http.addr 0.0.0.0 --http.api eth,net,web3 \
  --ws --ws.addr 0.0.0.0 --ws.api eth,net,web3
```

## Flashblocks flags

| Flag | Description | Default |
|---|---|---|
| `--flashblocks-url <URL>` | WebSocket endpoint streaming flashblocks. Alias `--websocket-url`. Setting it enables the feature | _disabled_ |
| `--max-pending-blocks-depth <N>` | Pending blocks held in memory | `3` |
| `--flashblocks.ping-interval <DUR>` | Ping interval, and also the pong deadline. A dead feed is detected after at most two intervals. Requires `--flashblocks-url` | `2s` |

## Flashblocks endpoints

Point `--flashblocks-url` at the endpoint for the network you are running.

| Network | Endpoint |
|---|---|
| Mainnet | `wss://mainnet.flashblocks.ethgas.com/ws` |
| Hoodi | `wss://hoodi.flashblocks.ethgas.com/ws` |

> **These are node infrastructure, not application endpoints.** Your users should query your RPC.
> Do not point an application at the stream directly, and do not expose it to them.

