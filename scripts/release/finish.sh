#!/usr/bin/env bash
# Completes a release asset directory: adds install.sh and install.ps1 with their .sha256,
# renders the Homebrew formula, writes SHA256SUMS over every asset and checks it.
#
# Usage: scripts/release/finish.sh VERSION DIST [--allow-missing]
#   --allow-missing is passed to homebrew.sh for single-OS dry runs.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 VERSION DIST [--allow-missing]" >&2
    exit 2
fi
version="$1"
dist="$2"
root="$(cd "$(dirname "$0")/../.." && pwd)"

cp "${root}/scripts/install.sh" "${root}/scripts/install.ps1" "$dist/"
"${root}/scripts/release/checksum.sh" "${dist}/install.sh" "${dist}/install.ps1"
"${root}/scripts/release/homebrew.sh" "$version" "$dist" ${3:+"$3"}
"${root}/scripts/release/checksum.sh" "${dist}/nomouse.rb"

(
    cd "$dist"
    rm -f SHA256SUMS
    find . -maxdepth 1 -type f -name '*.sha256' -print | sort | while read -r sum; do
        cat "$sum"
    done >SHA256SUMS
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum --check --quiet SHA256SUMS
    else
        shasum -a 256 --check --quiet SHA256SUMS
    fi
)
echo "finish: ${dist}/SHA256SUMS ($(wc -l <"${dist}/SHA256SUMS" | tr -d ' ') assets)" >&2
