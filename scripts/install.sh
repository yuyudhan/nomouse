#!/usr/bin/env bash
# Installs, updates or uninstalls nomouse on macOS and Linux, without sudo. Published with every
# GitHub release as releases/latest/download/install.sh; README.md#install has the one-line curl
# command that pipes it into bash.
#
# Piping is safe: all work happens in main, called on the last line, so bash has read the whole
# script before anything runs. Options go after `bash -s --`.
#
# What it installs, after checking each download against the release's .sha256 and running the
# downloaded nomouse once (`--version`), so a build this system cannot start fails the install:
#   macOS (Apple silicon and Intel): nomouse.app into /Applications (~/Applications when
#     /Applications is not writable), quarantine removed, plus a `nomouse` command in
#     ~/.local/bin that points into the app. Updating over an installed copy quits the running
#     one first, starts the new one after, and, when the code signature changed (every release
#     until they are Developer ID signed), removes the old copy's Accessibility entry, which
#     macOS would otherwise keep showing switched on while ignoring it.
#   Linux (x86_64, aarch64; X11 and Wayland; glibc 2.35 or later): the `nomouse` binary into
#     ~/.local/bin, plus the app launcher entry (GNOME: press Super, type nomouse) and its icons
#     into ${XDG_DATA_HOME:-~/.local/share}/applications and .../icons/hicolor. Systems the
#     build cannot run on (an older glibc, musl, no glibc loader as on NixOS) are refused before
#     anything is downloaded, with what to do instead.
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
#                          avoids GitHub's limit on anonymous downloads.
#   NOMOUSE_API_BASE       The GitHub API base URL for those downloads (default
#                          https://api.github.com).
set -euo pipefail

repo="yuyudhan/nomouse"
bundle_id="com.yuyudhan.nomouse"
# The Linux desktop entry's id and icon name, and the hicolor sizes the release ships.
app_id="io.github.yuyudhan.nomouse"
icon_sizes="16 22 24 32 48 64 128 256 512"
# The tray icons (symbolic: the panel recolours them), on and paused.
tray_icons="symbolic/apps/${app_id}-symbolic.svg symbolic/apps/${app_id}-paused-symbolic.svg"
# Every request: retry failures, give up on a connection that never opens, and abort a transfer
# that stalls (under 1 KB/s for 30 s) instead of hanging. --retry alone retries only timeouts and
# HTTP 408/429/5xx; install_release adds --retry-all-errors where curl has it (7.71+), so a
# network that cuts connections (`curl: (35) ... unexpected eof while reading`) is retried too.
net=(--fail --silent --show-error --location --connect-timeout 15 --retry 5 --retry-delay 2
    --speed-limit 1024 --speed-time 30)

# The Linux release is built on Ubuntu 22.04, so it needs that glibc or a later one.
min_glibc="2.35"
source_build_url="https://github.com/${repo}/blob/nightly/docs/install.md#building-from-source"

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
through the GitHub API, avoids anonymous rate limits), NOMOUSE_API_BASE.
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
  Check the network (a proxy or firewall that cuts connections to github.com or
  release-assets.githubusercontent.com), then run the installer again; or download the
  release assets in a browser and install from that folder with --from DIR."
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
    local file="$1" dest="$2" new
    new="$(dirname "$dest")/.$(basename "$dest").new"
    mkdir -p "$(dirname "$dest")"
    cp "$file" "$new"
    chmod 755 "$new"
    mv -f "$new" "$dest"
}

# The `--version` line of the downloaded program at $1, run before anything is replaced: a build
# this system cannot start (too old a glibc, a missing library or loader) fails the install with
# the loader's own message, HINT ($2, optional) after it, and leaves an installed copy as it was.
runnable_version() {
    local program="$1" hint="${2:-}" out status reason
    out="$("$program" --version 2>"${work}/version.err")" || {
        status=$?
        reason="$(tr '\n' ' ' <"${work}/version.err" | sed 's/ *$//')"
        [ -n "$reason" ] || reason="it exited with status ${status}"
        fail "the downloaded nomouse does not run on this system (${reason}); nothing was installed${hint:+. ${hint}}"
    }
    printf '%s\n' "$out"
}

# True when glibc version $1 (MAJOR.MINOR) is $2 or later.
version_at_least() {
    local have_major="${1%%.*}" have_minor="${1#*.}" want_major="${2%%.*}" want_minor="${2#*.}"
    have_minor="${have_minor%%.*}"
    want_minor="${want_minor%%.*}"
    [ "$have_major" -gt "$want_major" ] ||
        { [ "$have_major" -eq "$want_major" ] && [ "$have_minor" -ge "$want_minor" ]; }
}

