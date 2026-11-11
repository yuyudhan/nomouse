#!/usr/bin/env bash
# Prints the GitHub release notes for VERSION: the CHANGELOG.md section (without its heading),
# then how to install or update, with the one-line install from docs/install.md ("The one-line
# install", `curl -fsSL https://yuyudhan.github.io/nomouse/install | sh`), the same command as
# nomouse_core::update::INSTALL_COMMAND, which the macOS update window copies
# (crates/nomouse-core/src/update.rs tests that they match). On macOS it is the way to update: a
# browser quarantines a downloaded .dmg or .app.zip, the installer does not.
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
# A Markdown code fence, spelled with octal escapes so no backtick sits in this script.
fence="$(printf '\140\140\140')"
install="$(awk -v fence="$fence" '
    /^## The one-line install$/ { section = 1; next }
    section && /^## / { exit }
    section && $0 == fence "sh" { block = 1; next }
    block && $0 == fence { exit }
    block { print }
' "${root}/docs/install.md")"
if [ -z "$install" ] || [ "$(printf '%s\n' "$install" | wc -l)" -ne 1 ]; then
    echo "notes: docs/install.md has no one-line install under '## The one-line install'" >&2
    exit 1
fi
printf '%s\n' "$notes"
printf '\n## Install or update\n\n'
printf 'macOS 11+ and Linux (x86_64 and aarch64, glibc 2.35+), in a terminal, no sudo:\n\n'
printf '%s\n%s\n%s\n' "${fence}sh" "$install" "$fence"
cat <<'EOF'

On macOS, use it rather than the .dmg or .app.zip below: releases are not notarized yet, so a
browser marks the download as quarantined and macOS refuses to open it. If you did download one,
clear the flag once with "xattr -dr com.apple.quarantine /Applications/nomouse.app", or open it
and click **Open Anyway** in System Settings › Privacy & Security. After an update, switch
nomouse on again in the Accessibility pane. Windows, Homebrew and every other way:
EOF
# The release's own tag: main holds only the README, so a main link would 404.
printf '[docs/install.md](https://github.com/yuyudhan/nomouse/blob/v%s/docs/install.md).\n' "$version"
