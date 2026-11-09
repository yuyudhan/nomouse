#!/usr/bin/env bash
# End-to-end test of nomouse as a desktop app on Linux: the launcher entry, the tray icon and
# its menu, and the settings window. Everything runs in private sessions (a private
# XDG_RUNTIME_DIR, D-Bus session, HOME and config), never the user's desktop.
#
# X11: Xvfb + i3 + AT-SPI, a stand-in tray host (scripts/e2e/sni_watcher.py) and a stand-in
# XDG portal for the light and dark style (scripts/e2e/fake_portal.py).
#   - The launcher entry, run the way a desktop runs it (gio launch), starts the daemon and
#     opens the settings window; a second launch keeps the one window.
#   - The tray item registers; its menu, read over D-Bus as a tray host reads it, has the
#     Mac menu's rows; Pause releases the hotkeys and the window's switch follows; the
#     window's switch resumes.
#   - The window, driven over AT-SPI with real pointer and key events (xdotool): a hotkey
#     recorded on the Hotkeys page lands in config.toml, the daemon reloads and the new chord
#     starts hints; a theme picked on the Appearance page lands in config.toml and the daemon
#     reloads; Reset to defaults asks first, then clears the settings; Settings… in the tray
#     menu reopens the closed window.
# Wayland: headless Sway + waybar (a real tray host).
#   - The icon shows in waybar and a click on it opens the menu; Settings… opens the window
#     natively on Wayland, whose Hotkeys page shows the Sway bindings to add.
#
# Usage: scripts/e2e-desktop.sh [--display :N] [--keep] [--no-wayland] [--shots DIR]
#   --shots DIR  also save screenshots: settings-<page>-<light|dark>.png for every page and
#                tray-menu-<light|dark>.png (docs/media holds the README's copies).
# Needs: Xvfb, i3, xdotool, ImageMagick (import), dbus-daemon, at-spi2-core, gio
# (libglib2.0-bin), python3-gi with gir1.2-atspi-2.0, GTK 4 and libadwaita; for Wayland sway,
# waybar and grim.
set -euo pipefail

display=":97"
keep=0
wayland=1
shots=""
while (($#)); do
    case "$1" in
        --display) display="$2"; shift 2 ;;
        --keep) keep=1; shift ;;
        --no-wayland) wayland=0; shift ;;
        --shots) shots="$(mkdir -p "$2" && cd "$2" && pwd)"; shift 2 ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done

root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p nomouse -p nomouse-settings

