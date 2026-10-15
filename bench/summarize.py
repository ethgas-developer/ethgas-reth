"""The before/after table of run-ab.sh.

Usage: python3 -I summarize.py <out dir>
For each method and tag: p50 and p99 of each run, the mean of the two runs of each binary, and the
change of the means. Then the CPU per `eth_call`: the node's CPU time over the run's requests.
"""

import re
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
runs = {"before": ["before-1", "before-2"], "after": ["after-1", "after-2"]}
line_re = re.compile(
    r"^(\S+) (latest|pending) workers=\d+ seconds=\d+ requests=(\d+) rps=(\d+) errors=(\d+) "
    r"p50_us=(\d+) p90_us=(\d+) p99_us=(\d+) p999_us=(\d+) max_us=(\d+)"
)


def parse(path):
    out = {}
    for line in path.read_text().splitlines():
        m = line_re.match(line)
        if m:
            out[(m[1], m[2])] = {
                "requests": int(m[3]), "rps": int(m[4]), "errors": int(m[5]),
                "p50": int(m[6]), "p99": int(m[8]),
            }
    return out


matrix = {r: parse(root / r / "rpc-matrix" / "results.txt") for rs in runs.values() for r in rs}
mean = lambda xs, f: statistics.mean(x[f] for x in xs)

print("| Method | tag | p50 before (runs) | p50 after (runs) | p50 change | p99 before | p99 after | requests/s before | requests/s after | errors |")
print("|---|---|---|---|---|---|---|---|---|---|")
for key in matrix["before-1"]:
    b = [matrix[r][key] for r in runs["before"]]
    a = [matrix[r][key] for r in runs["after"]]
    b50, a50 = mean(b, "p50"), mean(a, "p50")
    print(
        f"| `{key[0]}` | {key[1]} | {b50:.0f} ({b[0]['p50']}, {b[1]['p50']}) "
        f"| {a50:.0f} ({a[0]['p50']}, {a[1]['p50']}) | {100 * (a50 - b50) / b50:+.1f} % "
        f"| {mean(b, 'p99'):.0f} | {mean(a, 'p99'):.0f} | {mean(b, 'rps'):,.0f} | {mean(a, 'rps'):,.0f} "
        f"| {sum(x['errors'] for x in a + b)} |"
    )

print()
print("CPU per `eth_call` (µs): the node's CPU time over the run's requests")
print()
print("| run | tag | requests/s | p50 µs | CPU per call µs |")
print("|---|---|---|---|---|")
for r in runs["before"] + runs["after"]:
    calls = parse(root / r / "call-cpu.txt")
    ticks = {}
    for line in (root / r / "call-cpu-ticks.txt").read_text().splitlines():
        tag, t, hz = line.split()
        ticks[tag] = int(t.split("=")[1]) / int(hz.split("=")[1])
    for tag in ("latest", "pending"):
        row = calls[("eth_call", tag)]
        print(f"| {r} | {tag} | {row['rps']:,} | {row['p50']} | {1e6 * ticks[tag] / row['requests']:.1f} |")
