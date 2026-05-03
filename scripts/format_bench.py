#!/usr/bin/env python3
"""Parse Criterion bench_output.txt and write a formatted Markdown table
to bench_table.txt, grouped by category with performance-tier icons.

Usage (from repo root):
    python3 scripts/format_bench.py

Input:  bench_output.txt  (produced by `cargo bench … | tee bench_output.txt`)
Output: bench_table.txt   (injected into README by the CI workflow)
"""
from __future__ import annotations
import re
from collections import defaultdict

# ---------------------------------------------------------------------------
# 1. Parse Criterion output
# ---------------------------------------------------------------------------
results: list[tuple[str, str]] = []
name: str | None = None

with open("bench_output.txt") as f:
    for line in f:
        # Standalone benchmark name line (no leading whitespace)
        m = re.match(r'^([A-Za-z]\S+(?:/\S+)?)\s*$', line)
        if m:
            name = m.group(1)
            continue
        # time: [low  median  high]  — inline or indented
        m = re.search(r'time:\s+\[([^\]]+)\]', line)
        if m:
            if not name:
                nm = re.match(r'^([A-Za-z]\S+(?:/\S+)?)\s+time:', line)
                if nm:
                    name = nm.group(1)
            if name:
                parts = m.group(1).split()
                # Criterion parts: [low_val low_unit med_val med_unit high_val high_unit]
                median = f"{parts[2]}\u202f{parts[3]}" if len(parts) >= 4 else parts[0]
                results.append((name, median))
            name = None

# ---------------------------------------------------------------------------
# 2. Categories  (order defines display order)
# ---------------------------------------------------------------------------
CATEGORIES: list[tuple[str, list[str]]] = [
    ("\U0001f4dd Write", [
        "write_single", "write_batch", "write_size", "write_heavy",
        "memtable_flush", "sstable_flush", "key_updates", "delete_operations",
    ]),
    ("\U0001f4da Read", [
        "read_memtable", "read_sstable", "read_latency",
    ]),
    ("\U0001f50d Scan", [
        "scan_sequential", "full_scan", "scan_limit", "scan_pagination",
        "range_scan", "prefix_scan", "iteration_sorted",
    ]),
    ("\U0001f310 YCSB", ["ycsb_type"]),
    ("\u26a1 Mixed Workload", [
        "workload_balanced", "workload_read_heavy", "workload_write_heavy",
    ]),
    ("\U0001f3d7\ufe0f SSTable", ["sstable_layer", "many_sstables"]),
    ("\U0001f9f5 Bloom Filter", ["bloom_filter"]),
    ("\U0001f4be Cache", ["cache_thrash"]),
    ("\U0001f9f5 Concurrency", ["concurrent"]),
    ("\U0001f4a1 Memory", ["memory_pressure"]),
]


def categorise(bench_name: str) -> str:
    lower = bench_name.lower()
    for label, prefixes in CATEGORIES:
        if any(lower.startswith(p) for p in prefixes):
            return label
    return "\u2699\ufe0f Other"


# ---------------------------------------------------------------------------
# 3. Performance-tier icon  (based on median latency)
# ---------------------------------------------------------------------------
UNIT_NS: dict[str, int] = {
    "ns": 1,
    "\u00b5s": 1_000, "us": 1_000,
    "ms": 1_000_000,
    "s":  1_000_000_000,
}


def to_ns(median: str) -> float | None:
    m = re.match(r'([\d.]+)\s*(\S+)', median)
    if not m:
        return None
    val, unit = float(m.group(1)), m.group(2).strip()
    return val * UNIT_NS.get(unit, 1)


def tier_icon(median: str) -> str:
    ns = to_ns(median)
    if ns is None:
        return ""
    if ns < 10_000:       return "\U0001f7e2"  # green   < 10 us
    if ns < 1_000_000:    return "\U0001f7e1"  # yellow  < 1 ms
    if ns < 100_000_000:  return "\U0001f7e0"  # orange  < 100 ms
    return "\U0001f534"                         # red    >= 100 ms


# ---------------------------------------------------------------------------
# 4. Group & render
# ---------------------------------------------------------------------------
groups: dict[str, list[tuple[str, str]]] = defaultdict(list)
for bench_name, median in results:
    groups[categorise(bench_name)].append((bench_name, median))

lines: list[str] = []
cat_order = [label for label, _ in CATEGORIES] + ["\u2699\ufe0f Other"]

for cat in cat_order:
    if cat not in groups:
        continue
    lines.append("")
    lines.append(f"**{cat}**")
    lines.append("")
    lines.append("| Benchmark | Median | Perf |")
    lines.append("|-----------|:------:|:----:|")
    for bench_name, median in groups[cat]:
        lines.append(f"| `{bench_name}` | {median} | {tier_icon(median)} |")

with open("bench_table.txt", "w") as f:
    f.write("\n".join(lines))

print(f"Parsed {len(results)} benchmarks across {len(groups)} categories.")
