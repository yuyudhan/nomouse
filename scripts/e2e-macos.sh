#!/usr/bin/env bash
# End-to-end test of the macOS backend against a fixture app, on the logged-in desktop.
#
# Compiles scripts/e2e/mac_app.swift (a window with buttons One..Ten, a Context button with a
# context menu, a text field, a 200-row list and a drag Source and Target, each updating an
# Accessibility-visible counter) and scripts/e2e/mac_probe.swift (Accessibility queries and a
# synthetic key poster), then drives `nomouse` with typed labels and asserts the fixture's
# counters through Accessibility: ten left clicks, a right click that opens the context menu,
# a double click, a drag, scroll mode on the list, search, and the daemon (hotkey click, a
# mode handed over its socket, no overlay window left after Escape).
#
# Usage: scripts/e2e-macos.sh [--bin <nomouse binary>] [--keep]
#   --bin   test this binary (for example nomouse.app/Contents/MacOS/nomouse) instead of a
#           fresh `cargo build --release`
#   --keep  keep the logs (also NOMOUSE_E2E_KEEP=1)
#
# Needs: macOS with a logged-in, unlocked desktop, swiftc (Xcode or the Command Line Tools),
# and the Accessibility grant for the process running this script (the terminal, or sshd for
# a remote run). It moves the real pointer and types into the fixture. Without the grant it
# fails at once; under CI (`CI=true`, GitHub's runners have no grant) it prints SKIP and
# passes.
#
# Determinism: only startup (the fixture window, the first scan) is waited for with bounded
# polling; every assertion is tried once with a fixed timeout and every nomouse process is
# reaped with a deadline, so a hung `nomouse` fails the run instead of stalling it.
set -euo pipefail

bin=""
keep="${NOMOUSE_E2E_KEEP:-0}"
while (($#)); do
    case "$1" in
    --bin)
        bin="$2"
        shift 2
        ;;
    --keep)
        keep=1
        shift
        ;;
    *)
        echo "usage: $0 [--bin <nomouse binary>] [--keep]" >&2
        exit 2
        ;;
    esac
done

