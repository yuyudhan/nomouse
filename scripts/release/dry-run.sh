#!/usr/bin/env bash
# Builds every Linux release asset locally into target/dist/ exactly as the release workflow
# does, without committing, tagging, pushing or publishing anything: both Linux archives with
# their .sha256, both .debs, install.sh, install.ps1, nomouse.rb (macOS checksum left
# "missing") and SHA256SUMS. The macOS and Windows assets come from the release workflow
# (`gh workflow run release.yml --ref nightly` builds them all without publishing).
#
# Usage: scripts/release/dry-run.sh [DIST]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
dist="${1:-${root}/target/dist}"
version="$(scripts/release/version.sh)"

rm -rf "$dist"
mkdir -p "$dist"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    binary="$(scripts/release/build.sh "$target")"
    scripts/release/package.sh "$target" "$binary" "$dist"
    scripts/release/deb.sh "$target" "$dist"
done
scripts/release/finish.sh "$version" "$dist" --allow-missing

echo "dry-run: nomouse ${version} assets in ${dist}:" >&2
ls -l "$dist" >&2
