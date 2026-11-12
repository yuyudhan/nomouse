#!/usr/bin/env bash
# End-to-end test of the Linux X11 backend on a private display.
#
# Starts Xvfb, a private D-Bus session, AT-SPI and i3 (never touching the user's session),
# opens a GTK test window and a Chromium data: page, drives `nomouse` with xdotool, and
# asserts that clicks, continuous mode, right clicks, ctrl+clicks, search-by-text and scroll
# mode (on the scroll area) land, that no overlay
# window is left behind, and measures hotkey-to-labels latency. It also runs the shared
# backend contract (crates/nomouse-linux/tests/contract.rs) against the real backend, and a
# Qt 5 window when PyQt5 is importable.
#
# Usage: scripts/e2e-linux.sh [--display :97] [--no-chromium] [--require-chromium]
#                             [--no-qt] [--require-qt] [--hold]
# Needs: Xvfb, i3, xdotool, xwininfo, dbus-daemon, at-spi-bus-launcher, python3-gi (GTK 3);
# Chromium/Chrome and PyQt5 are optional (skipped when absent, failed when --require-*).
# NOMOUSE_E2E_QT_PYTHON picks the interpreter that has PyQt5 (default: python3).
#
# Determinism: only startup readiness (Xvfb, the bus, i3, app windows) is waited for with
# bounded polling; every assertion is tried once with a fixed timeout, and every child is
# reaped with a deadline, so a hung `nomouse` fails the run instead of stalling it.
set -euo pipefail

display=":97"
chromium=1
require_chromium=0
qt=1
require_qt=0
hold=0
while (($#)); do
    case "$1" in
        --display) display="$2"; shift 2 ;;
        --no-chromium) chromium=0; shift ;;
        --require-chromium) require_chromium=1; shift ;;
        --no-qt) qt=0; shift ;;
        --require-qt) require_qt=1; shift ;;
        --hold) hold=1; shift ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done
qt_python="${NOMOUSE_E2E_QT_PYTHON:-python3}"

root="$(cd "$(dirname "$0")/.." && pwd)"
# Release build: the latency figures are meaningless for a debug zbus. The release flavour
# even when the caller's shell exports NOMOUSE_FLAVOUR=dev: the checks press the release
# hotkeys (ctrl+space) and write ~/.config/nomouse/.
export NOMOUSE_FLAVOUR=release
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse
# The contract test binary, built now: cargo cannot run once HOME points at the private dir.
contract_test="$(cargo test --quiet --release --no-run --message-format=json \
    --manifest-path "$root/Cargo.toml" -p nomouse-linux --test contract |
    python3 -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m["target"]["name"] == "contract" and m.get("executable"):
        print(m["executable"])')"

tmp="$(mktemp -d /tmp/nomouse-e2e.XXXXXX)"
# A private copy, so a concurrent `cargo build` cannot replace the binary mid-run.
cp "${CARGO_TARGET_DIR:-$root/target}/release/nomouse" "$tmp/nomouse"
cp "$contract_test" "$tmp/contract"
bin="$tmp/nomouse"
pids=()
cleanup() {
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
    # NOMOUSE_E2E_KEEP=1 keeps the logs (nomouse debug logs, app stderr) for a failed run.
    # Chrome's helper processes outlive the browser process and keep writing into its profile.
    pkill -f -- "--user-data-dir=$tmp" 2>/dev/null || true
    if [[ "${NOMOUSE_E2E_KEEP:-0}" == 1 ]]; then echo "logs kept in $tmp"; else rm -rf "$tmp" || { sleep 1; rm -rf "$tmp"; }; fi
}
trap cleanup EXIT

# A clean private environment: nothing from the user's desktop session leaks in.
unset I3SOCK SWAYSOCK WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS XDG_SESSION_TYPE DBUS_SESSION_BUS_ADDRESS
export DISPLAY="$display"
export XDG_RUNTIME_DIR="$tmp/run" XDG_CONFIG_HOME="$tmp/config" HOME="$tmp/home"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$HOME"
chmod 700 "$XDG_RUNTIME_DIR"
export GTK_MODULES=gail:atk-bridge NO_AT_BRIDGE=0 ACCESSIBILITY_ENABLED=1
export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1

passed=0
failed=0
pass() { echo "PASS: $*"; passed=$((passed + 1)); }
fail() { echo "FAIL: $*"; failed=$((failed + 1)); }

# wait_for <seconds> <command...>: polls every 20 ms.
wait_for() {
    local deadline=$(($(date +%s%N) + $1 * 1000000000)); shift
    until "$@" 2>/dev/null; do
        (($(date +%s%N) < deadline)) || return 1
        sleep 0.02
    done
}

