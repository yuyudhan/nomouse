#!/usr/bin/env bash
# Tests that a development build bundles apart from the release: scripts/release/app.sh makes
# "nomouse Dev.app" (bundle id com.yuyudhan.nomouse.dev) from a development binary and
# nomouse.app (com.yuyudhan.nomouse) from a release one, side by side in one folder.
#
# Usage: scripts/test-app-dev.sh [--bin <dev nomouse>] [--release-bin <release nomouse>]
#   --bin          bundle this development binary instead of a fresh
#                  `NOMOUSE_FLAVOUR=dev cargo build --release`
#   --release-bin  bundle this release binary instead of a fresh
#                  `NOMOUSE_FLAVOUR=release cargo build --release`
#
# Checks: each binary's `--version` names its flavour; the dev bundle's Info.plist has the dev
# bundle id and the name "nomouse Dev", its signing identifier is the dev id, and its zip is
# nomouse-dev-universal-apple-darwin.app.zip; the release bundle keeps nomouse.app,
# com.yuyudhan.nomouse and its zip; the two designated requirements differ (separate
# Accessibility grants); each bundled binary's `config path` is its own directory under
# ~/.config (nomouse-dev/ and nomouse/). macOS only; elsewhere it prints SKIP.
set -euo pipefail

dev_bin=""
release_bin=""
while (($#)); do
    case "$1" in
    --bin)
        dev_bin="$2"
        shift 2
        ;;
    --release-bin)
        release_bin="$2"
        shift 2
        ;;
    *)
        echo "usage: $0 [--bin <dev nomouse>] [--release-bin <release nomouse>]" >&2
        exit 2
        ;;
    esac
done
if [[ "$(uname -s)" != Darwin ]]; then
    echo "SKIP: nomouse.app is built only on macOS"
    exit 0
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d /tmp/nomouse-test-app-dev.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT
built="${CARGO_TARGET_DIR:-$root/target}/release/nomouse"

# Both flavours build into the same target/release/nomouse, so each is copied out before the
# other is built.
build() {
    local flavour="$1" out="$2"
    NOMOUSE_FLAVOUR="$flavour" cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse
    cp "$built" "$out"
}
if [[ -z "$dev_bin" ]]; then
    dev_bin="$tmp/nomouse-dev-binary"
    build dev "$dev_bin"
fi
if [[ -z "$release_bin" ]]; then
    release_bin="$tmp/nomouse-release-binary"
    build release "$release_bin"
fi

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
plist() {
    /usr/libexec/PlistBuddy -c "Print :$2" "$1/Contents/Info.plist"
}
signed_id() {
    codesign -dv "$1" 2>&1 | sed -n 's/^Identifier=//p'
}
requirement() {
    codesign -d -r- "$1" 2>&1 | grep 'designated =>'
}
# The bundled binary's config path with a scratch HOME and no XDG_CONFIG_HOME.
config_path() {
    env -u XDG_CONFIG_HOME HOME="$tmp/home" "$1/Contents/MacOS/nomouse" config path
}

version="$("$root/scripts/release/version.sh")"
check "the dev binary reports a dev build" \
    test "$("$dev_bin" --version)" = "nomouse ${version} (dev)"
check "the release binary reports a release build" \
    test "$("$release_bin" --version)" = "nomouse ${version}"

out="$tmp/apps"
env -u NOMOUSE_BUNDLE_ID "$root/scripts/release/app.sh" "$dev_bin" "$out" 2>"$tmp/dev.log"
env -u NOMOUSE_BUNDLE_ID "$root/scripts/release/app.sh" "$release_bin" "$out" 2>"$tmp/release.log"
dev_app="$out/nomouse Dev.app"
release_app="$out/nomouse.app"

check "the dev bundle is \"nomouse Dev.app\"" test -d "$dev_app"
check "the dev Info.plist has the dev bundle id" \
    test "$(plist "$dev_app" CFBundleIdentifier)" = "com.yuyudhan.nomouse.dev"
check "the dev Info.plist names the app \"nomouse Dev\"" \
    test "$(plist "$dev_app" CFBundleName)/$(plist "$dev_app" CFBundleDisplayName)" = "nomouse Dev/nomouse Dev"
check "the dev bundle is signed as com.yuyudhan.nomouse.dev" \
    test "$(signed_id "$dev_app")" = "com.yuyudhan.nomouse.dev"
check "codesign verifies the dev bundle" codesign --verify --deep --strict "$dev_app"
check "the dev zip is nomouse-dev-universal-apple-darwin.app.zip" \
    test -f "$out/nomouse-dev-universal-apple-darwin.app.zip"
check "the dev build's config is ~/.config/nomouse-dev/" \
    test "$(config_path "$dev_app")" = "$tmp/home/.config/nomouse-dev/config.toml"

check "the release bundle is still nomouse.app beside it" test -d "$release_app"
check "the release Info.plist keeps com.yuyudhan.nomouse" \
    test "$(plist "$release_app" CFBundleIdentifier)" = "com.yuyudhan.nomouse"
check "the release Info.plist keeps the name nomouse" \
    test "$(plist "$release_app" CFBundleName)" = "nomouse"
check "the release zip keeps its name" test -f "$out/nomouse-universal-apple-darwin.app.zip"
check "the release build's config stays ~/.config/nomouse/" \
    test "$(config_path "$release_app")" = "$tmp/home/.config/nomouse/config.toml"
requirements_read() {
    [[ -n "$(requirement "$dev_app")" && -n "$(requirement "$release_app")" ]]
}
check "both designated requirements are readable" requirements_read
check "the two bundles have different designated requirements (separate grants)" \
    test "$(requirement "$dev_app")" != "$(requirement "$release_app")"

echo "passed: ${passed}, failed: ${failed}"
if ((failed > 0)); then
    echo "--- app.sh (dev) ---"
    cat "$tmp/dev.log"
    echo "--- app.sh (release) ---"
    cat "$tmp/release.log"
    exit 1
fi