if [[ "$(uname -s)" != Darwin ]]; then
    echo "SKIP: macOS end-to-end tests run only on macOS"
    exit 0
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d /tmp/nomouse-e2e-macos.XXXXXX)"
pids=()
cleanup() {
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
    if [[ "$keep" == 1 ]]; then echo "logs kept in $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT

swiftc -O -o "$tmp/mac_probe" "$root/scripts/e2e/mac_probe.swift"
probe="$tmp/mac_probe"
if ! "$probe" trusted; then
    if [[ "${CI:-}" == true ]]; then
        echo "SKIP: this process is not trusted for Accessibility (CI has no grant)"
        exit 0
    fi
    echo "FAIL: this process is not trusted for Accessibility. Grant it to the terminal (or sshd)"
    echo "      in System Settings > Privacy & Security > Accessibility, then run again."
    exit 1
fi

if [[ -z "$bin" ]]; then
    cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse
    bin="${CARGO_TARGET_DIR:-$root/target}/release/nomouse"
fi
# A private copy, so a concurrent `cargo build` cannot replace the binary mid-run. A bundle's
# binary is run in place: copying it out would drop the bundle's identity and its grant.
if [[ "$bin" != *.app/Contents/MacOS/* ]]; then
    cp "$bin" "$tmp/nomouse"
    bin="$tmp/nomouse"
fi
swiftc -O -o "$tmp/mac_app" "$root/scripts/e2e/mac_app.swift"

# A private config and socket: the user's own config and daemon are never touched.
export XDG_CONFIG_HOME="$tmp/config" XDG_RUNTIME_DIR="$tmp/run"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

passed=0
failed=0
pass() {
    echo "PASS: $*"
    passed=$((passed + 1))
}
fail() {
    echo "FAIL: $*"
    failed=$((failed + 1))
}

# wait_for <seconds> <command...>: polls every 50 ms.
wait_for() {
    local tries=$(($1 * 20))
    shift
    until "$@" 2>/dev/null; do
        ((tries-- > 0)) || return 1
        sleep 0.05
    done
}

# reap <pid> [seconds]: waits for a child with a deadline; kills it and fails on timeout.
gone() { ! kill -0 "$1"; }
reap() {
    if wait_for "${2:-10}" gone "$1"; then
        wait "$1" 2>/dev/null || true
    else
        echo "  pid $1 still running after ${2:-10}s; killed"
        kill "$1" 2>/dev/null || true
        wait "$1" 2>/dev/null || true
        return 1
    fi
}

"$tmp/mac_app" >"$tmp/app.log" 2>&1 &
app=$!
pids+=("$app")
wait_for 15 "$probe" value "$app" count.One >/dev/null || {
    echo "FAIL: the fixture window did not appear"
    exit 1
}

echo "--- nomouse doctor"
"$bin" doctor || true

# value <identifier>: a fixture counter, read through Accessibility.
value() { "$probe" value "$app" "$1"; }
value_is() { [[ "$(value "$1")" == "$2" ]]; }
value_above() { (($(value "$1") > $2)); }
front() { "$probe" front "$app" || echo "  the fixture is not frontmost (frontmost pid $("$probe" frontmost))"; }
# label_for <log> <accessible name>: the label of the last scan's hint for that name.
label_for() { grep -E "hint [a-z]+ = [A-Za-z]+ \"$2\"$" "$1" | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/'; }
# label_for_role <log> <role>: the label of the last scan's first hint with that role.
label_for_role() { grep -E "hint [a-z]+ = $2 " "$1" | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/'; }

# run_hints <log> <args...>: brings the fixture to the front, starts `nomouse --standalone`
# (so it runs and logs here even while the daemon runs) and waits for its first scan.
run_hints() {
    local log="$1"
    shift
    front
    NOMOUSE_LOG=debug "$bin" --standalone "$@" 2>"$log" &
    hints_pid=$!
    wait_for 10 grep -q 'scan 1:' "$log"
}

echo "--- 10 left clicks on One..Ten"
clicks_ok=0
for name in One Two Three Four Five Six Seven Eight Nine Ten; do
    log="$tmp/click-$name.log"
    if ! run_hints "$log" hints; then
        fail "click $name: no labels"
        kill "$hints_pid" 2>/dev/null || true
        continue
    fi
    label="$(label_for "$log" "$name")"
    "$probe" type "$label"
    reap "$hints_pid" || true
    if wait_for 3 value_is "count.$name" 1; then
        clicks_ok=$((clicks_ok + 1))
    else
        echo "  click $name (label '$label'): count.$name is '$(value "count.$name")'"
    fi
done
if ((clicks_ok == 10)); then pass "10/10 left clicks landed"; else fail "$clicks_ok/10 left clicks landed"; fi

echo "--- right click opens the context menu"
log="$tmp/right.log"
if run_hints "$log" hints --action right-click; then
    "$probe" type "$(label_for "$log" Context)"
    reap "$hints_pid" || true
    if wait_for 3 value_is menus 1 && value_is count.Context 0; then pass "right click opened the context menu"; else fail "right click: menus '$(value menus)', left clicks '$(value count.Context)'"; fi
    "$probe" key escape
else
    fail "right click: no labels"
fi

echo "--- double click increments by two"
log="$tmp/double.log"
before="$(value count.Three)"
if run_hints "$log" hints --action double-click; then
    "$probe" type "$(label_for "$log" Three)"
    reap "$hints_pid" || true
    if wait_for 3 value_is count.Three $((before + 2)); then pass "double click counted two clicks"; else fail "double click: count.Three went from $before to '$(value count.Three)'"; fi
else
    fail "double click: no labels"
fi

echo "--- drag moves the Source onto the Target"
log="$tmp/drag.log"
if run_hints "$log" hints --action drag; then
    "$probe" type "$(label_for "$log" Source)"
    "$probe" type "$(label_for "$log" Target)"
    reap "$hints_pid" || true
    if wait_for 3 value_is drops 1; then pass "drag dropped the Source on the Target"; else fail "drag: drops '$(value drops)'"; fi
else
    fail "drag: no labels"
fi

echo "--- scroll mode moves the list"
log="$tmp/scroll.log"
if run_hints "$log" scroll; then
    label="$(label_for_role "$log" ScrollArea)"
    "$probe" type "$label"
    sleep 0.2
    "$probe" type jjj
    wait_for 3 value_above first-row 0 && scrolled=1 || scrolled=0
    "$probe" key escape
    reap "$hints_pid" || true
    if ((scrolled)) && [[ -n "$label" ]]; then pass "scroll mode scrolled the list (first row $(value first-row))"; else fail "scroll mode: label '$label', first row '$(value first-row)'"; fi
else
    fail "scroll mode: no labels"
fi

echo "--- search: 'seven' + Enter clicks Seven"
log="$tmp/search.log"
before="$(value count.Seven)"
if run_hints "$log" search; then
    "$probe" type seven
    "$probe" key return
    reap "$hints_pid" || true
    if wait_for 3 value_is count.Seven $((before + 1)); then pass "search clicked Seven"; else fail "search: count.Seven went from $before to '$(value count.Seven)'"; fi
else
    fail "search: no search box"
fi

echo "--- daemon: hotkey click, a mode over the socket, no panel left after Escape"
daemon_log="$tmp/daemon.log"
NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon=$!
pids+=("$daemon")
if wait_for 10 grep -q 'daemon ready' "$daemon_log"; then
    scans() { grep -c 'scan [0-9]*:' "$daemon_log" || true; }
    scans_above() { (($(scans) > $1)); }
    front
    before_scans="$(scans)"
    before="$(value count.Two)"
    "$probe" key ctrl+space
    if wait_for 5 scans_above "$before_scans"; then
        "$probe" type "$(label_for "$daemon_log" Two)"
        if wait_for 3 value_is count.Two $((before + 1)); then pass "daemon hotkey click landed"; else fail "daemon hotkey click: count.Two '$(value count.Two)'"; fi
    else
        fail "daemon hotkey: no scan after ctrl+space"
    fi
    front
    before_scans="$(scans)"
    windows_is() { [[ "$("$probe" windows "$daemon")" == "$1" ]]; }
    windows_above() { (($("$probe" windows "$daemon") > $1)); }
    if "$bin" hints && wait_for 5 scans_above "$before_scans" && wait_for 3 windows_above 0; then
        "$probe" key escape
        if wait_for 3 windows_is 0; then pass "no overlay panel left after Escape"; else fail "after Escape the daemon still has $("$probe" windows "$daemon") window(s)"; fi
    else
        fail "\`nomouse hints\` to the daemon: no scan or no panel ($("$probe" windows "$daemon" || true) windows)"
    fi
    kill "$daemon" 2>/dev/null || true
    reap "$daemon" 5 || true
else
    fail "daemon did not start: $(tail -n3 "$daemon_log")"
fi

echo "--- $passed passed, $failed failed"
((failed == 0))
