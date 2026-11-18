#!/usr/bin/env bash
# Repeatable Linux benchmark of the nomouse daemon on private sessions (never the user's).
#
# X11: a private Xvfb with i3, D-Bus and AT-SPI. Wayland: a private headless Sway (wlroots,
# pixman renderer) whose `bindsym Ctrl+space exec nomouse hints` reaches the daemon over its
# socket. Both open the e2e GTK window (scripts/e2e/gtk_app.py) and run the release daemon with
# the default config, then measure:
#
# - idle CPU and RSS: the daemon for --idle seconds after it is ready, nothing pressed;
# - hint show latency over --runs activations of Ctrl+space:
#     daemon: the daemon's own "labels shown <n> ms after the trigger" (hotkey event to the
#             first scan's labels handed to the overlay);
#     end to end: from the synthetic key press (window already focused) to the labels being
#             up, polled every 5 ms (X11: the overlay window is viewable, each poll an xdotool
#             process, reported as poll_cost_ms; Wayland: the daemon logged its first scan),
#             so it includes the key's trip through the X server or compositor; on Wayland
#             also Sway starting `nomouse hints` for its bindsym;
# - click accuracy: --rounds passes over the ten "Button N" buttons, each one activated, its
#   label typed, and the window title checked for "left: Button N";
# - active CPU and peak RSS of the daemon over the latency and click passes.
#
# Usage: scripts/bench/linux.sh [--x11] [--wayland] [--runs N] [--rounds N] [--idle S]
#                               [--out FILE] [--keep]
#   Neither --x11 nor --wayland runs both. Defaults: --runs 40 --rounds 3 --idle 30.
#   --out writes the summary (key=value lines) to FILE as well as stdout.
# Needs: as scripts/e2e-linux.sh (X11) and scripts/e2e-wayland.sh (Wayland).
set -euo pipefail

x11=0
wayland=0
runs=40
rounds=3
idle=30
out=""
keep=0
while (($#)); do
    case "$1" in
        --x11) x11=1; shift ;;
        --wayland) wayland=1; shift ;;
        --runs) runs="$2"; shift 2 ;;
        --rounds) rounds="$2"; shift 2 ;;
        --idle) idle="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        --keep) keep=1; shift ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done
if ((!x11 && !wayland)); then x11=1; wayland=1; fi

root="$(cd "$(dirname "$0")/../.." && pwd)"
# Release flavour and build: the daemon reads ~/.config/nomouse/ and binds Ctrl+space, and the
# latency of a debug build means nothing.
export NOMOUSE_FLAVOUR=release
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse
version="$("${CARGO_TARGET_DIR:-$root/target}/release/nomouse" --version)"
commit="$(git -C "$root" rev-parse --short HEAD)"

tmp="$(mktemp -d /tmp/nomouse-bench.XXXXXX)"
# A private copy, so a concurrent `cargo build` cannot replace the binary mid-run.
cp "${CARGO_TARGET_DIR:-$root/target}/release/nomouse" "$tmp/nomouse"
bin="$tmp/nomouse"
pids=()
# Each session runs in a subshell, which does not inherit the EXIT trap: it sets its own.
kill_pids() {
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
}
cleanup() {
    kill_pids
    if ((keep)); then echo "kept $tmp"; else rm -rf "$tmp" || { sleep 1; rm -rf "$tmp"; }; fi
}
trap cleanup EXIT

summary="$tmp/summary.txt"
: >"$summary"
emit() { echo "$*" | tee -a "$summary"; }

emit "nomouse=$version commit=$commit host=$(uname -srm) cpus=$(nproc)"
emit "cpu=$(awk -F': ' '/^model name/ {print $2; exit}' /proc/cpuinfo)"
emit "load_at_start=$(cut -d' ' -f1-3 /proc/loadavg)"

now_ms() { echo $(($(date +%s%N) / 1000000)); }
# wait_for <seconds> <command...>: polls every 5 ms.
wait_for() {
    local deadline=$(($(date +%s%N) + $1 * 1000000000))
    shift
    until "$@" 2>/dev/null; do
        (($(date +%s%N) < deadline)) || return 1
        sleep 0.005
    done
}

