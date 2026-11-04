#!/usr/bin/env python3
"""Update benchmark measurements without committing provenance-only changes."""

import argparse
from datetime import datetime, timezone
from pathlib import Path
import re

START = "<!-- BENCHMARK_RESULTS_START -->"
END = "<!-- BENCHMARK_RESULTS_END -->"


def update_report(report, table, run_url, commit, runner):
    text = report.read_text(encoding="utf-8")
    table = table.strip()
    if not re.search(r"^\| `[^`]+` \| [0-9]", table, re.MULTILINE):
        raise ValueError("refusing to publish a table without benchmark measurements")
    if START in table or END in table:
        raise ValueError("benchmark table contains report markers")
    if text.count(START) != 1 or text.count(END) != 1:
        raise ValueError("report must contain exactly one pair of benchmark markers")
    start = text.index(START)
    end = text.index(END)
    if end < start:
        raise ValueError("benchmark markers are out of order")

    previous = text[start + len(START):end].strip()
    # Provenance describes the last published measurements. Preserve it when
    # the table is identical, even if the new run has a different SHA or date.
    lines = previous.splitlines()
    while lines and (lines[0].startswith("> Auto-updated by CI ")
                     or lines[0].startswith("> Runner:") or not lines[0].strip()):
        lines.pop(0)
    if "\n".join(lines).strip() == table:
        return False

    timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    block = (f"{START}\n"
             f"> Auto-updated by CI on **{timestamp}** — [view run]({run_url})\n"
             f"> Runner: `{runner}` (GitHub-hosted). Commit: `{commit}`.\n\n"
             f"{table}\n\n")
    report.write_text(text[:start] + block + text[end:], encoding="utf-8")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--table", type=Path, required=True)
    parser.add_argument("--run-url", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--runner", required=True)
    args = parser.parse_args()
    try:
        changed = update_report(args.report, args.table.read_text(encoding="utf-8"),
                                args.run_url, args.commit, args.runner)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Cannot update benchmark report: {error}\n")
    print("Updated benchmark measurements." if changed else "Measurements unchanged; preserving report.")


if __name__ == "__main__":
    main()
