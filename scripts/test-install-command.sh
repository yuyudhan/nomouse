#!/usr/bin/env bash
# Tests the documented one-line install (docs/install.md, "The one-line install") against a
# local HTTP server: a 404 makes the command print one `release not found` line and exit
# non-zero without running anything, at once (a 404 is final, not retried); connections the
# network cuts are retried until install.sh arrives; a served install.sh is downloaded into a
# temporary directory and run with bash. Either way the folder it runs from and its TMPDIR are
# empty afterwards. The command is the documented one with only the release URL pointed at
# 127.0.0.1. (scripts/test-installer.sh runs the same command with the real install.sh and a
# fake release.)
#
# Usage: scripts/test-install-command.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
release="https://github.com/yuyudhan/nomouse/releases/latest/download"
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
[[ "$command" == *"${release}/install.sh"* ]] || {
    echo "test-install-command: no one-line install in docs/install.md: ${command}" >&2
    exit 1
}

# Serves /ok/install.sh and /flaky/install.sh (a script that leaves a marker; /flaky/ hangs up
# without answering its first two requests, as a network that cuts connections does) and
# answers 404 to everything else, counting the requests for each path.
mkdir -p "${tmp}/www/ok" "${tmp}/www/flaky" "${tmp}/home"
printf 'touch "%s/ran"\necho "install.sh ran: $*"\n' "$tmp" >"${tmp}/www/ok/install.sh"
cp "${tmp}/www/ok/install.sh" "${tmp}/www/flaky/install.sh"
python3 - "${tmp}/www" "${tmp}/port" "${tmp}/requests" <<'PY' &
import collections, http.server, os, socketserver, sys

www, port_file, log = sys.argv[1], sys.argv[2], sys.argv[3]
seen = collections.Counter()

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        with open(log, "a") as out:
            out.write(self.path + "\n")
        seen[self.path] += 1
        if self.path.startswith("/flaky/") and seen[self.path] <= 2:
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
# Caps curl's retries of a cut connection to a few seconds; the command line is unchanged.
printf 'retry-max-time = 3\n' >"${tmp}/home/.curlrc"

# Runs the documented command with the release URL replaced by $1, in a fresh folder (`here`)
# with a fresh TMPDIR (`scratch_tmp`).
run() {
    (cd "$here" && CURL_HOME="${tmp}/home" HOME="${tmp}/home" TMPDIR="$scratch_tmp" \
        bash -c "${command//"$release"/$1}")
}
fresh_dirs() {
    here="$(mktemp -d "${tmp}/run.XXXXXX")"
    scratch_tmp="$(mktemp -d "${tmp}/tmp.XXXXXX")"
}
# expect_clean NAME: passes NAME when `here` and `scratch_tmp` are both empty.
expect_clean() {
    local left
    left="$(find "$here" "$scratch_tmp" -mindepth 1 -maxdepth 1)"
    if [ -z "$left" ]; then
        pass "$1 leaves the folder and TMPDIR empty"
    else
        fail "$1 left files behind: $(echo "$left" | tr '\n' ' ')"
    fi
}

# 1. A 404: the command prints one `release not found` line, exits non-zero, runs nothing and
# asks only once.
rm -f "${tmp}/ran"
fresh_dirs
if run "${base}/missing" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
    fail "a 404 exited 0: $(cat "${tmp}/stdout" "${tmp}/stderr")"
elif [ -e "${tmp}/ran" ] || [ -s "${tmp}/stdout" ]; then
    fail "a 404 still ran a script: $(cat "${tmp}/stdout")"
elif [ "$(cat "${tmp}/stderr")" != "nomouse: release not found: ${base}/missing/install.sh" ]; then
    fail "a 404 did not print the one 'release not found' line: $(cat "${tmp}/stderr")"
else
    pass "a 404 exits non-zero with one line ($(cat "${tmp}/stderr")) and runs nothing"
fi
tries="$(grep -c '^/missing/install.sh$' "${tmp}/requests" || true)"
if [ "$tries" -eq 1 ]; then
    pass "a 404 is final: asked once"
else
    fail "a 404 was asked for ${tries} times"
fi
expect_clean "a 404"

# 2. Connections the network cuts are retried until install.sh arrives, which then runs.
rm -f "${tmp}/ran"
fresh_dirs
if out="$(run "${base}/flaky" 2>&1)" && [ -e "${tmp}/ran" ] && [[ "$out" == *"install.sh ran"* ]]; then
    pass "cut connections are retried, then install.sh runs ($(grep -c '^/flaky/install.sh$' "${tmp}/requests") requests)"
else
    fail "cut connections were not retried: ${out}"
fi
expect_clean "cut connections"

# 3. A served install.sh is downloaded, then run.
rm -f "${tmp}/ran"
fresh_dirs
if out="$(run "${base}/ok" 2>&1)" && [ -e "${tmp}/ran" ] && [[ "$out" == *"install.sh ran"* ]]; then
    pass "a served install.sh is downloaded and run"
else
    fail "the served install.sh did not run: ${out}"
fi
expect_clean "a served install.sh"

if [ "$failures" -ne 0 ]; then
    echo "test-install-command: ${failures} failed" >&2
    exit 1
fi
echo "test-install-command: all passed"