tmp="$(mktemp -d /tmp/nomouse-e2e-desktop.XXXXXX)"
# Private copies side by side, as installed, so a concurrent `cargo build` cannot replace them.
mkdir -p "$tmp/bin"
cp "${CARGO_TARGET_DIR:-$root/target}/release/nomouse" "${CARGO_TARGET_DIR:-$root/target}/release/nomouse-settings" "$tmp/bin/"
bin="$tmp/bin/nomouse"
settings="$tmp/bin/nomouse-settings"
driver=(python3 "$root/scripts/e2e/settings_driver.py")
tray=(python3 "$root/scripts/e2e/tray_menu.py")
pids=()
cleanup() {
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    pkill -f -- "$tmp/bin/" 2>/dev/null || true
    wait 2>/dev/null || true
    if ((keep)); then echo "kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT

# A clean private environment: nothing from the user's desktop session leaks in. GDK_BACKEND
# pins GTK to the private X server: GTK 4 would otherwise try Wayland first.
unset DISPLAY I3SOCK SWAYSOCK WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS XDG_SESSION_TYPE \
    DBUS_SESSION_BUS_ADDRESS XDG_CURRENT_DESKTOP HYPRLAND_INSTANCE_SIGNATURE GTK_THEME
export XDG_RUNTIME_DIR="$tmp/run" XDG_CONFIG_HOME="$tmp/config" HOME="$tmp/home" \
    XDG_DATA_HOME="$tmp/data" PATH="$tmp/bin:$PATH" NOMOUSE_LOG=debug
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$HOME" "$XDG_DATA_HOME/applications"
chmod 700 "$XDG_RUNTIME_DIR"
config="$XDG_CONFIG_HOME/nomouse/config.toml"

passed=0
failed=0
pass() { echo "PASS: $*"; passed=$((passed + 1)); }
fail() { echo "FAIL: $*"; failed=$((failed + 1)); }
check() {
    local what="$1"
    shift
    if "$@"; then pass "$what"; else fail "$what"; fi
}

# wait_for <seconds> <command...>: polls every 50 ms.
wait_for() {
    local deadline=$((SECONDS + $1))
    shift
    until "$@" 2>/dev/null; do
        ((SECONDS < deadline)) || return 1
        sleep 0.05
    done
}

# The launcher entry and icons, as install.sh puts them (Exec runs the private nomouse).
sed "s|^Exec=nomouse |Exec=$bin |; s|^TryExec=nomouse$|TryExec=$bin|" \
    "$root/packaging/linux/share/applications/io.github.yuyudhan.nomouse.desktop" \
    >"$XDG_DATA_HOME/applications/io.github.yuyudhan.nomouse.desktop"
cp -r "$root/packaging/linux/share/icons" "$XDG_DATA_HOME/"

session_bus() {
    dbus-daemon --session --fork --address="unix:path=$XDG_RUNTIME_DIR/bus" --print-pid >"$tmp/dbus.pid"
    pids+=("$(cat "$tmp/dbus.pid")")
    export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
    /usr/libexec/at-spi-bus-launcher --launch-immediately &
    pids+=($!)
    wait_for 10 dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus org.a11y.Bus.GetAddress >/dev/null
    dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus \
        org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null
}

daemon_property() {
    gdbus call --session -d io.github.yuyudhan.nomouse.Daemon -o /io/github/yuyudhan/nomouse/Daemon \
        -m org.freedesktop.DBus.Properties.Get io.github.yuyudhan.nomouse.Daemon1 "$1"
}
daemon_up() { daemon_property Version >/dev/null; }
paused_is() { [ "$(daemon_property Paused)" = "(<$1>,)" ]; }
windows() { xdotool search --onlyvisible --name '^nomouse Settings$' 2>/dev/null | wc -l; }
one_window() { [ "$(windows)" -eq 1 ]; }
no_window() { [ "$(windows)" -eq 0 ]; }
settings_processes() { pgrep -fc -- "^$settings" || true; }
config_has() { grep -qx "$1" "$config"; }
config_lacks() { ! grep -q "$1" "$config"; }
reloads() { grep -c 'config reloaded' "$tmp/daemon.log" || true; }
reloaded_after() { [ "$(reloads)" -gt "$1" ]; }
activations() { grep -cE 'nothing to label|scan [0-9]+:' "$tmp/daemon.log" || true; }
activated_after() { [ "$(activations)" -gt "$1" ]; }
switch_is() { [ "$("${driver[@]}" checked 'check box' 'nomouse is on')" = "$1" ]; }
tray_item() { head -n 1 "$tmp/items"; }
menu_has() { "${tray[@]}" "$(tray_item)" labels | grep -q "$1"; }
show_page() { "$settings" --page "$1" && sleep 0.4; }

# shoot <page> <style>: the window's screenshot into the shots dir.
shoot() {
    local window
    window="$(xdotool search --onlyvisible --name '^nomouse Settings$' | head -n 1)"
    import -window "$window" "$shots/settings-$1-$2.png"
}

pages=(general hotkeys labels appearance scroll search ignored advanced doctor)

# --- X11 ---------------------------------------------------------------------------------
export DISPLAY="$display" GDK_BACKEND=x11
Xvfb "$display" -screen 0 1100x780x24 -nolisten tcp >"$tmp/xvfb.log" 2>&1 &
pids+=($!)
wait_for 10 xdpyinfo -display "$display" >/dev/null || { echo "Xvfb did not start"; exit 1; }
session_bus
echo 2 >"$tmp/portal"
python3 "$root/scripts/e2e/fake_portal.py" "$tmp/portal" &
pids+=($!)
python3 "$root/scripts/e2e/sni_watcher.py" "$tmp/items" &
pids+=($!)
printf 'ipc-socket %s/i3.sock\ndefault_border none\nfocus_follows_mouse no\n' "$tmp" >"$tmp/i3.conf"
i3 -c "$tmp/i3.conf" >"$tmp/i3.log" 2>&1 &
pids+=($!)
wait_for 10 test -S "$tmp/i3.sock"
# The daemon the launcher starts writes here (launch.rs: the lock's path with .log).
daemon_log="$XDG_RUNTIME_DIR/nomouse.log"
ln -s "$daemon_log" "$tmp/daemon.log"

gio launch "$XDG_DATA_HOME/applications/io.github.yuyudhan.nomouse.desktop" >"$tmp/launch.log" 2>&1 || true
check "the launcher entry starts the daemon" wait_for 15 daemon_up
check "the launcher entry opens the settings window" wait_for 15 one_window
"${driver[@]}" wait 20
gio launch "$XDG_DATA_HOME/applications/io.github.yuyudhan.nomouse.desktop" >>"$tmp/launch.log" 2>&1 || true
sleep 1.5
check "launching again keeps one window and one settings process" \
    test "$(windows)/$(settings_processes)" = "1/1"

check "the tray item registers with the tray host" wait_for 10 test -s "$tmp/items"
"${tray[@]}" "$(tray_item)" labels >"$tmp/menu.txt"
for row in "$("$bin" --version): on" "Pause nomouse" "Show hints" "Continuous hints" "Scroll" "Search" \
    "Settings…" "Run doctor…" "Start at login" "Check for updates…" "About nomouse" "Quit nomouse"; do
    check "the tray menu has \"$row\"" grep -qF "$row" "$tmp/menu.txt"
done
check "the menu shows the hints hotkey" grep -qF "Ctrl+Space" "$tmp/menu.txt"
icon_name() { "${tray[@]}" "$(tray_item)" item IconName | grep -q "^$1\$"; }
check "the tray icon is the themed symbolic icon" icon_name io.github.yuyudhan.nomouse-symbolic

"${tray[@]}" "$(tray_item)" click "Pause"
check "Pause in the tray menu pauses the daemon" wait_for 5 paused_is true
check "the window's switch follows the pause" wait_for 5 switch_is 0
check "the paused icon shows" icon_name io.github.yuyudhan.nomouse-paused-symbolic
check "the menu offers Resume while paused" wait_for 5 menu_has "Resume nomouse"
"${driver[@]}" act 'check box' 'nomouse is on'
check "the window's switch resumes the daemon" wait_for 5 paused_is false

# Start at login, from the tray menu and back off from the window's switch.
autostart="$XDG_CONFIG_HOME/autostart/nomouse.desktop"
autostart_on() { grep -q "^Exec=.*nomouse daemon" "$autostart"; }
login_switch_is() { [ "$("${driver[@]}" checked 'check box' 'Start at login')" = "$1" ]; }
show_page general
"${tray[@]}" "$(tray_item)" click "Start at login"
check "Start at login in the tray menu writes the autostart entry" wait_for 5 autostart_on
check "the window's Start at login switch follows" wait_for 5 login_switch_is 1
"${driver[@]}" act 'check box' 'Start at login'
check "the window's switch removes the autostart entry" wait_for 5 test ! -e "$autostart"

# A recorded hotkey: Hotkeys page, click the Hints shortcut, press the new chord.
show_page hotkeys
before="$(reloads)"
"${driver[@]}" click 'list item' Hints 'push button' '*'
sleep 0.5
xdotool key super+shift+h
check "a recorded hotkey lands in config.toml" wait_for 5 config_has 'hints = "shift+super+h"'
check "the daemon reloads the recorded hotkey" wait_for 5 reloaded_after "$before"
check "the Hints button shows the new chord" wait_for 5 "${driver[@]}" has 'push button' 'Shift+Super+H'
before="$(activations)"
xdotool key super+shift+h
check "the new chord starts hints" wait_for 5 activated_after "$before"
xdotool key Escape

# A theme: Appearance page, click the Theme row and pick Dark in its list with the keys (the
# list is a popover, its own X window, whose position AT-SPI does not give): it opens on the
# current choice, Original, then Light, Dark, Auto.
show_page appearance
before="$(reloads)"
"${driver[@]}" click 'combo box' 'Theme*'
check "the Theme row opens its list" wait_for 5 "${driver[@]}" has filler GtkPopover label Dark
xdotool key Down Down Return
check "a picked theme lands in config.toml" wait_for 5 config_has 'theme = "dark"'
check "the daemon reloads the theme" wait_for 5 reloaded_after "$before"

# Reset to defaults asks first.
show_page advanced
"${driver[@]}" act 'push button' 'Reset…'
check "Reset to defaults asks first" wait_for 5 "${driver[@]}" has dialog '' 'push button' 'Reset*'
check "nothing is reset before the answer" config_has 'theme = "dark"'
before="$(reloads)"
"${driver[@]}" act dialog '' 'push button' 'Reset*'
check "Reset clears the recorded hotkey" wait_for 5 config_lacks 'hints ='
check "Reset clears the theme" config_lacks 'theme ='
check "the daemon reloads the defaults" wait_for 5 reloaded_after "$before"

# Closing the window leaves the daemon; Settings… in the tray menu opens it again.
"${driver[@]}" act frame '*' 'push button' Close
check "closing the window closes it" wait_for 5 no_window
check "the daemon keeps running without the window" daemon_up
"${tray[@]}" "$(tray_item)" click "Settings…"
check "Settings… in the tray menu opens the window" wait_for 15 one_window

if [ -n "$shots" ]; then
    # Light, in the new window (no toast from the steps above).
    "${driver[@]}" wait 20
    for page in "${pages[@]}"; do
        show_page "$page"
        sleep 0.6
        shoot "$page" light
    done
fi

if [ -n "$shots" ]; then
    # Dark: the portal says dark, and a new window reads it.
    echo 1 >"$tmp/portal"
    "${driver[@]}" wait 20
    "${driver[@]}" act frame '*' 'push button' Close
    wait_for 5 no_window
    "$settings" --page general >>"$tmp/settings.log" 2>&1 &
    wait_for 15 one_window
    "${driver[@]}" wait 20
    for page in "${pages[@]}"; do
        show_page "$page"
        sleep 0.6
        shoot "$page" dark
    done
fi

"${tray[@]}" "$(tray_item)" click "Quit"
check "Quit in the tray menu stops the daemon" wait_for 10 sh -c '! gdbus introspect --session -d io.github.yuyudhan.nomouse.Daemon -o / >/dev/null 2>&1'
pkill -f -- "^$settings" 2>/dev/null || true

# --- Wayland -----------------------------------------------------------------------------
if ((wayland)); then
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    pids=()
    unset DISPLAY GDK_BACKEND DBUS_SESSION_BUS_ADDRESS
    rm -rf "$XDG_RUNTIME_DIR" "$config"
    mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME/waybar"
    chmod 700 "$XDG_RUNTIME_DIR"
    session_bus
    export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman
    printf 'output HEADLESS-1 resolution 1280x800 position 0 0\nxwayland disable\n' >"$tmp/sway.conf"
    sway -c "$tmp/sway.conf" >"$tmp/sway.log" 2>&1 &
    pids+=($!)
    wait_for 10 compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" >/dev/null || { echo "sway did not start"; exit 1; }
    SWAYSOCK="$(compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" | head -n1)"
    export SWAYSOCK WAYLAND_DISPLAY=wayland-1 XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
    wait_for 10 test -S "$XDG_RUNTIME_DIR/wayland-1"
    # The tray is waybar's rightmost module: its 20 px icon is centred 20 px from the edge.
    cat >"$XDG_CONFIG_HOME/waybar/config" <<'EOF'
{ "layer": "top", "position": "top", "height": 30, "modules-left": ["clock"],
  "modules-right": ["tray"], "tray": { "icon-size": 20, "spacing": 8 } }
