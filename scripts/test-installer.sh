#!/usr/bin/env bash
# Tests install.sh without building nomouse: its pure functions (target and asset names, the
# GitHub release JSON parser) by sourcing it into sh, then whole installs from a fake release for
# this machine's OS: a stub `nomouse` in the archive layout the release uses (a tarball with the
# app launcher entry and icons from packaging/linux/share on Linux, nomouse.app zipped on macOS),
# with the release's SHA256SUMS. The fake release is installed with --from DIR and through a
# local fake GitHub API that refuses requests without the token, all with sh into a throwaway
# HOME in an env -i sandbox; then the documented one-liner pipes it into sh, dash, bash and zsh.
#
# scripts/test-install.sh is the other half: the same installer against real release assets.
#
# Usage: scripts/test-installer.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
installer="${root}/scripts/install.sh"
os="$(uname -s)"
case "$os" in
    Linux | Darwin) ;;
    *)
        echo "test-installer: skipped on ${os} (install.sh is for macOS and Linux)"
        exit 0
        ;;
esac

work="$(mktemp -d)"
server=""
flaky=""
www_server=""
cleanup() {
    local pid
    for pid in "$server" "$flaky" "$www_server"; do
        if [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    rm -rf "$work"
}
trap cleanup EXIT

failures=0
pass() {
    echo "ok   - $*"
}
fail() {
    echo "FAIL - $*"
    failures=$((failures + 1))
}
# check NAME EXPECTED ACTUAL
check() {
    if [ "$2" = "$3" ]; then pass "$1"; else fail "$1: expected '$2', got '$3'"; fi
}

# --- Pure functions --------------------------------------------------------------------------

# lib FUNCTION ARGS...: calls one installer function in sh, the shell the one-liner pipes into.
lib() {
    # shellcheck disable=SC2016 # expanded by that sh
    NOMOUSE_INSTALL_NO_MAIN=1 sh -c '. "$1" && shift && "$@"' lib "$installer" "$@"
}

check "asset_for Linux x86_64" x86_64-unknown-linux-gnu "$(lib asset_for Linux x86_64)"
check "asset_for Linux amd64" x86_64-unknown-linux-gnu "$(lib asset_for Linux amd64)"
check "asset_for Linux aarch64" aarch64-unknown-linux-gnu "$(lib asset_for Linux aarch64)"
check "asset_for Linux arm64" aarch64-unknown-linux-gnu "$(lib asset_for Linux arm64)"
check "asset_for Darwin arm64" universal-apple-darwin "$(lib asset_for Darwin arm64)"
check "asset_for Darwin x86_64" universal-apple-darwin "$(lib asset_for Darwin x86_64)"
for unsupported in "Linux riscv64" "Darwin ppc" "FreeBSD amd64" "MINGW64_NT-10.0 x86_64"; do
    # shellcheck disable=SC2086 # two words: OS and CPU
    if out="$(lib asset_for $unsupported 2>&1)"; then
        fail "asset_for ${unsupported} should fail, printed ${out}"
    else
        pass "asset_for ${unsupported} fails: ${out#nomouse: error: }"
    fi
done
check "archive_for macOS" nomouse-universal-apple-darwin.app.zip \
    "$(lib archive_for universal-apple-darwin)"
check "archive_for Linux" nomouse-aarch64-unknown-linux-gnu.tar.gz \
    "$(lib archive_for aarch64-unknown-linux-gnu)"

# The release JSON as the API prints it (pretty) and minified; the uploader's "url" and the
# release's own "name" come between and around the assets.
cat >"${work}/pretty.json" <<'EOF'
{
  "url": "https://api.github.com/repos/yuyudhan/nomouse/releases/7",
  "name": "nomouse v0.1.0",
  "tag_name": "v0.1.0",
  "assets": [
    {
      "url": "https://api.github.com/repos/yuyudhan/nomouse/releases/assets/11",
      "id": 11,
      "name": "nomouse-x86_64-unknown-linux-gnu.tar.gz",
      "uploader": { "url": "https://api.github.com/users/someone", "login": "someone" }
    },
    {
      "url": "https://api.github.com/repos/yuyudhan/nomouse/releases/assets/12",
      "id": 12,
      "name": "nomouse-x86_64-unknown-linux-gnu.tar.gz.sha256",
      "uploader": { "url": "https://api.github.com/users/someone", "login": "someone" }
    }
  ]
}
EOF
tr -d ' \n' <"${work}/pretty.json" >"${work}/min.json"
for form in pretty min; do
    check "asset_api_url (${form}) archive" \
        https://api.github.com/repos/yuyudhan/nomouse/releases/assets/11 \
        "$(lib asset_api_url nomouse-x86_64-unknown-linux-gnu.tar.gz "${work}/${form}.json")"
    check "asset_api_url (${form}) checksum" \
        https://api.github.com/repos/yuyudhan/nomouse/releases/assets/12 \
        "$(lib asset_api_url nomouse-x86_64-unknown-linux-gnu.tar.gz.sha256 "${work}/${form}.json")"
    check "asset_api_url (${form}) missing asset" "" \
        "$(lib asset_api_url nomouse-universal-apple-darwin.app.zip "${work}/${form}.json")"
    check "asset_api_url (${form}) the release name is not an asset" "" \
        "$(lib asset_api_url "nomouse v0.1.0" "${work}/${form}.json")"
done

# A release as large as a real one (megabytes of assets after the match): the lookup must
# still succeed under the installer's pipefail, where a reader that exits early used to kill
# its writer with SIGPIPE and fail the install without a word.
{
    printf '{"assets":[{"url":"https://api.github.com/repos/yuyudhan/nomouse/releases/assets/1",'
    printf '"name":"nomouse-x86_64-unknown-linux-gnu.tar.gz"}'
    for i in $(seq 2 20000); do
        printf ',{"url":"https://api.github.com/repos/yuyudhan/nomouse/releases/assets/%s","name":"filler-%s"}' "$i" "$i"
    done
    printf ']}\n'
} >"${work}/large.json"
if url="$(lib asset_api_url nomouse-x86_64-unknown-linux-gnu.tar.gz "${work}/large.json")"; then
    check "asset_api_url (large release) finds the first asset" \
        https://api.github.com/repos/yuyudhan/nomouse/releases/assets/1 "$url"
else
    fail "asset_api_url (large release) failed with status $?"
fi

# The settings window's missing libraries come from ldd's "not found" lines; a program ldd
# cannot read (a script) has none.
fakebin="${work}/fakebin"
mkdir -p "$fakebin"
cat >"${fakebin}/ldd" <<'LDD'
#!/bin/sh
case "$1" in
    *nomouse-settings)
        printf '\tlinux-vdso.so.1 (0x00007ffd)\n\tlibgtk-4.so.1 => not found\n'
        printf '\tlibadwaita-1.so.0 => not found\n\tlibc.so.6 => /lib/libc.so.6 (0x00007f)\n'
        ;;
    *) echo "	not a dynamic executable" >&2; exit 1 ;;
esac
LDD
chmod 755 "${fakebin}/ldd"
check "missing_libraries lists ldd's not-found libraries" "libgtk-4.so.1 libadwaita-1.so.0" \
    "$(PATH="${fakebin}:${PATH}" lib missing_libraries /opt/bin/nomouse-settings | tr '\n' ' ' | sed 's/ $//')"
check "missing_libraries is empty for a program ldd cannot read" "" \
    "$(PATH="${fakebin}:${PATH}" lib missing_libraries /opt/bin/script)"

