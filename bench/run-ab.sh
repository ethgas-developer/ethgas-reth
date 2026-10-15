#!/usr/bin/env bash
# Before and after in turn: run-node.sh for before, after, before, after; then the table.
# Usage: run-ab.sh <out dir> <before binary> <after binary>
set -u
out=$1; before=$2; after=$3
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
for r in before-1 after-1 before-2 after-2; do
  if [ "${r%-*}" = before ]; then bin=$before; else bin=$after; fi
  echo "$(date '+%F %T') start $r $(sha256sum "$bin" | cut -c1-16)" | tee -a "$out/run-ab.log"
  "$here/run-node.sh" "$out/$r" "$bin" > "$out/$r.out" 2>&1
  echo "$(date '+%F %T') end $r" | tee -a "$out/run-ab.log"
done
python3 -I "$here/summarize.py" "$out" | tee "$out/summary.md"
