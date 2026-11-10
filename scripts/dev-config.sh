#!/usr/bin/env bash
# Installs config/dev.example.toml (every option, at a development build's defaults) as the
# development build's config: $XDG_CONFIG_HOME/nomouse-dev/config.toml, else
# ~/.config/nomouse-dev/config.toml (%APPDATA%\nomouse-dev\config.toml on Windows). Never
# touches the installed nomouse's config (~/.config/nomouse/).
#
# Usage: scripts/dev-config.sh [--force]
#   An existing file is kept unless --force, which first copies it to
#   config.toml.bak-<UTC timestamp> next to it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
example="$root/config/dev.example.toml"

force=0
for arg in "$@"; do
    case "$arg" in
        --force) force=1 ;;
        -h | --help)
            sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "dev-config: unknown argument $arg (only --force)" >&2
            exit 2
            ;;
    esac
done

if [[ "${OS:-}" == "Windows_NT" ]]; then
    base="${APPDATA:?dev-config: APPDATA is not set}"
else
    base="${XDG_CONFIG_HOME:-}"
    if [[ -z "$base" ]]; then
        base="${HOME:?dev-config: HOME is not set}/.config"
    fi
fi
dir="$base/nomouse-dev"
target="$dir/config.toml"

if [[ -e "$target" || -L "$target" ]]; then
    if ((force == 0)); then
        echo "dev-config: $target exists; kept it (--force replaces it after a backup)" >&2
        exit 1
    fi
    backup="$target.bak-$(date -u +%Y%m%dT%H%M%SZ)"
    cp -p "$target" "$backup"
    echo "backed up $target to $backup"
fi

mkdir -p "$dir"
tmp="$(mktemp "$dir/.config.toml.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
cp "$example" "$tmp"
chmod 644 "$tmp"
mv -f "$tmp" "$target"
trap - EXIT
echo "installed $example as $target"