# next_steps: the Linux next steps, as the installer prints them under "Next:".
next_steps() {
    # shellcheck disable=SC2016 # expanded by the installer's sh
    lib eval 'linux_next_steps && printf "%s" "$todo"' 2>&1
}
# The next steps name the fix for the desktop the installer runs in: GNOME on Wayland cannot
# run nomouse, so it must say to log in with GNOME on Xorg instead of printing Sway bindings.
steps="$(XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=ubuntu:GNOME next_steps)" || true
if [[ "$steps" == *'"Ubuntu on Xorg"'* ]] && [[ "$steps" == *'"GNOME on Xorg"'* ]] &&
    [[ "$steps" != *Sway* ]]; then
    pass "GNOME on Wayland: the next steps say to log in with GNOME on Xorg"
else
    fail "GNOME on Wayland next steps: ${steps}"
fi
steps="$(XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway next_steps)" || true
if [[ "$steps" == *"bindsym Ctrl+space exec nomouse hints"* ]] && [[ "$steps" != *Xorg* ]]; then
    pass "Sway: the next steps bind nomouse hints in the compositor"
else
    fail "Sway next steps: ${steps}"
fi
steps="$(XDG_SESSION_TYPE=x11 XDG_CURRENT_DESKTOP=ubuntu:GNOME next_steps)" || true
if [[ "$steps" == *"settings → General → Start at login"* ]] &&
    [[ "$steps" == *"app launcher"* ]] && [[ "$steps" != *Xorg* ]]; then
    pass "GNOME on Xorg: the next steps start the daemon and point at Start at login"
else
    fail "GNOME on Xorg next steps: ${steps}"
fi

# The macOS next steps name the Accessibility list as System Settings shows it: macOS 27 calls
# it "Device Control and Data Access".
fakemac="${work}/fakemac"
mkdir -p "$fakemac"
cat >"${fakemac}/sw_vers" <<'EOF'
#!/bin/sh
[ "$1" = -productVersion ] && [ -n "${FAKE_SW_VERS:-}" ] || exit 1
echo "$FAKE_SW_VERS"
EOF
chmod 755 "${fakemac}/sw_vers"
for case in "27.0.1:Device Control and Data Access" "28.1:Device Control and Data Access" \
    "26.2:Accessibility" "15.5:Accessibility" "11.7.10:Accessibility" ":Accessibility"; do
    check "accessibility_pane on macOS '${case%%:*}'" "System Settings → Privacy & Security → ${case#*:}" \
        "$(FAKE_SW_VERS="${case%%:*}" PATH="${fakemac}:${PATH}" lib accessibility_pane)"
done

# Linux: which systems the glibc build is refused on before any download. getconf and ldd are
# stand-ins answering FAKE_GETCONF and FAKE_LDD (musl's ldd prints to stderr and fails); the
# fake roots hold the glibc loaders, the musl loader, or neither (NixOS).
fakelibc="${work}/fakelibc"
mkdir -p "${fakelibc}/bin" "${fakelibc}/glibc/lib64" "${fakelibc}/glibc/lib" "${fakelibc}/musl/lib" \
    "${fakelibc}/nixos"
touch "${fakelibc}/glibc/lib64/ld-linux-x86-64.so.2" "${fakelibc}/glibc/lib/ld-linux-aarch64.so.1" \
    "${fakelibc}/musl/lib/ld-musl-x86_64.so.1"
cat >"${fakelibc}/bin/getconf" <<'EOF'
#!/bin/sh
[ "$1" = GNU_LIBC_VERSION ] && [ -n "${FAKE_GETCONF:-}" ] || exit 1
echo "$FAKE_GETCONF"
EOF
cat >"${fakelibc}/bin/ldd" <<'EOF'
#!/bin/sh
[ "$1" = --version ] && [ -n "${FAKE_LDD:-}" ] || exit 1
case "$FAKE_LDD" in
*musl*) printf '%s\n' "$FAKE_LDD" >&2; exit 1 ;;
esac
printf '%s\nCopyright (C) 2022 Free Software Foundation, Inc.\n' "$FAKE_LDD"
EOF
chmod 755 "${fakelibc}/bin/getconf" "${fakelibc}/bin/ldd"
# libc_problem GETCONF LDD TARGET ROOT
libc_problem() {
    FAKE_GETCONF="$1" FAKE_LDD="$2" PATH="${fakelibc}/bin:${PATH}" \
        lib linux_libc_problem "$3" "${fakelibc}/$4"
}
x86=x86_64-unknown-linux-gnu
arm=aarch64-unknown-linux-gnu
build_link="https://github.com/yuyudhan/nomouse/blob/nightly/docs/install.md#building-from-source"
# expect_refusal NAME OUTPUT TEXT...: OUTPUT holds every TEXT and the build-from-source link.
expect_refusal() {
    local name="$1" out="$2" text
    shift 2
    for text in "$@" "$build_link"; do
        if [[ "$out" != *"$text"* ]]; then
            fail "${name}: '${text}' missing from: ${out}"
            return
        fi
    done
    pass "${name}"
}
expect_refusal "glibc 2.31 (getconf) is refused" "$(libc_problem "glibc 2.31" "" "$x86" glibc)" \
    "needs glibc 2.35 or later" "this system has glibc 2.31" "Ubuntu 22.04"
expect_refusal "glibc 2.34 (RHEL 9) is refused" "$(libc_problem "glibc 2.34" "" "$arm" glibc)" \
    "this system has glibc 2.34"
expect_refusal "glibc 2.31 (only ldd answers) is refused" \
    "$(libc_problem "" "ldd (Ubuntu GLIBC 2.31-0ubuntu9.16) 2.31" "$x86" glibc)" "this system has glibc 2.31"
check "glibc 2.35 is accepted" "" "$(libc_problem "glibc 2.35" "" "$x86" glibc)"
check "glibc 2.39 on aarch64 is accepted" "" "$(libc_problem "glibc 2.39" "" "$arm" glibc)"
check "glibc 2.36 (only ldd answers) is accepted" "" \
    "$(libc_problem "" "ldd (GNU libc) 2.36" "$x86" glibc)"
check "a glibc system that names no version is left to the binary's own run" "" \
    "$(libc_problem "" "" "$x86" glibc)"
expect_refusal "musl (ldd says so) is refused, even with a glibc loader shim" \
    "$(libc_problem "" "musl libc (x86_64)" "$x86" glibc)" "uses musl libc"
expect_refusal "musl (only its loader) is refused" "$(libc_problem "" "" "$x86" musl)" "uses musl libc"
expect_refusal "no glibc loader (NixOS, x86_64) is refused with nix-ld" \
    "$(libc_problem "glibc 2.40" "" "$x86" nixos)" "no glibc program loader at /lib64/ld-linux-x86-64.so.2" \
    "programs.nix-ld.enable = true"
expect_refusal "no glibc loader (NixOS, aarch64) names its loader" "$(libc_problem "" "" "$arm" nixos)" \
    "/lib/ld-linux-aarch64.so.1"
for case in "2.35 2.35 yes" "2.36 2.35 yes" "3.0 2.35 yes" "2.34 2.35 no" "2.4 2.35 no" "1.99 2.35 no"; do
    read -r have want expected <<<"$case"
    # A subshell: sourcing the installer here would replace this script's work and fail.
    if (lib version_at_least "$have" "$want"); then actual=yes; else actual=no; fi
    check "version_at_least ${have} ${want}" "$expected" "$actual"
done

# --- A fake release for this machine ----------------------------------------------------------

target="$(lib asset_for "$os" "$(uname -m)")"
archive="$(lib archive_for "$target")"
# The stand-in nomouse prints its version. `daemon` runs until `quit` asks it to stop (as the
# real one does over its command socket); `quit` is logged to calls.log.
calls="${work}/calls.log"
stub="#!/bin/sh
case \"\$1\" in
daemon) while [ ! -e '${work}/quit-requested' ]; do sleep 0.1; done ;;
quit) echo quit >>'${calls}'; touch '${work}/quit-requested' ;;
*) echo 'nomouse 0.1.0-fake' ;;
esac"
# A build this system cannot start, as the loader reports a glibc too old.
broken_stub="#!/bin/sh
echo 'nomouse: /lib/x86_64-linux-gnu/libc.so.6: version GLIBC_2.35 not found' >&2
exit 1"