start() { "$@" & pids+=($!); }

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

# skip_or_fail <required> <message>: a missing optional target skips, a required one fails.
skip_or_fail() { if (($1)); then fail "$2"; else echo "SKIP: $2"; fi; }

Xvfb "$display" -screen 0 1600x1000x24 -nolisten tcp >"$tmp/xvfb.log" 2>&1 &
pids+=($!)
wait_for 10 xdpyinfo -display "$display" >/dev/null || { echo "Xvfb did not start"; exit 1; }

dbus-daemon --session --fork --address="unix:path=$XDG_RUNTIME_DIR/bus" --print-pid >"$tmp/dbus.pid"
pids+=("$(cat "$tmp/dbus.pid")")
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
start /usr/libexec/at-spi-bus-launcher --launch-immediately
wait_for 10 dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus org.a11y.Bus.GetAddress >/dev/null
dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus \
    org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null

cat >"$tmp/i3.conf" <<EOF
ipc-socket $tmp/i3.sock
focus_follows_mouse no
font pango:DejaVu Sans 9
default_border none
EOF
start i3 -c "$tmp/i3.conf"
wait_for 10 test -S "$tmp/i3.sock"

echo "--- nomouse doctor on $display"
"$bin" doctor || true

start python3 "$root/scripts/e2e/gtk_app.py"
wait_for 15 xdotool search --name '^nomouse-e2e' >/dev/null || { echo "GTK window missing"; exit 1; }
gtk_win="$(xdotool search --name '^nomouse-e2e' | head -n1)"
# Let the toolkit register with AT-SPI.
sleep 1

echo "--- backend contract (real X11 + AT-SPI backend)"
if timeout 60 "$tmp/contract" --ignored --test-threads=1 >"$tmp/contract.log" 2>&1; then
    pass "the X11 backend passes the shared backend contract"
else
    fail "backend contract: $(cat "$tmp/contract.log")"
fi
if ((hold)); then
    # Debugging aid: keep the private session up and print how to join it.
    echo "export DISPLAY=$DISPLAY DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR XDG_CONFIG_HOME=$XDG_CONFIG_HOME" >"/tmp/nomouse-e2e-hold.env"
    echo "session held; source /tmp/nomouse-e2e-hold.env"
    sleep infinity
fi

