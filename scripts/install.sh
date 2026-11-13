#!/bin/sh
# Installs, updates or uninstalls nomouse on macOS and Linux for the current user, without sudo:
#
#   curl -fsSL https://yuyudhan.github.io/nomouse/install | sh
#   curl -fsSL https://yuyudhan.github.io/nomouse/install | sh -s -- --uninstall
#
# That URL serves this file byte for byte: it is the latest release's install.sh
# (releases/latest/download/install.sh), so `sh install.sh [options]` works the same.
#
# POSIX sh: it runs piped into sh (dash on Debian and Ubuntu, bash on macOS), bash and zsh.
# Everything happens in functions and the last line calls main, so a download cut short runs
# nothing. Every child that could read stdin (the script itself, when piped) gets /dev/null.
# Downloads go into one private `mktemp -d` directory under TMPDIR, removed on every exit
# (success, an error, Ctrl-C, HUP, TERM); nothing is written to the current directory.
#
# Each asset is checked against the release's SHA256SUMS (`--from DIR`: DIR/SHA256SUMS, or
# DIR/<asset>.sha256 without it), and the downloaded nomouse runs once (`--version`) before
# anything is replaced, so a build this system cannot start leaves an installed copy as it was.
#   macOS (Apple silicon and Intel): nomouse.app into /Applications (~/Applications when that
#     is not writable), quarantine removed, and ~/.local/bin/nomouse linked into the app.
#     Updating quits the running copy first and starts the new one after. The app is re-signed
#     with a self-signed certificate the installer creates once in its own keychain file
#     (~/Library/Keychains/nomouse-signing.keychain-db), so its designated requirement names
#     that certificate, not the build's hash, and macOS keeps the Accessibility grant across
#     updates; NOMOUSE_SIGN_IDENTITY=NAME signs with another identity, =none keeps the
#     release's signature, and a release already signed with a Developer ID is left as it is.
#     When the code signature still changed it removes the old copy's Accessibility entry,
#     which macOS would keep showing switched on while ignoring it.
#   Linux (x86_64, aarch64; glibc 2.35 or later): nomouse and nomouse-settings into
#     ~/.local/bin, the app launcher entry and its icons into ${XDG_DATA_HOME:-~/.local/share}.
#     Systems the build cannot run on (older glibc, musl, no glibc loader as on NixOS) are
#     refused before anything is downloaded.
#
# Output: a header, one `✓` line per step, `nomouse X.Y.Z is installed.` and the next steps on
# stdout; a failure is one `nomouse: error: <what happened>; <the fix>` line on stderr and exit
# status 1. Colour only on a terminal, without NO_COLOR, and when TERM is not dumb.
set -eu

repo="yuyudhan/nomouse"
bundle_id="com.yuyudhan.nomouse"
# The Linux desktop entry's id and icon name.
app_id="io.github.yuyudhan.nomouse"
releases_url="https://github.com/${repo}/releases"
source_build_url="https://github.com/${repo}/blob/nightly/docs/install.md#building-from-source"
# The Linux release is built on Ubuntu 22.04, so it needs that glibc or a later one.
min_glibc="2.35"

# Set by main and install_release.
from=""
version=""
work=""
retry_all=""
# The next steps, one indented line each, printed under "Next:" at the end.
todo=""
# Escape sequences; empty unless the stream is a colour terminal (set_colours).
bold=""
green=""
dim=""
red=""
reset=""
ereset=""

set_colours() {
    esc="$(printf '\033')"
    if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != dumb ]; then
        bold="${esc}[1m"
        green="${esc}[32m"
        dim="${esc}[2m"
        reset="${esc}[0m"
    fi
    if [ -t 2 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != dumb ]; then
        red="${esc}[31m"
        ereset="${esc}[0m"
    fi
}

header() {
    printf '%snomouse installer%s\n' "$bold" "$reset"
}

# step TEXT [DETAIL]: one done step; DETAIL is dimmed.
step() {
    if [ "$#" -ge 2 ]; then
        printf '  %s✓%s %s %s%s%s\n' "$green" "$reset" "$1" "$dim" "$2" "$reset"
    else
        printf '  %s✓%s %s\n' "$green" "$reset" "$1"
    fi
}

# next TEXT: adds a line to the next steps.
next() {
    todo="${todo}  ${1}
"
}

# finish SENTENCE: the closing line, then the next steps if there are any.
finish() {
    printf '%s%s%s\n' "$bold" "$1" "$reset"
    if [ -n "$todo" ]; then
        printf '\nNext:\n%s' "$todo"
    fi
}

