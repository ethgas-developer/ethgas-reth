#!/usr/bin/env bash
# One run of one node binary: a fresh `--dev` node fed by `bench_driver produce`, the RPC matrix
# (rpc-matrix.sh), then `eth_call` at `latest` and at `pending` with the node's CPU time read from
# /proc before and after. Stops the node and the producer at the end.
# Usage: run-node.sh <run dir> <node binary>
# WORKERS (default 16), ROW_SECONDS (default 10), CPU_SECONDS (default 20); DRIVER and PORT_BASE as
# rpc-matrix.sh.
set -u
R=$1; bin=$2
here=$(cd "$(dirname "$0")" && pwd)
drv=${DRIVER:-$here/../target/release/examples/bench_driver}
base=${PORT_BASE:-28500}
url=http://127.0.0.1:$((base + 45))
workers=${WORKERS:-16}
n=$(basename "$R")

"$here/devnode.sh" "$R" "$bin" > "$R.unit" 2>&1 || { cat "$R.unit"; exit 1; }
for _ in $(seq 1 60); do
  curl -s -m 1 -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' "$url" 2>/dev/null |
    grep -q result && break
  sleep 0.5
done
systemd-run --user --unit="producer-$n" --collect \
  -p StandardOutput="file:$R/producer.log" -p StandardError="file:$R/producer.log" \
  "$drv" produce --rpc "$url" --listen "127.0.0.1:$((base + 100))" --interval-ms 250 > /dev/null
# The producer deploys its token first; wait for the node's first pending snapshot, then settle.
for _ in $(seq 1 120); do
  h=$(curl -s -m 2 "http://127.0.0.1:$((base + 1001))" |
    awk '/^reth_reth_flashblocks_pending_snapshot_height / {print $2}')
  [ -n "$h" ] && [ "$h" != 0 ] && break
  sleep 1
done
sleep 10

pid=$(systemctl --user show -p MainPID --value "devnode-$n")
"$here/rpc-matrix.sh" "$R/rpc-matrix" "$pid" "$workers" "${ROW_SECONDS:-10}"

# CPU per call: the node's CPU time (utime + stime) over one run of `eth_call` at each tag.
for tag in latest pending; do
  t0=$(awk '{print $14 + $15}' "/proc/$pid/stat")
  "$drv" rpc --rpc "$url" --method call --tag "$tag" --workers "$workers" \
    --seconds "${CPU_SECONDS:-20}" >> "$R/call-cpu.txt" 2> /dev/null
  t1=$(awk '{print $14 + $15}' "/proc/$pid/stat")
  echo "$tag ticks=$((t1 - t0)) clk_tck=$(getconf CLK_TCK)" >> "$R/call-cpu-ticks.txt"
done

systemctl --user stop "producer-$n.service" "devnode-$n.service"
for _ in $(seq 1 30); do
  systemctl --user is-active --quiet "devnode-$n.service" || break
  sleep 1
done