title() { xdotool getwindowname "$1"; }
focus() { xdotool windowactivate --sync "$1" >/dev/null 2>&1 || xdotool windowfocus --sync "$1"; sleep 0.1; }
overlay_viewable() { xdotool search --onlyvisible --name '^nomouse-overlay$' >/dev/null; }
no_overlay_viewable() { ! overlay_viewable; }
scans() { grep -c 'scan [0-9]*:' "$1" || true; }
scans_at_least() { (($(scans "$1") >= $2)); }
# label_for <log> <accessible name>: the label of the last scan's hint for that name.
label_for() { grep -E "hint [a-z]+ = [A-Za-z]+ \"$2\"$" "$1" | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/'; }
# area_labels <log>: how many scroll areas the last scroll-mode scan labelled (with digits).
area_labels() { grep -cE 'hint [0-9] = ScrollArea ' "$1" || true; }
# title_is <window> <prefix>*: the title starts with the literal prefix; callers write it as
# a glob such as "left: Button 1 #*" (a trailing `*` is the only wildcard used).
title_is() {
    local current
    current="$(title "$1")"
    [[ "$current" == "${2%\*}"* ]]
}
# title_matches <window> <regex>: the title matches the extended regular expression.
title_matches() { title "$1" | grep -qE "$2"; }

# run_hints <log> <args...>: starts `nomouse hints`, waits for its first scan.
run_hints() {
    local log="$1"; shift
    # --standalone: run in this process (and log here) even while the daemon runs.
    NOMOUSE_LOG=debug "$bin" --standalone "$@" 2>"$log" &
    hints_pid=$!
    wait_for 10 grep -q 'scan 1:' "$log" && wait_for 5 overlay_viewable
}

echo "--- 10 single clicks on the GTK window"
clicks_ok=0
for i in $(seq 1 10); do
    focus "$gtk_win"
    log="$tmp/click$i.log"
    if ! run_hints "$log" hints; then fail "click $i: no labels"; kill "$hints_pid" 2>/dev/null || true; continue; fi
    label="$(label_for "$log" "Button $i")"
    xdotool type --delay 20 "$label"
    reap "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "left: Button $i #*"; then
        clicks_ok=$((clicks_ok + 1))
    else
        echo "  click $i (label '$label') left title '$(title "$gtk_win")'"
    fi
done
if ((clicks_ok == 10)); then pass "10/10 clicks landed"; else fail "$clicks_ok/10 clicks landed"; fi

echo "--- continuous mode: 5 clicks then Escape"
focus "$gtk_win"
log="$tmp/continuous.log"
before="$(title "$gtk_win" | sed -E 's/.*#([0-9]+)$/\1/')"
continuous_ok=1
if run_hints "$log" hints --continuous; then
    for i in 1 2 3 4 5; do
        wait_for 5 scans_at_least "$log" "$i" || { continuous_ok=0; break; }
        wait_for 5 overlay_viewable || { continuous_ok=0; break; }
        label="$(label_for "$log" "Button $((i + 3))")"
        xdotool type --delay 20 "$label"
        wait_for 3 title_is "$gtk_win" "left: Button $((i + 3)) #*" || { continuous_ok=0; break; }
    done
    wait_for 5 scans_at_least "$log" 6 && wait_for 5 overlay_viewable || continuous_ok=0
    xdotool key Escape
    reap "$hints_pid" || continuous_ok=0
else
    continuous_ok=0
fi
after="$(title "$gtk_win" | sed -E 's/.*#([0-9]+)$/\1/')"
if ((continuous_ok && after - before == 5)) && ! kill -0 "$hints_pid" 2>/dev/null; then
    pass "continuous mode: 5 clicks, re-labelled each time, Escape ended it"
else
    fail "continuous mode (ok=$continuous_ok, clicks $before -> $after)"
fi

echo "--- continuous mode ends after clicking a text field; the typed text lands in it"
focus "$gtk_win"
log="$tmp/continuous-field.log"
if run_hints "$log" hints --continuous; then
    xdotool type --delay 20 "$(label_for "$log" "Button 1")"
    if wait_for 3 title_is "$gtk_win" "left: Button 1 #*" && wait_for 5 scans_at_least "$log" 2 && wait_for 5 overlay_viewable; then
        xdotool type --delay 20 "$(label_for "$log" "Note")"
        if reap "$hints_pid" 5; then
            xdotool type --delay 20 "hello"
            if wait_for 3 title_is "$gtk_win" "typed: hello #*"; then
                pass "continuous mode ended at the text field and the typed text went into it"
            else
                fail "after the text field click the title is '$(title "$gtk_win")'"
            fi
        else
            fail "continuous mode kept running after clicking the text field ($(grep -E '"Note"$' "$log" | tail -n1))"
        fi
    else
        fail "continuous mode: the first click or the re-scan did not happen"
        kill "$hints_pid" 2>/dev/null || true
    fi
else
    fail "continuous mode before the text field: no labels"
fi

echo "--- right click"
focus "$gtk_win"
log="$tmp/right.log"
if run_hints "$log" hints --action right-click; then
    xdotool type --delay 20 "$(label_for "$log" "Button 2")"
    reap "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "right: Button 2 #*"; then pass "right click landed"; else fail "right click: title '$(title "$gtk_win")'"; fi
else
    fail "right click: no labels"
fi

echo "--- ctrl+click (modifier key 2, then a label)"
focus "$gtk_win"
log="$tmp/ctrl.log"
if run_hints "$log" hints; then
    xdotool type --delay 20 "2$(label_for "$log" "Button 5")"
    reap "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "ctrl-left: Button 5 #*"; then pass "ctrl+click landed with Control held"; else fail "ctrl+click: title '$(title "$gtk_win")', log: $(grep -E 'holds|Perform|error' "$log" | tr '\n' ' ')"; fi
else
    fail "ctrl+click: no labels"
fi

echo "--- focus-then-act: a stray Return does nothing; Tab three times, then Return, clicks the third label's button"
focus "$gtk_win"
log="$tmp/focus.log"
if run_hints "$log" hints; then
    # Hints are logged in label order; nothing is highlighted until Tab, which starts on the
    # first, so the Return before it must leave the labels up.
    third="$(grep -E 'hint [a-z]+ = ' "$log" | sed -n 3p | sed -E 's/.*"(.*)"$/\1/')"
    xdotool key --delay 30 Return Tab Tab Tab Return
    reap "$hints_pid" || true
    if [[ -n "$third" ]] && wait_for 3 title_is "$gtk_win" "left: $third #*"; then pass "Return, Tab Tab Tab Return clicked the third label ($third)"; else fail "focus-then-act: third '$third', title '$(title "$gtk_win")'"; fi
else
    fail "focus-then-act: no labels"
fi

echo "--- Homerow bindings: a label highlights, Return clicks, Shift-Return right clicks"
mkdir -p "$XDG_CONFIG_HOME/nomouse"
printf '[hints]\nkeys = "homerow"\n' >"$XDG_CONFIG_HOME/nomouse/config.toml"
focus "$gtk_win"
log="$tmp/homerow.log"
if run_hints "$log" hints; then
    xdotool type --delay 20 "$(label_for "$log" "Button 8")"
    sleep 0.2
    still_open=$(kill -0 "$hints_pid" 2>/dev/null && echo 1 || echo 0)
    xdotool key Return
    reap "$hints_pid" || true
    if ((still_open)) && wait_for 3 title_is "$gtk_win" "left: Button 8 #*"; then pass "homerow: the label waited for Return, which clicked"; else fail "homerow Return: open after label=$still_open, title '$(title "$gtk_win")'"; fi
else
    fail "homerow Return: no labels"
fi
focus "$gtk_win"
log="$tmp/homerow-right.log"
if run_hints "$log" hints; then
    xdotool type --delay 20 "$(label_for "$log" "Button 3")"
    xdotool key shift+Return
    reap "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "right: Button 3 #*"; then pass "homerow: Shift-Return right clicked"; else fail "homerow Shift-Return: title '$(title "$gtk_win")'"; fi
else
    fail "homerow Shift-Return: no labels"
fi
rm "$XDG_CONFIG_HOME/nomouse/config.toml"

echo "--- search mode: type a name, Enter clicks the best match"
focus "$gtk_win"
log="$tmp/search.log"
if run_hints "$log" search; then
    xdotool type --delay 20 "button6"
    xdotool key Return
    reap "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "left: Button 6 #*"; then pass "search mode clicked the match"; else fail "search mode: title '$(title "$gtk_win")'"; fi
else
    fail "search mode: no search box"
fi

echo "--- search mode: Tab, then a label picks another match"
focus "$gtk_win"
log="$tmp/search-tab.log"
if run_hints "$log" search; then
    # "button1" matches Button 1 (best) and Button 10; the second label is Button 10's.
    xdotool type --delay 20 "button1"
    xdotool key Tab
    xdotool type --delay 20 "s"
    reap "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "left: Button 10 #*"; then pass "search mode Tab + label clicked the second match"; else fail "search Tab: title '$(title "$gtk_win")'"; fi
else
    fail "search Tab: no search box"
fi

echo "--- search mode: a button whose only text is its tooltip is found (query with spaces)"
focus "$gtk_win"
log="$tmp/tooltip-hints.log"
if run_hints "$log" hints; then
    # The icon button has no accessible name: only its description (tooltip) holds text.
    grep -qE 'hint [a-z]+ = Button ""$' "$log" && nameless=1 || nameless=0
    xdotool key Escape
    reap "$hints_pid" || true
else
    nameless=0
fi
log="$tmp/search-tooltip.log"
if run_hints "$log" search; then
    xdotool type --delay 20 "keep a copy"
    xdotool key Return
    reap "$hints_pid" || true
    if ((nameless)) && wait_for 3 title_is "$gtk_win" "left: tooltip button #*"; then pass "search found the tooltip-only button by its description"; else fail "search by tooltip: nameless=$nameless, title '$(title "$gtk_win")'"; fi
else
    fail "search by tooltip: no search box"
fi

echo "--- semantic click (AT-SPI do_action)"
mkdir -p "$XDG_CONFIG_HOME/nomouse"
printf '[hints]\nsemantic_click = true\n' >"$XDG_CONFIG_HOME/nomouse/config.toml"
focus "$gtk_win"
log="$tmp/semantic.log"
if run_hints "$log" hints; then
    # Park the pointer far away: only the accessibility action can press the button.
    xdotool mousemove 1590 990
    xdotool type --delay 20 "$(label_for "$log" "Button 9")"
    reap "$hints_pid" || true
    pointer="$(xdotool getmouselocation --shell | tr '\n' ' ')"
    if wait_for 3 title_is "$gtk_win" "left: Button 9 #*" && [[ "$pointer" == *"X=1590 Y=990"* ]]; then
        pass "semantic click pressed the button without moving the pointer"
    else
        fail "semantic click: title '$(title "$gtk_win")', pointer $pointer"
    fi
else
    fail "semantic click: no labels"
fi
rm "$XDG_CONFIG_HOME/nomouse/config.toml"

echo "--- scroll mode: the largest list is active; j, G, gg, then Tab to the second list"
focus "$gtk_win"
log="$tmp/scroll.log"
if run_hints "$log" scroll; then
    areas="$(area_labels "$log")"
    rows="$(grep -cE 'hint [a-z0-9]+ = [A-Za-z]+ "Row [0-9]+"$' "$log" || true)"
    sleep 0.2
    xdotool key --delay 30 j j j
    wait_for 3 title_matches "$gtk_win" '^scrolled: [1-9][0-9]* #' && down=1 || down=0
    xdotool key shift+g
    wait_for 3 title_matches "$gtk_win" '^scrolled: [0-9]+ \(bottom\) #' && bottom=1 || bottom=0
    xdotool key --delay 30 g g
    wait_for 3 title_matches "$gtk_win" '^scrolled: 0 \(top\) #' && top=1 || top=0
    xdotool key --delay 30 Tab j j
    wait_for 3 title_matches "$gtk_win" '^scrolled 2: [1-9]' && second=1 || second=0
    xdotool key Escape
    reap "$hints_pid" || true
    if ((areas == 2 && rows == 0 && down && bottom && top && second)); then
        pass "scroll mode: j scrolled the large list, G reached its bottom, gg its top, Tab j scrolled the second list ($(title "$gtk_win"))"
    else
        fail "scroll mode: $areas area labels, $rows row hints, j=$down G=$bottom gg=$top Tab=$second, title '$(title "$gtk_win")'"
    fi
else
    fail "scroll mode: no labels"
fi

echo "--- hotkey daemon: latency and a click"
daemon_log="$tmp/daemon.log"
NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon_pid=$!
pids+=("$daemon_pid")
wait_for 10 grep -q 'daemon ready' "$daemon_log" || fail "daemon did not start"
if timeout 10 "$bin" daemon 2>"$tmp/second.log"; then fail "a second daemon started"; else
    if grep -q 'already running' "$tmp/second.log"; then pass "single instance: second daemon refused"; else fail "second daemon: $(cat "$tmp/second.log")"; fi
fi
external=()
for _ in 1 2 3 4 5; do
    focus "$gtk_win"
    t0=$(date +%s%N)
    xdotool key ctrl+space
    if wait_for 5 overlay_viewable; then
        external+=($((($(date +%s%N) - t0) / 1000000)))
    fi
    xdotool key Escape
    wait_for 5 no_overlay_viewable || true
done
internal="$(grep -oE 'labels shown [0-9]+ ms' "$daemon_log" | grep -oE '[0-9]+' | tr '\n' ' ')"
echo "  hotkey-to-labels (daemon, event to drawn): ${internal}ms"
echo "  hotkey-to-labels (xdotool to IsViewable, includes polling): ${external[*]} ms"
focus "$gtk_win"
before_scans="$(scans "$daemon_log")"
xdotool key ctrl+space
wait_for 5 scans_at_least "$daemon_log" $((before_scans + 1)) && wait_for 5 overlay_viewable
xdotool type --delay 20 "$(label_for "$daemon_log" "Button 7")"
if wait_for 3 title_is "$gtk_win" "left: Button 7 #*"; then pass "daemon hotkey click landed"; else fail "daemon hotkey click: '$(title "$gtk_win")'"; fi

echo "--- \`nomouse hints\` hands its mode to the running daemon"
focus "$gtk_win"
before_scans="$(scans "$daemon_log")"
if timeout 5 "$bin" hints && wait_for 5 scans_at_least "$daemon_log" $((before_scans + 1)) && wait_for 5 overlay_viewable; then
    xdotool type --delay 20 "$(label_for "$daemon_log" "Button 4")"
    if wait_for 3 title_is "$gtk_win" "left: Button 4 #*"; then pass "command socket: the daemon ran the session and the click landed"; else fail "command socket click: '$(title "$gtk_win")'"; fi
else
    fail "command socket: the daemon did not scan after \`nomouse hints\`"
fi

echo "--- toggle: the hotkey (and \`nomouse hints\`) pressed again dismisses the labels"
focus "$gtk_win"
before_title="$(title "$gtk_win")"
before_scans="$(scans "$daemon_log")"
xdotool key ctrl+space
if wait_for 5 scans_at_least "$daemon_log" $((before_scans + 1)) && wait_for 5 overlay_viewable; then
    xdotool key ctrl+space
    if wait_for 3 no_overlay_viewable && [[ "$(title "$gtk_win")" == "$before_title" ]]; then pass "hotkey twice: no overlay left, nothing clicked"; else fail "hotkey twice: overlay still viewable or title changed to '$(title "$gtk_win")'"; fi
else
    fail "hotkey toggle: no labels after the first press"
fi
focus "$gtk_win"
before_scans="$(scans "$daemon_log")"
if timeout 5 "$bin" hints && wait_for 5 scans_at_least "$daemon_log" $((before_scans + 1)) && wait_for 5 overlay_viewable; then
    timeout 5 "$bin" hints || true
    if wait_for 3 no_overlay_viewable; then pass "\`nomouse hints\` twice: no overlay left"; else fail "\`nomouse hints\` twice: overlay still viewable"; fi
else
    fail "socket toggle: no labels after the first \`nomouse hints\`"
fi

echo "--- ignore.apps, \`nomouse reload\` and the config file watch (the running daemon)"
config="$XDG_CONFIG_HOME/nomouse/config.toml"
mkdir -p "$XDG_CONFIG_HOME/nomouse"
reloads() { grep -c 'config reloaded' "$daemon_log" || true; }
reloads_at_least() { (($(reloads) >= $1)); }
ignores() { grep -c 'the focused app is ignored' "$daemon_log" || true; }
ignores_at_least() { (($(ignores) >= $1)); }
# last_labels: the labels of the daemon's latest scan, one per line.
last_labels() {
    awk '/scan [0-9]+:/ { start = NR } { line[NR] = $0 } END { for (i = start + 1; i <= NR; i++) print line[i] }' "$daemon_log" |
        grep -oE 'hint [a-z]+ = ' | cut -d' ' -f2
}
before="$(reloads)"
# GTK names the window after its program: WM_CLASS "gtk_app.py", "Gtk_app.py".
printf '[ignore]\napps = ["gtk_app.py"]\n' >"$config"
# A reload restarts the platform (X connection, hotkeys), which takes a moment on a busy host.
if timeout 5 "$bin" reload && wait_for 15 reloads_at_least $((before + 1)); then
    focus "$gtk_win"
    before_scans="$(scans "$daemon_log")"
    before_ignores="$(ignores)"
    xdotool key ctrl+space
    if wait_for 5 ignores_at_least $((before_ignores + 1)) && ! overlay_viewable && (($(scans "$daemon_log") == before_scans)); then
        pass "ignore.apps: the hotkey over the ignored GTK app showed nothing"
    else
        fail "ignore.apps: $(($(ignores) - before_ignores)) ignore(s), $(($(scans "$daemon_log") - before_scans)) scan(s)"
        xdotool key Escape
    fi
    if "$bin" doctor 2>/dev/null | grep -q 'ignored apps: gtk_app.py$'; then pass "doctor lists the ignored app"; else fail "doctor: $("$bin" doctor 2>&1 | grep 'ignored apps')"; fi
else
    fail "reload: the daemon did not reload after \`nomouse reload\`"
fi
kept() { grep -c 'keeping the running configuration' "$daemon_log" || true; }
kept_at_least() { (($(kept) >= $1)); }
before="$(kept)"
# i and o are not label characters: the file is invalid and the daemon keeps its settings.
printf '[hints]\nalphabet = "qwertyuiop"\n' >"$config"
if timeout 5 "$bin" reload && wait_for 15 kept_at_least $((before + 1)); then pass "nomouse reload: an invalid config is reported and the running settings stay"; else fail "nomouse reload: no report of the invalid config"; fi
before="$(reloads)"
printf '[hints]\nalphabet = "qwertyup"\n' >"$config"
if timeout 5 "$bin" reload && wait_for 15 reloads_at_least $((before + 1)); then
    focus "$gtk_win"
    before_scans="$(scans "$daemon_log")"
    xdotool key ctrl+space
    if wait_for 5 scans_at_least "$daemon_log" $((before_scans + 1)) && wait_for 5 overlay_viewable; then
        labels="$(last_labels)"
        xdotool key Escape
        if [[ -n "$labels" ]] && ! grep -qvE '^[qwertyup]+$' <<<"$labels"; then pass "nomouse reload: the next labels use the new alphabet ($(tr '\n' ' ' <<<"$labels" | cut -c1-40)...)"; else fail "nomouse reload: labels '$(tr '\n' ' ' <<<"$labels")'"; fi
    else
        fail "nomouse reload: no labels after the hotkey"
    fi
else
    fail "nomouse reload: the daemon did not reload"
fi
before="$(reloads)"
printf '[hints]\nalphabet = "zxcvbnm"\n' >"$config"
if wait_for 15 reloads_at_least $((before + 1)); then
    focus "$gtk_win"
    before_scans="$(scans "$daemon_log")"
    xdotool key ctrl+space
    if wait_for 5 scans_at_least "$daemon_log" $((before_scans + 1)) && wait_for 5 overlay_viewable; then
        labels="$(last_labels)"
        xdotool key Escape
        if [[ -n "$labels" ]] && ! grep -qvE '^[zxcvbnm]+$' <<<"$labels"; then pass "file watch: an edited config applies without \`nomouse reload\`"; else fail "file watch: labels '$(tr '\n' ' ' <<<"$labels")'"; fi
    else
        fail "file watch: no labels after the hotkey"
    fi
else
    fail "file watch: the daemon did not notice the edited config"
fi
before="$(reloads)"
rm "$config"
wait_for 15 reloads_at_least $((before + 1)) || fail "file watch: removing the config did not reload the defaults"

if ((chromium)); then
    chrome="$(command -v chromium || command -v chromium-browser || command -v google-chrome || true)"
    if [[ -n "$chrome" ]]; then
        echo "--- Chromium data: page"
        page='data:text/html,<title>start</title><body style="margin:40px"><button onclick="document.title=%27clicked%27">Go%20there</button></body>'
        chrome_flags=()
        # GitHub's Ubuntu runners forbid the unprivileged user namespaces Chrome's sandbox
        # needs (AppArmor), so Chrome aborts at start there; the throwaway CI profile runs
        # without it. Local runs keep the sandbox.
        if [[ "${CI:-}" == true ]]; then chrome_flags+=(--no-sandbox); fi
        "$chrome" "${chrome_flags[@]}" --force-renderer-accessibility --user-data-dir="$tmp/chrome" --no-first-run \
            --no-default-browser-check --disable-gpu --disable-dev-shm-usage --password-store=basic \
            --disable-features=Translate --new-window "$page" >"$tmp/chrome-stderr.log" 2>&1 &
        pids+=($!)
        if wait_for 30 xdotool search --name '^start' >/dev/null; then
            chrome_win="$(xdotool search --name '^start' | head -n1)"
            # Readiness: the renderer fills its accessibility tree seconds after the window maps.
            if ! python3 "$root/scripts/e2e/a11y_ready.py" "$(xdotool getwindowpid "$chrome_win")" "Go there" 30; then
                echo "  Chromium never exposed the button over AT-SPI; the hints scan below will show it"
            fi
            focus "$chrome_win"
            log="$tmp/chrome.log"
            if run_hints "$log" hints && label="$(label_for "$log" "Go there")" && [[ -n "$label" ]]; then
                xdotool type --delay 20 "$label"
                reap "$hints_pid" || true
                if wait_for 5 title_is "$chrome_win" "clicked*"; then pass "Chromium click landed"; else fail "Chromium: title '$(title "$chrome_win")'"; fi
            else
                kill "$hints_pid" 2>/dev/null || true
                fail "Chromium: no label for the button ($(grep -c 'hint ' "$log" || true) hints)"
                sed 's/^/    /' "$log" | tail -n 20
            fi
        else
            fail "Chromium window did not appear"
            sed 's/^/    /' "$tmp/chrome-stderr.log" | tail -n 20
        fi
    else
        skip_or_fail "$require_chromium" "no Chromium/Chrome installed"
    fi
fi

if ((chromium)); then
    chrome="$(command -v chromium || command -v chromium-browser || command -v google-chrome || true)"
    if [[ -n "$chrome" ]]; then
        echo "--- Chromium page with a link 6000 px below the viewport (hints.browser)"
        far='data:text/html,<title>tall</title><body style="margin:40px"><button>Near</button><div style="height:6000px"></div><a href="%23x" onclick="document.title=%27clicked faraway%27">Faraway link</a></body>'
        far_flags=()
        if [[ "${CI:-}" == true ]]; then far_flags+=(--no-sandbox); fi
        "$chrome" "${far_flags[@]}" --force-renderer-accessibility --user-data-dir="$tmp/chrome-tall" --no-first-run \
            --no-default-browser-check --disable-gpu --disable-dev-shm-usage --password-store=basic \
            --disable-features=Translate --new-window "$far" >"$tmp/chrome-tall-stderr.log" 2>&1 &
        pids+=($!)
        if wait_for 30 xdotool search --name '^tall' >/dev/null; then
            tall_win="$(xdotool search --name '^tall' | head -n1)"
            python3 "$root/scripts/e2e/a11y_ready.py" "$(xdotool getwindowpid "$tall_win")" "Near" 30 ||
                echo "  Chromium never exposed the page over AT-SPI; the checks below will show it"
            mkdir -p "$XDG_CONFIG_HOME/nomouse"

            # fast (the default): only the viewport is read, so the far link is not found.
            focus "$tall_win"
            log="$tmp/tall-fast.log"
            if run_hints "$log" search; then
                xdotool type --delay 20 "faraway link"
                xdotool key Return
                reap "$hints_pid" || true
                sleep 1
                if title_is "$tall_win" "tall*"; then pass "browser = fast: the link 6000 px down is not reached"; else fail "browser = fast: title '$(title "$tall_win")'"; fi
            else
                fail "browser = fast: no search box"
            fi

            printf '[hints]\nbrowser = "exhaustive"\n' >"$XDG_CONFIG_HOME/nomouse/config.toml"
            # exhaustive: hint mode labels only what is in view.
            focus "$tall_win"
            log="$tmp/tall-hints.log"
            if run_hints "$log" hints; then
                if grep -qE 'hint [a-z]+ = [A-Za-z]+ "Near"$' "$log" && ! grep -q '"Faraway link"$' "$log"; then
                    pass "browser = exhaustive: hint mode labels the visible button and not the far link"
                else
                    fail "browser = exhaustive: hint mode labels: $(grep 'hint ' "$log" | tr '\n' ' ')"
                fi
                xdotool key Escape
                reap "$hints_pid" || true
            else
                fail "browser = exhaustive: no labels"
            fi
            # exhaustive: search finds the link, scrolls it into view and clicks it. Five runs,
            # each after a reload that puts the title and the scroll position back.
            far_ok=0
            for i in 1 2 3 4 5; do
                focus "$tall_win"
                xdotool key F5
                wait_for 10 title_is "$tall_win" "tall*" || true
                sleep 1
                xdotool key ctrl+Home
                sleep 0.5
                log="$tmp/tall-exhaustive-$i.log"
                if run_hints "$log" search; then
                    xdotool type --delay 20 "faraway link"
                    xdotool key Return
                    reap "$hints_pid" || true
                    if wait_for 5 title_is "$tall_win" "clicked faraway*"; then
                        far_ok=$((far_ok + 1))
                    else
                        echo "  run $i: title '$(title "$tall_win")'"
                    fi
                else
                    kill "$hints_pid" 2>/dev/null || true
                    echo "  run $i: no search box"
                fi
            done
            if ((far_ok == 5)); then pass "browser = exhaustive: search scrolled to the link 6000 px down and clicked it (5/5)"; else fail "browser = exhaustive: $far_ok/5 clicks landed"; fi
            rm -f "$XDG_CONFIG_HOME/nomouse/config.toml"
        else
            fail "Chromium window with the tall page did not appear"
            sed 's/^/    /' "$tmp/chrome-tall-stderr.log" | tail -n 20
        fi
    fi
fi

if ((qt)); then
    if "$qt_python" -c 'import PyQt5.QtWidgets' 2>/dev/null; then
        echo "--- Qt 5 window"
        start "$qt_python" "$root/scripts/e2e/qt_app.py" 2>"$tmp/qt-stderr.log"
        if wait_for 15 xdotool search --name '^nomouse-qt' >/dev/null; then
            qt_win="$(xdotool search --name '^nomouse-qt' | head -n1)"
            # Readiness: Qt registers with AT-SPI after the window maps. A Qt built without the
            # AT-SPI bridge (the PyQt5 pip wheel) never does: use the distribution's PyQt5.
            if ! python3 "$root/scripts/e2e/a11y_ready.py" "$(xdotool getwindowpid "$qt_win")" "Qt Button 3" 20; then
                echo "  Qt never exposed its buttons over AT-SPI (is $qt_python's Qt built with the AT-SPI bridge?)"
            fi
            focus "$qt_win"
            log="$tmp/qt.log"
            if run_hints "$log" hints && label="$(label_for "$log" "Qt Button 3")" && [[ -n "$label" ]]; then
                xdotool type --delay 20 "$label"
                reap "$hints_pid" || true
                if wait_for 5 title_is "$qt_win" "qt: Qt Button 3 #*"; then pass "Qt click landed"; else fail "Qt: title '$(title "$qt_win")'"; fi
            else
                kill "$hints_pid" 2>/dev/null || true
                fail "Qt: no label for the button ($(grep -c 'hint ' "$log" || true) hints)"
                sed 's/^/    /' "$log" | tail -n 20
                sed 's/^/    qt: /' "$tmp/qt-stderr.log" | tail -n 10
            fi
        else
            fail "Qt window did not appear"
        fi
    else
        skip_or_fail "$require_qt" "PyQt5 not importable by $qt_python"
    fi
fi

kill "$daemon_pid" 2>/dev/null || true
reap "$daemon_pid" 5 || true
sleep 0.2
leftover="$(xwininfo -root -tree | grep -c '"nomouse-overlay"' || true)"
if ((leftover == 0)); then pass "no leftover overlay windows"; else fail "$leftover overlay windows left"; fi

echo "--- $passed passed, $failed failed"
((failed == 0))