fail() {
    printf 'nomouse: %serror%s: %s\n' "$red" "$ereset" "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
nomouse installer: installs, updates or uninstalls nomouse for your user on macOS and Linux
(no sudo).

Usage (options go after "sh -s --" when the script is piped):
  curl -fsSL https://yuyudhan.github.io/nomouse/install \
      | sh -s -- [options]
  sh install.sh [options]

Options:
  --version X.Y.Z   install that release instead of the latest (vX.Y.Z works too)
  --from DIR        install from a folder holding the release assets (no network)
  --uninstall       remove nomouse, keep the config
  --purge           with --uninstall, also delete the config directory
  -h, --help        show this help

Environment:
  NOMOUSE_VERSION         the release to install (--version wins)
  NOMOUSE_INSTALL_DIR     where the nomouse command goes (default ~/.local/bin)
  NOMOUSE_APP_DIR         macOS: where nomouse.app goes (default /Applications)
  NOMOUSE_INSTALL_BASE    a URL holding the release assets, for mirrors and local builds
  GH_TOKEN, GITHUB_TOKEN  download through the GitHub API with this token
  NOMOUSE_API_BASE        the API for those downloads (default https://api.github.com)
  NO_COLOR                no colours
  NOMOUSE_SIGN_IDENTITY   macOS: the code-signing identity for nomouse.app (default: a
                          self-signed "nomouse local signing", created once; none keeps the
                          release's own signature)
EOF
}

# PATH shown with ~ for HOME.
tilde() {
    case "$1" in
    "${HOME}"/*) printf '~%s' "${1#"$HOME"}" ;;
    *) printf '%s' "$1" ;;
    esac
}

# The release target for an OS and CPU, as `uname -s` and `uname -m` print them.
asset_for() {
    case "$1" in
    Linux)
        case "$2" in
        x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
        *) fail "no nomouse build for Linux $2; build from source: ${source_build_url}" ;;
        esac
        ;;
    Darwin)
        case "$2" in
        arm64 | x86_64) echo "universal-apple-darwin" ;;
        *) fail "no nomouse build for macOS $2; build from source: ${source_build_url}" ;;
        esac
        ;;
    MINGW* | MSYS* | CYGWIN*)
        fail "this is Windows ($1), which install.sh does not cover; run install.ps1 in PowerShell instead: irm ${releases_url}/latest/download/install.ps1 | iex"
        ;;
    *) fail "no nomouse build for $1 $2; build from source: ${source_build_url}" ;;
    esac
}

# The archive this target installs from.
archive_for() {
    case "$1" in
    *-apple-darwin) echo "nomouse-$1.app.zip" ;;
    *) echo "nomouse-$1.tar.gz" ;;
    esac
}

# The system as the first step names it: "macOS (Apple silicon)", "Linux (x86_64)". On macOS
# sysctl sees through Rosetta, which makes `uname -m` say x86_64 on Apple silicon.
platform_label() {
    case "$1" in
    Darwin)
        if [ "$(sysctl -n hw.optional.arm64 </dev/null 2>/dev/null || true)" = 1 ]; then
            echo "macOS (Apple silicon)"
        else
            echo "macOS (Intel)"
        fi
        ;;
    *) echo "Linux ($2)" ;;
    esac
}

# The API URL of release asset NAME in the GitHub release JSON in FILE, or nothing. Each asset
# object lists its "url" (…/releases/assets/ID) before its "name"; uploader URLs never match.
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
    ' "$2" </dev/null
}

# The hash for NAME in the sha256sum-format FILE ("<hash>  <name>"), or nothing.
listed_sha256() {
    awk -v want="$1" '$2 == want || $2 == "*" want { print $1; exit }' "$2" </dev/null
}

# The same rule as nomouse's own config path on macOS and Linux.
config_dir() {
    echo "${XDG_CONFIG_HOME:-${HOME}/.config}/nomouse"
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" </dev/null | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" </dev/null | cut -d ' ' -f 1
    else
        fail "found neither sha256sum nor shasum to verify the download; install one (coreutils or perl) and run the command again"
    fi
}

token() {
    printf '%s' "${GH_TOKEN:-${GITHUB_TOKEN:-}}"
}

# Every request: no progress output, follow redirects, retry failures, give up on a connection
# that never opens, and abort a transfer that stalls (under 1 KB/s for 30 s). --retry alone
# retries only timeouts and HTTP 408/429/5xx; with curl 7.71+ --retry-all-errors retries a
# connection the network cuts too. No --fail: download judges the HTTP status itself, so a 404
# is reported at once, not retried.
net() {
    curl --silent --show-error --location --connect-timeout 15 --retry 5 --retry-delay 2 \
        --speed-limit 1024 --speed-time 30 ${retry_all:+"$retry_all"} "$@" </dev/null
}

# download URL DEST WHAT [CURL OPTIONS...]: fetches URL into DEST and fails with one line unless
# the server answered 200 (a file:// URL has no status: curl reports 000). WHAT names what a
# 404 means: "" for the URL itself, or "release vX.Y.Z", with an optional ", reason" suffix.
download() {
    dl_url="$1"
    dl_dest="$2"
    dl_what="$3"
    shift 3
    if ! dl_code="$(net --stderr "${dl_dest}.err" --write-out '%{http_code}' \
        --output "$dl_dest" "$@" "$dl_url")"; then
        dl_reason="$(sed -n 's/^curl: ([0-9]*) //p' "${dl_dest}.err" | tail -n 1)"
        dl_host="${dl_url#*://}"
        dl_host="${dl_host%%/*}"
        fail "could not reach ${dl_host:-$dl_url} (${dl_reason:-no answer}); check the network or proxy and run the command again"
    fi
    rm -f "${dl_dest}.err"
    case "${dl_code}:${dl_url%%://*}" in
    200:* | 000:file) ;;
    404:*)
        case "$dl_what" in
        release\ *)
            fail "${dl_what%%,*} not found (HTTP 404${dl_what#"${dl_what%%,*}"}); the versions are at ${releases_url}"
            ;;
        *) fail "release not found: ${dl_url} (HTTP 404${dl_what}); the versions are at ${releases_url}" ;;
        esac
        ;;
    *) fail "download failed: ${dl_url} (HTTP ${dl_code}); run the command again later" ;;
    esac
}

# Fetches release asset NAME into DEST from wherever this run installs from: the --from
# directory, NOMOUSE_INSTALL_BASE, the GitHub API (with a token) or the public download URL.
fetch() {
    fe_name="$1"
    fe_dest="$2"
    if [ -n "$from" ]; then
        [ -f "${from}/${fe_name}" ] ||
            fail "${from} has no ${fe_name}; put every asset of the release in it and run the command again"
        cp "${from}/${fe_name}" "$fe_dest"
    elif [ -n "${NOMOUSE_INSTALL_BASE:-}" ]; then
        download "${NOMOUSE_INSTALL_BASE%/}/${fe_name}" "$fe_dest" ""
    elif [ -n "$(token)" ]; then
        fe_api="${NOMOUSE_API_BASE:-https://api.github.com}"
        fe_api="${fe_api%/}/repos/${repo}/releases"
        fe_release="${work}/release.json"
        if [ ! -f "$fe_release" ]; then
            # GitHub answers 404, not 401, when the token may not read the repository.
            if [ -n "$version" ]; then
                download "${fe_api}/tags/v${version}" "$fe_release" \
                    "release v${version}, or the token cannot read ${repo}" \
                    --header "Authorization: Bearer $(token)" \
                    --header "Accept: application/vnd.github+json"
            else
                download "${fe_api}/latest" "$fe_release" ", or the token cannot read ${repo}" \
                    --header "Authorization: Bearer $(token)" \
                    --header "Accept: application/vnd.github+json"
            fi
        fi
        fe_url="$(asset_api_url "$fe_name" "$fe_release")"
        [ -n "$fe_url" ] ||
            fail "release not found: the release has no asset ${fe_name}; the versions are at ${releases_url}"
        download "$fe_url" "$fe_dest" "" \
            --header "Authorization: Bearer $(token)" \
            --header "Accept: application/octet-stream"
    elif [ -n "$version" ]; then
        download "${releases_url}/download/v${version}/${fe_name}" "$fe_dest" "release v${version}"
    else
        download "${releases_url}/latest/download/${fe_name}" "$fe_dest" ""
    fi
}

# Fetches asset NAME into the work directory and checks it against the release's SHA256SUMS
# (from a --from folder without one: NAME.sha256).
fetch_verified() {
    fv_name="$1"
    fetch "$fv_name" "${work}/${fv_name}"
    if [ -n "$from" ] && [ ! -f "${from}/SHA256SUMS" ]; then
        fv_sums="${fv_name}.sha256"
        fetch "$fv_sums" "${work}/${fv_sums}"
        fv_expected="$(awk '{ print $1; exit }' "${work}/${fv_sums}" </dev/null)"
    else
        fv_sums="SHA256SUMS"
        fetch SHA256SUMS "${work}/SHA256SUMS"
        fv_expected="$(listed_sha256 "$fv_name" "${work}/SHA256SUMS")"
    fi
    fv_actual="$(sha256_of "${work}/${fv_name}")"
    if [ -z "$fv_expected" ] || [ "$fv_expected" != "$fv_actual" ]; then
        fail "checksum mismatch for ${fv_name} (expected ${fv_expected:-no ${fv_sums} line}, got ${fv_actual}); nothing was installed, run the command again"
    fi
    if [ -n "$from" ]; then
        step "Copied and verified" "(SHA-256 from ${fv_sums})"
    else
        step "Downloaded and verified" "(SHA-256 from ${fv_sums})"
    fi
}

# Puts FILE in place as DEST: copied beside it, then renamed over it, so a running daemon keeps
# its old inode and a failed copy never leaves a half-written binary behind.
place_binary() {
    pb_new="$(dirname "$2")/.$(basename "$2").new"
    mkdir -p "$(dirname "$2")"
    cp "$1" "$pb_new"
    chmod 755 "$pb_new"
    mv -f "$pb_new" "$2"
}

# The `--version` line of the downloaded program at $1, run before anything is replaced: a build
# this system cannot start (too old a glibc, a missing library or loader) fails the install with
# the loader's own message, HINT ($2, optional) after it, and leaves an installed copy as it was.
runnable_version() {
    rv_hint="${2:-}"
    rv_code=0
    rv_out="$("$1" --version </dev/null 2>"${work}/version.err")" || rv_code=$?
    if [ "$rv_code" -ne 0 ]; then
        rv_reason="$(tr '\n' ' ' <"${work}/version.err" | sed 's/ *$//')"
        [ -n "$rv_reason" ] || rv_reason="it exited with status ${rv_code}"
        fail "the downloaded nomouse does not run on this system (${rv_reason}); nothing was installed${rv_hint:+, and ${rv_hint}}"
    fi
    printf '%s\n' "$rv_out"
}

# True when glibc version $1 (MAJOR.MINOR) is $2 or later.
version_at_least() {
    va_have_major="${1%%.*}"
    va_have_minor="${1#*.}"
    va_have_minor="${va_have_minor%%.*}"
    va_want_major="${2%%.*}"
    va_want_minor="${2#*.}"
    va_want_minor="${va_want_minor%%.*}"
    [ "$va_have_major" -gt "$va_want_major" ] ||
        { [ "$va_have_major" -eq "$va_want_major" ] && [ "$va_have_minor" -ge "$va_want_minor" ]; }
}

# ldd's --version text (glibc prints it on stdout, musl on stderr with status 1), or nothing
# without ldd.
ldd_version_text() {
    command -v ldd >/dev/null 2>&1 || return 0
    ldd --version </dev/null 2>&1 || true
}

# This system's glibc version (MAJOR.MINOR), or nothing when neither getconf nor ldd names one
# (musl, or a system without either).
glibc_version() {
    gv="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"
    gv="$(printf '%s\n' "$gv" | sed -n '1s/^glibc \([0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')"
    if [ -z "$gv" ]; then
        # "ldd (GNU libc) 2.35", "ldd (Ubuntu GLIBC 2.35-0ubuntu3.8) 2.35".
        gv="$(ldd_version_text | sed -n '1s/^ldd .*) \([0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')"
    fi
    printf '%s' "$gv"
}

# The program loader the glibc build of TARGET starts through.
glibc_loader_for() {
    case "$1" in
    x86_64-*) echo /lib64/ld-linux-x86-64.so.2 ;;
    aarch64-*) echo /lib/ld-linux-aarch64.so.1 ;;
    esac
}

# True when this system's C library is musl: ldd says so, or ROOT ($1) has a musl loader and no
# glibc one ($2). (No glob: zsh fails on one that matches nothing.)
musl_system() {
    case "$(ldd_version_text)" in
    *musl*) return 0 ;;
    esac
    [ -e "${1}${2}" ] && return 1
    case "$(ls "${1}/lib" 2>/dev/null || true)" in
    *ld-musl-*) return 0 ;;
    esac
    return 1
}

# Why the Linux build for TARGET cannot run on this system, or nothing when it can (or this
# cannot tell; the downloaded binary's own --version run then has the last word). ROOT prefixes
# the loader paths: "" for this system, a fake root in tests.
linux_libc_problem() {
    lp_loader="$(glibc_loader_for "$1")"
    if musl_system "$2" "$lp_loader"; then
        echo "this system uses musl libc (Alpine, Void musl and others) and nomouse's Linux build needs glibc ${min_glibc} or later; build nomouse from source: ${source_build_url}"
    elif [ ! -e "${2}${lp_loader}" ]; then
        echo "this system has no glibc program loader at ${lp_loader} (NixOS and Guix keep it elsewhere), so nomouse's Linux build cannot start; on NixOS turn on nix-ld (programs.nix-ld.enable = true) and run the command again, or build nomouse from source: ${source_build_url}"
    else
        lp_have="$(glibc_version)"
        if [ -n "$lp_have" ] && ! version_at_least "$lp_have" "$min_glibc"; then
            echo "nomouse's Linux build needs glibc ${min_glibc} or later (Ubuntu 22.04, Debian 12, Fedora 36, RHEL 10 or newer) and this system has glibc ${lp_have}; build nomouse from source: ${source_build_url}"
        fi
    fi
}

# The shared libraries the program at $1 needs and this system lacks, one per line (ldd's
# "not found" lines); nothing when all are there or ldd is missing.
missing_libraries() {
    command -v ldd >/dev/null 2>&1 || return 0
    { ldd "$1" </dev/null 2>/dev/null || true; } | awk '/not found/ { print $1 }'
}

# How to install GTK 4 and libadwaita, the settings window's libraries, on this distribution.
gtk_install_hint() {
    if command -v apt-get >/dev/null 2>&1; then
        echo "sudo apt install libgtk-4-1 libadwaita-1-0"
    elif command -v dnf >/dev/null 2>&1; then
        echo "sudo dnf install gtk4 libadwaita"
    elif command -v pacman >/dev/null 2>&1; then
        echo "sudo pacman -S gtk4 libadwaita"
    elif command -v zypper >/dev/null 2>&1; then
        echo "sudo zypper install libgtk-4-1 libadwaita-1-0"
    else
        echo "GTK 4 (4.6 or later) and libadwaita (1.1 or later) from your package manager"
    fi
}

# Adds the PATH line to the next steps when BIN_DIR is not on PATH.
path_hint() {
    case ":${PATH}:" in
    *":$1:"*) ;;
    *)
        case "$1" in
        "${HOME}"/*) ph_dir="\$HOME${1#"$HOME"}" ;;
        *) ph_dir="$1" ;;
        esac
        next "Add $(tilde "$1") to PATH (in ~/.zshrc or ~/.profile): export PATH=\"${ph_dir}:\$PATH\""
        ;;
    esac
}

# The bundle identifier of the .app at $1, or nothing.
bundle_id_of() {
    /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$1/Contents/Info.plist" </dev/null 2>/dev/null || true
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

# Where macOS lists the apps allowed to control the computer, as System Settings names it:
# macOS 27 renamed the Accessibility list "Device Control and Data Access".
accessibility_pane() {
    ap_major="$(sw_vers -productVersion </dev/null 2>/dev/null || true)"
    ap_major="${ap_major%%.*}"
    case "$ap_major" in
    "" | *[!0-9]*) ap_major=0 ;;
    esac
    if [ "$ap_major" -ge 27 ]; then
        echo "System Settings → Privacy & Security → Device Control and Data Access"
    else
        echo "System Settings → Privacy & Security → Accessibility"
    fi
}

# The designated requirement of the app at $1, which macOS pins an Accessibility grant to (for
# an ad-hoc signed build, its exact code hash), or nothing when it is unsigned.
requirement_of() {
    codesign -d -r- "$1" </dev/null 2>/dev/null || true
}

# The process IDs of programs running from the app at $1, one per line: those whose command
# line has the app's executable as a word.
app_pids() {
    pi_exe="$1/Contents/MacOS/nomouse"
    ps -axo pid=,command= | while read -r pi_pid pi_command; do
        case " ${pi_command} " in
        *" ${pi_exe} "*) echo "$pi_pid" ;;
        esac
    done
}

# Waits up to $2 tenths of a second for every nomouse running from the app at $1 to end.
app_gone_within() {
    ag_tries=0
    while [ "$ag_tries" -lt "$2" ]; do
        [ -n "$(app_pids "$1")" ] || return 0
        sleep 0.1
        ag_tries=$((ag_tries + 1))
    done
    return 1
}

# Quits the nomouse running from the app at $1, the copy an update replaces: asks it over its
# command socket (`nomouse quit`), then, if it is still running after 5 s, terminates it.
quit_app() {
    "$1/Contents/MacOS/nomouse" quit </dev/null >/dev/null 2>&1 || true
    app_gone_within "$1" 50 && return 0
    app_pids "$1" | xargs kill 2>/dev/null || true
    app_gone_within "$1" 20 && return 0
    next "Quit the old nomouse from its menu bar icon's menu: it is still running the old code"
}

# Removes nomouse's entry from macOS's Accessibility list. After an update with a different code
# signature that entry stays switched on but belongs to the old copy: macOS ignores it for the
# new one, and switching it off and on again does not help.
reset_accessibility() {
    if tccutil reset Accessibility "$bundle_id" </dev/null >/dev/null 2>&1; then
        step "Removed the old copy's Accessibility entry" "(macOS ties it to the old build)"
    else
        next "If nomouse shows switched on but still asks: select it in the list, remove it with − and add the new one"
    fi
}

# The keychain file holding the self-signed identity: nomouse's own, so the login keychain and
# the keychain search list stay untouched and nothing prompts, also in a headless session.
signing_keychain() {
    echo "${HOME}/Library/Keychains/nomouse-signing.keychain-db"
}

# Creates the self-signed code-signing certificate $1 in the keychain file $2, unless it is
# already there. The keychain has an empty password, so signing needs no prompt. Returns 1 when
# the identity cannot be had.
ensure_signing_identity() {
    si_name="$1"
    si_keychain="$2"
    if [ -f "$si_keychain" ]; then
        security unlock-keychain -p "" "$si_keychain" </dev/null >/dev/null 2>&1 || return 1
        if security find-identity -p codesigning "$si_keychain" </dev/null 2>/dev/null |
            grep -qF "\"${si_name}\""; then
            return 0
        fi
        rm -f "$si_keychain"
    fi
    command -v openssl >/dev/null 2>&1 || return 1
    step "Creating the signing identity \"${si_name}\"" "($(tilde "$si_keychain"))"
    si_dir="${work}/signing"
    rm -rf "$si_dir"
    mkdir -p "$si_dir" "$(dirname "$si_keychain")"
    chmod 700 "$si_dir"
    cat >"${si_dir}/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = ${si_name}
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF
    si_pass="$(openssl rand -hex 16 </dev/null)"
    if openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "${si_dir}/cert.cnf" \
        -keyout "${si_dir}/key.pem" -out "${si_dir}/cert.pem" </dev/null >/dev/null 2>&1 &&
        openssl pkcs12 -export -inkey "${si_dir}/key.pem" -in "${si_dir}/cert.pem" \
            -out "${si_dir}/id.p12" -passout "pass:${si_pass}" </dev/null >/dev/null 2>&1 &&
        security create-keychain -p "" "$si_keychain" </dev/null >/dev/null 2>&1 &&
        security set-keychain-settings "$si_keychain" </dev/null >/dev/null 2>&1 &&
        security unlock-keychain -p "" "$si_keychain" </dev/null >/dev/null 2>&1 &&
        security import "${si_dir}/id.p12" -k "$si_keychain" -P "$si_pass" -T /usr/bin/codesign \
            </dev/null >/dev/null 2>&1 &&
        security set-key-partition-list -S apple-tool:,apple: -s -k "" "$si_keychain" \
            </dev/null >/dev/null 2>&1; then
        rm -rf "$si_dir"
        return 0
    fi
    rm -rf "$si_dir"
    rm -f "$si_keychain"
    return 1
}

# Signs the app at $1 so its designated requirement names a certificate, not the build's own
# hash, and macOS keeps the Accessibility grant across updates and reinstalls. NOMOUSE_SIGN_IDENTITY
# names another identity (taken from the keychain search list) or none to keep the release's
# signature; a release already signed with a Developer ID is left as it is. A failure leaves the
# release's ad-hoc signature and says so under Next.
sign_app() {
    sa_app="$1"
    sa_identity="${NOMOUSE_SIGN_IDENTITY:-}"
    sa_keychain=""
    case "$sa_identity" in
    none) return 0 ;;
    "")
        case "$(requirement_of "$sa_app")" in
        "" | *cdhash*) ;;
        *) return 0 ;;
        esac
        sa_identity="nomouse local signing"
        sa_keychain="$(signing_keychain)"
        if ! ensure_signing_identity "$sa_identity" "$sa_keychain"; then
            next "nomouse could not create its signing identity, so it keeps the release's signature and macOS asks for Accessibility again after every update"
            return 0
        fi
        ;;
    esac
    if [ -n "$sa_keychain" ]; then
        codesign --force --keychain "$sa_keychain" --sign "$sa_identity" "$sa_app" \
            </dev/null >/dev/null 2>&1 || sa_identity=""
    else
        codesign --force --sign "$sa_identity" "$sa_app" </dev/null >/dev/null 2>&1 ||
            sa_identity=""
    fi
    if [ -z "$sa_identity" ]; then
        next "nomouse could not sign the app with its identity, so it keeps the release's signature and macOS asks for Accessibility again after every update"
        return 0
    fi
    step "Signed nomouse.app as \"${sa_identity}\"" "(Accessibility stays allowed across updates)"
}

# The next steps on macOS. APP; UPDATE: an installed copy was replaced; REGRANT: its
# Accessibility entry was removed; STARTED: the new copy was started.
macos_next_steps() {
    mn_pane="$(accessibility_pane)"
    if [ "$4" = true ]; then
        mn_start="The new nomouse is starting from \"$1\""
    else
        mn_start="Start it: open \"$1\""
    fi
    if [ "$3" = true ]; then
        next "Switch on Accessibility once more: ${mn_pane} → nomouse (the app's signature changed, so macOS treats it as a new program)"
    elif [ "$2" != true ]; then
        next "Switch on Accessibility: ${mn_pane} → nomouse"
    fi
    if [ "$3" = true ] || [ "$2" != true ]; then
        next "${mn_start}; its Welcome window opens that pane, and nomouse restarts once allowed"
    elif [ "$4" != true ]; then
        # An update that keeps the grant: the copy was not running, so start the new one.
        next "$mn_start"
    fi
    next "Then press ctrl+space in any app and type a label to click it"
    next "Settings and start at login: its menu bar icon's menu; check everything with: nomouse doctor"
}

install_macos() {
    archive="$(archive_for "$target")"
    fetch_verified "$archive"
    ditto -x -k "${work}/${archive}" "${work}/unpacked" </dev/null
    [ -x "${work}/unpacked/nomouse.app/Contents/MacOS/nomouse" ] ||
        fail "${archive} has no nomouse.app/Contents/MacOS/nomouse; report it at https://github.com/${repo}/issues"
    [ "$(bundle_id_of "${work}/unpacked/nomouse.app")" = "$bundle_id" ] ||
        fail "${archive} holds an app that is not ${bundle_id}; report it at https://github.com/${repo}/issues"

    apps="$(app_dir)"
    app="${apps}/nomouse.app"
    mkdir -p "$apps"
    if [ -e "$app" ] && [ "$(bundle_id_of "$app")" != "$bundle_id" ]; then
        fail "${app} exists and is not nomouse (${bundle_id}); move it away and run the command again"
    fi
    # The release is not notarized yet, so drop the quarantine flag a browser download would set.
    xattr -dr com.apple.quarantine "${work}/unpacked/nomouse.app" </dev/null 2>/dev/null || true
    runnable_version "${work}/unpacked/nomouse.app/Contents/MacOS/nomouse" >/dev/null
    update=false
    running=false
    regrant=false
    started=false
    old_requirement=""
    if [ -e "$app" ]; then
        update=true
        old_requirement="$(requirement_of "$app")"
        # A copy left running would keep the old code, and its grant breaks at the next start.
        if [ -n "$(app_pids "$app")" ]; then
            running=true
            quit_app "$app"
            step "Quit the running nomouse to update it"
        fi
    fi
    rm -rf "${apps}/.nomouse.app.new"
    ditto "${work}/unpacked/nomouse.app" "${apps}/.nomouse.app.new" </dev/null
    sign_app "${apps}/.nomouse.app.new"
    rm -rf "$app"
    mv "${apps}/.nomouse.app.new" "$app"
    step "Installed $(tilde "$app")"

    mkdir -p "$bin_dir"
    ln -sfn "${app}/Contents/MacOS/nomouse" "${bin_dir}/nomouse"
    step "Linked $(tilde "${bin_dir}/nomouse")"
    if [ "$update" = true ]; then
        if [ -z "$old_requirement" ] || [ "$(requirement_of "$app")" != "$old_requirement" ]; then
            regrant=true
            reset_accessibility
        else
            step "Kept the Accessibility switch" "(same signature)"
        fi
    fi
    if [ "$running" = true ]; then
        if open "$app" </dev/null >/dev/null 2>&1; then
            started=true
            step "Started the new nomouse"
        fi
    fi
    macos_next_steps "$app" "$update" "$regrant" "$started"
    path_hint "$bin_dir"
}

install_linux() {
    command -v tar >/dev/null 2>&1 || fail "tar is required; install it and run the command again"
    problem="$(linux_libc_problem "$target" "")"
    # A --from directory may hold a build made for this system; its own run below decides.
    if [ -n "$problem" ] && [ -z "$from" ]; then
        fail "$problem"
    fi
    archive="$(archive_for "$target")"
    fetch_verified "$archive"
    tar -xzf "${work}/${archive}" -C "$work" </dev/null
    unpacked="${work}/nomouse-${target}"
    [ -f "${unpacked}/nomouse" ] ||
        fail "${archive} has no nomouse-${target}/nomouse; report it at https://github.com/${repo}/issues"
    runnable_version "${unpacked}/nomouse" "$problem" >/dev/null
    place_binary "${unpacked}/nomouse" "${bin_dir}/nomouse"
    step "Installed $(tilde "${bin_dir}/nomouse")"
    if [ -f "${unpacked}/nomouse-settings" ]; then
        place_binary "${unpacked}/nomouse-settings" "${bin_dir}/nomouse-settings"
        settings_missing="$(missing_libraries "${bin_dir}/nomouse-settings" | tr '\n' ' ' | sed 's/ $//')"
        if [ -n "$settings_missing" ]; then
            step "Installed $(tilde "${bin_dir}/nomouse-settings")" "(the settings window needs GTK 4 and libadwaita, missing here: ${settings_missing}; nomouse works without them)"
            next "For the settings window, install GTK 4 and libadwaita: $(gtk_install_hint)"
        else
            step "Installed $(tilde "${bin_dir}/nomouse-settings")" "(the settings window)"
        fi
    fi
    if [ -d "${unpacked}/share" ]; then
        install_desktop_entry "${unpacked}/share"
    else
        step "This release has no app launcher entry" "(nomouse 0.1.3 and later have one)"
    fi
    path_hint "$bin_dir"
    linux_next_steps
}

# Where desktop entries and icons go: the XDG data home, which every desktop's app launcher
# reads.
data_dir() {
    echo "${XDG_DATA_HOME:-${HOME}/.local/share}"
}

# The desktop entry's Exec and TryExec lines run the nomouse in BIN_DIR, which need not be on
# the desktop session's PATH. Exec quotes the path; one with a character Exec quoting would
# have to escape (" ` $ \ %) keeps plain `nomouse`, found on PATH.
desktop_exec_path() {
    case "$1" in
    *[\"\`\$\\%]*) echo nomouse ;;
    *) echo "$1/nomouse" ;;
    esac
}

# each_icon ACTION: runs ACTION ICON KIND for every hicolor icon the release ships (a path under
# share/icons/hicolor; KIND required, or optional for the tray icons, which releases before the
# tray have none). The list is literal words, which zsh splits as sh does.
each_icon() {
    for ei_size in 16 22 24 32 48 64 128 256 512; do
        "$1" "${ei_size}x${ei_size}/apps/${app_id}.png" required
    done
    "$1" "scalable/apps/${app_id}.svg" required
    "$1" "symbolic/apps/${app_id}-symbolic.svg" optional
    "$1" "symbolic/apps/${app_id}-paused-symbolic.svg" optional
}

copy_icon() {
    if [ ! -f "${share}/icons/hicolor/$1" ]; then
        [ "$2" = optional ] && return 0
        fail "the archive has no icon $1; report it at https://github.com/${repo}/issues"
    fi
    mkdir -p "$(dirname "${data}/icons/hicolor/$1")"
    cp "${share}/icons/hicolor/$1" "${data}/icons/hicolor/$1"
}

remove_icon() {
    rm -f "${data}/icons/hicolor/$1"
}

# Copies the release's desktop entry from SHARE (the archive's share/ folder) with Exec and
# TryExec pointing at bin_dir, and its icons, into the data dir; then refreshes the launcher's
# caches.
install_desktop_entry() {
    share="$1"
    data="$(data_dir)"
    de_exe="$(desktop_exec_path "$bin_dir")"
    [ -f "${share}/applications/${app_id}.desktop" ] ||
        fail "the archive has no ${app_id}.desktop; report it at https://github.com/${repo}/issues"
    mkdir -p "${data}/applications"
    while IFS= read -r de_line || [ -n "$de_line" ]; do
        case "$de_line" in
        "Exec=nomouse "*)
            if [ "$de_exe" = nomouse ]; then
                printf '%s\n' "$de_line"
            else
                printf 'Exec="%s" %s\n' "$de_exe" "${de_line#Exec=nomouse }"
            fi
            ;;
        "TryExec=nomouse") printf 'TryExec=%s\n' "$de_exe" ;;
        *) printf '%s\n' "$de_line" ;;
        esac
    done <"${share}/applications/${app_id}.desktop" >"${data}/applications/.${app_id}.new"
    mv -f "${data}/applications/.${app_id}.new" "${data}/applications/${app_id}.desktop"
    each_icon copy_icon
    refresh_desktop_caches
    step "Added nomouse to the app launcher" "($(tilde "${data}/applications/${app_id}.desktop"))"
}

# Removes the desktop entry and icons install_desktop_entry put in place.
remove_desktop_entry() {
    data="$(data_dir)"
    rd_removed=false
    if [ -e "${data}/applications/${app_id}.desktop" ]; then
        rm -f "${data}/applications/${app_id}.desktop"
        rd_removed=true
    fi
    each_icon remove_icon
    if [ "$rd_removed" = true ]; then
        refresh_desktop_caches
        step "Removed nomouse from the app launcher"
    fi
}

# Updates the desktop entry database and the hicolor icon cache under the data dir, with
# whichever of the tools is installed; launchers without the caches read the files directly.
refresh_desktop_caches() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "${data}/applications" </dev/null 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d "${data}/icons/hicolor" ]; then
        gtk-update-icon-cache -q -t -f "${data}/icons/hicolor" </dev/null 2>/dev/null || true
    fi
}

# The next steps for the Linux desktop this runs in: X11, GNOME on Wayland (which nomouse
# cannot drive, so the first step is switching to GNOME on Xorg), or another Wayland
# compositor. Detects GNOME the way nomouse does: a GNOME part in XDG_CURRENT_DESKTOP.
linux_next_steps() {
    ln_gnome=false
    case ":${XDG_CURRENT_DESKTOP:-}:" in
    *:[Gg][Nn][Oo][Mm][Ee]:*) ln_gnome=true ;;
    esac
    if [ "${XDG_SESSION_TYPE:-}" = wayland ] && [ "$ln_gnome" = false ]; then
        next "Run nomouse setup: it turns on the accessibility bus and explains anything missing"
        next "Let your compositor start nomouse and bind its modes, e.g. Sway: exec nomouse daemon, bindsym Ctrl+space exec nomouse hints"
        next "Hyprland: exec-once = nomouse daemon, bind = CTRL, space, exec, nomouse hints"
        if [ -e /dev/uinput ] && [ ! -w /dev/uinput ]; then
            next "KDE Plasma also needs write access to /dev/uinput: https://github.com/${repo}/blob/nightly/docs/install.md#linux-wayland"
        fi
        next "nomouse is in the app launcher too: picking it starts the daemon and opens its settings"
    else
        if [ "${XDG_SESSION_TYPE:-}" = wayland ]; then
            next "Log in with \"GNOME on Xorg\" (\"Ubuntu on Xorg\" on Ubuntu) from the gear on the login screen: GNOME on Wayland cannot show nomouse's labels or click (Mutter has no overlay layer, virtual pointer or app hotkeys)"
        fi
        next "Run nomouse setup: it turns on the accessibility bus and explains anything missing"
        next "Start it: pick nomouse in the app launcher (GNOME: press Super, type nomouse) or run nomouse; ctrl+space shows the labels"
        next "Start at login: nomouse's settings → General → Start at login, or the tray icon's menu (GNOME shows tray icons with the AppIndicator extension, on by default on Ubuntu)"
    fi
    next "Check everything at any time with: nomouse doctor"
}

# Removes this run's work directory, if it made one: the EXIT trap, so every way out of the
# script runs it.
remove_work() {
    if [ -n "$work" ]; then
        rm -rf "$work"
        work=""
    fi
}

# Creates `work`, the private directory under TMPDIR that every download and unpacked file of
# this run goes into. main has set the traps that remove it.
make_work_dir() {
    mw_tmp="${TMPDIR:-/tmp}"
    work="$(mktemp -d "${mw_tmp%/}/nomouse-install.XXXXXX" </dev/null)" ||
        fail "cannot create a temporary directory in ${mw_tmp}; set TMPDIR to a writable directory and run the command again"
}

install_release() {
    os="$(uname -s)"
    arch="$(uname -m)"
    target="$(asset_for "$os" "$arch")"
    if [ -z "$from" ]; then
        command -v curl >/dev/null 2>&1 || fail "curl is required; install it and run the command again"
        if curl --retry-all-errors --version </dev/null >/dev/null 2>&1; then
            retry_all="--retry-all-errors"
        fi
    fi
    make_work_dir

    header
    if [ -n "$from" ]; then
        source_label="from $(tilde "$from")"
    elif [ -n "$version" ]; then
        source_label="release v${version}"
    else
        source_label="latest release"
    fi
    if [ -z "$from" ] && [ -n "${NOMOUSE_INSTALL_BASE:-}" ]; then
        source_label="${source_label} from ${NOMOUSE_INSTALL_BASE%/}"
    fi
    step "$(platform_label "$os" "$arch"):" "$(archive_for "$target"), ${source_label}"
    case "$target" in
    *-apple-darwin) install_macos ;;
    *) install_linux ;;
    esac
    installed_version="$("${bin_dir}/nomouse" --version </dev/null 2>/dev/null || true)"
    installed_version="${installed_version#nomouse }"
    finish "nomouse ${installed_version:-} is installed."
}

uninstall() {
    header
    un_removed=false
    if [ -L "${bin_dir}/nomouse" ]; then
        un_link="$(readlink "${bin_dir}/nomouse" </dev/null)"
        case "$un_link" in
        */nomouse.app/Contents/MacOS/nomouse)
            un_app="${un_link%/Contents/MacOS/nomouse}"
            if [ -d "$un_app" ] && [ "$(bundle_id_of "$un_app")" = "$bundle_id" ]; then
                rm -rf "$un_app"
                step "Removed $(tilde "$un_app")"
                un_removed=true
            fi
            ;;
        esac
    fi
    if [ -e "${bin_dir}/nomouse" ] || [ -L "${bin_dir}/nomouse" ]; then
        rm -f "${bin_dir}/nomouse"
        step "Removed $(tilde "${bin_dir}/nomouse")"
        un_removed=true
    else
        step "nomouse is not installed in $(tilde "$bin_dir")"
    fi
    if [ -e "${bin_dir}/nomouse-settings" ]; then
        rm -f "${bin_dir}/nomouse-settings"
        step "Removed $(tilde "${bin_dir}/nomouse-settings")"
    fi
    if [ "$(uname -s)" = Linux ]; then
        remove_desktop_entry
        # Start at login (the settings window's or the tray menu's switch) would run a removed
        # nomouse.
        un_autostart="${XDG_CONFIG_HOME:-${HOME}/.config}/autostart/nomouse.desktop"
        if [ -f "$un_autostart" ]; then
            rm -f "$un_autostart"
            step "Removed start at login" "($(tilde "$un_autostart"))"
        fi
    elif [ "$un_removed" = true ]; then
        next "Remove its Accessibility entry too: tccutil reset Accessibility ${bundle_id}"
        next "Remove nomouse from System Settings → General → Login Items if you added it"
    fi
    un_dir="$(config_dir)"
    if [ "$purge" = true ]; then
        if [ -e "$un_dir" ]; then
            rm -rf "$un_dir"
            step "Deleted the config directory $(tilde "$un_dir")"
        else
            step "No config directory at $(tilde "$un_dir")"
        fi
    elif [ -d "$un_dir" ]; then
        step "Kept the config in $(tilde "$un_dir")" "(add --purge to delete it)"
    fi
    finish "nomouse is uninstalled."
}

main() {
    # Set here, not in a helper: zsh runs an EXIT trap set inside a function when that function
    # returns. HUP, INT and TERM exit with their usual status through the EXIT trap.
    trap remove_work EXIT
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    set_colours
    action=install
    purge=false
    version="${NOMOUSE_VERSION:-}"
    while [ "$#" -gt 0 ]; do
        case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version needs a value; for example --version 0.1.9"
            version="$2"
            shift 2
            ;;
        --from)
            [ "$#" -ge 2 ] || fail "--from needs a directory; give the folder holding the release assets"
            [ -d "$2" ] || fail "--from: no directory $2; give the folder holding the release assets"
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
        *) fail "unknown option $1; see --help" ;;
        esac
    done
    version="${version#v}"
    if [ -n "$version" ] &&
        ! expr "x${version}" : 'x[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*$' >/dev/null; then
        fail "--version wants MAJOR.MINOR.PATCH, got ${version}; for example --version 0.1.9"
    fi
    if [ "$purge" = true ] && [ "$action" != uninstall ]; then
        fail "--purge only goes with --uninstall; run with --uninstall --purge"
    fi
    [ -n "${HOME:-}" ] || fail "HOME is not set; set it to your home directory and run the command again"
    bin_dir="${NOMOUSE_INSTALL_DIR:-${HOME}/.local/bin}"

    if [ "$action" = uninstall ]; then
        uninstall
    else
        install_release
    fi
}

# Tests source this file with NOMOUSE_INSTALL_NO_MAIN=1 to call its functions one by one.
[ "${NOMOUSE_INSTALL_NO_MAIN:-}" = 1 ] || main "$@"
