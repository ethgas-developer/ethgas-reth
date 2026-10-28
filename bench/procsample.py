"""Samples one process's CPU and memory from /proc once a second.

Usage: python3 -I procsample.py <pid> <seconds> <out.csv>
Columns: t_s, cpu_pct (of one core), rss_mib, hwm_mib, threads.
"""
import os
import sys
import time

pid, seconds, out = int(sys.argv[1]), float(sys.argv[2]), sys.argv[3]
tick = os.sysconf("SC_CLK_TCK")


def read():
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    cpu = (int(fields[11]) + int(fields[12])) / tick  # utime + stime
    status = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            key, _, value = line.partition(":")
            status[key] = value.strip()
    kib = lambda k: int(status.get(k, "0 kB").split()[0])
    return cpu, kib("VmRSS") / 1024, kib("VmHWM") / 1024, int(status.get("Threads", "0"))


start = time.monotonic()
cpu0, *_ = read()
prev_t, prev_cpu = start, cpu0
with open(out, "w") as f:
    f.write("t_s,cpu_pct,rss_mib,hwm_mib,threads\n")
    while time.monotonic() - start < seconds:
        time.sleep(1)
        try:
            cpu, rss, hwm, threads = read()
        except FileNotFoundError:
            break
        now = time.monotonic()
        f.write(f"{now - start:.1f},{100 * (cpu - prev_cpu) / (now - prev_t):.1f},{rss:.1f},{hwm:.1f},{threads}\n")
        f.flush()
        prev_t, prev_cpu = now, cpu
