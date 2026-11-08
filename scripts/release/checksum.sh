#!/usr/bin/env bash
# Writes FILE.sha256 beside each FILE, in `sha256sum` format ("<hash>  <name>"), so both the
# installers and `sha256sum -c` can check it.
#
# Usage: scripts/release/checksum.sh FILE...
set -euo pipefail

for file in "$@"; do
    dir="$(dirname "$file")"
    name="$(basename "$file")"
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$dir" && sha256sum "$name" >"${name}.sha256")
    else
        (cd "$dir" && shasum -a 256 "$name" >"${name}.sha256")
    fi
done
