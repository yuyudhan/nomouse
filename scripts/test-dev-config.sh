#!/usr/bin/env bash
# End-to-end test of scripts/dev-config.sh in a throwaway HOME: installs
# config/dev.example.toml as ~/.config/nomouse-dev/config.toml (and under $XDG_CONFIG_HOME),
# keeps an existing file with exit 0 and a message, backs it up and replaces it with --force,
# never writes the release's ~/.config/nomouse, and refuses unknown arguments.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo/scripts/dev-config.sh"
example="$repo/config/dev.example.toml"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT

fail() {
    echo "test-dev-config: $*" >&2
    exit 1
}

run() {
    env -u XDG_CONFIG_HOME -u OS HOME="$sandbox/home" "$script" "$@"
}

mkdir -p "$sandbox/home"
target="$sandbox/home/.config/nomouse-dev/config.toml"

run >"$sandbox/first.log" || fail "the first run failed: $(cat "$sandbox/first.log")"
cmp -s "$example" "$target" || fail "config.toml was not installed"
[[ ! -e "$sandbox/home/.config/nomouse" ]] || fail "the release directory was written"

echo "# mine" >>"$target"
mine="$(cat "$target")"
run >"$sandbox/second.log" 2>&1 || fail "keeping an existing file exited $?: $(cat "$sandbox/second.log")"
[[ "$(cat "$target")" == "$mine" ]] || fail "an existing config.toml was overwritten"
grep -qF "dev-config: kept $target" "$sandbox/second.log" || fail "no 'kept' line: $(cat "$sandbox/second.log")"
if compgen -G "$target.bak-*" >/dev/null; then
    fail "a backup was made without --force"
fi

run --force >"$sandbox/forced.log" || fail "--force failed: $(cat "$sandbox/forced.log")"
cmp -s "$example" "$target" || fail "--force did not replace config.toml"
backups=("$target".bak-*)
[[ ${#backups[@]} -eq 1 && -f "${backups[0]}" ]] || fail "expected one backup, got: ${backups[*]}"
[[ "$(cat "${backups[0]}")" == "$mine" ]] || fail "the backup does not hold the replaced file"
[[ ! -e "$sandbox/home/.config/nomouse" ]] || fail "the release directory was written"

env -u OS XDG_CONFIG_HOME="$sandbox/xdg" HOME="$sandbox/home" "$script" >/dev/null
cmp -s "$example" "$sandbox/xdg/nomouse-dev/config.toml" || fail "XDG_CONFIG_HOME was not followed"

if run --frobnicate 2>/dev/null; then
    fail "an unknown argument was accepted"
fi
echo "test-dev-config: ok"
