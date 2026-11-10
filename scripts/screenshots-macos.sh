#!/usr/bin/env bash
# Regenerates the documentation's pictures of the menu bar menu and every settings page, light
# and dark, into docs/media/macos/. nomouse.app draws its own windows into PNG files
# (`nomouse screenshots`), so no Screen Recording permission is needed; the windows it opens
# close again before it exits.
#
# Usage: scripts/screenshots-macos.sh [--bin <nomouse binary>]
# The pictures use a scratch home with an example config, so they show
# ~/.config/nomouse/config.toml and a couple of ignored apps, never this Mac's own settings.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin=""
while [ $# -gt 0 ]; do
    case "$1" in
    --bin)
        bin="$2"
        shift 2
        ;;
    *)
        echo "usage: $0 [--bin <nomouse binary>]" >&2
        exit 2
        ;;
    esac
done

if [ -z "$bin" ]; then
    # The app bundle, so the menu offers Start at login as installed copies do. The release
    # flavour even when the shell exports NOMOUSE_FLAVOUR=dev: app.sh names a dev binary's
    # bundle "nomouse Dev.app", and the pictures show the release hotkeys.
    (cd "$root" && NOMOUSE_FLAVOUR=release cargo build --release -p nomouse)
    "$root/scripts/release/app.sh" "$root/target/release/nomouse" "$root/target/app" >/dev/null
    bin="$root/target/app/nomouse.app/Contents/MacOS/nomouse"
fi

home="$(mktemp -d)"
trap 'rm -rf "$home"' EXIT
mkdir -p "$home/.config/nomouse"
cat >"$home/.config/nomouse/config.toml" <<'EOF'
[ignore]
apps = ["com.apple.Terminal", "com.mitchellh.ghostty"]

[macos]
enhanced_ui_apps = ["com.google.Chrome"]
EOF

out="$root/docs/media/macos"
rm -rf "$out"
env -u XDG_CONFIG_HOME HOME="$home" "$bin" screenshots "$out"
