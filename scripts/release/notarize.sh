#!/usr/bin/env bash
# Notarizes one signed macOS artifact with Apple's notary service and staples the ticket.
#
# Usage: scripts/release/notarize.sh PATH
#   PATH   a Developer ID signed nomouse.app (zipped for the upload, then stapled in place) or
#          .dmg (uploaded and stapled as is)
#
# Environment (an App Store Connect API key with the Developer role):
#   NOMOUSE_NOTARY_KEY        path to the AuthKey_<id>.p8 private key file
#   NOMOUSE_NOTARY_KEY_ID     the key id
#   NOMOUSE_NOTARY_ISSUER     the issuer id of the App Store Connect team
#
# Fails when any of them is missing, when the service rejects the upload (the notary log is
# printed), or when stapling fails. Ad-hoc signed artifacts cannot be notarized.
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 PATH" >&2
    exit 2
fi
target="${1%/}"
for name in NOMOUSE_NOTARY_KEY NOMOUSE_NOTARY_KEY_ID NOMOUSE_NOTARY_ISSUER; do
    if [ -z "${!name:-}" ]; then
        echo "notarize: ${name} is not set (see CONTRIBUTING.md#releases)" >&2
        exit 1
    fi
done
[ -e "$target" ] || {
    echo "notarize: no ${target}" >&2
    exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
case "$target" in
*.app)
    upload="${work}/$(basename "$target").zip"
    ditto -c -k --norsrc --noextattr --noqtn --keepParent "$target" "$upload"
    ;;
*.dmg) upload="$target" ;;
*)
    echo "notarize: ${target} is neither a .app nor a .dmg" >&2
    exit 1
    ;;
esac

credentials=(--key "$NOMOUSE_NOTARY_KEY" --key-id "$NOMOUSE_NOTARY_KEY_ID" --issuer "$NOMOUSE_NOTARY_ISSUER")
result="${work}/result.json"
xcrun notarytool submit "$upload" "${credentials[@]}" --wait --timeout 30m \
    --output-format json >"$result"
status="$(plutil -extract status raw -o - "$result")"
id="$(plutil -extract id raw -o - "$result")"
if [ "$status" != "Accepted" ]; then
    echo "notarize: ${target} was not accepted (${status}); the notary log follows" >&2
    xcrun notarytool log "$id" "${credentials[@]}" >&2 || true
    exit 1
fi
xcrun stapler staple "$target"
xcrun stapler validate "$target"
echo "notarize: ${target} notarized (${id}) and stapled" >&2
