# ethgas-reth

A [reth](https://github.com/paradigmxyz/reth)-based Ethereum execution node with
[flashblocks](https://docs.flashbots.net/flashblocks) support.

## Build

```bash
cargo build --release
```

The binary is produced at `./target/release/ethgas-node`.

## Run

Minimal node (no flashblocks):

```bash
./target/release/ethgas-node node \
    --chain hoodi \
    --http \
    --http.api eth,net,web3
```

With flashblocks enabled:

```bash
RUST_LOG=info,ethgas_reth_flashblocks=debug \
./target/release/ethgas-node node \
    --chain hoodi \
    --full \
    --flashblocks-url ws://localhost:1111 \
    --max-pending-blocks-depth 3 \
    --engine.persistence-threshold 0 \
    --engine.memory-block-buffer-target 0 \
    --authrpc.addr 0.0.0.0 \
    --authrpc.port 8551 \
    --http \
    --http.api eth,net,web3
```

### Flashblocks flags

| Flag | Description | Default |
|---|---|---|
| `--flashblocks-url <URL>` | WebSocket endpoint streaming flashblock updates. Enables flashblocks when set. | _disabled_ |
| `--max-pending-blocks-depth <N>` | Max pending blocks to retain in memory. | `3` |
| `--flashblocks.cached-execution` | Enable cached execution via the flashblocks-aware engine validator. Requires `--flashblocks-url`. | `false` |

When `--flashblocks-url` is set, requests with the `pending` block tag are
served from flashblock-derived state, and the following extra subscriptions are
available via `eth_subscribe`:

- `newFlashblocks` — fires on every new flashblock with the current pending block
- `pendingLogs` — logs from the latest flashblock matching a filter
- `newFlashblockTransactions` — transactions from the latest flashblock; accepts
  `true` (full tx + logs + gas), a log filter (full tx where any log matches), or
  no param (hashes only)

## Tests

```bash
cargo test --workspace
```
