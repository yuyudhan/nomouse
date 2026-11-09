#!/usr/bin/env bash
# End-to-end test of the Wayland backend on a private headless Sway.
#
# Starts a private D-Bus session, AT-SPI and Sway (WLR_BACKENDS=headless,
# WLR_LIBINPUT_NO_DEVICES=1, pixman renderer, a private XDG_RUNTIME_DIR; never touching the
# user's session), opens the GTK test window twice, natively on Wayland and on XWayland side by
# side (title bars on, so the native window's content is offset from its container), types
# labels with wtype and asserts that clicks, right clicks and scroll mode land in both, that
# labels are drawn (grim screenshots differ) and removed, that the daemon takes
# `nomouse hints` over its socket and from a Sway `bindsym`, and that a second daemon is refused.
#
# Usage: scripts/e2e-wayland.sh [--keep] [--hold]
#   --keep  keep the temp dir (logs, screenshots); --hold  stop after setup and keep the
#   private session up for debugging (prints how to join it).
# Needs: sway (1.7+), Xwayland, wtype, grim, dbus-daemon, at-spi-bus-launcher, python3-gi
# (GTK 3).
set -euo pipefail

keep=0
hold=0
while (($#)); do
    case "$1" in
        --keep) keep=1; shift ;;
        --hold) hold=1; keep=1; shift ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done

root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse
# The shared backend contract test (crates/nomouse-linux/tests/contract.rs), built now: cargo
# cannot run once HOME points at the private dir. It drives whichever backend the session gets.
contract_test="$(cargo test --quiet --release --no-run --message-format=json \
    --manifest-path "$root/Cargo.toml" -p nomouse-linux --test contract |
    python3 -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m["target"]["name"] == "contract" and m.get("executable"):
        print(m["executable"])')"

tmp="$(mktemp -d /tmp/nomouse-e2e-wayland.XXXXXX)"
# Private copies, so a concurrent `cargo build` cannot replace the binaries mid-run.
cp "${CARGO_TARGET_DIR:-$root/target}/release/nomouse" "$tmp/nomouse"
cp "$contract_test" "$tmp/contract"
bin="$tmp/nomouse"
pids=()
cleanup() {
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    if ((keep)); then echo "kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT

# A clean private environment: nothing from the user's desktop session leaks in.
unset DISPLAY I3SOCK SWAYSOCK WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS XDG_SESSION_TYPE \
    DBUS_SESSION_BUS_ADDRESS XDG_CURRENT_DESKTOP HYPRLAND_INSTANCE_SIGNATURE
export XDG_RUNTIME_DIR="$tmp/run" XDG_CONFIG_HOME="$tmp/config" HOME="$tmp/home"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$HOME"
chmod 700 "$XDG_RUNTIME_DIR"
export GTK_MODULES=gail:atk-bridge NO_AT_BRIDGE=0 ACCESSIBILITY_ENABLED=1
export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman

passed=0
failed=0
pass() { echo "PASS: $*"; passed=$((passed + 1)); }
fail() { echo "FAIL: $*"; failed=$((failed + 1)); }

# wait_for <seconds> <command...>: polls every 20 ms.
wait_for() {
    local deadline=$((SECONDS + $1))
    shift
    until "$@"; do
        ((SECONDS < deadline)) || return 1
        sleep 0.02
    done
}

# GTK animates a window's focus change (backdrop styles) for a moment after focus moves or the
# overlay hands the keyboard back, so screenshots are compared once the screen is steady.
# screen_matches <png> <out png>: takes a screenshot into <out png> and compares it with <png>.
screen_matches() { grim "$2" && cmp -s "$1" "$2"; }
# steady: a new screenshot equals <png>; otherwise it replaces <png> for the next try.
steady() {
    grim "$1.next"
    if cmp -s "$1" "$1.next"; then return 0; fi
    mv "$1.next" "$1"
    sleep 0.1
    return 1
}
# steady_screenshot <png>: a screenshot equal to one taken 100 ms later (tried for 3 s).
steady_screenshot() {
    grim "$1"
    sleep 0.1
    wait_for 3 steady "$1" || true
    rm -f "$1.next"
}

# reap <pid> [seconds]: waits for a child with a deadline; kills it and fails on timeout.
gone() { ! kill -0 "$1" 2>/dev/null; }
reap() {
    if wait_for "${2:-10}" gone "$1"; then
        wait "$1" 2>/dev/null || true
    else
        kill "$1" 2>/dev/null || true
        fail "pid $1 did not exit within ${2:-10} s"
        return 1
    fi
}

dbus-daemon --session --fork --address="unix:path=$XDG_RUNTIME_DIR/bus" --print-pid >"$tmp/dbus.pid"
pids+=("$(cat "$tmp/dbus.pid")")
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
/usr/libexec/at-spi-bus-launcher --launch-immediately &
pids+=($!)
wait_for 10 dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus org.a11y.Bus.GetAddress >/dev/null
dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus \
    org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null

cat >"$tmp/sway.conf" <<EOF
output HEADLESS-1 resolution 1600x1000 position 0 0
xwayland enable
default_border normal 2
focus_follows_mouse no
bindsym Ctrl+space exec $bin hints
EOF
sway -c "$tmp/sway.conf" >"$tmp/sway.log" 2>&1 &
pids+=($!)
wait_for 10 compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" >/dev/null || { echo "sway did not start"; cat "$tmp/sway.log"; exit 1; }
SWAYSOCK="$(compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" | head -n1)"
export SWAYSOCK
wait_for 10 test -S "$XDG_RUNTIME_DIR/wayland-1" || { echo "no Wayland socket"; exit 1; }
export WAYLAND_DISPLAY=wayland-1
# The headless backend has no input devices. An idle virtual keyboard (it would type one `x`
# after a day) gives the seat a keyboard for the whole run, as a real keyboard would; without
# it every wtype call would add and remove the seat's only keyboard.
wtype -s 86400000 x &
pids+=($!)

