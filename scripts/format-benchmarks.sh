#!/bin/bash
set -euo pipefail

# format-benchmarks.sh — Format benchmark results for README
# Usage: ./scripts/format-benchmarks.sh < raw-benchmarks.txt
#        ./scripts/format-benchmarks.sh /path/to/benchmark-output.txt
#
# Wraps the output between <!-- BENCHMARK_RESULTS_START --> and
# <!-- BENCHMARK_RESULTS_END --> markers.
# Returns 0 on success, non-zero on failure.

RAW_FILE="${1:-/dev/stdin}"

# Determine if input is readable
if [ ! -e "$RAW_FILE" ] && [ "$RAW_FILE" != "/dev/stdin" ]; then
  echo "Error: file not found: $RAW_FILE" >&2
  exit 1
fi

# Helper: produce a fallback markdown table from Criterion output
format_fallback() {
  local input="$1"
  echo "> 🤖 Auto-formatado por fallback — $(date -u '+%Y-%m-%d %H:%M UTC')"
  echo ""
  echo "| Benchmark | Mediana |"
  echo "|-----------|--------|"

  # Parse Criterion output — supports both inline and multi-line formats
  # Inline:   bench_name/param   time:   [low median high]
  # Multi-line:
  #   bench_name/param
  #                    time:   [low median high]
  perl -e '
    my $name = "";
    while (<>) {
      chomp;
      # Inline: "bench_name   time:   [low median high]"
      if (/^([a-zA-Z]\S+(?:\/\S+)?)\s+time:\s+\[([^\]]+)\]/) {
        $name = $1;
        my @vals = split(/\s+/, $2);
        my $median = "$vals[2] $vals[3]";
        $median =~ s/^\s+|\s+$//g;
        print "| `$name` | $median |\n";
        $name = "";
      }
      # Standalone name line (no leading whitespace)
      elsif (/^([a-zA-Z]\S+(?:\/\S+)?)\s*$/) {
        $name = $1;
      }
      # Timing on its own line after name was set
      elsif ($name && /^\s+time:\s+\[([^\]]+)\]/) {
        my @vals = split(/\s+/, $1);
        my $median = "$vals[2] $vals[3]";
        $median =~ s/^\s+|\s+$//g;
        print "| `$name` | $median |\n";
        $name = "";
      }
      # Also handle "test bench_name ... bench: 12345 ns/iter (+/- 123)" format
      elsif (/^test\s+(\S+)\s+.*bench:\s+(\d+)\s+ns\/iter/) {
        print "| `$1` | $2 ns |\n";
      }
    }
  ' "$input"
}

# ---- Main ----

# Try OpenCode first
if command -v opencode &>/dev/null; then
  echo "OpenCode found. Using AI formatting..." >&2
  # Pipe raw output through opencode run for AI formatting
  if FORMATTED=$(opencode run --prompt "Format the following benchmark results as a clean Markdown summary table with columns: Benchmark, Metric, Value. Use appropriate units and group related benchmarks together. Output ONLY the markdown." < "$RAW_FILE" 2>/dev/null); then
    echo "<!-- BENCHMARK_RESULTS_START -->"
    echo "> 🤖 Auto-formatted by OpenCode AI — $(date -u '+%Y-%m-%d %H:%M UTC')"
    echo ""
    echo "$FORMATTED"
    echo ""
    echo "<!-- BENCHMARK_RESULTS_END -->"
    exit 0
  else
    echo "OpenCode formatting failed (exit code $?), falling back..." >&2
  fi
else
  echo "OpenCode not found. Using fallback formatter." >&2
fi

# Fallback: produce a simple markdown table
echo "<!-- BENCHMARK_RESULTS_START -->"
format_fallback "$RAW_FILE"
echo ""
echo "<!-- BENCHMARK_RESULTS_END -->"
