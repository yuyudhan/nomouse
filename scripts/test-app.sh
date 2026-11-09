#!/usr/bin/env bash
# Tests scripts/release/app.sh: builds nomouse.app twice from one binary and checks the bundle,
# its signature and the release zip.
#
# Usage: scripts/test-app.sh [--bin <nomouse binary>]
#   --bin  bundle this binary instead of a fresh `cargo build --release`
#
# Checks: Info.plist (bundle id, LSUIElement, version from Cargo.toml, executable, icon); the
# icon is an .icns; `codesign --verify --deep --strict` passes and the signing identifier is the
# bundle id; both builds have the same designated requirement, so an Accessibility grant to the
# first still applies to the rebuild; the zip holds the bundle and matches its .sha256; the
# bundled binary reports the version. macOS only; elsewhere it prints SKIP.
set -euo pipefail

bin=""
while (($#)); do
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
if [[ "$(uname -s)" != Darwin ]]; then
    echo "SKIP: nomouse.app is built only on macOS"
    exit 0
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z "$bin" ]]; then
    cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse
    bin="${CARGO_TARGET_DIR:-$root/target}/release/nomouse"
fi
tmp="$(mktemp -d /tmp/nomouse-test-app.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT
version="$("$root/scripts/release/version.sh")"
bundle_id="${NOMOUSE_BUNDLE_ID:-com.yuyudhan.nomouse}"

passed=0
failed=0
check() {
    local what="$1"
    shift
    if "$@"; then
        echo "PASS: $what"
        passed=$((passed + 1))
    else
        echo "FAIL: $what"
        failed=$((failed + 1))
    fi
}

"$root/scripts/release/app.sh" "$bin" "$tmp/first" 2>"$tmp/first.log"
"$root/scripts/release/app.sh" "$bin" "$tmp/second" 2>"$tmp/second.log"
app="$tmp/first/nomouse.app"
plist="$app/Contents/Info.plist"

key_is() { [[ "$(plutil -extract "$1" raw -o - "$plist")" == "$2" ]]; }
check "bundle id is $bundle_id" key_is CFBundleIdentifier "$bundle_id"
check "LSUIElement is true (no Dock icon)" key_is LSUIElement true
check "version is $version" key_is CFBundleShortVersionString "$version"
check "executable is nomouse" key_is CFBundleExecutable nomouse
check "icon file is AppIcon" key_is CFBundleIconFile AppIcon
is_icns() { [[ "$(head -c 4 "$app/Contents/Resources/AppIcon.icns")" == icns ]]; }
check "AppIcon.icns is an icns file" is_icns
check "codesign --verify --deep --strict" codesign --verify --deep --strict "$app"
identifier_is() { codesign -dv "$app" 2>&1 | grep -qx "Identifier=$bundle_id"; }
check "signing identifier is the bundle id" identifier_is
requirement() { codesign -d -r- "$1" 2>&1 | grep 'designated =>'; }
same_requirement() {
    local first second
    first="$(requirement "$app")"
    second="$(requirement "$tmp/second/nomouse.app")"
    [[ -n "$first" && "$first" == "$second" ]]
}
check "a rebuild keeps the designated requirement (the grant survives)" same_requirement
zip_ok() {
    local entries
    entries="$(unzip -Z1 "$tmp/first/nomouse-universal-apple-darwin.app.zip")"
    (cd "$tmp/first" && shasum -a 256 --check --quiet nomouse-universal-apple-darwin.app.zip.sha256) &&
        grep -qx 'nomouse.app/Contents/MacOS/nomouse' <<<"$entries" && ! grep -q '__MACOSX' <<<"$entries"
}
check "the zip holds the bundle (no __MACOSX) and matches its checksum" zip_ok
unzipped_verifies() {
    mkdir -p "$tmp/unzipped"
    ditto -x -k "$tmp/first/nomouse-universal-apple-darwin.app.zip" "$tmp/unzipped" &&
        codesign --verify --deep --strict "$tmp/unzipped/nomouse.app"
}
check "the bundle unzipped from the zip still verifies" unzipped_verifies
reports_version() { "$app/Contents/MacOS/nomouse" --version | grep -q "$version"; }
check "the bundled binary reports $version" reports_version

echo "--- $passed passed, $failed failed"
((failed == 0))
