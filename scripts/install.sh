#!/usr/bin/env bash
# Installs, updates or uninstalls nomouse on macOS and Linux, without sudo. Published with every
# GitHub release as releases/latest/download/install.sh; README.md#install has the one-line curl
# command that pipes it into bash.
#
# Piping is safe: all work happens in main, called on the last line, so bash has read the whole
# script before anything runs. Options go after `bash -s --`.
#
# What it installs, after checking each download against the release's .sha256:
#   macOS (Apple silicon and Intel): nomouse.app into /Applications (~/Applications when
#     /Applications is not writable), quarantine removed, plus a `nomouse` command in
#     ~/.local/bin that points into the app.
#   Linux (x86_64, aarch64; X11 and Wayland): the `nomouse` binary into ~/.local/bin.
#
# Options:
#   --version X.Y.Z    Install that release instead of the latest.
#   --from DIR         Install from a directory holding the release assets (no network).
#   --uninstall        Remove nomouse; keep the config file.
#   --purge            With --uninstall, also delete the nomouse config directory.
#   -h, --help         Show this help.
#
# Environment:
#   NOMOUSE_INSTALL_DIR    Where the `nomouse` command goes (default ~/.local/bin).
#   NOMOUSE_APP_DIR        macOS: where nomouse.app goes (default /Applications).
#   NOMOUSE_INSTALL_BASE   URL of a directory holding the release assets, overriding GitHub
#                          (for mirrors and for testing a locally built release).
#   GH_TOKEN, GITHUB_TOKEN A GitHub token: downloads go through the GitHub API with it, which
#                          also works while the repository is private.
#   NOMOUSE_API_BASE       The GitHub API base URL for those downloads (default
#                          https://api.github.com).
set -euo pipefail

repo="yuyudhan/nomouse"
bundle_id="com.yuyudhan.nomouse"
# Every request: retry transient failures, give up on a connection that never opens, and abort
# a transfer that stalls (under 1 KB/s for 30 s) instead of hanging.
net=(--fail --silent --show-error --location --connect-timeout 15 --retry 3 --retry-delay 2
    --speed-limit 1024 --speed-time 30)

say() {
    printf 'nomouse: %s\n' "$*"
}

# An indented line of the "next steps" block.
step() {
    printf '  %s\n' "$*"
}

fail() {
    printf 'nomouse: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Install, update or uninstall nomouse (macOS and Linux, no sudo).

Options (after "bash -s --" when piped):
  --version X.Y.Z    install that release instead of the latest
  --from DIR         install from a directory holding the release assets
  --uninstall        remove nomouse, keep the config
  --purge            with --uninstall, also delete the config directory

Environment: NOMOUSE_INSTALL_DIR (default ~/.local/bin), NOMOUSE_APP_DIR (macOS, default
/Applications), NOMOUSE_INSTALL_BASE (asset URL base), GH_TOKEN or GITHUB_TOKEN (download
through the GitHub API, needed while the repository is private), NOMOUSE_API_BASE.
EOF
}

# The release target for an OS and CPU, as `uname -s` and `uname -m` print them.
asset_for() {
    local os="$1" arch="$2"
    case "$os" in
        Linux)
            case "$arch" in
                x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
                aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
                *) fail "no Linux build for ${arch}; supported: x86_64, aarch64" ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64 | x86_64) echo "universal-apple-darwin" ;;
                *) fail "no macOS build for ${arch}; supported: arm64, x86_64" ;;
            esac
            ;;
        MINGW* | MSYS* | CYGWIN*)
            fail "on Windows run install.ps1 in PowerShell instead (see the README)"
            ;;
        *) fail "unsupported OS ${os}; nomouse runs on macOS, Linux and Windows" ;;
    esac
}

# The archive this target installs from.
archive_for() {
    case "$1" in
        *-apple-darwin) echo "nomouse-$1.app.zip" ;;
        *) echo "nomouse-$1.tar.gz" ;;
    esac
}

