#!/usr/bin/env bash
# Builds nomouse.app around one macOS binary, signs it and zips it for the release.
#
# Usage: scripts/release/app.sh BINARY OUTDIR
#   BINARY   the built nomouse for macOS (universal or one architecture)
#   OUTDIR   where nomouse.app, nomouse-universal-apple-darwin.app.zip and its .sha256 go
#
# The bundle: Contents/MacOS/nomouse (with no arguments it runs the daemon with its menu bar
# item), Contents/Info.plist from packaging/macos/Info.plist.in (LSUIElement: no Dock icon;
# version from scripts/release/version.sh), Contents/Resources/AppIcon.icns built from the
# logo PNG files in assets/logo/png (scripts/render-logo.sh draws them from assets/logo/*.svg).
#
# Environment:
#   NOMOUSE_BUNDLE_ID         bundle identifier (default com.yuyudhan.nomouse)
#   NOMOUSE_SIGN_IDENTITY     codesign identity; "-" (the default) signs ad hoc. A Developer ID
#                             identity also gets the hardened runtime and a secure timestamp,
#                             as notarization requires.
#   NOMOUSE_NOTARY_KEY, NOMOUSE_NOTARY_KEY_ID, NOMOUSE_NOTARY_ISSUER
#                             when all three are set (Developer ID signing only) the bundle is
#                             notarized and stapled by scripts/release/notarize.sh before it
#                             is zipped.
#
# macOS keys the Accessibility grant to the signature's designated requirement. An ad-hoc
# signature's requirement is its code hash, so the grant survives rebuilding the same binary
# but not a new version; a Developer ID signature keeps it across versions.
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 BINARY OUTDIR" >&2
    exit 2
fi
binary="$1"
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$2"
outdir="$(cd "$2" && pwd)"
[ -f "$binary" ] || {
    echo "app: no binary at ${binary}" >&2
    exit 1
}
bundle_id="${NOMOUSE_BUNDLE_ID:-com.yuyudhan.nomouse}"
identity="${NOMOUSE_SIGN_IDENTITY:--}"
version="$("${root}/scripts/release/version.sh")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="${outdir}/nomouse.app"
rm -rf "$app"
mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"
cp "$binary" "${app}/Contents/MacOS/nomouse"
chmod 755 "${app}/Contents/MacOS/nomouse"
sed -e "s/@VERSION@/${version}/g" -e "s/@BUNDLE_ID@/${bundle_id}/g" \
    "${root}/packaging/macos/Info.plist.in" >"${app}/Contents/Info.plist"
plutil -lint -s "${app}/Contents/Info.plist"
iconset="${work}/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    cp "${root}/assets/logo/png/nomouse-${size}.png" "${iconset}/icon_${size}x${size}.png"
    cp "${root}/assets/logo/png/nomouse-$((size * 2)).png" "${iconset}/icon_${size}x${size}@2x.png"
done
iconutil -c icns -o "${app}/Contents/Resources/AppIcon.icns" "$iconset"

sign=(codesign --force --sign "$identity" --identifier "$bundle_id")
if [ "$identity" != "-" ]; then
    sign+=(--options runtime --timestamp)
fi
"${sign[@]}" "$app"
codesign --verify --deep --strict "$app"
if [ -n "${NOMOUSE_NOTARY_KEY_ID:-}" ]; then
    "${root}/scripts/release/notarize.sh" "$app"
fi

archive="nomouse-universal-apple-darwin.app.zip"
rm -f "${outdir}/${archive}"
# Signatures live in files, so extended attributes (provenance, quarantine) are left out.
(cd "$outdir" && ditto -c -k --norsrc --noextattr --noqtn --keepParent nomouse.app "$archive")
"${root}/scripts/release/checksum.sh" "${outdir}/${archive}"
