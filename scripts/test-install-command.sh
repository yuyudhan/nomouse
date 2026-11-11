#!/usr/bin/env bash
# Tests the documented one-line install (docs/install.md, "The one-line install") against a
# local HTTP server, piped into every shell it may meet (sh, dash, bash, zsh): a 404 runs
# nothing and asks once (curl -f prints its one line); a served script runs and gets the options
# after `sh -s --`; the real install.sh cut off mid-download, or just before its last line, runs
# nothing, because only that last line starts the install. Either way the folder it runs from and
# its TMPDIR are empty afterwards. The command is the documented one with only its URL pointed
# at 127.0.0.1. (scripts/test-installer.sh pipes the real install.sh into each shell against a
# fake release.)
#
# Usage: scripts/test-install-command.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
pages="https://yuyudhan.github.io/nomouse/install"
tmp="$(mktemp -d)"
server_pid=""
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT

failures=0
pass() { echo "ok: $*"; }
fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

# The one line in the first sh block under "## The one-line install".
fence="$(printf '\140\140\140')"
command="$(awk -v fence="$fence" '
    /^## The one-line install$/ { section = 1; next }
    section && /^## / { exit }
    section && $0 == fence "sh" { block = 1; next }
    block && $0 == fence { exit }
    block { print }
' "${root}/docs/install.md")"
[ "$command" = "curl -fsSL ${pages} | sh" ] || {
    echo "test-install-command: docs/install.md's one-line install is not 'curl -fsSL ${pages} | sh': ${command}" >&2
    exit 1
}

# Serves /ok/install (a script that leaves a marker and prints its arguments), /half/install
# (the real install.sh, cut off halfway) and /lastline/install (the real install.sh without its
# last line, then cut off): both cut ones announce the whole length and hang up early, as a
# dropped connection does. Everything else is a 404. Every request is logged.
mkdir -p "${tmp}/www/ok" "${tmp}/home"
printf 'touch "%s/ran"\necho "install.sh ran: $*"\n' "$tmp" >"${tmp}/www/ok/install"
python3 - "${tmp}/www" "${tmp}/port" "${tmp}/requests" "${root}/scripts/install.sh" <<'PY' &
import http.server, os, socketserver, sys

www, port_file, log, installer = sys.argv[1:5]
with open(installer, "rb") as source:
    script = source.read()