# The API URL of release asset NAME in the GitHub release JSON in FILE, or nothing. Each asset
# object lists its "url" (…/releases/assets/ID) before its "name"; uploader URLs never match.
# awk reads the file itself, one comma-separated record at a time: with a pipe in front, its
# early exit would kill the writer with SIGPIPE, and pipefail would fail the whole lookup.
asset_api_url() {
    awk -v want="$1" '
        BEGIN { RS = "," }
        match($0, /"url": *"[^"]*\/releases\/assets\/[0-9]+"/) {
            url = substr($0, RSTART, RLENGTH)
            sub(/^"url": *"/, "", url)
            sub(/"$/, "", url)
        }
        match($0, /"name": *"[^"]*"/) {
            name = substr($0, RSTART, RLENGTH)
            sub(/^"name": *"/, "", name)
            sub(/"$/, "", name)
            if (name == want && url != "") {
                print url
                exit
            }
        }
    ' "$2"
}

# The same rule as nomouse's own config path on macOS and Linux.
config_dir() {
    echo "${XDG_CONFIG_HOME:-${HOME}/.config}/nomouse"
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "need sha256sum or shasum to verify the download"
    fi
}

token() {
    printf '%s' "${GH_TOKEN:-${GITHUB_TOKEN:-}}"
}

# Fetches release asset NAME into DEST from wherever this run installs from: the --from
# directory, NOMOUSE_INSTALL_BASE, the GitHub API (with a token) or the public download URL.
# The globals `from`, `version` and `work` are set by main and install_release.
fetch() {
    local name="$1" dest="$2" api release url
    if [ -n "$from" ]; then
        [ -f "${from}/${name}" ] || fail "${from} has no ${name}"
        cp "${from}/${name}" "$dest"
    elif [ -n "${NOMOUSE_INSTALL_BASE:-}" ]; then
        curl "${net[@]}" --output "$dest" "${NOMOUSE_INSTALL_BASE%/}/${name}" ||
            fail "download failed: ${NOMOUSE_INSTALL_BASE%/}/${name}"
    elif [ -n "$(token)" ]; then
        api="${NOMOUSE_API_BASE:-https://api.github.com}"
        api="${api%/}/repos/${repo}/releases"
        release="${work}/release.json"
        if [ ! -f "$release" ]; then
            if [ -n "$version" ]; then api="${api}/tags/v${version}"; else api="${api}/latest"; fi
            curl "${net[@]}" --output "$release" \
                --header "Authorization: Bearer $(token)" \
                --header "Accept: application/vnd.github+json" "$api" ||
                fail "cannot read the release from ${api} (is the token allowed to read ${repo}?)"
        fi
        url="$(asset_api_url "$name" "$release")"
        [ -n "$url" ] || fail "the release has no asset ${name}"
        curl "${net[@]}" --output "$dest" \
            --header "Authorization: Bearer $(token)" \
            --header "Accept: application/octet-stream" "$url" ||
            fail "download failed: ${name} from ${url}"
    else
        if [ -n "$version" ]; then
            url="https://github.com/${repo}/releases/download/v${version}/${name}"
        else
            url="https://github.com/${repo}/releases/latest/download/${name}"
        fi
        curl "${net[@]}" --output "$dest" "$url" || fail "download failed: ${url}
  If the repository is private, download through the API with a GitHub token:
  curl -fsSL ... | GH_TOKEN=\"\$(gh auth token)\" bash"
    fi
}

# Fetches NAME and its .sha256 into the work directory and checks one against the other.
fetch_verified() {
    local name="$1" expected actual
    fetch "$name" "${work}/${name}"
    fetch "${name}.sha256" "${work}/${name}.sha256"
    expected="$(cut -d ' ' -f 1 <"${work}/${name}.sha256")"
    actual="$(sha256_of "${work}/${name}")"
    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        fail "checksum mismatch for ${name} (expected ${expected}, got ${actual})"
    fi
    say "checksum ok: ${name} (${actual})"
}

# Puts FILE in place as DEST: copied beside it, then renamed over it, so a running daemon keeps
# its old inode and a failed copy never leaves a half-written binary behind.
place_binary() {
    local file="$1" dest="$2"
    mkdir -p "$(dirname "$dest")"
    cp "$file" "$(dirname "$dest")/.nomouse.new"
    chmod 755 "$(dirname "$dest")/.nomouse.new"
    mv -f "$(dirname "$dest")/.nomouse.new" "$dest"
}

path_hint() {
    case ":${PATH}:" in
        *":$1:"*) ;;
        *) step "add $1 to PATH, e.g. in ~/.profile or ~/.zshrc: export PATH=\"$1:\$PATH\"" ;;
    esac
}

