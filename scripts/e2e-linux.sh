#!/usr/bin/env bash
# End-to-end test of the Linux X11 backend on a private display.
#
# Starts Xvfb, a private D-Bus session, AT-SPI and i3 (never touching the user's session),
# opens a GTK test window and a Chromium data: page, drives `nomouse` with xdotool, and
# asserts that clicks, continuous mode, right clicks and scroll mode land, that no overlay
# window is left behind, and measures hotkey-to-labels latency.
#
# Usage: scripts/e2e-linux.sh [--display :97] [--no-chromium]
# Needs: Xvfb, i3, xdotool, xwininfo, dbus-daemon, at-spi-bus-launcher, python3-gi (GTK 3);
# Chromium/Chrome is optional (skipped when absent or with --no-chromium).
set -euo pipefail

display=":97"
chromium=1
hold=0
while (($#)); do
    case "$1" in
        --display) display="$2"; shift 2 ;;
        --no-chromium) chromium=0; shift ;;
        --hold) hold=1; shift ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done

root="$(cd "$(dirname "$0")/.." && pwd)"
# Release build: the latency figures are meaningless for a debug zbus.
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse

tmp="$(mktemp -d /tmp/nomouse-e2e.XXXXXX)"
# A private copy, so a concurrent `cargo build` cannot replace the binary mid-run.
cp "${CARGO_TARGET_DIR:-$root/target}/release/nomouse" "$tmp/nomouse"
bin="$tmp/nomouse"
pids=()
cleanup() {
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
    rm -rf "$tmp"
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
title_is() { [[ "$(title "$1")" == $2 ]]; }

# run_hints <log> <args...>: starts `nomouse hints`, waits for its first scan.
run_hints() {
    local log="$1"; shift
    NOMOUSE_LOG=debug "$bin" "$@" 2>"$log" &
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
    wait "$hints_pid" || true
    if wait_for 3 title_is "$gtk_win" "left: Button $i #*"; then
        clicks_ok=$((clicks_ok + 1))
    else
        echo "  click $i (label '$label') left title '$(title "$gtk_win")'"
    fi
done
((clicks_ok == 10)) && pass "10/10 clicks landed" || fail "$clicks_ok/10 clicks landed"

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
    wait "$hints_pid" || true
else
    continuous_ok=0
fi
after="$(title "$gtk_win" | sed -E 's/.*#([0-9]+)$/\1/')"
if ((continuous_ok && after - before == 5)) && ! kill -0 "$hints_pid" 2>/dev/null; then
    pass "continuous mode: 5 clicks, re-labelled each time, Escape ended it"
else
    fail "continuous mode (ok=$continuous_ok, clicks $before -> $after)"
fi

echo "--- right click"
focus "$gtk_win"
log="$tmp/right.log"
if run_hints "$log" hints --action right-click; then
    xdotool type --delay 20 "$(label_for "$log" "Button 2")"
    wait "$hints_pid" || true
    wait_for 3 title_is "$gtk_win" "right: Button 2 #*" && pass "right click landed" || fail "right click: title '$(title "$gtk_win")'"
else
    fail "right click: no labels"
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
    wait "$hints_pid" || true
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

echo "--- scroll mode"
focus "$gtk_win"
log="$tmp/scroll.log"
if run_hints "$log" scroll; then
    xdotool type --delay 20 "$(label_for "$log" "Row 2")"
    sleep 0.2
    xdotool key --delay 30 j j j
    wait_for 3 title_is "$gtk_win" "scrolled: *" && scrolled=1 || scrolled=0
    xdotool key Escape
    wait "$hints_pid" || true
    ((scrolled)) && pass "scroll mode scrolled ($(title "$gtk_win"))" || fail "scroll mode: title '$(title "$gtk_win")'"
else
    fail "scroll mode: no labels"
fi

echo "--- hotkey daemon: latency and a click"
daemon_log="$tmp/daemon.log"
NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon_pid=$!
pids+=("$daemon_pid")
wait_for 10 grep -q 'daemon ready' "$daemon_log" || fail "daemon did not start"
if "$bin" daemon 2>"$tmp/second.log"; then fail "a second daemon started"; else
    grep -q 'already running' "$tmp/second.log" && pass "single instance: second daemon refused" || fail "second daemon: $(cat "$tmp/second.log")"
fi
external=()
for trial in 1 2 3 4 5; do
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
wait_for 3 title_is "$gtk_win" "left: Button 7 #*" && pass "daemon hotkey click landed" || fail "daemon hotkey click: '$(title "$gtk_win")'"

if ((chromium)); then
    chrome="$(command -v chromium || command -v chromium-browser || command -v google-chrome || true)"
    if [[ -n "$chrome" ]]; then
        echo "--- Chromium data: page"
        page='data:text/html,<title>start</title><body style="margin:40px"><button onclick="document.title=%27clicked%27">Go%20there</button></body>'
        "$chrome" --force-renderer-accessibility --user-data-dir="$tmp/chrome" --no-first-run \
            --no-default-browser-check --disable-gpu --disable-dev-shm-usage --password-store=basic \
            --disable-features=Translate --new-window "$page" >"$tmp/chrome-stderr.log" 2>&1 &
        pids+=($!)
        if wait_for 30 xdotool search --name '^start' >/dev/null; then
            chrome_win="$(xdotool search --name '^start' | head -n1)"
            sleep 2
            focus "$chrome_win"
            log="$tmp/chrome.log"
            if run_hints "$log" hints && label="$(label_for "$log" "Go there")" && [[ -n "$label" ]]; then
                xdotool type --delay 20 "$label"
                wait "$hints_pid" || true
                wait_for 5 title_is "$chrome_win" "clicked*" && pass "Chromium click landed" || fail "Chromium: title '$(title "$chrome_win")'"
            else
                kill "$hints_pid" 2>/dev/null || true
                fail "Chromium: no label for the button ($(grep -c 'hint ' "$log" || true) hints)"
                grep 'hint ' "$log" | sed 's/^.*hint /    hint /' || true
            fi
        else
            fail "Chromium window did not appear"
        fi
    else
        echo "SKIP: no Chromium/Chrome installed"
    fi
fi

kill "$daemon_pid" 2>/dev/null || true
wait "$daemon_pid" 2>/dev/null || true
sleep 0.2
leftover="$(xwininfo -root -tree | grep -c '"nomouse-overlay"' || true)"
((leftover == 0)) && pass "no leftover overlay windows" || fail "$leftover overlay windows left"

echo "--- $passed passed, $failed failed"
((failed == 0))