# shell_window <xdg_shell|xwayland> <key>: the key (pid, name, ...) of that window.
shell_window() { swaymsg -t get_tree | python3 "$root/scripts/e2e/sway_tree.py" field "$1" "$2"; }
title() { shell_window "$1" name; }
# title_is <shell> <pattern>: the title starts with the pattern before its trailing `*`.
title_is() {
    local current
    current="$(title "$1")"
    [[ "$current" == "${2%\*}"* ]]
}
has_window() { [[ -n "$(shell_window "$1" pid)" ]]; }
focus() { swaymsg "[pid=$(shell_window "$1" pid)] focus" >/dev/null; sleep 0.2; }
scans() { grep -c 'scan [0-9]*:' "$1" || true; }
scans_at_least() { (($(scans "$1") >= $2)); }
# label_for <log> <accessible name>: the label of the last scan's hint for that name.
label_for() { grep -E "hint [a-z]+ = [A-Za-z]+ \"$2\"$" "$1" | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/'; }
# run_hints <log> <args...>: starts `nomouse --standalone <args>`, waits for its first scan.
run_hints() {
    local log="$1"
    shift
    NOMOUSE_LOG=debug "$bin" --standalone "$@" 2>"$log" &
    hints_pid=$!
    wait_for 10 scans_at_least "$log" 1
}

# GTK_OVERLAY_SCROLLING=0: fixed scrollbars, so fading overlay scrollbars cannot change the
# screenshots compared below.
swaymsg "exec env GDK_BACKEND=wayland GTK_OVERLAY_SCROLLING=0 python3 $root/scripts/e2e/gtk_app.py" >/dev/null
wait_for 15 has_window xdg_shell || { echo "native GTK window missing"; exit 1; }
swaymsg "exec env GDK_BACKEND=x11 GTK_OVERLAY_SCROLLING=0 python3 $root/scripts/e2e/gtk_app.py" >/dev/null
wait_for 15 has_window xwayland || { echo "XWayland GTK window missing"; exit 1; }
# Let the toolkits register with AT-SPI.
sleep 1

if ((hold)); then
    env_file="/tmp/nomouse-e2e-wayland-hold.env"
    {
        echo "export WAYLAND_DISPLAY=$WAYLAND_DISPLAY SWAYSOCK=$SWAYSOCK XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
        echo "export DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS XDG_CONFIG_HOME=$XDG_CONFIG_HOME"
        echo "nomouse=$bin"
    } >"$env_file"
    echo "session held; source $env_file"
    sleep infinity
fi

echo "--- nomouse doctor on headless Sway"
"$bin" doctor || true

echo "--- backend contract (real Wayland + AT-SPI backend)"
if timeout 60 "$tmp/contract" --ignored --test-threads=1 >"$tmp/contract.log" 2>&1; then
    pass "the Wayland backend passes the shared backend contract"
else
    fail "backend contract: $(cat "$tmp/contract.log")"
fi

echo "--- window layout (sway get_tree)"
swaymsg -t get_tree | python3 "$root/scripts/e2e/sway_tree.py" layout

for shell in xdg_shell xwayland; do
    echo "--- $shell: 10 single clicks"
    clicks_ok=0
    for i in $(seq 1 10); do
        focus "$shell"
        log="$tmp/click-$shell-$i.log"
        if run_hints "$log" hints && label="$(label_for "$log" "Button $i")" && [[ -n "$label" ]]; then
            if ((i == 1)); then
                grim "$tmp/labels-$shell.png"
            fi
            wtype "$label"
            reap "$hints_pid" || true
            if wait_for 3 title_is "$shell" "left: Button $i #*"; then
                clicks_ok=$((clicks_ok + 1))
            else
                echo "  Button $i: title '$(title "$shell")'"
            fi
        else
            kill "$hints_pid" 2>/dev/null || true
            echo "  Button $i: no label ($(grep -c 'hint ' "$log" || true) hints)"
        fi
    done
    if ((clicks_ok == 10)); then pass "$shell: 10/10 clicks landed"; else fail "$shell: $clicks_ok/10 clicks landed"; fi

    echo "--- $shell: right click"
    focus "$shell"
    log="$tmp/right-$shell.log"
    if run_hints "$log" hints --action right-click && label="$(label_for "$log" "Button 3")" && [[ -n "$label" ]]; then
        wtype "$label"
        reap "$hints_pid" || true
        if wait_for 3 title_is "$shell" "right: Button 3 #*"; then pass "$shell: right click landed"; else fail "$shell: right click: '$(title "$shell")'"; fi
    else
        kill "$hints_pid" 2>/dev/null || true
        fail "$shell: right click: no label"
    fi

    echo "--- $shell: scroll mode"
    focus "$shell"
    log="$tmp/scroll-$shell.log"
    if run_hints "$log" scroll && label="$(grep -E 'hint [a-z]+ = ScrollArea' "$log" | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/')" && [[ -n "$label" ]]; then
        wtype "$label"
        sleep 0.3
        wtype jjjj
        sleep 0.3
        wtype -k Escape
        reap "$hints_pid" || true
        if wait_for 3 title_is "$shell" "scrolled: *"; then pass "$shell: scroll landed ($(title "$shell"))"; else fail "$shell: scroll: '$(title "$shell")'"; fi
    else
        kill "$hints_pid" 2>/dev/null || true
        fail "$shell: scroll: no scroll-area label"
    fi
done

echo "--- labels are drawn, and removed when the session ends"
focus xdg_shell
steady_screenshot "$tmp/before-cancel.png"
log="$tmp/cancel.log"
if run_hints "$log" hints; then
    sleep 0.3
    grim "$tmp/during-cancel.png"
    wtype -k Escape
    reap "$hints_pid" || true
    if ! cmp -s "$tmp/before-cancel.png" "$tmp/during-cancel.png"; then pass "labels visible on screen while up"; else fail "no labels in the screenshot"; fi
    if wait_for 3 screen_matches "$tmp/before-cancel.png" "$tmp/after-cancel.png"; then pass "Escape left no overlay on screen"; else fail "the screen differs after Escape"; fi
else
    kill "$hints_pid" 2>/dev/null || true
    fail "cancel: no labels"
fi

echo "--- daemon: nomouse hints over the socket, and a Sway bindsym"
daemon_log="$tmp/daemon.log"
NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon_pid=$!
pids+=("$daemon_pid")
if wait_for 10 grep -q 'daemon ready' "$daemon_log"; then pass "$(grep -o 'linux-wayland daemon ready.*' "$daemon_log")"; else fail "daemon did not start: $(cat "$daemon_log")"; fi
if timeout 10 "$bin" daemon 2>"$tmp/second.log"; then fail "a second daemon started"; else
    if grep -q 'already running' "$tmp/second.log"; then pass "single instance: second daemon refused"; else fail "second daemon: $(cat "$tmp/second.log")"; fi
fi
focus xdg_shell
before="$(scans "$daemon_log")"
started_ms=$(date +%s%3N)
if "$bin" hints && wait_for 5 scans_at_least "$daemon_log" $((before + 1)); then
    echo "  command-to-scan: $(($(date +%s%3N) - started_ms)) ms (includes polling)"
    sleep 0.2
    wtype "$(label_for "$daemon_log" "Button 6")"
    if wait_for 3 title_is xdg_shell "left: Button 6 #*"; then pass "IPC: \`nomouse hints\` reached the daemon and the click landed"; else fail "IPC click: '$(title xdg_shell)'"; fi
else
    fail "IPC: the daemon did not scan after \`nomouse hints\`"
fi
focus xwayland
before="$(scans "$daemon_log")"
wtype -M ctrl -k space -m ctrl
if wait_for 5 scans_at_least "$daemon_log" $((before + 1)); then
    sleep 0.2
    wtype "$(label_for "$daemon_log" "Button 8")"
    if wait_for 3 title_is xwayland "left: Button 8 #*"; then pass "Sway bindsym Ctrl+space -> daemon -> click landed"; else fail "bindsym click: '$(title xwayland)'"; fi
else
    fail "Sway bindsym did not reach the daemon"
fi
grep -oE 'labels shown [0-9]+ ms' "$daemon_log" | sed 's/^/  daemon: /' || true

echo "--- toggle: the Sway binding pressed again dismisses the labels"
focus xdg_shell
steady_screenshot "$tmp/before-toggle.png"
title_before="$(title xdg_shell)"
before="$(scans "$daemon_log")"
wtype -M ctrl -k space -m ctrl
if wait_for 5 scans_at_least "$daemon_log" $((before + 1)); then
    sleep 0.3
    wtype -M ctrl -k space -m ctrl
    if wait_for 5 grep -q 'cancelled' "$daemon_log"; then
        if wait_for 3 screen_matches "$tmp/before-toggle.png" "$tmp/after-toggle.png" && [[ "$(title xdg_shell)" == "$title_before" ]]; then pass "binding twice: no overlay left, nothing clicked"; else fail "binding twice: the screen or title changed ('$(title xdg_shell)')"; fi
    else
        fail "binding twice: the session did not end"
    fi
else
    fail "toggle: the binding did not reach the daemon"
fi

echo "--- focus-then-act: Tab twice, then Return, clicks the third label's button"
focus xdg_shell
before="$(scans "$daemon_log")"
if "$bin" hints && wait_for 5 scans_at_least "$daemon_log" $((before + 1)); then
    sleep 0.2
    # The last scan's hints in label order: the third line names the third label's element.
    third="$(awk '/scan [0-9]+:/ { n = 0 } /hint [a-z]+ = / { n++; if (n == 3) line = $0 } END { print line }' "$daemon_log" | sed -E 's/.*"(.*)"$/\1/')"
    wtype -k Tab -k Tab -k Return
    if [[ -n "$third" ]] && wait_for 3 title_is xdg_shell "left: $third #*"; then pass "Tab Tab Return clicked the third label ($third)"; else fail "focus-then-act: third '$third', title '$(title xdg_shell)'"; fi
else
    fail "focus-then-act: the daemon did not scan"
fi

# SIGTERM skips destructors, so the socket file stays; a client must treat it as no daemon.
kill "$daemon_pid" 2>/dev/null || true
reap "$daemon_pid" 5 || true
focus xdg_shell
log="$tmp/after-daemon.log"
NOMOUSE_LOG=debug "$bin" hints 2>"$log" >"$tmp/after-daemon.out" &
hints_pid=$!
if wait_for 10 scans_at_least "$log" 1; then
    sleep 0.3
    wtype "$(label_for "$log" "Button 9")"
    reap "$hints_pid" || true
    if wait_for 3 title_is xdg_shell "left: Button 9 #*"; then pass "without a daemon (stale socket) \`nomouse hints\` runs in-process"; else fail "in-process after daemon exit: '$(title xdg_shell)'"; fi
else
    kill "$hints_pid" 2>/dev/null || true
    fail "without a daemon \`nomouse hints\` did not scan: $(cat "$tmp/after-daemon.out" "$log")"
fi

echo "--- $passed passed, $failed failed"
((failed == 0))
