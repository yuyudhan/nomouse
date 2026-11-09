#!/usr/bin/env bash
# Prepares Developer ID signing and notarization for the macOS release job from repository
# secrets, or leaves the release ad-hoc signed when they are absent.
#
# Usage (GitHub Actions, macOS runner): scripts/release/signing.sh
#
# Environment, from repository secrets:
#   MACOS_CERTIFICATE           base64 of the "Developer ID Application" certificate and key (.p12)
#   MACOS_CERTIFICATE_PASSWORD  the .p12 password
#   MACOS_SIGN_IDENTITY         the identity, e.g. "Developer ID Application: Name (TEAMID)"
#   NOTARY_KEY                  base64 of the App Store Connect API key (AuthKey_<id>.p8)
#   NOTARY_KEY_ID, NOTARY_ISSUER  that key's id and its issuer id
#
# With the certificate it imports it into a temporary keychain (in RUNNER_TEMP, deleted with
# the runner) and exports NOMOUSE_SIGN_IDENTITY through GITHUB_ENV; with the notary key it also
# exports NOMOUSE_NOTARY_KEY, NOMOUSE_NOTARY_KEY_ID and NOMOUSE_NOTARY_ISSUER, which app.sh and
# dmg.sh read. A notary key without a certificate is an error: only Developer ID signed code
# can be notarized.
set -euo pipefail

: "${GITHUB_ENV:?signing.sh runs in GitHub Actions}"
: "${RUNNER_TEMP:?signing.sh runs in GitHub Actions}"

if [ -z "${MACOS_CERTIFICATE:-}" ]; then
    if [ -n "${NOTARY_KEY:-}" ]; then
        echo "signing: NOTARY_KEY is set but MACOS_CERTIFICATE is not" >&2
        exit 1
    fi
    echo "signing: no MACOS_CERTIFICATE secret, nomouse.app stays ad-hoc signed" >&2
    exit 0
fi
for name in MACOS_CERTIFICATE_PASSWORD MACOS_SIGN_IDENTITY; do
    if [ -z "${!name:-}" ]; then
        echo "signing: MACOS_CERTIFICATE is set but ${name} is not" >&2
        exit 1
    fi
done

keychain="${RUNNER_TEMP}/nomouse-signing.keychain-db"
keychain_password="$(uuidgen)"
certificate="${RUNNER_TEMP}/nomouse-certificate.p12"
printf '%s' "$MACOS_CERTIFICATE" | base64 --decode >"$certificate"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$certificate" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign
rm -f "$certificate"
security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain" \
    >/dev/null
# Keep the login keychain searchable and put the temporary one first.
existing=()
while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"
    existing+=("${line//\"/}")
done < <(security list-keychains -d user)
security list-keychains -d user -s "$keychain" "${existing[@]}"
if ! security find-identity -v -p codesigning "$keychain" | grep -qF "\"${MACOS_SIGN_IDENTITY}\""; then
    echo "signing: the certificate holds no valid identity named ${MACOS_SIGN_IDENTITY}" >&2
    exit 1
fi
echo "NOMOUSE_SIGN_IDENTITY=${MACOS_SIGN_IDENTITY}" >>"$GITHUB_ENV"
echo "signing: nomouse.app will be signed by ${MACOS_SIGN_IDENTITY}" >&2

if [ -z "${NOTARY_KEY:-}" ]; then
    echo "signing: no NOTARY_KEY secret, the release is signed but not notarized" >&2
    exit 0
fi
for name in NOTARY_KEY_ID NOTARY_ISSUER; do
    if [ -z "${!name:-}" ]; then
        echo "signing: NOTARY_KEY is set but ${name} is not" >&2
        exit 1
    fi
done
key="${RUNNER_TEMP}/AuthKey_${NOTARY_KEY_ID}.p8"
printf '%s' "$NOTARY_KEY" | base64 --decode >"$key"
chmod 600 "$key"
{
    echo "NOMOUSE_NOTARY_KEY=${key}"
    echo "NOMOUSE_NOTARY_KEY_ID=${NOTARY_KEY_ID}"
    echo "NOMOUSE_NOTARY_ISSUER=${NOTARY_ISSUER}"
} >>"$GITHUB_ENV"
echo "signing: nomouse.app and the disk image will be notarized" >&2
