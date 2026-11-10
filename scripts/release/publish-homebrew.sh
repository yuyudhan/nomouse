#!/usr/bin/env bash
# Publishes a release's Homebrew formula and cask to the tap: DIST/nomouse.rb becomes
# Formula/nomouse.rb and DIST/nomouse-app.rb becomes Casks/nomouse.rb in yuyudhan/homebrew-tap,
# committed as "🔖 Update nomouse to VERSION" and pushed to the tap's default branch.
#
# Usage: scripts/release/publish-homebrew.sh VERSION DIST [--no-install]
#   DIST         the assembled release assets (finish.sh rendered nomouse.rb and nomouse-app.rb)
#   --no-install check with `brew style` only, without installing the release from the tap
#
# Before pushing, on macOS with Homebrew, the new tap commit is tapped from the local clone
# under a throwaway name and checked: `brew style` on the formula and the cask, then (unless
# --no-install) `brew install --cask` (nomouse.app reports VERSION), `brew uninstall --cask`,
# `brew install --formula`, `brew test` and `brew uninstall --formula`. Installing downloads the
# published release assets, so run it after the GitHub release exists. Without Homebrew the
# checks are skipped with a notice (the CI tap test runs them on macOS).
#
# Environment:
#   NOMOUSE_TAP_REMOTE   the tap's git URL (default https://github.com/yuyudhan/homebrew-tap.git;
#                        tests point it at a local bare repository)
#   GH_TOKEN             in GitHub Actions: a token that may push to the tap (the
#                        HOMEBREW_TAP_TOKEN secret). Elsewhere `gh`'s login is used.
#
# Pushing to GitHub goes through `gh auth git-credential`, so no token reaches the git config.
# A tap that already has these files unchanged is left alone, so a rerun is harmless.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ] || { [ "$#" -eq 3 ] && [ "$3" != "--no-install" ]; }; then
    echo "usage: $0 VERSION DIST [--no-install]" >&2
    exit 2
fi
version="$1"
dist="$(cd "$2" && pwd)"
install="yes"
[ "${3:-}" = "--no-install" ] && install="no"
remote="${NOMOUSE_TAP_REMOTE:-https://github.com/yuyudhan/homebrew-tap.git}"

# The formula names its version in the release URLs, the cask in its version stanza.
for file in nomouse.rb nomouse-app.rb; do
    [ -f "${dist}/${file}" ] || {
        echo "publish-homebrew: ${dist}/${file} is missing (run scripts/release/finish.sh)" >&2
        exit 1
    }
    if grep -q '"missing"' "${dist}/${file}"; then
        echo "publish-homebrew: ${dist}/${file} has a missing checksum (a dry-run render)" >&2
        exit 1
    fi
    marker="/releases/download/v${version}/"
    [ "$file" = nomouse-app.rb ] && marker="version \"${version}\""
    if ! grep -qF "$marker" "${dist}/${file}"; then
        echo "publish-homebrew: ${dist}/${file} is not for version ${version}" >&2
        exit 1
    fi
done

git_remote=(git)
if [[ "$remote" == https://github.com/* ]]; then
    if [ -n "${GITHUB_ACTIONS:-}" ] && [ -z "${GH_TOKEN:-}" ]; then
        echo "::error title=Homebrew tap not updated::The HOMEBREW_TAP_TOKEN secret is not set. Add a fine-grained token with Contents read and write on yuyudhan/homebrew-tap as that repository secret, or publish from a machine with gh logged in: scripts/release/publish-homebrew.sh ${version} <release assets>" >&2
        exit 1
    fi
    command -v gh >/dev/null || {
        echo "publish-homebrew: pushing to ${remote} needs the GitHub CLI (gh)" >&2
        exit 1
    }
    git_remote=(git -c credential.helper= -c 'credential.helper=!gh auth git-credential')
fi

work="$(mktemp -d)"
tap_name="nomouse-release/verify-$$"
tapped="no"
trusted="no"
cleanup() {
    if [ "$trusted" = "yes" ]; then
        brew untrust --tap "$tap_name" >/dev/null 2>&1 || true
    fi
    if [ "$tapped" = "yes" ]; then
        brew untap --force "$tap_name" >/dev/null 2>&1 || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

clone="${work}/tap"
"${git_remote[@]}" clone --quiet --depth 1 "$remote" "$clone"
mkdir -p "${clone}/Formula" "${clone}/Casks"
cp "${dist}/nomouse.rb" "${clone}/Formula/nomouse.rb"
cp "${dist}/nomouse-app.rb" "${clone}/Casks/nomouse.rb"
git -C "$clone" add Formula/nomouse.rb Casks/nomouse.rb
if git -C "$clone" diff --cached --quiet; then
    echo "publish-homebrew: the tap already has nomouse ${version}" >&2
    exit 0
fi
git -C "$clone" -c user.name="nomouse release" -c user.email="nomouse-release@users.noreply.github.com" \
    commit --quiet -m "🔖 Update nomouse to ${version}"

if [[ "$(uname -s)" == Darwin ]] && command -v brew >/dev/null; then
    brew tap "$tap_name" "$clone" >/dev/null
    tapped="yes"
    # Homebrew 7 loads formulae and casks from third-party taps only once they are trusted.
    if brew trust --help >/dev/null 2>&1; then
        brew trust --tap "$tap_name" >/dev/null
        trusted="yes"
    fi
    brew style --formula "${tap_name}/nomouse"
    brew style --cask "${tap_name}/nomouse"
    if [ "$install" = "yes" ]; then
        export HOMEBREW_NO_AUTO_UPDATE=1
        # --online downloads the release assets, so it belongs with the install check.
        brew audit --strict --online --formula "${tap_name}/nomouse"
        brew audit --strict --online --cask "${tap_name}/nomouse"
        brew install --cask "${tap_name}/nomouse"
        installed="$(/Applications/nomouse.app/Contents/MacOS/nomouse --version)"
        echo "publish-homebrew: the cask installed ${installed}" >&2
        [[ "$installed" == *"$version"* ]] || {
            echo "publish-homebrew: the cask installed '${installed}', not ${version}" >&2
            exit 1
        }
        brew uninstall --cask "${tap_name}/nomouse"
        brew install --formula "${tap_name}/nomouse"
        brew test "${tap_name}/nomouse"
        brew uninstall --formula "${tap_name}/nomouse"
    fi
else
    echo "publish-homebrew: no Homebrew on this machine, brew style and the install check are skipped" >&2
fi

"${git_remote[@]}" -C "$clone" push --quiet origin HEAD
echo "publish-homebrew: ${remote} has nomouse ${version} (Formula/nomouse.rb, Casks/nomouse.rb)" >&2