# make_release DIR PROGRAM: the release archive holding PROGRAM as nomouse, its .sha256 and the
# release's SHA256SUMS.
make_release() {
    local dir="$1" program="$2" stage
    stage="$(mktemp -d "${work}/stage.XXXXXX")"
    mkdir -p "$dir"
    if [ "$os" = Darwin ]; then
        mkdir -p "${stage}/nomouse.app/Contents/MacOS"
        printf '%s\n' "$program" >"${stage}/nomouse.app/Contents/MacOS/nomouse"
        chmod 755 "${stage}/nomouse.app/Contents/MacOS/nomouse"
        cat >"${stage}/nomouse.app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.yuyudhan.nomouse</string>
    <key>CFBundleExecutable</key>
    <string>nomouse</string>
</dict>
</plist>
EOF
        (cd "$stage" && ditto -c -k --keepParent nomouse.app "${dir}/${archive}")
    else
        mkdir -p "${stage}/nomouse-${target}"
        printf '%s\n' "$program" >"${stage}/nomouse-${target}/nomouse"
        printf '#!/bin/sh\necho "nomouse-settings 0.1.0-fake"\n' >"${stage}/nomouse-${target}/nomouse-settings"
        chmod 755 "${stage}/nomouse-${target}/nomouse" "${stage}/nomouse-${target}/nomouse-settings"
        cp -R "${root}/packaging/linux/share" "${stage}/nomouse-${target}/"
        tar -C "$stage" -czf "${dir}/${archive}" "nomouse-${target}"
    fi
    "${root}/scripts/release/checksum.sh" "${dir}/${archive}"
    cat "${dir}/${archive}.sha256" >"${dir}/SHA256SUMS"
}
release="${work}/release"
make_release "$release" "$stub"
broken="${work}/broken"
make_release "$broken" "$broken_stub"

home="${work}/home"
decoy="${work}/decoy"
mkdir -p "$home" "${decoy}/config/nomouse" "${decoy}/bin" "${decoy}/Applications"
touch "${decoy}/config/nomouse/config.toml" "${decoy}/bin/nomouse"
export XDG_CONFIG_HOME="${decoy}/config" NOMOUSE_INSTALL_DIR="${decoy}/bin"
export NOMOUSE_APP_DIR="${decoy}/Applications" GH_TOKEN=decoy-token

# Stand-ins for the macOS commands an update runs that act on the real login session: tccutil
# (would reset the real nomouse's Accessibility entry), open (would start apps) and osascript.
# They log their arguments to calls.log; every sandbox PATH starts with them.
safe="${work}/safe-bin"
mkdir -p "$safe"
for tool in tccutil open osascript; do
    printf '#!/bin/sh\necho "%s $*" >>"%s"\n' "$tool" "$calls" >"${safe}/${tool}"
    chmod 755 "${safe}/${tool}"
done
sandbox_path="${safe}:/usr/bin:/bin:/usr/sbin:/sbin"

# Runs the installer with only HOME, PATH and TERM from outside (plus the arguments' NAME=VALUE
# words, where a PATH must keep sandbox_path), so the decoys above can never reach it. macOS
# apps go into HOME/Applications, and the installer does not re-sign them (a test turns it on).
sandbox() {
    local apps=()
    if [ "$os" = Darwin ]; then apps=(NOMOUSE_APP_DIR="${home}/Applications"); fi
    env -i HOME="$home" PATH="$sandbox_path" TERM=dumb NOMOUSE_SIGN_IDENTITY=none "${apps[@]}" "$@"
}
bin="${home}/.local/bin/nomouse"
settings="${home}/.local/bin/nomouse-settings"
autostart="${home}/.config/autostart/nomouse.desktop"
installed() {
    [ -x "$bin" ] && [ "$("$bin" --version)" = "nomouse 0.1.0-fake" ] || return 1
    if [ "$os" = Linux ]; then
        [ -x "$settings" ] && [ "$("$settings")" = "nomouse-settings 0.1.0-fake" ] || return 1
    fi
    if [ "$os" = Darwin ]; then
        [ -L "$bin" ] && [ -x "${home}/Applications/nomouse.app/Contents/MacOS/nomouse" ] || return 1
    fi
}
absent() {
    [ ! -e "$bin" ] && [ ! -L "$bin" ] && [ ! -e "${home}/Applications/nomouse.app" ] &&
        [ ! -e "$settings" ] && [ ! -e "$autostart" ] && [ -z "$(launcher_files)" ]
}

# The app launcher entry (Linux): the desktop entry and its icons in the sandbox's data dir.
app_id="io.github.yuyudhan.nomouse"
data="${home}/.local/share"
entry="${data}/applications/${app_id}.desktop"
icon_sizes="16 22 24 32 48 64 128 256 512"
# Every launcher file present (the entry, its icons, the tray icons), one per line.
launcher_files() {
    local size file
    for file in "$entry" "${data}/icons/hicolor/scalable/apps/${app_id}.svg" \
        "${data}/icons/hicolor/symbolic/apps/${app_id}-symbolic.svg" \
        "${data}/icons/hicolor/symbolic/apps/${app_id}-paused-symbolic.svg"; do
        [ -e "$file" ] && echo "$file"
    done
    for size in $icon_sizes; do
        file="${data}/icons/hicolor/${size}x${size}/apps/${app_id}.png"
        [ -e "$file" ] && echo "$file"
    done
    return 0
}

# --- Installs from a directory ---------------------------------------------------------------

esc="$(printf '\033')"
if out="$(sandbox sh "$installer" --from "$release" 2>&1)" && installed &&
    [[ "$out" == "nomouse installer"* ]] &&
    [[ "$out" == *"✓ Copied and verified (SHA-256 from SHA256SUMS)"* ]] &&
    [[ "$out" == *"nomouse 0.1.0-fake is installed."* ]] && [[ "$out" == *$'\nNext:\n'* ]]; then
    pass "--from installs, verifies against SHA256SUMS and prints the next steps"
else
    fail "--from install: ${out}"
fi
if [[ "$out" != *"$esc"* ]]; then
    pass "output that is not a terminal has no escape codes"
else
    fail "escape codes in output that is not a terminal: ${out}"
fi
if [[ "$out" == *"Add ~/.local/bin to PATH (in ~/.zshrc or ~/.profile): export PATH=\"\$HOME/.local/bin:\$PATH\""* ]]; then
    pass "a bin dir off PATH gets the PATH line under Next"
else
    fail "no PATH line: ${out}"
fi
pane="$(lib accessibility_pane)"
if [ "$os" = Darwin ]; then
    if [[ "$out" == *"✓ Installed ~/Applications/nomouse.app"* ]] &&
        [[ "$out" == *"✓ Linked ~/.local/bin/nomouse"* ]] &&
        [[ "$out" == *$'Next:\n  Switch on Accessibility: '"${pane} → nomouse"* ]] &&
        [[ "$out" == *"Welcome window opens that pane"* ]] && [[ "$out" != *"once more"* ]]; then
        pass "a first install puts Accessibility first, named as this macOS does (${pane##*→ })"
    else
        fail "first install next steps: ${out}"
    fi
    check "a first install quits, resets and starts nothing" "" "$(cat "$calls" 2>/dev/null || true)"
fi