# ldd's --version text (glibc prints it on stdout, musl on stderr with status 1), or nothing
# without ldd.
ldd_version_text() {
    command -v ldd >/dev/null 2>&1 || return 0
    ldd --version 2>&1 || true
}

# This system's glibc version (MAJOR.MINOR), or nothing when neither getconf nor ldd names one
# (musl, or a system without either).
glibc_version() {
    local out first
    if out="$(getconf GNU_LIBC_VERSION 2>/dev/null)" && [[ "$out" =~ ^glibc\ ([0-9]+\.[0-9]+) ]]; then
        echo "${BASH_REMATCH[1]}"
        return 0
    fi
    # "ldd (GNU libc) 2.35", "ldd (Ubuntu GLIBC 2.35-0ubuntu3.8) 2.35".
    out="$(ldd_version_text)"
    first="${out%%$'\n'*}"
    if [[ "$first" =~ ^ldd\ .*\)\ ([0-9]+\.[0-9]+) ]]; then
        echo "${BASH_REMATCH[1]}"
    fi
}

# The program loader the glibc build of TARGET starts through.
glibc_loader_for() {
    case "$1" in
    x86_64-*) echo /lib64/ld-linux-x86-64.so.2 ;;
    aarch64-*) echo /lib/ld-linux-aarch64.so.1 ;;
    esac
}

# True when this system's C library is musl: ldd says so, or ROOT has a musl loader and no glibc
# one.
musl_system() {
    local root="$1" loader="$2" musl
    [[ "$(ldd_version_text)" == *musl* ]] && return 0
    [ -e "${root}${loader}" ] && return 1
    for musl in "${root}"/lib/ld-musl-*; do
        [ -e "$musl" ] && return 0
    done
    return 1
}

# Why the Linux build for TARGET cannot run on this system, or nothing when it can (or this
# cannot tell; the downloaded binary's own --version run then has the last word). ROOT prefixes
# the loader paths: "" for this system, a fake root in tests.
linux_libc_problem() {
    local target="$1" root="$2" loader have
    loader="$(glibc_loader_for "$target")"
    if musl_system "$root" "$loader"; then
        echo "this system uses musl libc (Alpine, Void musl and others), and nomouse's Linux build needs glibc ${min_glibc} or later. Build nomouse from source instead: ${source_build_url}"
    elif [ ! -e "${root}${loader}" ]; then
        echo "this system has no glibc program loader at ${loader} (NixOS and Guix keep it elsewhere), so nomouse's Linux build cannot start. On NixOS turn on nix-ld (programs.nix-ld.enable = true) and run this installer again, or build nomouse from source: ${source_build_url}"
    else
        have="$(glibc_version)"
        if [ -n "$have" ] && ! version_at_least "$have" "$min_glibc"; then
            echo "nomouse's Linux build needs glibc ${min_glibc} or later (Ubuntu 22.04, Debian 12, Fedora 36, RHEL 10 or newer); this system has glibc ${have}. Build nomouse from source instead: ${source_build_url}"
        fi
    fi
}

# The shared libraries the program at $1 needs and this system lacks, one per line (ldd's
# "not found" lines); nothing when all are there or ldd is missing.
missing_libraries() {
    command -v ldd >/dev/null 2>&1 || return 0
    { ldd "$1" 2>/dev/null || true; } | awk '/not found/ { print $1 }'
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
        echo "install GTK 4 (4.6 or later) and libadwaita (1.1 or later) with your package manager"
    fi
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

# Where macOS lists the apps allowed to control the computer, as System Settings names it:
# macOS 27 renamed the Accessibility list "Device Control and Data Access".
accessibility_pane() {
    local major
    major="$(sw_vers -productVersion 2>/dev/null || true)"
    major="${major%%.*}"
    if [[ "$major" =~ ^[0-9]+$ ]] && [ "$major" -ge 27 ]; then
        echo "System Settings > Privacy & Security > Device Control and Data Access"
    else
        echo "System Settings > Privacy & Security > Accessibility"
    fi
}

# The designated requirement of the app at $1, which macOS pins an Accessibility grant to (for
# an ad-hoc signed build, its exact code hash), or nothing when it is unsigned.
requirement_of() {
    codesign -d -r- "$1" 2>/dev/null || true
}

# The process IDs of programs running from the app at $1, one per line: those whose command
# line has the app's executable as a word.
app_pids() {
    local exe="$1/Contents/MacOS/nomouse" pid command
    ps -axo pid=,command= | while read -r pid command; do
        case " ${command} " in
        *" ${exe} "*) echo "$pid" ;;
        esac
    done
}

