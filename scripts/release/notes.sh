#!/usr/bin/env bash
# Prints the CHANGELOG.md section for VERSION (without its heading): the GitHub release notes.
#
# Usage: scripts/release/notes.sh VERSION
set -euo pipefail
version="${1:?usage: $0 VERSION}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
notes="$(awk -v heading="## [${version}]" '
    index($0, heading) == 1 { inside = 1; next }
    inside && /^## \[/ { exit }
    inside { print }
' "${root}/CHANGELOG.md")"
if [ -z "${notes//[[:space:]]/}" ]; then
    echo "notes: CHANGELOG.md has no entries under '## [${version}]'" >&2
    exit 1
fi
printf '%s\n' "$notes"