# A terminal gets colour, unless NO_COLOR is set or TERM is dumb. (Python's pty gives the run a
# terminal; the installer's stdout and stderr are both on it.)
on_tty() {
    python3 -c 'import os, pty, sys
out = bytearray()
def read(fd):
    data = os.read(fd, 1024)
    out.extend(data)
    return data
status = pty.spawn(sys.argv[1:], read)
sys.stdout.buffer.write(bytes(out))
sys.exit(os.waitstatus_to_exitcode(status))' "$@" </dev/null >"${work}/tty.out" 2>&1 || true
    cat "${work}/tty.out"
}
if [[ "$(on_tty env -i HOME="$home" PATH="$sandbox_path" TERM=xterm-256color \
    NOMOUSE_APP_DIR="${home}/Applications" sh "$installer" --from "$release")" == *"${esc}[32m✓"* ]]; then
    pass "a terminal gets a green ✓"
else
    fail "no colour on a terminal: $(cat "${work}/tty.out")"
fi
for plain in "NO_COLOR=1 TERM=xterm-256color" "TERM=dumb"; do
    # shellcheck disable=SC2086 # NAME=VALUE words
    if [[ "$(on_tty env -i HOME="$home" PATH="$sandbox_path" $plain \
        NOMOUSE_APP_DIR="${home}/Applications" sh "$installer" --from "$release")" != *"$esc"* ]] &&
        grep -q 'is installed' "${work}/tty.out"; then
        pass "a terminal with ${plain} gets no escape codes"
    else
        fail "escape codes with ${plain}: $(cat "${work}/tty.out")"
    fi
done
for plain in "NO_COLOR=1 TERM=xterm-256color" "TERM=dumb" "TERM=xterm-256color"; do
    # shellcheck disable=SC2086 # NAME=VALUE words
    err="$(env -i HOME="$home" PATH="$sandbox_path" $plain sh "$installer" --bogus 2>&1 || true)"
    check "an error with ${plain}, not on a terminal, has no escape codes" \
        "nomouse: error: unknown option --bogus; see --help" "$err"
done

if [ "$os" = Linux ]; then
    expected=$(($(echo "$icon_sizes" | wc -w) + 4))
    check "the launcher entry, every icon size and the tray icons are installed" "$expected" \
        "$(launcher_files | wc -l | tr -d ' ')"
    check "the entry runs the installed nomouse" "Exec=\"${bin}\" launch" \
        "$(grep -m 1 '^Exec=' "$entry" || true)"
    check "the doctor action runs it too" "Exec=\"${bin}\" launch --doctor" \
        "$(grep '^Exec=' "$entry" | sed -n 2p)"
    check "TryExec names the installed nomouse" "TryExec=${bin}" "$(grep '^TryExec=' "$entry" || true)"
    for key in "Name=nomouse" "Icon=${app_id}" "Categories=Utility;Accessibility;" \
        "Keywords=mouse;keyboard;click;hints;homerow;accessibility;" "Actions=doctor;"; do
        if grep -qxF "$key" "$entry"; then pass "the entry has ${key}"; else fail "the entry lacks ${key}"; fi
    done
    if cmp -s "${root}/packaging/linux/share/icons/hicolor/48x48/apps/${app_id}.png" \
        "${data}/icons/hicolor/48x48/apps/${app_id}.png"; then
        pass "the icons are the release's"
    else
        fail "the installed 48x48 icon differs from packaging/linux/share"
    fi
    if [[ "$out" == *"✓ Installed ~/.local/bin/nomouse"* ]] &&
        [[ "$out" == *"✓ Installed ~/.local/bin/nomouse-settings"* ]] &&
        [[ "$out" == *"✓ Added nomouse to the app launcher"* ]]; then
        pass "the Linux steps name the binaries and the launcher entry"
    else
        fail "Linux steps: ${out}"
    fi
    if command -v desktop-file-validate >/dev/null 2>&1; then
        for file in "$entry" "${root}/packaging/linux/share/applications/${app_id}.desktop"; do
            if out="$(desktop-file-validate "$file" 2>&1)" && [ -z "$out" ]; then
                pass "desktop-file-validate accepts ${file#"${work}/"}"
            else
                fail "desktop-file-validate ${file}: ${out}"
            fi
        done
    else
        echo "skip - desktop-file-validate is not installed (Debian/Ubuntu: desktop-file-utils)"
    fi
    spaced="${work}/with space"
    if out="$(sandbox NOMOUSE_INSTALL_DIR="${spaced}/bin" sh "$installer" --from "$release" 2>&1)" &&
        grep -qxF "Exec=\"${spaced}/bin/nomouse\" launch" "$entry"; then
        pass "a bin dir with a space is quoted in Exec"
    else
        fail "bin dir with a space: ${out}"
    fi
    sandbox NOMOUSE_INSTALL_DIR="${spaced}/bin" sh "$installer" --uninstall >/dev/null 2>&1
    check "--uninstall removes the launcher entry and icons" "" "$(launcher_files)"
    sandbox sh "$installer" --from "$release" >/dev/null 2>&1
fi

: >"$calls"
if out="$(sandbox sh "$installer" --from "$release" 2>&1)" && installed; then
    pass "installing again replaces the installed copy"
    if [ "$os" = Darwin ]; then
        # The stub is unsigned, so the copies' signatures cannot match: the old entry goes.
        check "an update with a new signature resets the Accessibility entry, nothing else" \
            "tccutil reset Accessibility com.yuyudhan.nomouse" "$(cat "$calls")"
        if [[ "$out" == *"✓ Removed the old copy's Accessibility entry"* ]] &&
            [[ "$out" == *"Switch on Accessibility once more: ${pane} → nomouse"* ]] &&
            [[ "$out" == *"signature changed"* ]] && [[ "$out" == *"Start it: open"* ]]; then
            pass "the update says to switch nomouse on once more in ${pane##*→ }"
        else
            fail "update wording: ${out}"
        fi
    fi
else
    fail "reinstall: ${out}"
fi

if [ "$os" = Darwin ]; then
    # Updating over a running copy: it is quit before the app is replaced, the entry is reset,
    # and the new copy is started.
    : >"$calls"
    rm -f "${work}/quit-requested"
    "${home}/Applications/nomouse.app/Contents/MacOS/nomouse" daemon &
    old=$!
    out="$(sandbox sh "$installer" --from "$release" 2>&1)" || out="exit $?: ${out}"
    kill "$old" 2>/dev/null || true
    if wait "$old"; then quit_by_installer=yes; else quit_by_installer=no; fi
    check "an update over a running copy quits it first" yes "$quit_by_installer"
    check "then resets the Accessibility entry and starts the new copy" \
        "quit
tccutil reset Accessibility com.yuyudhan.nomouse
open ${home}/Applications/nomouse.app" "$(cat "$calls")"
    if installed && [[ "$out" == *"✓ Quit the running nomouse to update it"* ]] &&
        [[ "$out" == *"✓ Started the new nomouse"* ]] &&
        [[ "$out" == *"The new nomouse is starting from"* ]] && [[ "$out" == *"once more"* ]]; then
        pass "the update over a running copy says so and asks for the switch again"
    else
        fail "update over a running copy: ${out}"
    fi

    # The same signature (a reinstall of the same build, or a Developer ID signed release)
    # keeps the grant: nothing is reset.
    same="${work}/same-signature"
    mkdir -p "$same"
    printf '#!/bin/sh\necho "designated => identifier com.yuyudhan.nomouse and anchor apple generic"\n' \
        >"${same}/codesign"
    chmod 755 "${same}/codesign"
    : >"$calls"
    if out="$(sandbox PATH="${same}:${sandbox_path}" sh "$installer" --from "$release" 2>&1)" &&
        installed && [ ! -s "$calls" ] && [[ "$out" != *"once more"* ]] &&
        [[ "$out" == *"✓ Kept the Accessibility switch (same signature)"* ]]; then
        pass "an update with the same signature keeps the Accessibility entry"
    else
        fail "update with the same signature (calls: $(cat "$calls")): ${out}"
    fi

    # The installer's own signing (the sandbox turns it off above): two different builds
    # installed in turn get one designated requirement that names a certificate, not a hash.
    variant="${work}/release-variant"
    make_release "$variant" "${stub}
