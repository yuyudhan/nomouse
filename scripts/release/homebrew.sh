#!/usr/bin/env bash
# Renders packaging/homebrew/nomouse.rb.in into DIST/nomouse.rb with the version and the
# checksums of the archives in DIST and, when DIST has the macOS disk image,
# packaging/homebrew/nomouse-app.rb.in into the cask DIST/nomouse-app.rb.
#
# Usage: scripts/release/homebrew.sh VERSION DIST [--allow-missing]
#   --allow-missing leaves "missing" for archives not in DIST (a single-OS dry run); a real
#   release must have all three. The cask is skipped while releases carry no disk image.
#
# Environment:
#   NOMOUSE_BUNDLE_ID         the app's bundle identifier (default com.yuyudhan.nomouse)
#   NOMOUSE_MACOS_SIGNING     how the disk image's nomouse.app is signed, as
#                             scripts/release/signature.sh reports it: "ad-hoc" (the default)
#                             or "developer-id". An ad-hoc cask clears the download quarantine
#                             (Gatekeeper refuses an app that is not notarized) and says that
#                             macOS asks for Accessibility again after every upgrade.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 VERSION DIST [--allow-missing]" >&2
    exit 2
fi
version="$1"
dist="$2"
allow_missing="${3:-}"
root="$(cd "$(dirname "$0")/../.." && pwd)"

formula="$(<"${root}/packaging/homebrew/nomouse.rb.in")"
formula="${formula//@VERSION@/${version}}"
for name in universal-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    sum_file="${dist}/nomouse-${name}.tar.gz.sha256"
    if [ -f "$sum_file" ]; then
        sum="$(cut -d ' ' -f 1 <"$sum_file")"
    elif [ "$allow_missing" = "--allow-missing" ]; then
        sum="missing"
    else
        echo "homebrew: ${sum_file} is missing" >&2
        exit 1
    fi
    key="$(printf '%s' "$name" | tr '[:lower:]-' '[:upper:]_')"
    formula="${formula//@SHA256_${key}@/${sum}}"
done
printf '%s\n' "$formula" >"${dist}/nomouse.rb"
echo "homebrew: ${dist}/nomouse.rb" >&2

dmg_sum="${dist}/nomouse-universal-apple-darwin.dmg.sha256"
if [ -f "$dmg_sum" ]; then
    signing="${NOMOUSE_MACOS_SIGNING:-ad-hoc}"
    case "$signing" in
    ad-hoc)
        quarantine='  # This build is ad-hoc signed, not notarized, so Gatekeeper would refuse to open it after a
  # download. Clearing the quarantine flag is the same as approving it once in System Settings.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/nomouse.app"]
  end
'
        caveat='
    This build is ad-hoc signed, not notarized by Apple; Homebrew cleared the
    download quarantine so it opens. macOS ties the Accessibility grant to the
    signature, so after every upgrade switch nomouse on again in
    System Settings > Privacy & Security > Accessibility.'
        ;;
    developer-id)
        quarantine=""
        caveat=""
        ;;
    *)
        echo "homebrew: NOMOUSE_MACOS_SIGNING is '${signing}', expected ad-hoc or developer-id" >&2
        exit 1
        ;;
    esac
    # The marker lines are replaced whole (an empty block drops the line), line by line rather
    # than with ${var//pattern/replacement}, whose quoting differs between bash 3.2 and 5.2.
    {
        while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
            @QUARANTINE@) [ -z "$quarantine" ] || printf '%s\n' "$quarantine" ;;
            @SIGNING_CAVEAT@) [ -z "$caveat" ] || printf '%s\n' "$caveat" ;;
            *)
                line="${line//@VERSION@/${version}}"
                line="${line//@SHA256_DMG@/$(cut -d ' ' -f 1 <"$dmg_sum")}"
                line="${line//@BUNDLE_ID@/${NOMOUSE_BUNDLE_ID:-com.yuyudhan.nomouse}}"
                printf '%s\n' "$line"
                ;;
            esac
        done <"${root}/packaging/homebrew/nomouse-app.rb.in"
    } >"${dist}/nomouse-app.rb"
    echo "homebrew: ${dist}/nomouse-app.rb (${signing})" >&2
fi