# Quits the nomouse running from the app at $1, the copy an update replaces: asks it over its
# command socket (`nomouse quit`), then, if it is still running after 5 s, terminates it.
quit_app() {
    local app="$1" _
    "${app}/Contents/MacOS/nomouse" quit >/dev/null 2>&1 || true
    for _ in $(seq 50); do
        [ -n "$(app_pids "$app")" ] || return 0
        sleep 0.1
    done
    app_pids "$app" | xargs kill 2>/dev/null || true
    for _ in $(seq 20); do
        [ -n "$(app_pids "$app")" ] || return 0
        sleep 0.1
    done
    say "the old nomouse is still running: quit it from its menu bar icon's menu"
}

# Removes nomouse's entry from macOS's Accessibility list. After an update with a different code
# signature that entry stays switched on but belongs to the old copy: macOS ignores it for the
# new one, and switching it off and on again does not help.
reset_accessibility() {
    if tccutil reset Accessibility "$bundle_id" >/dev/null 2>&1; then
        say "removed the old copy's Accessibility entry (macOS ties it to the old build)"
    else
        say "could not remove the old copy's Accessibility entry: if nomouse shows switched on"
        say "but still asks, select it, remove it with the - button and add the new one"
    fi
}

# The numbered next steps on macOS. UPDATE: an installed copy was replaced; REGRANT: its
# Accessibility entry was removed; STARTED: the new copy was started.
macos_next_steps() {
    local app="$1" update="$2" regrant="$3" started="$4" pane
    pane="$(accessibility_pane)"
    if [ "$started" = true ]; then
        step "1. the new nomouse is starting from \"${app}\""
    else
        step "1. open \"${app}\""
    fi
    if [ "$regrant" = true ]; then
        step "   macOS asks for Accessibility again after every update until nomouse's releases are"
        step "   signed with an Apple Developer ID. The Welcome window asks for it: in"
        step "   ${pane}, switch nomouse on once more."
        step "   It notices the switch and restarts by itself."
    elif [ "$update" = true ]; then
        step "   its keycap icon appears in the menu bar; its Accessibility switch still applies."
    else
        step "   its keycap icon appears in the menu bar and a Welcome window asks for"
        step "   Accessibility: in ${pane},"
        step "   switch on nomouse. It notices the switch and restarts by itself."
    fi
    step "2. press ctrl+space in any app and type a label to click it."
    step "3. settings and start at login: the menu bar icon's menu."
    step "check everything at any time with: nomouse doctor"
}

install_macos() {
    local target="$1" bin_dir="$2" archive apps app new_version old_requirement=""
    local update=false running=false regrant=false started=false
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
    new_version="$(runnable_version "${work}/unpacked/nomouse.app/Contents/MacOS/nomouse")"
    if [ -e "$app" ]; then
        update=true
        old_requirement="$(requirement_of "$app")"
        # A copy left running would keep the old code, and its grant breaks at the next start.
        if [ -n "$(app_pids "$app")" ]; then
            running=true
            say "quitting the running nomouse to update it"
            quit_app "$app"
        fi
    fi
    rm -rf "${apps}/.nomouse.app.new"
    ditto "${work}/unpacked/nomouse.app" "${apps}/.nomouse.app.new"
    rm -rf "$app"
    mv "${apps}/.nomouse.app.new" "$app"

    mkdir -p "$bin_dir"
    ln -sfn "${app}/Contents/MacOS/nomouse" "${bin_dir}/nomouse"

    say "installed ${new_version} to ${app}"
    say "the nomouse command: ${bin_dir}/nomouse -> ${app}/Contents/MacOS/nomouse"
    if [ "$update" = true ] &&
        { [ -z "$old_requirement" ] || [ "$(requirement_of "$app")" != "$old_requirement" ]; }; then
        regrant=true
        reset_accessibility
    fi
    if [ "$running" = true ]; then
        if open "$app" >/dev/null 2>&1; then
            started=true
        else
            say "could not start ${app} (no desktop session?)"
        fi
    fi
    echo
    say "next steps:"
    path_hint "$bin_dir"
    macos_next_steps "$app" "$update" "$regrant" "$started"
}

