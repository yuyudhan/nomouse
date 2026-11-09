#!/usr/bin/env bash
# Installs, updates or uninstalls nomouse on Linux and macOS, without sudo. Published with every
# GitHub release as releases/latest/download/install.sh; README.md#install has the one-line curl
# command that pipes it into bash.
#
# Piping is safe: all work happens in main, called on the last line, so bash has read the whole
# script before anything runs. Options go after `bash -s --`.
#
# It downloads the release archive for this OS and CPU, checks it against the release's .sha256,
# and puts the `nomouse` binary in ~/.local/bin (or NOMOUSE_INSTALL_DIR). On macOS the archive
# holds one universal binary for Apple silicon and Intel.
#
# Options:
#   --version X.Y.Z    Install that release instead of the latest.
#   --uninstall        Remove the binary; keep the config file.
#   --purge            With --uninstall, also delete the nomouse config directory.
#   -h, --help         Show this help.
#
# Environment:
#   NOMOUSE_INSTALL_DIR    Where the binary goes (default ~/.local/bin).
#   NOMOUSE_INSTALL_BASE   URL of the directory holding the release assets, overriding GitHub
#                          (for mirrors and for testing a locally built release).
set -euo pipefail

repo="yuyudhan/nomouse"
# Every request: retry transient failures, give up on a connection that never opens, and abort
# a transfer that stalls (under 1 KB/s for 30 s) instead of hanging.
net=(--fail --silent --show-error --location --connect-timeout 15 --retry 3 --retry-delay 2
    --speed-limit 1024 --speed-time 30)

say() {
    printf 'nomouse: %s\n' "$*"
}

fail() {
    printf 'nomouse: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Install, update or uninstall nomouse (Linux and macOS, no sudo).

Options (after "bash -s --" when piped):
  --version X.Y.Z    install that release instead of the latest
  --uninstall        remove the binary, keep the config
  --purge            with --uninstall, also delete the config directory

Environment: NOMOUSE_INSTALL_DIR (default ~/.local/bin), NOMOUSE_INSTALL_BASE (asset URL base).
EOF
}

# The release asset name for this machine.
asset_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os" in
        Linux)
            case "$arch" in
                x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
                aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
                *) fail "no Linux build for ${arch}; supported: x86_64, aarch64" ;;
            esac
            ;;
        Darwin) echo "universal-apple-darwin" ;;
        MINGW* | MSYS* | CYGWIN*)
            fail "on Windows run install.ps1 in PowerShell instead (see the README)"
            ;;
        *) fail "unsupported OS ${os}; nomouse runs on Linux, macOS and Windows" ;;
    esac
}

# The same rule as nomouse's own config path on Linux and macOS.
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

download() {
    curl "${net[@]}" --output "$2" "$1" || fail "download failed: $1"
}

install_release() {
    local version="$1" bin_dir="$2" target base archive tmp expected actual
    target="$(asset_target)"
    if [ -n "${NOMOUSE_INSTALL_BASE:-}" ]; then
        base="${NOMOUSE_INSTALL_BASE%/}"
    elif [ -n "$version" ]; then
        base="https://github.com/${repo}/releases/download/v${version}"
    else
        base="https://github.com/${repo}/releases/latest/download"
    fi
    archive="nomouse-${target}.tar.gz"

    command -v curl >/dev/null 2>&1 || fail "curl is required"
    command -v tar >/dev/null 2>&1 || fail "tar is required"
    tmp="$(mktemp -d)"
    # shellcheck disable=SC2064 # expand now: tmp is local and gone when the trap fires
    trap "rm -rf '${tmp}'" EXIT

    say "downloading ${archive} from ${base}"
    download "${base}/${archive}" "${tmp}/${archive}"
    download "${base}/${archive}.sha256" "${tmp}/${archive}.sha256"
    expected="$(cut -d ' ' -f 1 <"${tmp}/${archive}.sha256")"
    actual="$(sha256_of "${tmp}/${archive}")"
    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        fail "checksum mismatch for ${archive} (expected ${expected}, got ${actual})"
    fi
    say "checksum ok (${actual})"

    tar -xzf "${tmp}/${archive}" -C "$tmp"
    [ -f "${tmp}/nomouse-${target}/nomouse" ] || fail "archive has no nomouse-${target}/nomouse"

    mkdir -p "$bin_dir"
    # Copy beside the old binary, then rename over it: a running daemon keeps its old inode and
    # a failed copy never leaves a half-written binary behind.
    cp "${tmp}/nomouse-${target}/nomouse" "${bin_dir}/.nomouse.new"
    chmod 755 "${bin_dir}/.nomouse.new"
    if [ "$(uname -s)" = "Darwin" ]; then
        xattr -d com.apple.quarantine "${bin_dir}/.nomouse.new" 2>/dev/null || true
    fi
    mv -f "${bin_dir}/.nomouse.new" "${bin_dir}/nomouse"

    say "installed $("${bin_dir}/nomouse" --version) to ${bin_dir}/nomouse"
    case ":${PATH}:" in
        *":${bin_dir}:"*) ;;
        *) say "add ${bin_dir} to PATH, e.g. in ~/.profile: export PATH=\"${bin_dir}:\$PATH\"" ;;
    esac
    if [ "$(uname -s)" = "Darwin" ]; then
        say "grant Accessibility and Input Monitoring to your terminal (or ${bin_dir}/nomouse)"
        say "in System Settings > Privacy & Security, then run: nomouse doctor"
    else
        say "next: nomouse doctor (checks the accessibility bus and the display)"
    fi
}

uninstall() {
    local bin_dir="$1" purge="$2" dir
    if [ -e "${bin_dir}/nomouse" ]; then
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

main() {
    local version="" action=install purge=false bin_dir
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version)
                [ "$#" -ge 2 ] || fail "--version needs a value"
                version="${2#v}"
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
        install_release "$version" "$bin_dir"
    fi
}

main "$@"
