# RPC latency: before and after a change

These scripts measure the RPC response time of two `ethgas-node` binaries, one after the other,
on one machine: a "before" build and an "after" build that differ by one change. They were written
for `perf: read the pending bundle in place`, and they work for any change on the RPC read path.

Each run starts a fresh `--dev` node, feeds it flashblocks from `bench_driver produce`, and loads
it with `bench_driver rpc` (the driver is
[`crates/execution/flashblocks-node/examples/bench_driver.rs`](../crates/execution/flashblocks-node/examples/bench_driver.rs)):

- **7 methods at `latest` and at `pending`**: `eth_call` (an ERC-20 `balanceOf`), `eth_getBalance`,
  `eth_getTransactionCount`, `eth_estimateGas` and `eth_simulateV1` (an ERC-20 `transfer`),
  `eth_getLogs` (the token's logs in one block) and `eth_getBlockByNumber(tag, false)`. 16 workers,
  10 s per row. The driver prints p50, p90, p99, p99.9, the maximum and the requests per second.
- **The CPU per call**: `eth_call` at both tags for 20 s more, with the node's CPU time read from
  `/proc` before and after.

`run-ab.sh` runs before, after, before, after; `summarize.py` prints the table.

| File | What it does |
|---|---|
| `run-ab.sh` | the four runs, then the table |
| `run-node.sh` | one run of one binary: the node, the producer, the matrix, the CPU per call |
| `devnode.sh` | a fresh `--dev` node in its own systemd user unit |
| `rpc-matrix.sh` | the 7 methods at both tags against one node, with the node's CPU and memory sampled |
| `procsample.py` | the sampler: CPU, RSS and threads of one process, once a second |
| `summarize.py` | the before/after table |

## Requirements

- Linux with a systemd user session (`systemd-run --user`), `python3` and `curl`.
- Free ports: `PORT_BASE` (default 28500) plus 3, 45, 46, 51, 100 and 1001.
- A quiet machine. The node, the producer and the driver share it; other load moves the numbers.

## 1. Build both sides the same way

Give "before" and "after" the same profile, features and `RUSTFLAGS`: a rebuild of the same source
with other settings can move a number by itself (about 7 % on `eth_getLogs` was seen). With the
Docker image's flags (`Dockerfile`):

```bash
git worktree add ../ethgas-reth-before <the commit before the change>
for dir in ../ethgas-reth-before .; do
  (cd "$dir" && RUSTFLAGS="-C target-cpu=x86-64-v3 -C target-feature=+pclmulqdq" \
    cargo build -j 8 --profile maxperf -p ethgas-node --bin ethgas-node --locked)
done
mkdir -p ../bench-bins
cp ../ethgas-reth-before/target/maxperf/ethgas-node ../bench-bins/ethgas-node-before
cp target/maxperf/ethgas-node ../bench-bins/ethgas-node-after
```

Keep copies outside `target/`: a later build overwrites the binary at the same path.

## 2. Build the driver

```bash
cargo build -j 8 --release -p ethgas-flashblocks-node --example bench_driver
```

Both sides use this one driver binary (`target/release/examples/bench_driver`; another one through
`DRIVER`).

## 3. Run

```bash
bench/run-ab.sh <out dir> ../bench-bins/ethgas-node-before ../bench-bins/ethgas-node-after
```

It takes about 17 minutes. Each run leaves its files in `<out dir>/<run>/`: the node's logs, the
producer's log, `rpc-matrix/` (`results.txt`, `first-answers.txt`, `node-proc.csv`),
`call-cpu.txt` and `call-cpu-ticks.txt`. The table goes to `<out dir>/summary.md`.

Settings, from the environment: `WORKERS` (16), `ROW_SECONDS` (10), `CPU_SECONDS` (20),
`PORT_BASE` (28500), `DRIVER`, and `NODE_ENV` for the node's environment (a build with
`jemalloc-prof` needs `NODE_ENV="_RJEM_MALLOC_CONF=prof_active:false"`).

## 4. Read the result

- **Compare before with after at the same tag.** The two runs of one binary agree within a few µs
  at p50; if they do not, something else ran on the machine: run again.
- **`latest` against `pending` is fair here only for the state reads.** The producer's
  transactions never reach the pool, so the canonical blocks are empty: at `latest`,
  `eth_getLogs` finds no logs and `eth_getBlockByNumber` no transactions.
- **`eth_getBalance` at `pending` always reads the state here.** The producer sends no balances in
  its flashblocks. The real builder sends the balance of every account that the pending block
  touched, and the node answers from that map without a state read.
- **The producer's feed is 8 flashblocks a block** (127 transactions, then 25 in each flashblock),
  on 4 s blocks. The real builder at a 200 ms interval sends about 56 a block on 12 s slots.

## Reference results

`perf: read the pending bundle in place`, before (`4e732ec`) and after, both built with
`--profile profiling --features jemalloc-prof` and `-C target-cpu=native`, on an AMD Ryzen 9
9950X3D (16 cores, 32 threads), with the defaults above. At `pending`, p50 in µs, the mean of
the two runs of each binary:

| Method | before | after | p99 before → after |
|---|---|---|---|
| `eth_call` | 230 | 124 | 394 → 228 |
| `eth_getBalance` | 198 | 98 | 361 → 196 |
| `eth_getTransactionCount` | 198 | 90 | 360 → 190 |
| `eth_estimateGas` | 250 | 134 | 410 → 240 |
| `eth_simulateV1` | 310 | 184 | 487 → 309 |
| `eth_getLogs` (the change does not touch it) | 452 | 458 | 852 → 874 |
| `eth_getBlockByNumber(false)` (the change does not touch it) | 156 | 158 | 331 → 339 |

At `latest`, every method changed by −1.2 to +1.4 %. CPU per `pending` `eth_call`: 326 → 84 µs
(`latest`: 76 µs).

A rerun of the same two binaries with these scripts, while a devnet loaded the same machine, gave
the same changes at `pending` within 0.6 points (−41.0 to −54.9 % at p50), absolute numbers about
10 % higher (`eth_call` 252 → 135 µs), and 332 → 88 µs of CPU per `pending` `eth_call`. The two
runs of each binary agreed within 5 µs at p50.
