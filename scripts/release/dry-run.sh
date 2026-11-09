#!/usr/bin/env bash
# Builds this machine's Linux release assets locally into target/dist/ exactly as the release
# workflow does, without committing, tagging, pushing or publishing anything: the Linux archive
# with its .sha256, the .deb, install.sh, install.ps1, nomouse.rb (macOS checksum left
# "missing") and SHA256SUMS. The other architecture (the settings window links the system's
# GTK, so there is no cross build), macOS and Windows come from the release workflow
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
target="$(rustc -vV | sed -n 's/^host: //p')"
binary="$(scripts/release/build.sh "$target")"
scripts/release/package.sh "$target" "$binary" "$dist"
scripts/release/deb.sh "$target" "$dist"
scripts/release/finish.sh "$version" "$dist" --allow-missing

echo "dry-run: nomouse ${version} assets in ${dist}:" >&2
ls -l "$dist" >&2
