#!/usr/bin/env bash
# Tests scripts/release/app.sh, dmg.sh and the cask homebrew.sh renders: builds nomouse.app
# twice from one binary and checks the bundle, its signature, the release zip, the disk image
# and the cask.
#
# Usage: scripts/test-app.sh [--bin <nomouse binary>]
#   --bin  bundle this binary instead of a fresh `cargo build --release`
#
# Checks: Info.plist (bundle id, LSUIElement, version from Cargo.toml, executable, icon); the
# icon is an .icns; `codesign --verify --deep --strict` passes and the signing identifier is the
# bundle id; both builds have the same designated requirement, so an Accessibility grant to the
# first still applies to the rebuild; the zip holds the bundle and matches its .sha256; the
# bundled binary reports the version; the disk image verifies, matches its .sha256 and holds a
# verifying nomouse.app next to a link to /Applications (read with 7-Zip's `7zz`, so nothing is
# mounted); the cask names the version, the image's checksum and URL and the bundle id, and is
# valid Ruby. macOS only; elsewhere it prints SKIP.
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
"$root/scripts/release/dmg.sh" "$tmp/first/nomouse.app" "$tmp/first" 2>"$tmp/dmg.log"
"$root/scripts/release/homebrew.sh" "$version" "$tmp/first" --allow-missing 2>"$tmp/homebrew.log"
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

dmg="$tmp/first/nomouse-universal-apple-darwin.dmg"
dmg_ok() {
    hdiutil verify -quiet "$dmg" &&
        (cd "$tmp/first" && shasum -a 256 --check --quiet nomouse-universal-apple-darwin.dmg.sha256)
}
check "the disk image verifies and matches its checksum" dmg_ok
volume="$tmp/volume/nomouse $version"
extracted() {
    if ! command -v 7zz >/dev/null; then
        echo "  7zz is not installed (brew install sevenzip)"
        return 1
    fi
    7zz x -y -o"$tmp/volume" "$dmg" >/dev/null
}
check "the disk image unpacks (volume 'nomouse $version')" extracted
check "the bundle in the disk image verifies" codesign --verify --deep --strict "$volume/nomouse.app"
# 7-Zip rebases absolute links when it extracts them, so the link is read from the image.
applications_link() {
    local entry="nomouse $version/Applications"
    7zz l -slt "$dmg" "$entry" | grep -qx 'Mode = lrwxr-xr-x' &&
        [[ "$(7zz x -so "$dmg" "$entry" 2>/dev/null)" == /Applications ]]
}
check "the disk image links /Applications" applications_link

cask="$tmp/first/nomouse-app.rb"
cask_has() { grep -qF "$1" "$cask"; }
check "the cask names version $version" cask_has "version \"$version\""
check "the cask has the image's checksum" cask_has "sha256 \"$(cut -d ' ' -f 1 <"$dmg.sha256")\""
check "the cask downloads the release's disk image" cask_has "/releases/download/v#{version}/nomouse-universal-apple-darwin.dmg"
check "the cask quits $bundle_id before uninstalling" cask_has "uninstall quit: \"$bundle_id\""
check "the cask is valid Ruby" ruby -c "$cask"

echo "--- $passed passed, $failed failed"
((failed == 0))
