#!/usr/bin/env bash
# Tests scripts/release/homebrew.sh and publish-homebrew.sh against a local bare tap: renders the
# formula and the cask for fake checksums (ad-hoc and Developer ID), publishes them, and checks
# the tap's files, its commit, that a rerun pushes nothing and that a dry-run render (missing
# checksums) or a version mismatch is refused. On macOS with Homebrew the publish step also runs
# `brew style` on both files (no install: the fake release does not exist).
#
# Usage: scripts/release/test-publish-homebrew.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
version="9.8.7"

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

fake_dist() {
    local dist="$1" name
    mkdir -p "$dist"
    for name in universal-apple-darwin.tar.gz x86_64-unknown-linux-gnu.tar.gz \
        aarch64-unknown-linux-gnu.tar.gz universal-apple-darwin.dmg; do
        printf '%s  nomouse-%s\n' "$(printf '%064d' 7)" "$name" >"${dist}/nomouse-${name}.sha256"
    done
}

git init --quiet --bare --initial-branch=main "$tmp/tap.git"
git clone --quiet "$tmp/tap.git" "$tmp/seed" 2>/dev/null
echo "# homebrew-tap" >"$tmp/seed/README.md"
git -C "$tmp/seed" add README.md
git -C "$tmp/seed" -c user.name=t -c user.email=t@example.invalid commit --quiet -m "Initial commit"
git -C "$tmp/seed" push --quiet origin HEAD:main
# file:// so the publish script's shallow clone works as it does against GitHub.
export NOMOUSE_TAP_REMOTE="file://$tmp/tap.git"

fake_dist "$tmp/adhoc"
"$root/scripts/release/homebrew.sh" "$version" "$tmp/adhoc" 2>/dev/null
fake_dist "$tmp/devid"
NOMOUSE_MACOS_SIGNING=developer-id "$root/scripts/release/homebrew.sh" "$version" "$tmp/devid" 2>/dev/null

cask="$tmp/adhoc/nomouse-app.rb"
all_have() {
    local text="$1" file
    shift
    for file; do grep -qF "$text" "$file" || return 1; done
}
# Succeeds when the command fails; its stderr is dropped.
refused() { ! "$@" 2>/dev/null; }
valid_ruby() {
    local file
    for file; do ruby -c "$file" >/dev/null || return 1; done
}
last_subject_is() { [[ "$(git -C "$1" log -1 --format=%s)" == "$2" ]]; }
publish="$root/scripts/release/publish-homebrew.sh"

check "no placeholder is left in the formula or the casks" \
    refused grep -q "@[A-Z_0-9]*@" "$tmp/adhoc/nomouse.rb" "$cask" "$tmp/devid/nomouse-app.rb"
check "the ad-hoc cask clears the quarantine" all_have '"com.apple.quarantine", "{{appdir}}/nomouse.app"' "$cask"
check "the ad-hoc cask says the grant is asked again after an upgrade" \
    all_have 'System Settings > Privacy & Security > Accessibility.' "$cask"
check "the Developer ID cask keeps the quarantine" refused grep -q quarantine "$tmp/devid/nomouse-app.rb"
check "both casks depend on macOS" all_have 'depends_on :macos' "$cask" "$tmp/devid/nomouse-app.rb"
check "both casks quit the app before uninstalling" \
    all_have 'uninstall quit: "com.yuyudhan.nomouse"' "$cask" "$tmp/devid/nomouse-app.rb"
if command -v ruby >/dev/null; then
    check "the formula and the casks are valid Ruby" \
        valid_ruby "$tmp/adhoc/nomouse.rb" "$cask" "$tmp/devid/nomouse-app.rb"
fi
check "an unknown NOMOUSE_MACOS_SIGNING is refused" \
    refused env NOMOUSE_MACOS_SIGNING=team "$root/scripts/release/homebrew.sh" "$version" "$tmp/devid"

check "publish-homebrew.sh publishes to the tap" "$publish" "$version" "$tmp/adhoc" --no-install
git clone --quiet "$tmp/tap.git" "$tmp/after"
check "the tap has Formula/nomouse.rb" cmp -s "$tmp/after/Formula/nomouse.rb" "$tmp/adhoc/nomouse.rb"
check "the tap has Casks/nomouse.rb" cmp -s "$tmp/after/Casks/nomouse.rb" "$cask"
check "the tap keeps its other files" test -f "$tmp/after/README.md"
check "the tap commit names the version" last_subject_is "$tmp/after" "🔖 Update nomouse to ${version}"
before="$(git -C "$tmp/tap.git" rev-parse main)"
"$publish" "$version" "$tmp/adhoc" --no-install 2>/dev/null
check "a rerun pushes nothing" test "$(git -C "$tmp/tap.git" rev-parse main)" = "$before"

mkdir -p "$tmp/dry"
cp "$tmp/adhoc"/nomouse*.rb "$tmp/dry/"
sed -i.bak 's/sha256 "0*7"/sha256 "missing"/' "$tmp/dry/nomouse.rb"
check "a dry-run render (missing checksum) is refused" refused "$publish" "$version" "$tmp/dry" --no-install
check "a version mismatch is refused" refused "$publish" 1.0.0 "$tmp/adhoc" --no-install
check "the tap is unchanged after the refusals" test "$(git -C "$tmp/tap.git" rev-parse main)" = "$before"

echo "--- $passed passed, $failed failed"
((failed == 0))