# a different build"
    sandbox sh "$installer" --uninstall >/dev/null 2>&1
    : >"$calls"
    app="${home}/Applications/nomouse.app"
    if out="$(sandbox NOMOUSE_SIGN_IDENTITY= sh "$installer" --from "$release" 2>&1)" && installed &&
        [[ "$out" == *'✓ Signed nomouse.app as "nomouse local signing"'* ]]; then
        first="$(codesign -d -r- "$app" 2>/dev/null | sed -n 's/^designated => //p')"
        first_exe="$(shasum -a 256 "${app}/Contents/MacOS/nomouse")"
        if out="$(sandbox NOMOUSE_SIGN_IDENTITY= sh "$installer" --from "$variant" 2>&1)" && installed &&
            [ "$first_exe" != "$(shasum -a 256 "${app}/Contents/MacOS/nomouse")" ]; then
            second="$(codesign -d -r- "$app" 2>/dev/null | sed -n 's/^designated => //p')"
            check "a rebuild signed by the installer keeps the designated requirement" "$first" "$second"
            if [[ "$second" == 'identifier "com.yuyudhan.nomouse" and certificate leaf = H"'* ]]; then
                pass "the requirement names the certificate, not the build's hash"
            else
                fail "requirement is not certificate based: ${second}"
            fi
            if [[ "$out" == *"✓ Kept the Accessibility switch (same signature)"* ]] &&
                [[ "$out" != *"once more"* ]] && [ ! -s "$calls" ]; then
                pass "an update of a signed build keeps the Accessibility entry"
            else
                fail "signed update (calls: $(cat "$calls")): ${out}"
            fi
        else
            fail "second signed install: ${out}"
        fi
    else
        fail "signed install: ${out}"
    fi
    sandbox sh "$installer" --uninstall >/dev/null 2>&1
    sandbox sh "$installer" --from "$release" >/dev/null 2>&1
fi

# A build this system cannot start fails the install with the loader's message and leaves the
# installed copy as it was.
: >"$calls"
if out="$(sandbox sh "$installer" --from "$broken" 2>&1)"; then
    fail "a build that cannot run here should fail the install: ${out}"
elif [[ "$out" == *"nomouse: error: the downloaded nomouse does not run on this system ("* ]] &&
    [[ "$out" == *"GLIBC_2.35 not found"* ]] && [[ "$out" == *"; nothing was installed"* ]] &&
    [[ "$out" != *"is installed."* ]] && [[ "$out" != *"Next:"* ]] && installed && [ ! -s "$calls" ]; then
    pass "a build that cannot run here fails with its error and keeps the installed copy"
else
    fail "a build that cannot run here (calls: $(cat "$calls")): ${out}"
fi

mkdir -p "${home}/.config/nomouse"
touch "${home}/.config/nomouse/config.toml"
if [ "$os" = Linux ]; then
    # Start at login, as the settings window or the tray menu writes it.
    mkdir -p "$(dirname "$autostart")"
    printf '[Desktop Entry]\nType=Application\nName=nomouse\nExec=%s daemon\n' "$bin" >"$autostart"
fi
out="$(sandbox sh "$installer" --uninstall 2>&1)"
if absent && [ -f "${home}/.config/nomouse/config.toml" ] &&
    [[ "$out" == *"✓ Removed ~/.local/bin/nomouse"* ]] &&
    [[ "$out" == *"✓ Kept the config in ~/.config/nomouse (add --purge to delete it)"* ]] &&
    [[ "$out" == *"nomouse is uninstalled."* ]]; then
    pass "--uninstall removes nomouse and keeps the config"
else
    fail "--uninstall: ${out}"
fi
if [ "$os" = Darwin ]; then
    if [[ "$out" == *"✓ Removed ~/Applications/nomouse.app"* ]] &&
        [[ "$out" == *"tccutil reset Accessibility com.yuyudhan.nomouse"* ]]; then
        pass "--uninstall on macOS removes the app and names the Accessibility entry to remove"
    else
        fail "--uninstall on macOS: ${out}"
    fi
fi

out="$(sandbox sh "$installer" --uninstall --purge 2>&1)"
if [[ "$out" == *"✓ Deleted the config directory ~/.config/nomouse"* ]] && [ ! -e "${home}/.config/nomouse" ]; then
    pass "--uninstall --purge names and deletes only the config directory"
else
    fail "--purge: ${out}"
fi

# A folder without SHA256SUMS: the asset's own .sha256 checks it.
own="${work}/own-sha256"
mkdir -p "$own"
cp "${release}/${archive}" "${release}/${archive}.sha256" "$own/"
if out="$(sandbox sh "$installer" --from "$own" 2>&1)" && installed &&
    [[ "$out" == *"✓ Copied and verified (SHA-256 from ${archive}.sha256)"* ]]; then
    pass "--from a folder without SHA256SUMS verifies against the asset's .sha256"
else
    fail "--from with only the .sha256: ${out}"
fi
sandbox sh "$installer" --uninstall >/dev/null 2>&1

# A wrong SHA256SUMS line, and a wrong .sha256 without SHA256SUMS, abort without installing.
for sums in SHA256SUMS "${archive}.sha256"; do
    bad="${work}/bad-${sums}"
    mkdir -p "$bad"
    cp "${release}/${archive}" "$bad/"
    printf '%064d  %s\n' 0 "$archive" >"${bad}/${sums}"
    if out="$(sandbox sh "$installer" --from "$bad" 2>&1)"; then
        fail "a wrong ${sums} should abort: ${out}"
    elif absent && [[ "$out" == *"nomouse: error: checksum mismatch for ${archive} (expected $(printf '%064d' 0), got "*"); nothing was installed, run the command again"* ]]; then
        pass "a wrong ${sums} aborts without installing"
    else
        fail "wrong ${sums}: ${out}"
    fi
done

empty="${work}/empty"
mkdir -p "$empty"
if out="$(sandbox sh "$installer" --from "$empty" 2>&1)"; then
    fail "a directory without the archive should fail: ${out}"
elif [[ "$out" == *"has no ${archive}; "* ]] && absent; then
    pass "a directory without the archive names the missing asset"
else
    fail "missing asset: ${out}"
fi

# NOMOUSE_VERSION picks the release (the step names it), and vX.Y.Z works too.
if out="$(sandbox NOMOUSE_VERSION=v0.1.0 NOMOUSE_INSTALL_BASE="file://${release}" sh "$installer" 2>&1)" &&
    installed && [[ "$out" == *", release v0.1.0 from file://${release}"* ]]; then
    pass "NOMOUSE_VERSION=v0.1.0 installs release v0.1.0"
else
    fail "NOMOUSE_VERSION: ${out}"
fi
sandbox sh "$installer" --uninstall >/dev/null 2>&1
if out="$(sandbox NOMOUSE_VERSION=1.2 sh "$installer" --from "$release" 2>&1)"; then
    fail "NOMOUSE_VERSION=1.2 should be refused: ${out}"
else
    pass "NOMOUSE_VERSION=1.2 is refused: ${out#nomouse: error: }"
fi