# The bundle identifier of the .app at $1, or nothing.
bundle_id_of() {
    /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$1/Contents/Info.plist" 2>/dev/null || true
}

# The folder nomouse.app goes into on macOS.
app_dir() {
    if [ -n "${NOMOUSE_APP_DIR:-}" ]; then
        echo "${NOMOUSE_APP_DIR%/}"
    elif [ -w /Applications ]; then
        echo /Applications
    else
        echo "${HOME}/Applications"
    fi
}

install_macos() {
    local target="$1" bin_dir="$2" archive apps app
    archive="$(archive_for "$target")"
    fetch_verified "$archive"
    ditto -x -k "${work}/${archive}" "${work}/unpacked"
    [ -x "${work}/unpacked/nomouse.app/Contents/MacOS/nomouse" ] ||
        fail "${archive} has no nomouse.app/Contents/MacOS/nomouse"
    [ "$(bundle_id_of "${work}/unpacked/nomouse.app")" = "$bundle_id" ] ||
        fail "${archive} holds an app that is not ${bundle_id}"

    apps="$(app_dir)"
    app="${apps}/nomouse.app"
    mkdir -p "$apps"
    if [ -e "$app" ] && [ "$(bundle_id_of "$app")" != "$bundle_id" ]; then
        fail "${app} exists and is not nomouse (${bundle_id}); move it away first"
    fi
    # The release is not notarized yet, so drop the quarantine flag a browser download would set.
    xattr -dr com.apple.quarantine "${work}/unpacked/nomouse.app" 2>/dev/null || true
    rm -rf "${apps}/.nomouse.app.new"
    ditto "${work}/unpacked/nomouse.app" "${apps}/.nomouse.app.new"
    rm -rf "$app"
    mv "${apps}/.nomouse.app.new" "$app"

    mkdir -p "$bin_dir"
    ln -sfn "${app}/Contents/MacOS/nomouse" "${bin_dir}/nomouse"

    say "installed $("${bin_dir}/nomouse" --version) to ${app}"
    say "the nomouse command: ${bin_dir}/nomouse -> ${app}/Contents/MacOS/nomouse"
    echo
    say "next steps:"
    path_hint "$bin_dir"
    step "1. open \"${app}\""
    step "   nomouse asks for Accessibility: System Settings > Privacy & Security >"
    step "   Accessibility, switch on nomouse. It notices the switch and restarts by itself."
    step "2. press ctrl+space in any app and type a label to click it."
    step "3. start at login: System Settings > General > Login Items > + > nomouse."
    step "check everything at any time with: nomouse doctor"
}

install_linux() {
    local target="$1" bin_dir="$2" archive
    command -v tar >/dev/null 2>&1 || fail "tar is required"
    archive="$(archive_for "$target")"
    fetch_verified "$archive"
    tar -xzf "${work}/${archive}" -C "$work"
    [ -f "${work}/nomouse-${target}/nomouse" ] || fail "${archive} has no nomouse-${target}/nomouse"
    place_binary "${work}/nomouse-${target}/nomouse" "${bin_dir}/nomouse"

    say "installed $("${bin_dir}/nomouse" --version) to ${bin_dir}/nomouse"
    echo
    say "next steps:"
    path_hint "$bin_dir"
    step "1. nomouse setup     (turns on the accessibility bus, explains anything missing)"
    case "${XDG_SESSION_TYPE:-}" in
        wayland)
            step "2. Wayland: let your compositor start nomouse and bind its modes, e.g. Sway:"
            step "     exec nomouse daemon"
            step "     bindsym Ctrl+space exec nomouse hints"
            step "   Hyprland: exec-once = nomouse daemon / bind = CTRL, space, exec, nomouse hints"
            if [ -e /dev/uinput ] && [ ! -w /dev/uinput ]; then
                step "   KDE Plasma also needs write access to /dev/uinput:"
                step "   https://github.com/${repo}/blob/nightly/docs/install.md#linux-wayland"
            fi
            ;;
        *)
            step "2. nomouse           (starts the daemon; ctrl+space shows the labels)"
            step "3. start at login: add \"${bin_dir}/nomouse daemon\" to your desktop's"
            step "   startup applications"
            ;;
    esac
    step "check everything at any time with: nomouse doctor"
}

