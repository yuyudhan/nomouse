#!/usr/bin/env bash
# End-to-end test of install.sh against a locally built release directory (target/dist by
# default): serves it over HTTP on 127.0.0.1 and installs, runs, uninstalls and purges into a
# throwaway HOME.
#
# Every installer run goes through `sandbox`, which starts from an empty environment
# (env -i) with only HOME (the temp dir), a minimal PATH and TERM=dumb, so nothing the caller
# has set - XDG_CONFIG_HOME, NOMOUSE_*, a real HOME - can point the installer at real files.
# The harness proves that about itself: it plants decoy XDG_CONFIG_HOME and NOMOUSE_INSTALL_DIR
# directories in its own environment and fails if the sandbox sees them or anything touches them.
#
# Usage: scripts/test-install.sh [DIST]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dist="$(cd "${1:-${root}/target/dist}" && pwd)"
[ -f "${dist}/install.sh" ] || {
    echo "test-install: no install.sh in ${dist}; run just release-dry-run first" >&2
    exit 1
}

work="$(mktemp -d)"
server=""
cleanup() {
    if [ -n "$server" ]; then
        kill "$server" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

home="${work}/home"
decoy="${work}/decoy"
mkdir -p "$home" "${decoy}/config/nomouse" "${decoy}/bin"
touch "${decoy}/config/nomouse/config.toml" "${decoy}/bin/nomouse"
export XDG_CONFIG_HOME="${decoy}/config" NOMOUSE_INSTALL_DIR="${decoy}/bin"

failures=0
pass() {
    echo "ok   - $*"
}
fail() {
    echo "FAIL - $*"
    failures=$((failures + 1))
}

port=$((20000 + RANDOM % 20000))
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$dist" >/dev/null 2>&1 &
server=$!
base="http://127.0.0.1:${port}"
for _ in $(seq 50); do
    curl -fsS -o /dev/null "${base}/install.sh" 2>/dev/null && break
    sleep 0.1
done

sandbox() {
    env -i HOME="$home" PATH=/usr/bin:/bin TERM=dumb "$@"
}

# 1. The sandbox environment is exactly HOME, PATH and TERM.
seen="$(sandbox env | cut -d = -f 1 | sort | tr '\n' ' ')"
if [ "$seen" = "HOME PATH TERM " ]; then
    pass "sandbox environment is only HOME PATH TERM"
else
    fail "sandbox environment leaks: ${seen}"
fi

# 2. Install.
if out="$(sandbox NOMOUSE_INSTALL_BASE="$base" bash "${dist}/install.sh" 2>&1)" &&
    [[ "$out" == *"checksum ok"* ]] && [ -x "${home}/.local/bin/nomouse" ]; then
    pass "install.sh installs into \$HOME/.local/bin with a verified checksum"
else
    fail "install: ${out}"
fi

# 3. The installed binary runs.
if version="$(sandbox "${home}/.local/bin/nomouse" --version)" && [[ "$version" == "nomouse "* ]]; then
    version_installed="$version"
    pass "installed binary runs: ${version}"
else
    fail "nomouse --version: ${version:-no output}"
fi

# 3a. The settings window is installed beside it. With GTK 4 and libadwaita here it runs
# (--version needs no display); without them (a bare CI runner) the installer must have named
# the packages to add.
settings="${home}/.local/bin/nomouse-settings"
if [ ! -x "$settings" ]; then
    fail "no settings window at ${settings}"
elif ldd "$settings" 2>/dev/null | grep -q 'not found'; then
    if [[ "$out" == *"the settings window needs GTK 4 and libadwaita"* ]] &&
        [[ "$out" == *"install them with:"* ]]; then
        pass "without GTK 4 here, install.sh says which packages the settings window needs"
    else
        fail "GTK 4 is missing here and install.sh did not say so: ${out}"
    fi
elif version="$(sandbox "$settings" --version)" &&
    [ "$version" = "nomouse-settings ${version_installed#nomouse }" ]; then
    pass "the settings window is installed beside nomouse and runs: ${version}"
else
    fail "nomouse-settings --version: ${version:-no output}"
fi

# 3b. The app launcher entry and its icons are installed, and the .deb ships the same files.
entry="${home}/.local/share/applications/io.github.yuyudhan.nomouse.desktop"
if grep -qxF "Exec=\"${home}/.local/bin/nomouse\" launch" "$entry" 2>/dev/null &&
    [ -f "${home}/.local/share/icons/hicolor/256x256/apps/io.github.yuyudhan.nomouse.png" ] &&
    [ -f "${home}/.local/share/icons/hicolor/symbolic/apps/io.github.yuyudhan.nomouse-symbolic.svg" ] &&
    [ -f "${home}/.local/share/icons/hicolor/symbolic/apps/io.github.yuyudhan.nomouse-paused-symbolic.svg" ]; then
    pass "the app launcher entry runs the installed nomouse and has its icons and the tray icons"
else
    fail "no app launcher entry at ${entry}"
fi
deb="${dist}/nomouse_amd64.deb"
if [ -f "$deb" ] && command -v dpkg-deb >/dev/null 2>&1; then
    mkdir -p "${work}/deb" "${work}/tar"
    dpkg-deb -x "$deb" "${work}/deb"
    tar -xzf "${dist}/nomouse-x86_64-unknown-linux-gnu.tar.gz" -C "${work}/tar"
    if diff -r "${work}/deb/usr/share/applications" "${work}/tar/nomouse-x86_64-unknown-linux-gnu/share/applications" &&
        diff -r "${work}/deb/usr/share/icons" "${work}/tar/nomouse-x86_64-unknown-linux-gnu/share/icons"; then
        pass "the .deb and the tarball ship the same launcher entry and icons"
    else
        fail "the .deb's usr/share differs from the tarball's share/"
    fi
    if cmp -s "${work}/deb/usr/bin/nomouse-settings" "${work}/tar/nomouse-x86_64-unknown-linux-gnu/nomouse-settings"; then
        pass "the .deb and the tarball ship the same nomouse-settings"
    else
        fail "the .deb's usr/bin/nomouse-settings is missing or differs from the tarball's"
    fi
    if dpkg-deb -f "$deb" Recommends | grep -q 'libgtk-4-1' &&
        dpkg-deb -f "$deb" Recommends | grep -q 'libadwaita-1-0'; then
        pass "the .deb recommends GTK 4 and libadwaita"
    else
        fail "the .deb's Recommends: $(dpkg-deb -f "$deb" Recommends)"
    fi
else
    echo "skip - no ${deb} or no dpkg-deb: the .deb's launcher files are not compared"
fi

# 4. Uninstall keeps the config.
sandbox "${home}/.local/bin/nomouse" config init >/dev/null
out="$(sandbox bash "${dist}/install.sh" --uninstall 2>&1)"
if [ ! -e "${home}/.local/bin/nomouse" ] && [ ! -e "${home}/.local/bin/nomouse-settings" ] &&
    [ -f "${home}/.config/nomouse/config.toml" ] && [ ! -e "$entry" ]; then
    pass "--uninstall removes both binaries and the launcher entry and keeps the config"
else
    fail "--uninstall: ${out}"
fi

# 5. Purge names the path, then deletes only it.
out="$(sandbox bash "${dist}/install.sh" --uninstall --purge 2>&1)"
if [[ "$out" == *"--purge deletes: ${home}/.config/nomouse"* ]] && [ ! -e "${home}/.config/nomouse" ]; then
    pass "--purge lists ${home}/.config/nomouse before deleting it"
else
    fail "--purge: ${out}"
fi

# 6. A wrong checksum aborts without installing.
mkdir -p "${dist}/.bad-checksum"
cp "${dist}/nomouse-x86_64-unknown-linux-gnu.tar.gz" "${dist}/.bad-checksum/"
echo "0000  nomouse-x86_64-unknown-linux-gnu.tar.gz" >"${dist}/.bad-checksum/nomouse-x86_64-unknown-linux-gnu.tar.gz.sha256"
if out="$(sandbox NOMOUSE_INSTALL_BASE="${base}/.bad-checksum" bash "${dist}/install.sh" 2>&1)"; then
    fail "a wrong checksum installed anyway"
elif [[ "$out" == *"checksum mismatch"* ]] && [ ! -e "${home}/.local/bin/nomouse" ]; then
    pass "a wrong checksum fails and installs nothing"
else
    fail "wrong checksum: ${out}"
fi
rm -rf "${dist}/.bad-checksum"

# 7. The caller's environment was never used.
if [ -f "${decoy}/config/nomouse/config.toml" ] && [ -f "${decoy}/bin/nomouse" ]; then
    pass "decoy XDG_CONFIG_HOME and NOMOUSE_INSTALL_DIR untouched"
else
    fail "the installer reached the caller's XDG_CONFIG_HOME or NOMOUSE_INSTALL_DIR"
fi

if [ "$failures" -ne 0 ]; then
    echo "test-install: ${failures} failure(s)" >&2
    exit 1
fi
echo "test-install: all passed"
