#!/usr/bin/env bash
# Runs `bench_driver rpc` for 7 methods at `latest` and at `pending` against one node, and samples
# the node's CPU and memory once a second meanwhile.
# Usage: rpc-matrix.sh <out dir> <node pid> [workers] [seconds per row]
# DRIVER: the driver binary (default target/release/examples/bench_driver); PORT_BASE as devnode.sh.
set -u
out=$1; pid=$2; workers=${3:-16}; seconds=${4:-10}
here=$(cd "$(dirname "$0")" && pwd)
drv=${DRIVER:-$here/../target/release/examples/bench_driver}
url=http://127.0.0.1:$((${PORT_BASE:-28500} + 45))
mkdir -p "$out"
{ date; uptime; echo "node pid $pid"; sha256sum "$drv"; } > "$out/context.txt"
python3 -I "$here/procsample.py" "$pid" $((14 * (seconds + 3) + 10)) "$out/node-proc.csv" &
for m in call balance nonce estimate simulate logs block; do
  for tag in latest pending; do
    "$drv" rpc --rpc "$url" --method "$m" --tag "$tag" --workers "$workers" --seconds "$seconds" \
      >> "$out/results.txt" 2>> "$out/first-answers.txt"
  done
done
wait
{ date; uptime; } >> "$out/context.txt"
