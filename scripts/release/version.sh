#!/usr/bin/env bash
# Prints the release version: `version` in the root Cargo.toml [workspace.package], the single
# source of truth every crate inherits.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(awk '
    /^\[/ { section = $0 }
    section == "[workspace.package]" && $1 == "version" {
        gsub(/[" ]/, "", $3)
        print $3
        exit
    }
' "${root}/Cargo.toml")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "version: no MAJOR.MINOR.PATCH version in [workspace.package] (got '${version}')" >&2
    exit 1
}
echo "$version"