install_linux() {
    local target="$1" bin_dir="$2" archive settings_missing problem new_version
    command -v tar >/dev/null 2>&1 || fail "tar is required"
    problem="$(linux_libc_problem "$target" "")"
    # A --from directory may hold a build made for this system; its own run below decides.
    if [ -n "$problem" ] && [ -z "$from" ]; then
        fail "$problem"
    fi
    archive="$(archive_for "$target")"
    fetch_verified "$archive"
    tar -xzf "${work}/${archive}" -C "$work"
    [ -f "${work}/nomouse-${target}/nomouse" ] || fail "${archive} has no nomouse-${target}/nomouse"
    new_version="$(runnable_version "${work}/nomouse-${target}/nomouse" "$problem")"
    place_binary "${work}/nomouse-${target}/nomouse" "${bin_dir}/nomouse"

    say "installed ${new_version} to ${bin_dir}/nomouse"
    if [ -f "${work}/nomouse-${target}/nomouse-settings" ]; then
        place_binary "${work}/nomouse-${target}/nomouse-settings" "${bin_dir}/nomouse-settings"
        say "installed the settings window to ${bin_dir}/nomouse-settings"
        settings_missing="$(missing_libraries "${bin_dir}/nomouse-settings")"
        if [ -n "$settings_missing" ]; then
            say "the settings window needs GTK 4 and libadwaita, which are missing here"
            say "($(echo "$settings_missing" | tr '\n' ' ' | sed 's/ $//')); install them with:"
            step "$(gtk_install_hint)"
            say "nomouse itself works without them; the launcher then shows a notification"
        fi
    fi
    if [ -d "${work}/nomouse-${target}/share" ]; then
        install_desktop_entry "${work}/nomouse-${target}/share" "$bin_dir"
    else
        say "this release has no app launcher entry (nomouse 0.1.3 and later have one)"
    fi
    echo
    say "next steps:"
    path_hint "$bin_dir"
    linux_next_steps "$bin_dir"
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

# Copies the release's desktop entry from SHARE (the archive's share/ folder) with Exec and
# TryExec pointing at BIN_DIR, and its icons, into the data dir; then refreshes the launcher's
# caches.
install_desktop_entry() {
    local share="$1" bin_dir="$2" data exe line size icon
    data="$(data_dir)"
    exe="$(desktop_exec_path "$bin_dir")"
    [ -f "${share}/applications/${app_id}.desktop" ] || fail "the archive has no ${app_id}.desktop"
    mkdir -p "${data}/applications"
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
        "Exec=nomouse "*)
            if [ "$exe" = nomouse ]; then
                printf '%s\n' "$line"
            else
                printf 'Exec="%s" %s\n' "$exe" "${line#Exec=nomouse }"
            fi
            ;;
        "TryExec=nomouse") printf 'TryExec=%s\n' "$exe" ;;
        *) printf '%s\n' "$line" ;;
        esac
    done <"${share}/applications/${app_id}.desktop" >"${data}/applications/.${app_id}.new"
    mv -f "${data}/applications/.${app_id}.new" "${data}/applications/${app_id}.desktop"
    for size in $icon_sizes scalable; do
        if [ "$size" = scalable ]; then
            icon="scalable/apps/${app_id}.svg"
        else
            icon="${size}x${size}/apps/${app_id}.png"
        fi
        [ -f "${share}/icons/hicolor/${icon}" ] || fail "the archive has no icon ${icon}"
        mkdir -p "$(dirname "${data}/icons/hicolor/${icon}")"
        cp "${share}/icons/hicolor/${icon}" "${data}/icons/hicolor/${icon}"
    done
    for icon in $tray_icons; do
        # Releases before the tray icon have none.
        [ -f "${share}/icons/hicolor/${icon}" ] || continue
        mkdir -p "$(dirname "${data}/icons/hicolor/${icon}")"
        cp "${share}/icons/hicolor/${icon}" "${data}/icons/hicolor/${icon}"
    done
    refresh_desktop_caches "$data"
    say "added nomouse to the app launcher: ${data}/applications/${app_id}.desktop"
}

