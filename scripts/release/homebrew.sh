#!/usr/bin/env bash
# Renders packaging/homebrew/nomouse.rb.in into DIST/nomouse.rb with the version and the
# checksums of the archives in DIST and, when DIST has the macOS disk image,
# packaging/homebrew/nomouse-app.rb.in into the cask DIST/nomouse-app.rb.
#
# Usage: scripts/release/homebrew.sh VERSION DIST [--allow-missing]
#   --allow-missing leaves "missing" for archives not in DIST (a single-OS dry run); a real
#   release must have all three. The cask is skipped while releases carry no disk image.
#
# Environment:
#   NOMOUSE_BUNDLE_ID         the app's bundle identifier (default com.yuyudhan.nomouse)
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

dmg_sum="${dist}/nomouse-universal-apple-darwin.dmg.sha256"
if [ -f "$dmg_sum" ]; then
    cask="$(<"${root}/packaging/homebrew/nomouse-app.rb.in")"
    cask="${cask//@VERSION@/${version}}"
    cask="${cask//@SHA256_DMG@/$(cut -d ' ' -f 1 <"$dmg_sum")}"
    cask="${cask//@BUNDLE_ID@/${NOMOUSE_BUNDLE_ID:-com.yuyudhan.nomouse}}"
    printf '%s\n' "$cask" >"${dist}/nomouse-app.rb"
    echo "homebrew: ${dist}/nomouse-app.rb" >&2
fi
