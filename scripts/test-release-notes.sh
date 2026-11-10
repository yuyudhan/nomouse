#!/usr/bin/env bash
# Tests scripts/release/notes.sh against the newest released CHANGELOG.md section: the notes
# link docs/install.md at the release's own tag (vX.Y.Z), never at main, which holds only the
# README; and a version with no changelog section is refused.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo/scripts/release/notes.sh"

fail() {
    echo "test-release-notes: $*" >&2
    exit 1
}

version="$(sed -n 's/^## \[\([0-9][0-9.]*\)\].*/\1/p' "$repo/CHANGELOG.md" | sed -n 1p)"
[[ -n "$version" ]] || fail "CHANGELOG.md has no released section"

notes="$("$script" "$version")" || fail "notes.sh $version failed"
link="https://github.com/yuyudhan/nomouse/blob/v${version}/docs/install.md"
grep -qF "($link)" <<<"$notes" || fail "the notes do not link $link"
if grep -qF "/blob/main/" <<<"$notes"; then
    fail "the notes link main, which has no docs/install.md"
fi

if "$script" 0.0.0-none >/dev/null 2>&1; then
    fail "a version without a changelog section was accepted"
fi
echo "test-release-notes: ok ($version)"