EOF
    click_at() {
        swaymsg -q "seat seat0 cursor set $1 $2"
        swaymsg -q "seat seat0 cursor press button1"
        swaymsg -q "seat seat0 cursor release button1"
    }
    sway_window() { swaymsg -t get_tree | grep -q '"app_id": "io.github.yuyudhan.nomouse"'; }
    watcher_up() { gdbus introspect --session -d org.kde.StatusNotifierWatcher -o /StatusNotifierWatcher >/dev/null; }
    # A screenshot of the bar and the menu under the icon (grim -g x,y wxh).
    menu_shot() { grim -g "1000,0 280x340" "$1"; }

    for style in light dark; do
        if [ "$style" = dark ]; then theme="Adwaita:dark"; else theme="Adwaita"; fi
        GTK_THEME="$theme" waybar >"$tmp/waybar-$style.log" 2>&1 &
        waybar_pid=$!
        wait_for 10 watcher_up
        "$bin" daemon >"$tmp/daemon-wayland.log" 2>&1 &
        daemon_pid=$!
        wait_for 10 daemon_up
        sleep 1.5
        grim -g "1000,0 280x30" "$tmp/bar-$style.png"
        click_at 1260 15
        sleep 1
        menu_shot "$tmp/menu-$style.png"
        if [ "$style" = light ]; then
            check "the icon shows in waybar's tray" python3 -c "