install_release() {
    local bin_dir="$1" target
    target="$(asset_for "$(uname -s)" "$(uname -m)")"
    if [ -z "$from" ]; then
        command -v curl >/dev/null 2>&1 || fail "curl is required"
    fi
    work="$(mktemp -d)"
    # shellcheck disable=SC2064 # expand now: the path is fixed for this run
    trap "rm -rf '${work}'" EXIT

    if [ -n "$from" ]; then
        say "installing from ${from}"
    elif [ -n "$version" ]; then
        say "installing nomouse ${version} for ${target}"
    else
        say "installing the latest nomouse for ${target}"
    fi
    case "$target" in
        *-apple-darwin) install_macos "$target" "$bin_dir" ;;
        *) install_linux "$target" "$bin_dir" ;;
    esac
}

uninstall() {
    local bin_dir="$1" purge="$2" dir app link
    if [ -L "${bin_dir}/nomouse" ]; then
        link="$(readlink "${bin_dir}/nomouse")"
        case "$link" in
            */nomouse.app/Contents/MacOS/nomouse)
                app="${link%/Contents/MacOS/nomouse}"
                if [ -d "$app" ] && [ "$(bundle_id_of "$app")" = "$bundle_id" ]; then
                    rm -rf "$app"
                    say "removed ${app}"
                fi
                ;;
        esac
    fi
    if [ -e "${bin_dir}/nomouse" ] || [ -L "${bin_dir}/nomouse" ]; then
        rm -f "${bin_dir}/nomouse"
        say "removed ${bin_dir}/nomouse"
    else
        say "nomouse is not installed in ${bin_dir}"
    fi
    dir="$(config_dir)"
    if [ "$purge" = true ]; then
        if [ -e "$dir" ]; then
            say "--purge deletes: ${dir}"
            rm -rf "$dir"
            say "removed ${dir}"
        else
            say "--purge: no config directory at ${dir}"
        fi
    elif [ -d "$dir" ]; then
        say "kept the config in ${dir} (add --purge to delete it)"
    fi
}

from=""
version=""
work=""

main() {
    local action=install purge=false bin_dir
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version)
                [ "$#" -ge 2 ] || fail "--version needs a value"
                version="${2#v}"
                shift 2
                ;;
            --from)
                [ "$#" -ge 2 ] || fail "--from needs a directory"
                [ -d "$2" ] || fail "--from: no directory $2"
                from="$(cd "$2" && pwd)"
                shift 2
                ;;
            --uninstall)
                action=uninstall
                shift
                ;;
            --purge)
                purge=true
                shift
                ;;
            -h | --help)
                usage
                return 0
                ;;
            *) fail "unknown option $1 (see --help)" ;;
        esac
    done
    if [ -n "$version" ] && ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        fail "--version wants MAJOR.MINOR.PATCH, got ${version}"
    fi
    if [ "$purge" = true ] && [ "$action" != uninstall ]; then
        fail "--purge only goes with --uninstall"
    fi
    [ -n "${HOME:-}" ] || fail "HOME is not set"
    bin_dir="${NOMOUSE_INSTALL_DIR:-${HOME}/.local/bin}"

    if [ "$action" = uninstall ]; then
        uninstall "$bin_dir" "$purge"
    else
        install_release "$bin_dir"
    fi
}

# Tests source this file with NOMOUSE_INSTALL_NO_MAIN=1 to call its functions one by one.
if [ "${NOMOUSE_INSTALL_NO_MAIN:-}" != 1 ]; then
    main "$@"
fi
