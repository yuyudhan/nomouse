#!/usr/bin/env bash
# Renders packaging/homebrew/nomouse.rb.in into DIST/nomouse.rb with the version and the
# checksums of the archives in DIST.
#
# Usage: scripts/release/homebrew.sh VERSION DIST [--allow-missing]
#   --allow-missing leaves "missing" for archives not in DIST (a single-OS dry run); a real
#   release must have all three.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 VERSION DIST [--allow-missing]" >&2
    exit 2
fi
version="$1"
dist="$2"
allow_missing="${3:-}"
root="$(cd "$(dirname "$0")/../.." && pwd)"

formula="$(<"${root}/packaging/homebrew/nomouse.rb.in")"
formula="${formula//@VERSION@/${version}}"
for name in universal-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    sum_file="${dist}/nomouse-${name}.tar.gz.sha256"
    if [ -f "$sum_file" ]; then
        sum="$(cut -d ' ' -f 1 <"$sum_file")"
    elif [ "$allow_missing" = "--allow-missing" ]; then
        sum="missing"
    else
        echo "homebrew: ${sum_file} is missing" >&2
        exit 1
    fi
    key="$(printf '%s' "$name" | tr '[:lower:]-' '[:upper:]_')"
    formula="${formula//@SHA256_${key}@/${sum}}"
done
printf '%s\n' "$formula" >"${dist}/nomouse.rb"
echo "homebrew: ${dist}/nomouse.rb" >&2