# cpu_ticks <pid>: user + system clock ticks the process has used.
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
# rss_kib <pid> <VmRSS|VmHWM>
rss_kib() { awk -v k="$2:" '$1 == k {print $2}' "/proc/$1/status"; }
ticks_per_s="$(getconf CLK_TCK)"
# cpu_percent <ticks> <ms>: percent of one core.
cpu_percent() { python3 -c "print(f'{$1 / $ticks_per_s / ($2 / 1000) * 100:.2f}')"; }
# stats <name> <values...>: n, min, p50, p95, max, mean (nearest-rank percentiles).
stats() {
    local name="$1"
    shift
    python3 - "$name" "$@" <<'EOF'
import math, sys
name, xs = sys.argv[1], sorted(int(v) for v in sys.argv[2:])
if not xs:
    print(f"{name}: n=0")
    sys.exit()
rank = lambda p: xs[max(0, math.ceil(p / 100 * len(xs)) - 1)]
print(f"{name}: n={len(xs)} min={xs[0]} p50={rank(50)} p95={rank(95)} max={xs[-1]} mean={sum(xs) / len(xs):.1f} ms")
EOF
}

scans() { grep -c 'scan [0-9]*:' "$1" || true; }
scans_at_least() { (($(scans "$1") >= $2)); }
# label_for <log> <accessible name>: the label of the last scan's hint for that name.
label_for() { grep -E "hint [a-z]+ = [A-Za-z]+ \"$2\"$" "$1" | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/'; }
shown_ms() { grep -oE 'labels shown [0-9]+ ms' "$1" | grep -oE '[0-9]+' | tr '\n' ' '; }

# private_bus: a D-Bus session and AT-SPI with accessibility switched on.
private_bus() {
    dbus-daemon --session --fork --address="unix:path=$XDG_RUNTIME_DIR/bus" --print-pid >"$tmp/dbus.pid"
    pids+=("$(cat "$tmp/dbus.pid")")
    export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
    /usr/libexec/at-spi-bus-launcher --launch-immediately &
    pids+=($!)
    wait_for 10 dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus org.a11y.Bus.GetAddress >/dev/null
    dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus \
        org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null
}

# private_env <name>: a clean private environment; nothing from the user's session leaks in.
private_env() {
    unset DISPLAY I3SOCK SWAYSOCK WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS XDG_SESSION_TYPE \
        DBUS_SESSION_BUS_ADDRESS XDG_CURRENT_DESKTOP HYPRLAND_INSTANCE_SIGNATURE
    export XDG_RUNTIME_DIR="$tmp/$1/run" XDG_CONFIG_HOME="$tmp/$1/config" HOME="$tmp/$1/home"
    mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$HOME"
    chmod 700 "$XDG_RUNTIME_DIR"
    export GTK_MODULES=gail:atk-bridge NO_AT_BRIDGE=0 ACCESSIBILITY_ENABLED=1
}

# outcomes <log>: how many activations the daemon has ended ("cancelled", "performed ...").
outcomes() { grep -cE 'cancelled$|performed [0-9]+ action' "$1" || true; }
outcomes_at_least() { (($(outcomes "$1") >= $2)); }

# measure <session> <daemon pid> <daemon log> <focus fn> <press fn> <up fn> <down fn> <click fn>:
# the idle, latency, click and active passes, summarised under <session>. Each activation
# starts with the test window focused and ends once the daemon logged its outcome, so one
# activation never toggles the previous one off.
measure() {
    local session="$1" daemon="$2" log="$3" focus="$4" press="$5" up="$6" down="$7" click="$8"
    local t0 ticks0 i before ended start external=() hits=0 tries=0 misses=()

    sleep 2
    ticks0="$(cpu_ticks "$daemon")"
    t0="$(now_ms)"
    sleep "$idle"
    emit "$session.idle_cpu_percent=$(cpu_percent $(($(cpu_ticks "$daemon") - ticks0)) $(($(now_ms) - t0)))"
    emit "$session.idle_rss_kib=$(rss_kib "$daemon" VmRSS)"

    ticks0="$(cpu_ticks "$daemon")"
    t0="$(now_ms)"
    local first_shown
    first_shown="$(grep -c 'labels shown' "$log" || true)"
    for ((i = 1; i <= runs; i++)); do
        "$focus"
        before="$(scans "$log")"
        ended="$(outcomes "$log")"
        start="$(date +%s%N)"
        "$press"
        if wait_for 5 "$up" "$before"; then
            external+=($((($(date +%s%N) - start) / 1000000)))
        fi
        "$down"
        wait_for 5 outcomes_at_least "$log" $((ended + 1)) || true
    done
    local internal
    internal="$(grep -oE 'labels shown [0-9]+ ms' "$log" | tail -n +$((first_shown + 1)) | grep -oE '[0-9]+' | tr '\n' ' ')"
    # shellcheck disable=SC2086 # one argument per sample
    emit "$session.$(stats latency_daemon $internal)"
    emit "$session.$(stats latency_end_to_end "${external[@]}")"
    emit "$session.hints_per_scan=$(grep -oE 'labels shown [0-9]+ ms after the trigger \([0-9]+ hints' "$log" | tail -n1 | grep -oE '[0-9]+ hints' | grep -oE '[0-9]+')"

    local r
    for ((r = 1; r <= rounds; r++)); do
        for i in $(seq 1 10); do
            tries=$((tries + 1))
            if "$click" "$i"; then hits=$((hits + 1)); else misses+=("r$r:Button$i"); fi
        done
    done
    emit "$session.active_cpu_percent=$(cpu_percent $(($(cpu_ticks "$daemon") - ticks0)) $(($(now_ms) - t0)))"
    emit "$session.peak_rss_kib=$(rss_kib "$daemon" VmHWM)"
    emit "$session.clicks=$hits/$tries"
    if ((${#misses[@]})); then emit "$session.misses=${misses[*]}"; fi
}

bench_x11() {
    trap kill_pids EXIT
    private_env x11
    export DISPLAY=:96
    export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
    Xvfb "$DISPLAY" -screen 0 1600x1000x24 -nolisten tcp >"$tmp/x11/xvfb.log" 2>&1 &
    pids+=($!)
    wait_for 10 xdpyinfo -display "$DISPLAY" >/dev/null || { echo "Xvfb did not start" >&2; return 1; }
    private_bus
    printf 'ipc-socket %s\nfocus_follows_mouse no\nfont pango:DejaVu Sans 9\ndefault_border none\n' \
        "$tmp/x11/i3.sock" >"$tmp/x11/i3.conf"
    i3 -c "$tmp/x11/i3.conf" >"$tmp/x11/i3.log" 2>&1 &
    pids+=($!)
    wait_for 10 test -S "$tmp/x11/i3.sock"
    python3 "$root/scripts/e2e/gtk_app.py" &
    pids+=($!)
    wait_for 15 xdotool search --name '^nomouse-e2e' >/dev/null || { echo "GTK window missing" >&2; return 1; }
    x11_win="$(xdotool search --name '^nomouse-e2e' | head -n1)"
    sleep 1
    x11_log="$tmp/x11/daemon.log"
    NOMOUSE_LOG=debug "$bin" daemon 2>"$x11_log" &
    local daemon=$!
    pids+=("$daemon")
    wait_for 10 grep -q 'daemon ready' "$x11_log" || { echo "X11 daemon did not start" >&2; return 1; }
    # What one end-to-end poll costs (an xdotool process and an X round trip), to read the
    # end-to-end figure against: the labels are found up to one poll after they appeared.
    local start
    start="$(date +%s%N)"
    for _ in $(seq 1 20); do x11_viewable || true; done
    emit "x11.poll_cost_ms=$((($(date +%s%N) - start) / 20000000))"
    measure x11 "$daemon" "$x11_log" x11_focus x11_press x11_up x11_down x11_click
}
x11_focus() { xdotool windowactivate --sync "$x11_win" >/dev/null 2>&1 || xdotool windowfocus --sync "$x11_win"; }
x11_viewable() { xdotool search --onlyvisible --name '^nomouse-overlay$' >/dev/null; }
x11_hidden() { ! x11_viewable; }
x11_press() { xdotool key ctrl+space; }
x11_up() { x11_viewable; }
x11_down() { xdotool key Escape; wait_for 5 x11_hidden || true; }
x11_click() {
    local before ended label
    x11_focus
    before="$(scans "$x11_log")"
    ended="$(outcomes "$x11_log")"
    xdotool key ctrl+space
    if ! wait_for 5 scans_at_least "$x11_log" $((before + 1)) || ! wait_for 5 x11_viewable; then
        x11_down
        return 1
    fi
    label="$(label_for "$x11_log" "Button $1")"
    [[ -n "$label" ]] || { x11_down; return 1; }
    xdotool type --delay 20 "$label"
    wait_for 5 outcomes_at_least "$x11_log" $((ended + 1)) || true
    wait_for 3 x11_title_is "left: Button $1 #" || { x11_down; return 1; }
}
x11_title_is() { [[ "$(xdotool getwindowname "$x11_win")" == "$1"* ]]; }

bench_wayland() {
    trap kill_pids EXIT
    private_env wayland
    export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman
    private_bus
    printf 'output HEADLESS-1 resolution 1600x1000 position 0 0\nxwayland disable\ndefault_border none\nfocus_follows_mouse no\nbindsym Ctrl+space exec %s hints\n' \
        "$bin" >"$tmp/wayland/sway.conf"
    sway -c "$tmp/wayland/sway.conf" >"$tmp/wayland/sway.log" 2>&1 &
    pids+=($!)
    wait_for 10 compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" >/dev/null || { echo "sway did not start" >&2; return 1; }
    SWAYSOCK="$(compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" | head -n1)"
    export SWAYSOCK
    wait_for 10 test -S "$XDG_RUNTIME_DIR/wayland-1" || { echo "no Wayland socket" >&2; return 1; }
    export WAYLAND_DISPLAY=wayland-1
    # The headless backend has no input devices: an idle virtual keyboard keeps one on the
    # seat for the whole run (see scripts/e2e-wayland.sh).
    wtype -s 86400000 x &
    pids+=($!)
    swaymsg "exec env GDK_BACKEND=wayland python3 $root/scripts/e2e/gtk_app.py" >/dev/null
    wait_for 15 wl_has_window || { echo "native GTK window missing" >&2; return 1; }
    sleep 1
    wl_log="$tmp/wayland/daemon.log"
    NOMOUSE_LOG=debug "$bin" daemon 2>"$wl_log" &
    local daemon=$!
    pids+=("$daemon")
    wait_for 10 grep -q 'daemon ready' "$wl_log" || { echo "Wayland daemon did not start" >&2; return 1; }
    measure wayland "$daemon" "$wl_log" wl_focus wl_press wl_up wl_down wl_click
}
wl_field() { swaymsg -t get_tree | python3 "$root/scripts/e2e/sway_tree.py" field xdg_shell "$1"; }
wl_has_window() { [[ -n "$(wl_field pid)" ]]; }
wl_focus() { swaymsg "[pid=$(wl_field pid)] focus" >/dev/null; }
# The key goes through Sway's `bindsym ... exec nomouse hints`, which reaches the daemon socket.
wl_press() { wtype -M ctrl -k space -m ctrl; }
# The labels are up once the daemon logged the activation's first scan: Wayland gives a
# client no way to see another client's layer surface.
wl_up() { scans_at_least "$wl_log" $(($1 + 1)); }
wl_down() { wtype -k Escape; }
wl_click() {
    local before ended label
    wl_focus
    before="$(scans "$wl_log")"
    ended="$(outcomes "$wl_log")"
    wtype -M ctrl -k space -m ctrl
    wait_for 5 scans_at_least "$wl_log" $((before + 1)) || { wl_down; return 1; }
    label="$(label_for "$wl_log" "Button $1")"
    [[ -n "$label" ]] || { wl_down; return 1; }
    wtype -d 20 "$label"
    wait_for 5 outcomes_at_least "$wl_log" $((ended + 1)) || true
    wait_for 3 wl_title_is "left: Button $1 #" || { wl_down; return 1; }
}
wl_title_is() { [[ "$(wl_field name)" == "$1"* ]]; }

status=0
if ((x11)); then (bench_x11) || status=1; fi
if ((wayland)); then (bench_wayland) || status=1; fi
emit "load_at_end=$(cut -d' ' -f1-3 /proc/loadavg)"
if [[ -n "$out" ]]; then cp "$summary" "$out"; fi
exit "$status"