import sys
from gi.repository import GdkPixbuf
p = GdkPixbuf.Pixbuf.new_from_file('$tmp/bar-light.png')
row = p.get_pixels(); n = p.get_n_channels(); s = p.get_rowstride()
px = lambda x, y: row[y * s + x * n:y * s + x * n + 3]
sys.exit(0 if len({px(x, y) for x in range(250, 280) for y in range(5, 25)}) > 3 else 1)"
            check "a click on the icon opens the menu" \
                sh -c "! cmp -s '$tmp/bar-light.png' '$tmp/menu-light.png' && [ \$(stat -c %s '$tmp/menu-light.png') -gt \$(stat -c %s '$tmp/bar-light.png') ]"
        fi
        if [ -n "$shots" ]; then cp "$tmp/menu-$style.png" "$shots/tray-menu-$style.png"; fi
        # Close the menu: a click on the empty desktop.
        click_at 400 500
        sleep 0.5
        if [ "$style" = light ]; then
            item="$(gdbus call --session -d org.kde.StatusNotifierWatcher -o /StatusNotifierWatcher \
                -m org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems |
                grep -o ':[0-9.]*' | head -n 1)"
            "${tray[@]}" "$item" click "Settings…"
            check "Settings… opens the window natively on Wayland" wait_for 15 sway_window
            "$settings" --page hotkeys
            sleep 1
            check "the Hotkeys page on Sway shows the bindsym lines" \
                sh -c "python3 '$root/scripts/e2e/settings_driver.py' texts | grep -qF 'bindsym Ctrl+space exec nomouse hints'"
            pkill -f -- "^$settings" 2>/dev/null || true
        fi
        "$bin" quit >/dev/null 2>&1 || true
        wait "$daemon_pid" 2>/dev/null || true
        kill "$waybar_pid" 2>/dev/null || true
        wait "$waybar_pid" 2>/dev/null || true
    done
fi

echo
echo "e2e-desktop: $passed passed, $failed failed"
((failed == 0))
