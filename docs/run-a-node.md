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

The node takes every reth `--engine.*` flag, as `reth node` does.

## Flashblocks flags

| Flag | Description | Default |
|---|---|---|
| `--flashblocks-url <URL>` | WebSocket endpoint streaming flashblocks. Alias `--websocket-url`. Setting it enables the feature | _disabled_ |
| `--max-pending-blocks-depth <N>` | How many blocks the canonical tip can move past the first block of the pre-confirmed state before the node drops that state | `3` |
| `--flashblocks.ping-interval <DUR>` | Ping interval, and also the pong deadline. A dead feed is detected after at most two intervals. Requires `--flashblocks-url` | `2s` |
| `--flashblocks.fee-max-age <DUR>` | How long the builder's inclusion fee is served after its flashblock arrived, and at most until its block is sealed, by `ethgas_inclusionPriorityFee`, `eth_maxPriorityFeePerGas`, `eth_gasPrice` and `eth_fillTransaction`. Requires `--flashblocks-url` | `15s` |
| `--flashblocks.fee-ceiling <GWEI>` | The highest builder inclusion fee served, in gwei; a higher one gives way to the node's own suggestion. Requires `--flashblocks-url` | `1000` |

**When the feed drops**, the node tries to connect again, at most 2 s apart, and serves flashblocks
again from the next block that the producer starts after it is connected. Until then, once the
block in progress is sealed, `pending` resolves as it does
[without flashblock data](./rpc-overview.md#pending-on-this-node).

**After a restart**, the node uses no flashblock until its consensus client makes a new block
canonical.

**On a public endpoint**, size reth's `--rpc.max-connections` (500 by default) for clients that
open connections and read nothing. The node ends such a client's flashblocks subscriptions 10 s
after its unread messages fill the connection's network buffers, but those messages stay in the
connection until it closes: a few MB for a connection whose subscriptions all sent their largest
messages. reth sends no WebSocket
ping, so put the endpoint behind a proxy that closes idle connections.

## Flashblocks endpoints

Point `--flashblocks-url` at the endpoint for the network you are running.

| Network | Endpoint |
|---|---|
| Mainnet | `wss://mainnet.flashblocks.ethgas.com/ws` |
| Hoodi | `wss://hoodi.flashblocks.ethgas.com/ws` |

> **These are node infrastructure, not application endpoints.** Your users should query your RPC.
> Do not point an application at the stream directly, and do not expose it to them.

