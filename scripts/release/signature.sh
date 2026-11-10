#!/usr/bin/env bash
# Reports how a built nomouse.app is signed and checks that it is what the release asked for.
#
# Usage: scripts/release/signature.sh APP
#   Prints "developer-id" or "ad-hoc" on stdout.
#
# Environment:
#   NOMOUSE_SIGN_IDENTITY   the identity app.sh signed with; "-" (the default) means ad hoc.
#
# With a Developer ID identity the designated requirement must name Apple's anchor and the
# team (`anchor apple generic` and `certificate leaf[subject.OU]`): that requirement is what
# keeps the Accessibility grant across updates. An ad-hoc signature's requirement is its code
# hash, so every update asks for Accessibility again; in GitHub Actions that is reported as a
# warning annotation on the run.
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 APP" >&2
    exit 2
fi
app="${1%/}"
[ -d "${app}/Contents/MacOS" ] || {
    echo "signature: ${app} is not an app bundle" >&2
    exit 1
}
identity="${NOMOUSE_SIGN_IDENTITY:--}"
# An implicit requirement (ad hoc) is printed as a comment: "# designated => cdhash H\"…\"".
requirement="$(codesign -d -r- "$app" 2>&1 | sed -n -E 's/^(# )?designated => //p')"
echo "signature: designated requirement: ${requirement}" >&2

if [ "$identity" = "-" ]; then
    if [[ "$requirement" != cdhash* ]]; then
        echo "signature: ${app} should be ad-hoc signed but its requirement is not a code hash" >&2
        exit 1
    fi
    message="nomouse.app is ad-hoc signed: macOS asks for Accessibility again after every update, and a downloaded disk image needs Privacy & Security > Open Anyway. Add the MACOS_CERTIFICATE, MACOS_CERTIFICATE_PASSWORD, MACOS_SIGN_IDENTITY, NOTARY_KEY, NOTARY_KEY_ID and NOTARY_ISSUER secrets (CONTRIBUTING.md#releases) to sign with a Developer ID."
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "::warning title=ad-hoc signed release::${message}" >&2
    else
        echo "signature: ${message}" >&2
    fi
    echo "ad-hoc"
    exit 0
fi

for part in "anchor apple generic" "certificate leaf[subject.OU]"; do
    if [[ "$requirement" != *"$part"* ]]; then
        echo "signature: ${app} is signed by ${identity} but its requirement lacks '${part}'" >&2
        exit 1
    fi
done
echo "signature: ${app} is Developer ID signed; the Accessibility grant survives updates" >&2
echo "developer-id"