# Removes the desktop entry and icons install_desktop_entry put in place.
remove_desktop_entry() {
    local data size removed=false
    data="$(data_dir)"
    if [ -e "${data}/applications/${app_id}.desktop" ]; then
        rm -f "${data}/applications/${app_id}.desktop"
        removed=true
    fi
    for size in $icon_sizes; do
        rm -f "${data}/icons/hicolor/${size}x${size}/apps/${app_id}.png"
    done
    rm -f "${data}/icons/hicolor/scalable/apps/${app_id}.svg"
    for icon in $tray_icons; do
        rm -f "${data}/icons/hicolor/${icon}"
    done
    if [ "$removed" = true ]; then
        refresh_desktop_caches "$data"
        say "removed nomouse from the app launcher: ${data}/applications/${app_id}.desktop"
    fi
}

# Updates the desktop entry database and the hicolor icon cache under DATA, with whichever of
# the tools is installed; launchers without the caches read the files directly.
refresh_desktop_caches() {
    local data="$1"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "${data}/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d "${data}/icons/hicolor" ]; then
        gtk-update-icon-cache -q -t -f "${data}/icons/hicolor" 2>/dev/null || true
    fi
}

# The numbered next steps for the Linux desktop this runs in: X11, GNOME on Wayland (which
# nomouse cannot drive, so the first step is switching to GNOME on Xorg), or another Wayland
# compositor. Detects GNOME the way nomouse does: a GNOME part in XDG_CURRENT_DESKTOP.
linux_next_steps() {
    local bin_dir="$1" gnome=false
    case ":${XDG_CURRENT_DESKTOP:-}:" in
    *:[Gg][Nn][Oo][Mm][Ee]:*) gnome=true ;;
    esac
    if [ "${XDG_SESSION_TYPE:-}" = wayland ] && [ "$gnome" = true ]; then
        step "1. GNOME on Wayland cannot show nomouse's labels or click (Mutter has no overlay"
        step "   layer, virtual pointer or app hotkeys). Log out, click the gear on the login"
        step "   screen, choose \"GNOME on Xorg\" (Ubuntu: \"Ubuntu on Xorg\") and log in; then:"
        step "2. nomouse setup     (turns on the accessibility bus, explains anything missing)"
        step "3. nomouse           (starts the daemon; ctrl+space shows the labels)"
        step "4. start at login: nomouse's settings > General > Start at login"
    elif [ "${XDG_SESSION_TYPE:-}" = wayland ]; then
        step "1. nomouse setup     (turns on the accessibility bus, explains anything missing)"
        step "2. Wayland: let your compositor start nomouse and bind its modes, e.g. Sway:"
        step "     exec nomouse daemon"
        step "     bindsym Ctrl+space exec nomouse hints"
        step "   Hyprland: exec-once = nomouse daemon / bind = CTRL, space, exec, nomouse hints"
        if [ -e /dev/uinput ] && [ ! -w /dev/uinput ]; then
            step "   KDE Plasma also needs write access to /dev/uinput:"
            step "   https://github.com/${repo}/blob/nightly/docs/install.md#linux-wayland"
        fi
    else
        step "1. nomouse setup     (turns on the accessibility bus, explains anything missing)"
        step "2. nomouse           (starts the daemon; ctrl+space shows the labels)"
        step "3. start at login: nomouse's settings > General > Start at login (or the tray menu)"
    fi
    step "nomouse is in the app launcher (GNOME: press Super, type nomouse): picking it starts the"
    step "daemon and opens its settings; the tray icon's menu has the same (GNOME shows tray icons"
    step "with the AppIndicator extension, on by default on Ubuntu)"
    step "check everything at any time with: nomouse doctor"
}

install_release() {
    local bin_dir="$1" target
    target="$(asset_for "$(uname -s)" "$(uname -m)")"
    if [ -z "$from" ]; then
        command -v curl >/dev/null 2>&1 || fail "curl is required"
        if curl --retry-all-errors --version >/dev/null 2>&1; then
            net+=(--retry-all-errors)
        fi
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
    local bin_dir="$1" purge="$2" dir app link autostart
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
    if [ -e "${bin_dir}/nomouse-settings" ]; then
        rm -f "${bin_dir}/nomouse-settings"
        say "removed ${bin_dir}/nomouse-settings"
    fi
    if [ "$(uname -s)" = Linux ]; then
        remove_desktop_entry
        # Start at login (the settings window's or the tray menu's switch) would run a removed
        # nomouse.
        autostart="${XDG_CONFIG_HOME:-${HOME}/.config}/autostart/nomouse.desktop"
        if [ -f "$autostart" ]; then
            rm -f "$autostart"
            say "removed start at login: ${autostart}"
        fi
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
