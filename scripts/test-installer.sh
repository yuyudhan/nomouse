#!/usr/bin/env bash
# Tests install.sh without building nomouse: its pure functions (target and asset names, the
# GitHub release JSON parser) by sourcing it, then whole installs from a fake release for this
# machine's OS: a stub `nomouse` in the archive layout the release uses (a tarball with the app
# launcher entry and icons from packaging/linux/share on Linux, nomouse.app zipped on macOS).
# The fake release is installed with --from DIR and through a local fake GitHub API that
# refuses requests without the token, all into a throwaway HOME in an env -i sandbox.
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
cleanup() {
    local pid
    for pid in "$server" "$flaky"; do
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

lib() {
    # shellcheck disable=SC1090 # the installer under test
    NOMOUSE_INSTALL_NO_MAIN=1 source "$installer"
    "$@"
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

# The next steps name the fix for the desktop the installer runs in: GNOME on Wayland cannot
# run nomouse, so it must say to log in with GNOME on Xorg instead of printing Sway bindings.
steps="$(XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=ubuntu:GNOME lib linux_next_steps /opt/bin 2>&1)" || true
if [[ "$steps" == *'"Ubuntu on Xorg"'* ]] && [[ "$steps" == *'"GNOME on Xorg"'* ]] &&
    [[ "$steps" != *Sway* ]]; then
    pass "GNOME on Wayland: the next steps say to log in with GNOME on Xorg"
else
    fail "GNOME on Wayland next steps: ${steps}"
fi
steps="$(XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway lib linux_next_steps /opt/bin 2>&1)" || true
if [[ "$steps" == *"bindsym Ctrl+space exec nomouse hints"* ]] && [[ "$steps" != *Xorg* ]]; then
    pass "Sway: the next steps bind nomouse hints in the compositor"
else
    fail "Sway next steps: ${steps}"
fi
steps="$(XDG_SESSION_TYPE=x11 XDG_CURRENT_DESKTOP=ubuntu:GNOME lib linux_next_steps /opt/bin 2>&1)" || true
if [[ "$steps" == *"settings > General > Start at login"* ]] &&
    [[ "$steps" == *"app launcher"* ]] && [[ "$steps" != *Xorg* ]]; then
    pass "GNOME on Xorg: the next steps start the daemon and point at Start at login"
else
    fail "GNOME on Xorg next steps: ${steps}"
fi

# --- A fake release for this machine ----------------------------------------------------------

target="$(lib asset_for "$os" "$(uname -m)")"
archive="$(lib archive_for "$target")"
release="${work}/release"
stage="${work}/stage"
mkdir -p "$release" "$stage"
stub='#!/bin/sh
echo "nomouse 0.1.0-fake"'
if [ "$os" = Darwin ]; then
    mkdir -p "${stage}/nomouse.app/Contents/MacOS"
    printf '%s\n' "$stub" >"${stage}/nomouse.app/Contents/MacOS/nomouse"
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
    (cd "$stage" && ditto -c -k --keepParent nomouse.app "${release}/${archive}")
else
    mkdir -p "${stage}/nomouse-${target}"
    printf '%s\n' "$stub" >"${stage}/nomouse-${target}/nomouse"
    printf '#!/bin/sh\necho "nomouse-settings 0.1.0-fake"\n' >"${stage}/nomouse-${target}/nomouse-settings"
    chmod 755 "${stage}/nomouse-${target}/nomouse" "${stage}/nomouse-${target}/nomouse-settings"
    cp -R "${root}/packaging/linux/share" "${stage}/nomouse-${target}/"
    tar -C "$stage" -czf "${release}/${archive}" "nomouse-${target}"
fi
"${root}/scripts/release/checksum.sh" "${release}/${archive}"

home="${work}/home"
decoy="${work}/decoy"
mkdir -p "$home" "${decoy}/config/nomouse" "${decoy}/bin" "${decoy}/Applications"
touch "${decoy}/config/nomouse/config.toml" "${decoy}/bin/nomouse"
export XDG_CONFIG_HOME="${decoy}/config" NOMOUSE_INSTALL_DIR="${decoy}/bin"
export NOMOUSE_APP_DIR="${decoy}/Applications" GH_TOKEN=decoy-token

# Runs the installer with only HOME, PATH and TERM from outside (plus the arguments' NAME=VALUE
# words), so the decoys above can never reach it. macOS apps go into HOME/Applications.
sandbox() {
    local apps=()
    if [ "$os" = Darwin ]; then apps=(NOMOUSE_APP_DIR="${home}/Applications"); fi
    env -i HOME="$home" PATH=/usr/bin:/bin:/usr/sbin:/sbin TERM=dumb "${apps[@]}" "$@"
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

if out="$(sandbox bash "$installer" --from "$release" 2>&1)" && installed &&
    [[ "$out" == *"checksum ok: ${archive}"* ]] && [[ "$out" == *"next steps:"* ]]; then
    pass "--from installs, verifies and prints the next steps"
else
    fail "--from install: ${out}"
fi

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
    if out="$(sandbox NOMOUSE_INSTALL_DIR="${spaced}/bin" bash "$installer" --from "$release" 2>&1)" &&
        grep -qxF "Exec=\"${spaced}/bin/nomouse\" launch" "$entry"; then
        pass "a bin dir with a space is quoted in Exec"
    else
        fail "bin dir with a space: ${out}"
    fi
    sandbox NOMOUSE_INSTALL_DIR="${spaced}/bin" bash "$installer" --uninstall >/dev/null 2>&1
    check "--uninstall removes the launcher entry and icons" "" "$(launcher_files)"
    sandbox bash "$installer" --from "$release" >/dev/null 2>&1
fi

if out="$(sandbox bash "$installer" --from "$release" 2>&1)" && installed; then
    pass "installing again replaces the installed copy"
else
    fail "reinstall: ${out}"
fi

mkdir -p "${home}/.config/nomouse"
touch "${home}/.config/nomouse/config.toml"
if [ "$os" = Linux ]; then
    # Start at login, as the settings window or the tray menu writes it.
    mkdir -p "$(dirname "$autostart")"
    printf '[Desktop Entry]\nType=Application\nName=nomouse\nExec=%s daemon\n' "$bin" >"$autostart"
fi
out="$(sandbox bash "$installer" --uninstall 2>&1)"
if absent && [ -f "${home}/.config/nomouse/config.toml" ] && [[ "$out" == *"kept the config"* ]]; then
    pass "--uninstall removes nomouse and keeps the config"
else
    fail "--uninstall: ${out}"
fi

out="$(sandbox bash "$installer" --uninstall --purge 2>&1)"
if [[ "$out" == *"--purge deletes: ${home}/.config/nomouse"* ]] && [ ! -e "${home}/.config/nomouse" ]; then
    pass "--uninstall --purge names and deletes only the config directory"
else
    fail "--purge: ${out}"
fi

bad="${work}/bad"
mkdir -p "$bad"
cp "${release}/${archive}" "$bad/"
echo "0000  ${archive}" >"${bad}/${archive}.sha256"
if out="$(sandbox bash "$installer" --from "$bad" 2>&1)"; then
    fail "a wrong checksum should abort: ${out}"
elif absent && [[ "$out" == *"checksum mismatch"* ]]; then
    pass "a wrong checksum aborts without installing"
else
    fail "wrong checksum: ${out}"
fi

empty="${work}/empty"
mkdir -p "$empty"
if out="$(sandbox bash "$installer" --from "$empty" 2>&1)"; then
    fail "a directory without the archive should fail: ${out}"
elif [[ "$out" == *"has no ${archive}"* ]] && absent; then
    pass "a directory without the archive names the missing asset"
else
    fail "missing asset: ${out}"
fi

for args in "--version 1.2" "--purge" "--bogus" "--from ${work}/nowhere"; do
    # shellcheck disable=SC2086 # the words are the options under test
    if out="$(sandbox bash "$installer" $args 2>&1)"; then
        fail "${args} should be refused: ${out}"
    else
        pass "${args} is refused: ${out#nomouse: error: }"
    fi
done

# --- Installs through a fake GitHub API ------------------------------------------------------

api="${work}/api"
releases="${api}/repos/yuyudhan/nomouse/releases"
mkdir -p "${releases}/assets" "${releases}/tags"
cp "${release}/${archive}" "${releases}/assets/101"
cp "${release}/${archive}.sha256" "${releases}/assets/102"
port=$((20000 + RANDOM % 20000))
base="http://127.0.0.1:${port}"
cat >"${releases}/latest" <<EOF
{"url":"${base}/repos/yuyudhan/nomouse/releases/1","name":"nomouse v0.1.0","assets":[
{"url":"${base}/repos/yuyudhan/nomouse/releases/assets/101","id":101,"name":"${archive}",
"uploader":{"url":"${base}/users/u"}},
{"url":"${base}/repos/yuyudhan/nomouse/releases/assets/102","id":102,"name":"${archive}.sha256",
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

if out="$(sandbox GH_TOKEN=test-token NOMOUSE_API_BASE="$base" bash "$installer" 2>&1)" && installed; then
    pass "GH_TOKEN installs the latest release through the API"
else
    fail "API install: ${out}"
fi
sandbox bash "$installer" --uninstall >/dev/null 2>&1

if out="$(sandbox GITHUB_TOKEN=test-token NOMOUSE_API_BASE="$base" bash "$installer" --version 0.1.0 2>&1)" &&
    installed; then
    pass "GITHUB_TOKEN with --version installs that tag through the API"
else
    fail "API install of a tag: ${out}"
fi
sandbox bash "$installer" --uninstall >/dev/null 2>&1

if out="$(sandbox GH_TOKEN=wrong-token NOMOUSE_API_BASE="$base" bash "$installer" 2>&1)"; then
    fail "a refused token should fail: ${out}"
elif absent && [[ "$out" == *"cannot read the release"* ]]; then
    pass "a token the API refuses fails before installing"
else
    fail "refused token: ${out}"
fi

if out="$(sandbox GH_TOKEN=test-token NOMOUSE_API_BASE="$base" bash "$installer" --version 9.9.9 2>&1)"; then
    fail "a missing release should fail: ${out}"
elif absent; then
    pass "a release that does not exist fails before installing"
else
    fail "missing release: ${out}"
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
    if out="$(sandbox NOMOUSE_INSTALL_BASE="http://127.0.0.1:${flaky_port}" bash "$installer" 2>&1)" &&
        installed; then
        pass "downloads retry connections the network cuts"
    else
        fail "install through a network that cuts connections: ${out}"
    fi
    sandbox bash "$installer" --uninstall >/dev/null 2>&1
else
    echo "skip - this curl has no --retry-all-errors (7.71+): cut connections are not retried"
fi

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
