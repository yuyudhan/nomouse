#!/usr/bin/env bash
# Packs a built nomouse.app into the release disk image: nomouse.app next to a link to
# /Applications, compressed (UDZO), signed with the app's identity when that is a Developer ID.
# The HFS+ image is written by `hdiutil makehybrid` and compressed by `hdiutil convert`, so no
# volume is ever mounted: that also works where policy forbids mounting disk images.
#
# Usage: scripts/release/dmg.sh APP OUTDIR
#   APP      nomouse.app from scripts/release/app.sh (notarized and stapled first for a release)
#   OUTDIR   where nomouse-universal-apple-darwin.dmg and its .sha256 go
#
# Environment:
#   NOMOUSE_SIGN_IDENTITY     codesign identity, as for app.sh; "-" (the default) leaves the
#                             image unsigned, since an ad-hoc signature on a disk image means
#                             nothing to Gatekeeper.
#   NOMOUSE_NOTARY_KEY, NOMOUSE_NOTARY_KEY_ID, NOMOUSE_NOTARY_ISSUER
#                             when all three are set the image is notarized and stapled by
#                             scripts/release/notarize.sh.
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 APP OUTDIR" >&2
    exit 2
fi
app="${1%/}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$2"
outdir="$(cd "$2" && pwd)"
[ -d "${app}/Contents/MacOS" ] || {
    echo "dmg: ${app} is not an app bundle" >&2
    exit 1
}
identity="${NOMOUSE_SIGN_IDENTITY:--}"
version="$("${root}/scripts/release/version.sh")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="${work}/nomouse"
mkdir -p "$stage"
ditto "$app" "${stage}/nomouse.app"
ln -s /Applications "${stage}/Applications"

image="${outdir}/nomouse-universal-apple-darwin.dmg"
rm -f "$image"
hdiutil makehybrid -quiet -hfs -hfs-volume-name "nomouse ${version}" -o "${work}/raw.dmg" "$stage"
hdiutil convert -quiet "${work}/raw.dmg" -format UDZO -imagekey zlib-level=9 -o "$image"
if [ "$identity" != "-" ]; then
    codesign --force --sign "$identity" --timestamp "$image"
    codesign --verify --strict "$image"
fi
if [ -n "${NOMOUSE_NOTARY_KEY_ID:-}" ]; then
    "${root}/scripts/release/notarize.sh" "$image"
fi
hdiutil verify -quiet "$image"
"${root}/scripts/release/checksum.sh" "$image"
echo "dmg: ${image} (${version})" >&2
