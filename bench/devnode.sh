#!/usr/bin/env bash
# Starts ethgas-node on a fresh `--dev` chain (20 funded dev accounts, 4 s blocks) in its own
# systemd user unit, `devnode-<run dir name>`, fed by the flashblocks of `bench_driver produce`.
# Ports from PORT_BASE (default 28500): HTTP +45, WS +46, authrpc +51, p2p +3, metrics +1001; the
# producer listens on +100. IPC is off.
# Usage: devnode.sh <run dir> <node binary> [more node arguments]
# NODE_ENV adds environment to the node, e.g. NODE_ENV="RUST_LOG=info".
set -eu
dir=$1; bin=$2; shift 2
base=${PORT_BASE:-28500}
mkdir -p "$dir"
rm -rf "${dir:?}/data"
sha256sum "$bin" > "$dir/binary.sha256"
unit="devnode-$(basename "$dir")"
envargs=()
for kv in ${NODE_ENV:-}; do envargs+=("--setenv=$kv"); done
args=(node --dev --dev.block-time 4s --datadir "$dir/data"
  --http --http.addr 127.0.0.1 --http.port $((base + 45)) --http.api eth,net,web3,debug
  --ws --ws.addr 127.0.0.1 --ws.port $((base + 46)) --ws.api eth,net,web3
  --authrpc.addr 127.0.0.1 --authrpc.port $((base + 51))
  --port $((base + 3)) --disable-discovery --ipcdisable
  --metrics 127.0.0.1:$((base + 1001))
  --flashblocks-url ws://127.0.0.1:$((base + 100))
  --log.file.directory "$dir/logs"
  "$@")
echo "$bin ${args[*]}" > "$dir/command.txt"
systemd-run --user --unit="$unit" --collect -p MemoryMax=16G "${envargs[@]}" \
  -p StandardOutput="file:$dir/node.log" -p StandardError="file:$dir/node.log" \
  "$bin" "${args[@]}" > /dev/null
echo "$unit"