if [ "$os" = Linux ]; then
    # glibc 2.31 (Ubuntu 20.04): refused before anything is downloaded, nothing installed.
    oldglibc="${work}/old-glibc"
    mkdir -p "$oldglibc"
    printf '#!/bin/sh\necho "glibc 2.31"\n' >"${oldglibc}/getconf"
    chmod 755 "${oldglibc}/getconf"
    if out="$(sandbox PATH="${oldglibc}:${sandbox_path}" NOMOUSE_INSTALL_BASE="file://${release}" \
        sh "$installer" 2>&1)"; then
        fail "glibc 2.31 should be refused: ${out}"
    elif [[ "$out" == *"needs glibc 2.35 or later"* ]] && [[ "$out" == *"this system has glibc 2.31"* ]] &&
        [[ "$out" == *"$build_link"* ]] && [[ "$out" != *"verified"* ]] && absent; then
        pass "glibc 2.31 is refused before the download, naming the fix"
    else
        fail "glibc 2.31: ${out}"
    fi
fi

for args in "--version 1.2" "--purge" "--bogus" "--from ${work}/nowhere" "--version"; do
    # shellcheck disable=SC2086 # the words are the options under test
    if out="$(sandbox sh "$installer" $args 2>&1)"; then
        fail "${args} should be refused: ${out}"
    elif [[ "$out" == "nomouse: error: "*"; "* ]] && [ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" = 1 ]; then
        pass "${args} is refused with one line: ${out#nomouse: error: }"
    else
        fail "${args}: want one 'nomouse: error: <what>; <fix>' line, got: ${out}"
    fi
done
if out="$(sandbox sh "$installer" --help 2>&1)" &&
    [[ "$out" == *"curl -fsSL https://yuyudhan.github.io/nomouse/install"*"| sh -s -- [options]"* ]]; then
    pass "--help shows the piped form with sh -s --"
else
    fail "--help: ${out}"
fi

# --- Installs through a fake GitHub API ------------------------------------------------------

api="${work}/api"
releases="${api}/repos/yuyudhan/nomouse/releases"
mkdir -p "${releases}/assets" "${releases}/tags"
cp "${release}/${archive}" "${releases}/assets/101"
cp "${release}/${archive}.sha256" "${releases}/assets/102"
cp "${release}/SHA256SUMS" "${releases}/assets/103"
port=$((20000 + RANDOM % 20000))
base="http://127.0.0.1:${port}"
cat >"${releases}/latest" <<EOF
{"url":"${base}/repos/yuyudhan/nomouse/releases/1","name":"nomouse v0.1.0","assets":[
{"url":"${base}/repos/yuyudhan/nomouse/releases/assets/101","id":101,"name":"${archive}",
"uploader":{"url":"${base}/users/u"}},
{"url":"${base}/repos/yuyudhan/nomouse/releases/assets/102","id":102,"name":"${archive}.sha256",
"uploader":{"url":"${base}/users/u"}},
{"url":"${base}/repos/yuyudhan/nomouse/releases/assets/103","id":103,"name":"SHA256SUMS",
"uploader":{"url":"${base}/users/u"}}]}
EOF
cp "${releases}/latest" "${releases}/tags/v0.1.0"

# Serves the tree, answering 404 without the token's Authorization header and without the
# Accept header each kind of request needs, as GitHub does for a private repository.
python3 - "$api" "$port" >"${work}/api.log" 2>&1 <<'EOF' &
import http.server, os, sys
root, port = sys.argv[1], int(sys.argv[2])
class Api(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = os.path.join(root, self.path.lstrip("/"))
        accept = "application/octet-stream" if "/assets/" in self.path else "application/vnd.github+json"
        ok = (self.headers.get("Authorization") == "Bearer test-token"
              and self.headers.get("Accept") == accept and os.path.isfile(path))
        if not ok:
            self.send_response(404)
            self.end_headers()
            return
        with open(path, "rb") as f:
            body = f.read()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", port), Api).serve_forever()
EOF
server=$!
for _ in $(seq 50); do
    curl --silent --output /dev/null "${base}/" && break
    sleep 0.1
done

if out="$(sandbox GH_TOKEN=test-token NOMOUSE_API_BASE="$base" sh "$installer" 2>&1)" && installed &&
    [[ "$out" == *"✓ Downloaded and verified (SHA-256 from SHA256SUMS)"* ]]; then
    pass "GH_TOKEN installs the latest release through the API, checked against SHA256SUMS"
else
    fail "API install: ${out}"
fi
sandbox sh "$installer" --uninstall >/dev/null 2>&1

if out="$(sandbox GITHUB_TOKEN=test-token NOMOUSE_API_BASE="$base" sh "$installer" --version 0.1.0 2>&1)" &&
    installed; then
    pass "GITHUB_TOKEN with --version installs that tag through the API"
else
    fail "API install of a tag: ${out}"
fi
sandbox sh "$installer" --uninstall >/dev/null 2>&1

if out="$(sandbox GH_TOKEN=test-token NOMOUSE_API_BASE="$base" NOMOUSE_VERSION=0.1.0 sh "$installer" 2>&1)" &&
    installed && [[ "$out" == *", release v0.1.0"* ]]; then
    pass "NOMOUSE_VERSION installs that tag through the API"
else
    fail "API install with NOMOUSE_VERSION: ${out}"
fi
sandbox sh "$installer" --uninstall >/dev/null 2>&1

if out="$(sandbox GH_TOKEN=test-token NOMOUSE_API_BASE="$base" NOMOUSE_VERSION=9.9.9 sh "$installer" --version v0.1.0 2>&1)" &&
    installed; then
    pass "--version wins over NOMOUSE_VERSION"
else
    fail "--version over NOMOUSE_VERSION: ${out}"
fi
sandbox sh "$installer" --uninstall >/dev/null 2>&1

if out="$(sandbox GH_TOKEN=wrong-token NOMOUSE_API_BASE="$base" sh "$installer" 2>/dev/null)"; then
    fail "a refused token should fail: ${out}"
fi
err="$(sandbox GH_TOKEN=wrong-token NOMOUSE_API_BASE="$base" sh "$installer" 2>&1 >/dev/null || true)"
if absent && [ "$err" = "nomouse: error: release not found: ${base}/repos/yuyudhan/nomouse/releases/latest (HTTP 404, or the token cannot read yuyudhan/nomouse); the versions are at https://github.com/yuyudhan/nomouse/releases" ]; then
    pass "a token the API refuses fails with one line before installing"
else
    fail "refused token: ${err}"
fi

err="$(sandbox GH_TOKEN=test-token NOMOUSE_API_BASE="$base" sh "$installer" --version 9.9.9 2>&1 >/dev/null)" &&
    fail "a missing release should fail"
if absent && [ "$err" = "nomouse: error: release v9.9.9 not found (HTTP 404, or the token cannot read yuyudhan/nomouse); the versions are at https://github.com/yuyudhan/nomouse/releases" ]; then
    pass "a release that does not exist fails with one line naming it"
else
    fail "missing release: ${err}"
fi

# --- Installs through a network that cuts connections ----------------------------------------

# Some networks cut a share of connections to GitHub's download host mid-handshake (`curl: (35)
# ... unexpected eof while reading`). This server hangs up without answering the first two
# requests for every file; the installer's own retries must get through.
if curl --retry-all-errors --version >/dev/null 2>&1; then
    flaky_port=$((port + 1))
    python3 - "$release" "$flaky_port" >"${work}/flaky.log" 2>&1 <<'EOF' &
import collections, http.server, os, sys
root, port = sys.argv[1], int(sys.argv[2])
seen = collections.Counter()
class Flaky(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        seen[self.path] += 1
        path = os.path.join(root, self.path.lstrip("/"))
        if self.path == "/" or seen[self.path] <= 2 or not os.path.isfile(path):
            self.close_connection = True
            return
        with open(path, "rb") as f:
            body = f.read()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", port), Flaky).serve_forever()
EOF
    flaky=$!
    # Up once curl gets past connecting: the server hangs up on "/", curl exits 52, not 7.
    for _ in $(seq 50); do
        status=0
        curl --silent --output /dev/null "http://127.0.0.1:${flaky_port}/" || status=$?
        [ "$status" -ne 7 ] && break
        sleep 0.1
    done
    if out="$(sandbox NOMOUSE_INSTALL_BASE="http://127.0.0.1:${flaky_port}" sh "$installer" 2>&1)" &&
        installed; then
        pass "downloads retry connections the network cuts"
    else
        fail "install through a network that cuts connections: ${out}"
    fi
    sandbox sh "$installer" --uninstall >/dev/null 2>&1
else
    echo "skip - this curl has no --retry-all-errors (7.71+): cut connections are not retried"
fi

# --- The one-line install leaves nothing behind ----------------------------------------------

# The documented one-liner (the first sh block under "## The one-line install" in
# docs/install.md), with only its URL pointed at a local server, run from a fresh empty folder
# with TMPDIR set to a fresh empty directory, piped into each shell. Whether it installs, gets a
# 404 for the installer or an asset, or a checksum mismatch, both must be empty afterwards: the
# script arrives on stdin and every download lives in a temporary directory removed on every exit.
pages="https://yuyudhan.github.io/nomouse/install"
fence="$(printf '\140\140\140')"
one_liner="$(awk -v fence="$fence" '
    /^## The one-line install$/ { section = 1; next }
    section && /^## / { exit }
    section && $0 == fence "sh" { block = 1; next }
    block && $0 == fence { exit }
    block { print }
' "${root}/docs/install.md")"
check "docs/install.md's one-line install" "curl -fsSL ${pages} | sh" "$one_liner"

# Each folder of the tree is one release: ok (install and good assets), badsum (a SHA256SUMS
# that does not match), noasset (install only), hang (install, and an archive the server never
# finishes sending). Anything else, such as missing/install, is a 404.
www="${work}/www"
for case in ok badsum noasset hang; do
    mkdir -p "${www}/${case}"
    cp "$installer" "${www}/${case}/install"
done
cp "${release}/${archive}" "${release}/SHA256SUMS" "${www}/ok/"
cp "${release}/${archive}" "${www}/badsum/"
printf '%064d  %s\n' 0 "$archive" >"${www}/badsum/SHA256SUMS"
python3 - "$www" "${work}/www.port" >"${work}/www.log" 2>&1 <<'EOF' &
import os, socketserver, sys, time
import http.server
root, port_file = sys.argv[1], sys.argv[2]
class Release(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith("/hang/") and not self.path.endswith("/install"):
            self.send_response(200)
            self.send_header("Content-Length", "1000000")
            self.end_headers()
            self.wfile.write(b"x" * 1000)
            self.wfile.flush()
            time.sleep(60)
            return
        path = os.path.join(root, self.path.lstrip("/"))
        if os.path.isfile(path):
            with open(path, "rb") as f:
                body = f.read()
            self.send_response(200)
        else:
            body = b"Not Found"
            self.send_response(404)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
# TCPServer, not HTTPServer, whose fully qualified name lookup is slow on macOS runners.
class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    daemon_threads = True
server = Server(("127.0.0.1", 0), Release)
with open(port_file + ".tmp", "w") as out:
    out.write(str(server.server_address[1]))
os.rename(port_file + ".tmp", port_file)
server.serve_forever()
EOF
www_server=$!
for _ in $(seq 300); do
    [ -s "${work}/www.port" ] && break
    kill -0 "$www_server" 2>/dev/null || break
    sleep 0.1
done
www_base="http://127.0.0.1:$(cat "${work}/www.port" 2>/dev/null || echo 0)"
# Caps curl's retries of a refused connection to a few seconds; the command line stays the
# documented one.
printf 'retry-max-time = 3\n' >"${home}/.curlrc"

# macOS's mktemp -d ignores TMPDIR when given no template, and the check below must see every
# temporary directory: this mktemp, first on the PATH of these runs, makes a bare `mktemp -d`
# honour TMPDIR, as GNU mktemp does, and logs every path it creates to TEST_MKTEMP_LOG, so the
# check also proves each one was removed.
shim="${work}/shim-bin"
mkdir -p "$shim"
cat >"${shim}/mktemp" <<EOF
#!/bin/sh
if [ "\$#" -eq 1 ] && [ "\$1" = -d ]; then
    set -- -d "\${TMPDIR:-/tmp}/tmp.XXXXXXXXXX"
fi
made="\$($(command -v mktemp) "\$@")" || exit 1
echo "\$made" >>"\$TEST_MKTEMP_LOG"
echo "\$made"
EOF
chmod 755 "${shim}/mktemp"

# leftovers DIR...: every entry still in the DIRs, one per line.
leftovers() {
    local dir
    for dir in "$@"; do
        find "$dir" -mindepth 1 -maxdepth 1
    done
}

# fresh_dirs: a new empty folder to run from (`here`), a new empty TMPDIR (`scratch_tmp`) and
# the log of the temporary directories the run makes (`made`, beside them).
fresh_dirs() {
    here="$(mktemp -d "${work}/here.XXXXXX")"
    scratch_tmp="$(mktemp -d "${work}/tmp.XXXXXX")"
    made="${scratch_tmp}.made"
    : >"$made"
}

# one_line SHELL CASE [ASSETS [OPTIONS [PATH]]]: runs the one-liner piping install from the
# release at CASE (a folder of the tree, or a full URL base) into SHELL, with the installer's
# assets from ASSETS (default: the same), OPTIONS after `SHELL -s --` as the docs place them,
# and PATH first on the PATH; from `here` with TMPDIR `scratch_tmp` (call fresh_dirs first).
# Prints the output, returns the status.
one_line() {
    local shell="$1" case="$2" assets="${3:-}" options="${4:-}" extra="${5:-}" url line
    url="$case"
    [[ "$url" == *://* ]] || url="${www_base}/${case}"
    [[ "$assets" == *://* ]] || assets="${www_base}/${assets:-$case}"
    line="${one_liner//"$pages"/${url}/install}"
    line="${line%| sh}| ${shell}${options:+ -s -- ${options}}"
    (cd "$here" && sandbox PATH="${extra:+${extra}:}${shim}:${sandbox_path}" TEST_MKTEMP_LOG="$made" \
        TMPDIR="$scratch_tmp" NOMOUSE_INSTALL_BASE="$assets" \
        sh -c "$line")
}

# expect_clean NAME [TEMPS]: passes NAME when the run made TEMPS temporary directories (default:
# at least one), all inside its TMPDIR, and its folder and TMPDIR are both empty afterwards.
expect_clean() {
    local left dir count
    left="$(leftovers "$here" "$scratch_tmp")"
    while IFS= read -r dir; do
        if [ -e "$dir" ]; then left="${left} ${dir}"; fi
        if [[ "$dir" != "${scratch_tmp}/"* ]]; then left="${left} ${dir}(outside TMPDIR)"; fi
    done <"$made"
    count="$(wc -l <"$made" | tr -d ' ')"
    if [ -z "${2:-}" ] && [ "$count" -eq 0 ]; then
        fail "$1 made no temporary directory (is the mktemp shim on its PATH?)"
    elif [ -n "${2:-}" ] && [ "$count" -ne "$2" ]; then
        fail "$1 made ${count} temporary directories, not $2"
    elif [ -z "$left" ]; then
        pass "$1 leaves the folder and TMPDIR empty (${count} temporary directories removed)"
    else
        fail "$1 left files behind: $(echo "$left" | tr '\n' ' ')"
    fi
}

# A port nothing listens on: opened, then closed again.
closed_port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
nowhere="http://127.0.0.1:${closed_port}"
# Stand-in uname for systems the release has no build for.
for system in "FreeBSD amd64" "Linux riscv64"; do
    dir="${work}/uname-${system%% *}"
    mkdir -p "$dir"
    # shellcheck disable=SC2016 # the stand-in's own script, expanded when it runs
    printf '#!/bin/sh\ncase "$1" in -m) echo %s ;; *) echo %s ;; esac\n' \
        "${system#* }" "${system%% *}" >"${dir}/uname"
    chmod 755 "${dir}/uname"
done

# The shells the one-liner may be piped into: sh (bash in POSIX mode on macOS, dash on Debian
# and Ubuntu), dash, bash and zsh where installed.
shells="sh"
for shell in dash bash zsh; do
    if command -v "$shell" >/dev/null 2>&1; then shells="${shells} ${shell}"; fi
done
echo "one-liner shells: ${shells}"
for shell in $shells; do
    fresh_dirs
    if out="$(one_line "$shell" ok 2>&1)" && installed &&
        [[ "$out" == *"✓ Downloaded and verified (SHA-256 from SHA256SUMS)"* ]] &&
        [[ "$out" == *"nomouse 0.1.0-fake is installed."* ]]; then
        pass "${shell}: the one-liner installs"
    else
        fail "${shell}: the one-liner did not install: ${out}"
    fi
    expect_clean "${shell}: an install" 1

    fresh_dirs
    if out="$(one_line "$shell" ok 2>&1)" && installed && [[ "$out" == *"nomouse 0.1.0-fake is installed."* ]]; then
        pass "${shell}: running the one-liner again reinstalls in place"
    else
        fail "${shell}: the second one-liner run did not install: ${out}"
    fi
    expect_clean "${shell}: a second install" 1
    sandbox sh "$installer" --uninstall >/dev/null 2>&1

    fresh_dirs
    if out="$(one_line "$shell" ok ok "--version v0.1.0" 2>&1)" && installed &&
        [[ "$out" == *", release v0.1.0 from ${www_base}/ok"* ]]; then
        pass "${shell}: the one-liner with -s -- --version v0.1.0 installs that version"
    else
        fail "${shell}: the one-liner with --version v0.1.0: ${out}"
    fi
    expect_clean "${shell}: an install with --version" 1

    fresh_dirs
    if out="$(one_line "$shell" ok ok "--uninstall" 2>&1)" && absent &&
        [[ "$out" == *"nomouse is uninstalled."* ]]; then
        pass "${shell}: the one-liner with -s -- --uninstall uninstalls"
    else
        fail "${shell}: the one-liner with --uninstall: ${out}"
    fi
    expect_clean "${shell}: an uninstall" 0

    # case|install from|assets from|PATH first|temporary directories|the one error line holds
    # A failure of the one-liner's own download is curl's line, and the shell, given an empty
    # script, runs nothing (its exit status is the shell's, 0, without pipefail).
    for spec in \
        "installer not found|missing|||0|curl: (22) " \
        "no answer for the installer|${nowhere}|ok||0|curl: (7) " \
        "missing release asset|noasset|||1|nomouse: error: release not found: ${www_base}/noasset/${archive} (HTTP 404); the versions are at https://github.com/yuyudhan/nomouse/releases" \
        "network error in the installer|ok|${nowhere}||1|nomouse: error: could not reach 127.0.0.1:${closed_port} (" \
        "bad checksum|badsum|||1|nomouse: error: checksum mismatch for ${archive} (expected $(printf '%064d' 0), got " \
        "unsupported OS|ok||${work}/uname-FreeBSD|0|nomouse: error: no nomouse build for FreeBSD amd64; build from source: ${build_link}" \
        "unsupported CPU|ok||${work}/uname-Linux|0|nomouse: error: no nomouse build for Linux riscv64; build from source: ${build_link}"; do
        IFS='|' read -r name case assets extra temps want <<<"$spec"
        fresh_dirs
        status=0
        one_line "$shell" "$case" "$assets" "" "$extra" >"${work}/stdout" 2>"${work}/stderr" || status=$?
        if [[ "$want" == nomouse:* ]] && [ "$status" -eq 0 ]; then
            fail "${shell}: ${name} exited 0: $(cat "${work}/stdout" "${work}/stderr")"
        elif ! absent; then
            fail "${shell}: ${name} installed something"
        elif [[ "$want" == curl:* ]] && [ -s "${work}/stdout" ]; then
            fail "${shell}: ${name} ran a script: $(cat "${work}/stdout")"
        else
            pass "${shell}: ${name} installs nothing (exit ${status})"
        fi
        errors="$(cat "${work}/stderr")"
        if [ "$(printf '%s\n' "$errors" | wc -l | tr -d ' ')" = 1 ] && [[ "$errors" == *"$want"* ]]; then
            pass "${shell}: ${name}: one error line: ${errors}"
        else
            fail "${shell}: ${name}: want one error line holding '${want}', got: ${errors}"
        fi
        expect_clean "${shell}: ${name}" "$temps"
    done
done

# Interrupted (Ctrl-C) or killed mid-download, install.sh still removes its work directory. The
# installer runs in its own process group, started through Python so that, though started in
# the background, it does not inherit an ignored SIGINT; each signal goes to the whole group, as
# Ctrl-C in a terminal does, while curl waits on the hanging archive.
apps=()
if [ "$os" = Darwin ]; then apps=(NOMOUSE_APP_DIR="${home}/Applications"); fi
for shell in $shells; do
    for signal in INT TERM HUP; do
        fresh_dirs
        rm -f "${work}/hang.pid"
        (cd "$here" && python3 -c 'import os, signal, sys
os.setsid()
signal.signal(signal.SIGINT, signal.SIG_DFL)
with open(sys.argv[1], "w") as out:
    out.write(str(os.getpid()))
os.execvp(sys.argv[2], sys.argv[2:])' "${work}/hang.pid" env -i HOME="$home" \
            PATH="${shim}:${sandbox_path}" TEST_MKTEMP_LOG="$made" TERM=dumb "${apps[@]}" \
            TMPDIR="$scratch_tmp" NOMOUSE_INSTALL_BASE="${www_base}/hang" \
            "$shell" "$installer" >"${work}/hang.log" 2>&1) &
        hung=$!
        for _ in $(seq 300); do
            [ -n "$(find "$scratch_tmp" -mindepth 2 -name "$archive" 2>/dev/null)" ] && break
            sleep 0.1
        done
        if [ -z "$(find "$scratch_tmp" -mindepth 2 -name "$archive" 2>/dev/null)" ]; then
            fail "${shell} ${signal}: the installer never started downloading: $(cat "${work}/hang.log")"
        fi
        kill -s "$signal" -- "-$(cat "${work}/hang.pid")" 2>/dev/null || true
        status=0
        # (wait's stderr: bash's own "Hangup" notice for a job a signal ended.)
        wait "$hung" 2>/dev/null || status=$?
        if [ "$status" -eq 0 ]; then
            fail "${shell} ${signal}: the interrupted installer exited 0"
        else
            pass "${shell} ${signal}: the interrupted installer exits non-zero (${status})"
        fi
        expect_clean "${shell} ${signal} mid-download" 1
    done
done
kill "$www_server" 2>/dev/null || true
rm -f "${home}/.curlrc"

# --- The caller's environment was never used -------------------------------------------------

if [ -f "${decoy}/config/nomouse/config.toml" ] && [ -f "${decoy}/bin/nomouse" ] &&
    [ -z "$(ls -A "${decoy}/Applications")" ]; then
    pass "the caller's XDG_CONFIG_HOME, NOMOUSE_INSTALL_DIR and NOMOUSE_APP_DIR were never used"
else
    fail "the installer touched the caller's environment"
fi

if [ "$failures" -ne 0 ]; then
    echo "test-installer: ${failures} failed"
    exit 1
fi
echo "test-installer: all passed"
