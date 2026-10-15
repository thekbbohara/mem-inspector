#!/usr/bin/env python3
"""Per-process elapsed time and Linux peak RSS using wait4, without GNU time.
Usage: python3 scripts/measure.py target/release/examples/benchmark scan|hash ROOT
Generation and compilation are intentionally outside this measurement.
"""
import os
import sys
import time

if len(sys.argv) != 4 or sys.argv[2] not in ("scan", "hash"):
    raise SystemExit("usage: measure.py BENCHMARK_BINARY scan|hash ROOT")
start = time.monotonic_ns()
pid = os.fork()
if pid == 0:
    try:
        os.execv(sys.argv[1], sys.argv[1:])
    except OSError as error:
        print(error, file=sys.stderr, flush=True)
        os._exit(127)
_, status, usage = os.wait4(pid, 0)
print(f"wall_ms={(time.monotonic_ns()-start)/1_000_000:.3f} peak_rss_kib={usage.ru_maxrss} user_s={usage.ru_utime:.6f} system_s={usage.ru_stime:.6f}", flush=True)
raise SystemExit(os.waitstatus_to_exitcode(status))