lines = script.rstrip(b"\n").split(b"\n")
cut = {
    "/half/install": script[: len(script) // 2],
    "/lastline/install": b"\n".join(lines[:-1]) + b"\n",
}

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        with open(log, "a") as out:
            out.write(self.path + "\n")
        if self.path in cut:
            self.send_response(200)
            self.send_header("Content-Length", str(len(script)))
            self.end_headers()
            self.wfile.write(cut[self.path])
            self.wfile.flush()
            self.close_connection = True
            return
        path = os.path.join(www, self.path.lstrip("/"))
        if os.path.isfile(path):
            with open(path, "rb") as source:
                body = source.read()
            self.send_response(200)
        else:
            body = b"Not Found"
            self.send_response(404)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

# socketserver's TCPServer, not http.server.HTTPServer: HTTPServer looks up the host's fully
# qualified name when it binds, which takes many seconds on GitHub's macOS runners.
server = socketserver.TCPServer(("127.0.0.1", 0), Handler)
with open(port_file + ".tmp", "w") as out:
    out.write(str(server.server_address[1]))
os.rename(port_file + ".tmp", port_file)
server.serve_forever()
PY
server_pid=$!
for _ in $(seq 300); do
    [ -s "${tmp}/port" ] && break
    kill -0 "$server_pid" 2>/dev/null || break
    sleep 0.1
done
[ -s "${tmp}/port" ] || {
    echo "test-install-command: the local HTTP server did not start" >&2
    exit 1
}
base="http://127.0.0.1:$(cat "${tmp}/port")"

# run SHELL CASE [OPTIONS]: the documented command with its URL replaced by CASE's on the local
# server, piped into SHELL (with `-s -- OPTIONS` when given), in a fresh folder (`here`) with a
# fresh TMPDIR (`scratch_tmp`) and HOME. NOMOUSE_INSTALL_BASE points at no server, so a script
# that did start would fail at its first download, not install anything real.
run() {
    local line="${command//"$pages"/${base}/$2/install}"
    line="${line%| sh}| $1${3:+ -s -- $3}"
    (cd "$here" && env -i HOME="${tmp}/home" PATH="$PATH" TMPDIR="$scratch_tmp" TERM=dumb \
        NOMOUSE_INSTALL_BASE="${base}/nowhere" sh -c "$line")
}
fresh_dirs() {
    here="$(mktemp -d "${tmp}/run.XXXXXX")"
    scratch_tmp="$(mktemp -d "${tmp}/tmp.XXXXXX")"
    rm -f "${tmp}/ran"
}
# expect_clean NAME: passes NAME when `here`, `scratch_tmp` and HOME are all empty.
expect_clean() {
    local left
    left="$(find "$here" "$scratch_tmp" "${tmp}/home" -mindepth 1 -maxdepth 1)"
    if [ -z "$left" ]; then
        pass "$1 leaves the folder, TMPDIR and HOME empty"
    else
        fail "$1 left files behind: $(echo "$left" | tr '\n' ' ')"
    fi
}

shells="sh"
for shell in dash bash zsh; do
    if command -v "$shell" >/dev/null 2>&1; then shells="${shells} ${shell}"; fi
done
echo "shells: ${shells}"
for shell in $shells; do
    # 1. A 404: curl prints its one line, the shell gets an empty script and runs nothing, and
    # curl asks only once.
    fresh_dirs
    : >"${tmp}/requests"
    run "$shell" missing >"${tmp}/stdout" 2>"${tmp}/stderr" || true
    if [ -e "${tmp}/ran" ] || [ -s "${tmp}/stdout" ]; then
        fail "${shell}: a 404 still ran a script: $(cat "${tmp}/stdout")"
    elif [ "$(cat "${tmp}/stderr")" != "curl: (22) The requested URL returned error: 404" ]; then
        fail "${shell}: a 404 did not print curl's one line: $(cat "${tmp}/stderr")"
    else
        pass "${shell}: a 404 prints one line ($(cat "${tmp}/stderr")) and runs nothing"
    fi
    tries="$(grep -c '^/missing/install$' "${tmp}/requests" || true)"
    if [ "$tries" -eq 1 ]; then
        pass "${shell}: a 404 is asked for once"
    else
        fail "${shell}: a 404 was asked for ${tries} times"
    fi
    expect_clean "${shell}: a 404"

    # 2. A served script runs, with the options after `sh -s --`.
    fresh_dirs
    if out="$(run "$shell" ok "--version 0.1.0" 2>&1)" && [ -e "${tmp}/ran" ] &&
        [ "$out" = "install.sh ran: --version 0.1.0" ]; then
        pass "${shell}: a served script runs with the options after -s --"
    else
        fail "${shell}: the served script did not run with its options: ${out}"
    fi
    expect_clean "${shell}: a served script"

    # 3. The real install.sh cut off halfway, or right before its last line, runs nothing: no
    # header, no download, nothing installed.
    for cut in half lastline; do
        fresh_dirs
        : >"${tmp}/requests"
        run "$shell" "$cut" >"${tmp}/stdout" 2>"${tmp}/stderr" || true
        if [ -s "${tmp}/stdout" ] || grep -q '^/nowhere/' "${tmp}/requests"; then
            fail "${shell}: install.sh cut off (${cut}) still ran: $(cat "${tmp}/stdout" "${tmp}/stderr")"
        elif ! grep -q '^curl: (18) ' "${tmp}/stderr"; then
            fail "${shell}: install.sh cut off (${cut}): curl did not report the cut: $(cat "${tmp}/stderr")"
        else
            pass "${shell}: install.sh cut off (${cut}) runs nothing"
        fi
        expect_clean "${shell}: install.sh cut off (${cut})"
    done
done

if [ "$failures" -ne 0 ]; then
    echo "test-install-command: ${failures} failed" >&2
    exit 1
fi
echo "test-install-command: all passed"
