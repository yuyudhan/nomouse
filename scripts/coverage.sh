#!/usr/bin/env bash
# Line coverage (cargo-llvm-cov) of the platform-independent code, with an enforced floor.
#
# Runs the workspace tests under instrumentation, then reports line coverage for two groups:
#   core      crates/nomouse-core/src
#   cli-logic the binary's non-OS logic: crates/nomouse/src/{run,lock}.rs
# and fails when either drops below its floor in scripts/coverage-floors.txt. The floors are
# the measured value minus 2 points when they were last raised; raise them as coverage grows.
#
# Usage: scripts/coverage.sh [--html]   (--html also writes target/llvm-cov/html)
# Needs: cargo-llvm-cov and the llvm-tools-preview rustup component.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
html=0
while (($#)); do
    case "$1" in
        --html) html=1; shift ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done

cargo llvm-cov clean --workspace
cargo llvm-cov --no-report --workspace --all-features
mkdir -p target/llvm-cov
cargo llvm-cov report --json --summary-only --output-path target/llvm-cov/summary.json
if ((html)); then
    cargo llvm-cov report --html --output-dir target/llvm-cov
fi

python3 - "$root" target/llvm-cov/summary.json scripts/coverage-floors.txt <<'EOF'
import json
import sys

root, summary_path, floors_path = sys.argv[1:4]
groups = {
    "core": lambda path: path.startswith("crates/nomouse-core/src/"),
    "cli-logic": lambda path: path in ("crates/nomouse/src/run.rs", "crates/nomouse/src/lock.rs"),
}
floors = {}
with open(floors_path, encoding="utf-8") as floors_file:
    for line in floors_file:
        line = line.split("#", 1)[0].strip()
        if line:
            name, value = line.split()
            floors[name] = float(value)

with open(summary_path, encoding="utf-8") as summary_file:
    files = json.load(summary_file)["data"][0]["files"]
totals = {name: [0, 0] for name in groups}
for entry in files:
    path = entry["filename"].removeprefix(root + "/")
    for name, matches in groups.items():
        if matches(path):
            lines = entry["summary"]["lines"]
            totals[name][0] += lines["covered"]
            totals[name][1] += lines["count"]

failed = False
print(f"{'group':<10} {'lines':>13} {'coverage':>9} {'floor':>7}")
for name, (covered, count) in totals.items():
    if count == 0:
        print(f"{name:<10} no instrumented lines")
        failed = True
        continue
    percent = 100.0 * covered / count
    floor = floors.get(name)
    verdict = "" if floor is None or percent >= floor else "  BELOW FLOOR"
    floor_text = "-" if floor is None else f"{floor:.1f}"
    print(f"{name:<10} {covered:>6}/{count:<6} {percent:>8.2f}% {floor_text:>7}{verdict}")
    failed |= bool(verdict) or floor is None
sys.exit(1 if failed else 0)
EOF
