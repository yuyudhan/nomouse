#!/usr/bin/env bash
# End-to-end test of the macOS backend against a fixture app, on the logged-in desktop.
#
# Compiles scripts/e2e/mac_app.swift (a window with buttons One..Ten, a Context button with a
# context menu, a button whose only text is its tooltip, a text field, a 200-row and a 40-row
# list and a drag Source and Target, each updating an Accessibility-visible counter) and
# scripts/e2e/mac_probe.swift (Accessibility queries and a synthetic key poster), then drives
# `nomouse` with typed labels and asserts the fixture's counters through Accessibility: ten
# left clicks, a right click that opens the context menu, a double click, a drag, scroll mode
# (largest list first, j, G, gg, Tab to the other list), search (by name and by tooltip),
# focus-then-act, the Homerow key bindings, continuous mode ending at the text field, the
# `auto` theme, the daemon (hotkey click, a mode handed over its socket, no overlay window
# left after Escape, the hotkey and `nomouse hints` toggling the labels off), `ignore.apps`
# with `nomouse reload`, the menu bar item and settings window (Settings…, a hotkey recorded
# and a theme picked land in config.toml and the daemon reloads them, Quit), and hotkeys and
# labels on the Dvorak layout.
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
# The Dvorak check switches the user's keyboard layout; cleanup puts it back (and removes
# Dvorak from the input sources when the run added it), even when a check fails.
layout_before=""
dvorak_added=0
cleanup() {
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
    if [[ -n "$layout_before" ]]; then
        "$probe" select-layout "$layout_before" || true
        if ((dvorak_added)); then "$probe" disable-layout "$dvorak" || true; fi
    fi
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
# The binary's own config directory and hints hotkey: a development build (`--bin` with a debug
# or NOMOUSE_FLAVOUR=dev build) reads nomouse-dev/ and binds ctrl+alt+shift+space, a release
# build nomouse/ and ctrl+space.
stem="$(basename "$(dirname "$("$bin" config path)")")"
hints_key="$("$bin" config show | sed -n 's/^hints = "\(.*\)"$/\1/p')"
[[ -n "$stem" && -n "$hints_key" ]] || {
    echo "FAIL: \`$bin config path\` or \`config show\` gave no config directory or hints hotkey"
    exit 1
}
# The menu bar item's name: "nomouse Dev" for a development build.
if [[ "$stem" == nomouse-dev ]]; then item_name="nomouse Dev"; else item_name=nomouse; fi
echo "--- $("$bin" --version): config in \$XDG_CONFIG_HOME/$stem, hints hotkey $hints_key"

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
# front: the fixture to the front. Ends the run when the fixture is gone or another app's
# window stays in front of it (a floating panel would take the clicks): the desktop is shared,
# so that is reported instead of failing every check after it. Transient system windows (the
# input source indicator macOS shows after typing into a field) get three seconds to go.
front() {
    local covering="" attempt
    kill -0 "$app" 2>/dev/null || {
        echo "FAIL: the fixture app exited (another program quit or closed it); $passed passed so far"
        exit 1
    }
    for attempt in 1 2 3 4 5 6; do
        "$probe" front "$app" || echo "  the fixture is not frontmost (frontmost pid $("$probe" frontmost))"
        covering="$("$probe" covering "$app" | tr '\n' ' ')"
        [[ -z "$covering" ]] && return 0
        ((attempt < 6)) && sleep 0.5
    done
    echo "FAIL: windows of ${covering}are in front of the fixture; close them and run again ($passed passed so far)"
    exit 1
}
# label_for <log> <accessible name>: the label of the last scan's hint for that name, or
# nothing.
label_for() { { grep -E "hint [a-z]+ = [A-Za-z]+ \"$2\"$" "$1" || true; } | tail -n1 | sed -E 's/.*hint ([a-z]+) = .*/\1/'; }

# run_hints <log> <args...>: brings the fixture to the front, starts `nomouse --standalone`
# (so it runs and logs here even while the daemon runs) and waits for its first scan.
run_hints() {
    local log="$1"
    shift
    front
    NOMOUSE_LOG=debug "$bin" --standalone "$@" 2>"$log" &
    hints_pid=$!
    pids+=("$hints_pid")
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

echo "--- scroll mode: the larger list is active; j, G, gg, Shift-j, then Tab to the smaller list"
log="$tmp/scroll.log"
if run_hints "$log" scroll; then
    # Areas are labelled with digits; the largest starts active, so keys scroll it at once.
    areas="$({ grep -E 'hint [0-9] = ScrollArea ' "$log" || true; } | wc -l | tr -d ' ')"
    "$probe" type jjj
    wait_for 3 value_above first-row 0 && down=1 || down=0
    after_j="$(value first-row)"
    "$probe" type G
    wait_for 5 value_is last-row 199 && bottom=1 || bottom=0
    "$probe" type gg
    wait_for 5 value_is first-row 0 && top=1 || top=0
    # Shift-j scrolls scroll.dash_step (15) lines, five times j's step: further than jjj did.
    "$probe" type J
    wait_for 3 value_above first-row "$after_j" && dash=1 || dash=0
    "$probe" key tab
    sleep 0.2
    "$probe" type jjj
    wait_for 3 value_above first-row.b 0 && second=1 || second=0
    "$probe" key escape
    reap "$hints_pid" || true
    if ((areas == 2 && down && bottom && top && dash && second)); then
        pass "scroll mode: j scrolled the larger list, G showed its last row, gg its first, Shift-j dashed, Tab moved to the smaller list"
    else
        fail "scroll mode: $areas areas labelled; j $down (first row $after_j), G $bottom, gg $top, Shift-j $dash (first row $(value first-row)), Tab and j $second (first row $(value first-row.b))"
    fi
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

echo "--- search: a button whose only text is its tooltip is found (a query with spaces)"
log="$tmp/tooltip.log"
before="$(value count.Tooltip)"
if run_hints "$log" search; then
    "$probe" type "keep a copy"
    "$probe" key return
    reap "$hints_pid" || true
    if wait_for 3 value_is count.Tooltip $((before + 1)); then pass "search by tooltip clicked the untitled button"; else fail "tooltip search: count.Tooltip went from $before to '$(value count.Tooltip)'"; fi
else
    fail "tooltip search: no search box"
fi

echo "--- focus-then-act: Tab moves the highlight, Return clicks the highlighted button"
log="$tmp/focus.log"
before="$(value count.Three)"
if run_hints "$log" hints; then
    # Hints are logged in label order and the highlight starts on the first; the menu bar's
    # come first, so Three is some way down.
    position="$({ grep -E 'hint [a-z]+ = ' "$log" || true; } | { grep -nE '= Button "Three"$' || true; } | head -n1 | cut -d: -f1)"
    tabs=()
    for ((i = 1; i < ${position:-0}; i++)); do tabs+=(tab); done
    if ((${#tabs[@]})); then "$probe" key "${tabs[@]}"; fi
    "$probe" key return
    reap "$hints_pid" || true
    if [[ -n "$position" ]] && wait_for 3 value_is count.Three $((before + 1)); then pass "Tab $((position - 1)) times and Return clicked Three (label $position)"; else fail "focus-then-act: Three is label '${position}', count.Three went from $before to '$(value count.Three)'"; fi
else
    fail "focus-then-act: no labels"
fi

echo "--- Homerow keys: a label highlights; Return clicks, twice double clicks, Shift right clicks, Cmd cmd-clicks"
homerow_config="$tmp/homerow-config"
mkdir -p "$homerow_config/$stem"
printf '[hints]\nkeys = "homerow"\n' >"$homerow_config/$stem/config.toml"
# homerow <log> <name> <chords...>: labels with the Homerow keys, types <name>'s label, then
# presses the chords.
homerow() {
    local log="$1" name="$2"
    shift 2
    XDG_CONFIG_HOME="$homerow_config" run_hints "$log" hints || return 1
    "$probe" type "$(label_for "$log" "$name")"
    "$probe" key "$@"
    reap "$hints_pid" || true
}
homerow_ok=0
before="$(value count.Four)"
if homerow "$tmp/homerow-click.log" Four return && wait_for 3 value_is count.Four $((before + 1)); then homerow_ok=$((homerow_ok + 1)); else echo "  Return: count.Four went from $before to '$(value count.Four)'"; fi
before="$(value count.Four)"
if homerow "$tmp/homerow-double.log" Four return return && wait_for 3 value_is count.Four $((before + 2)); then homerow_ok=$((homerow_ok + 1)); else echo "  Return twice: count.Four went from $before to '$(value count.Four)'"; fi
before="$(value menus)"
clicks="$(value count.Context)"
if homerow "$tmp/homerow-right.log" Context shift+return && wait_for 3 value_is menus $((before + 1)) && value_is count.Context "$clicks"; then homerow_ok=$((homerow_ok + 1)); else echo "  Shift-Return: menus went from $before to '$(value menus)', left clicks from $clicks to '$(value count.Context)'"; fi
"$probe" key escape
before="$(value cmd-clicks)"
if homerow "$tmp/homerow-cmd.log" Five cmd+return && wait_for 3 value_is cmd-clicks $((before + 1)); then homerow_ok=$((homerow_ok + 1)); else echo "  Cmd-Return: cmd-clicks went from $before to '$(value cmd-clicks)'"; fi
if ((homerow_ok == 4)); then pass "Homerow keys: Return, Return twice, Shift-Return and Cmd-Return acted as bound"; else fail "Homerow keys: $homerow_ok/4 bindings acted"; fi

echo "--- continuous mode ends after clicking the text field; typed text lands in it"
log="$tmp/continuous-field.log"
before="$(value count.One)"
if run_hints "$log" hints --continuous; then
    "$probe" type "$(label_for "$log" One)"
    if wait_for 3 value_is count.One $((before + 1)) && wait_for 5 grep -q 'scan 2:' "$log"; then
        sleep 0.3
        "$probe" type "$(label_for "$log" "Text field")"
        if reap "$hints_pid" 5; then
            "$probe" type hello
            if wait_for 3 value_is field hello; then pass "continuous mode ended at the text field and the typed text landed in it"; else fail "continuous mode: the text field holds '$(value field)'"; fi
        else
            fail "continuous mode did not end after clicking the text field"
        fi
    else
        fail "continuous mode: count.One went from $before to '$(value count.One)' or no second scan"
        kill "$hints_pid" 2>/dev/null || true
        reap "$hints_pid" || true
    fi
else
    fail "continuous mode: no labels"
fi

echo "--- theme auto follows the system appearance"
system=Light
if [[ "$(defaults read -g AppleInterfaceStyle 2>/dev/null || true)" == Dark ]]; then system=Dark; fi
theme_config="$tmp/theme-config"
mkdir -p "$theme_config/$stem"
printf '[overlay]\ntheme = "auto"\n' >"$theme_config/$stem/config.toml"
log="$tmp/theme.log"
if XDG_CONFIG_HOME="$theme_config" run_hints "$log" hints; then
    "$probe" key escape
    reap "$hints_pid" || true
    if grep -q "labels use the $system colours" "$log"; then pass "theme auto: $(grep -oE "labels use the $system colours \(background #[0-9a-f]{6}\)" "$log" | head -n1), as the system appearance"; else fail "theme auto: the system appearance is $system, the log says '$(grep -E 'labels use' "$log" || echo nothing)'"; fi
else
    fail "theme auto: no labels"
fi

# scans: how many scans the daemon logged so far (in "$daemon_log").
scans() { grep -c 'scan [0-9]*:' "$daemon_log" || true; }
scans_above() { (($(scans) > $1)); }

echo "--- daemon: hotkey click, a mode over the socket, no panel left after Escape"
daemon_log="$tmp/daemon.log"
NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon=$!
pids+=("$daemon")
if wait_for 10 grep -q 'daemon ready' "$daemon_log"; then
    front
    before_scans="$(scans)"
    before="$(value count.Two)"
    "$probe" key "$hints_key"
    if wait_for 5 scans_above "$before_scans"; then
        "$probe" type "$(label_for "$daemon_log" Two)"
        if wait_for 3 value_is count.Two $((before + 1)); then pass "daemon hotkey click landed"; else fail "daemon hotkey click: count.Two '$(value count.Two)'"; fi
    else
        fail "daemon hotkey: no scan after $hints_key"
    fi
    front
    before_scans="$(scans)"
    # Overlay panels only: the daemon's menu bar item (and a settings window) are windows too.
    windows_is() { [[ "$("$probe" panels "$daemon")" == "$1" ]]; }
    windows_above() { (($("$probe" panels "$daemon") > $1)); }
    if "$bin" hints && wait_for 5 scans_above "$before_scans" && wait_for 3 windows_above 0; then
        "$probe" key escape
        if wait_for 3 windows_is 0; then pass "no overlay panel left after Escape"; else fail "after Escape the daemon still has $("$probe" panels "$daemon") panel(s)"; fi
    else
        fail "\`nomouse hints\` to the daemon: no scan or no panel ($("$probe" panels "$daemon" || true) panels)"
    fi
    echo "--- toggle: the hotkey, and \`nomouse hints\`, pressed again dismiss the labels"
    front
    before_scans="$(scans)"
    before="$(value count.Two)"
    "$probe" key "$hints_key"
    if wait_for 5 scans_above "$before_scans" && wait_for 3 windows_above 0; then
        "$probe" key "$hints_key"
        if wait_for 3 windows_is 0 && kill -0 "$daemon" && value_is count.Two "$before"; then pass "the hotkey pressed twice left no overlay and clicked nothing"; else fail "hotkey twice: $("$probe" panels "$daemon" || true) panel(s), count.Two went from $before to '$(value count.Two)'"; fi
    else
        fail "toggle: no labels after $hints_key"
    fi
    front
    before_scans="$(scans)"
    if "$bin" hints && wait_for 5 scans_above "$before_scans" && wait_for 3 windows_above 0; then
        "$bin" hints
        if wait_for 3 windows_is 0; then pass "\`nomouse hints\` twice left no overlay"; else fail "\`nomouse hints\` twice: the daemon still has $("$probe" panels "$daemon") panel(s)"; fi
    else
        fail "toggle over the socket: no scan or no panel"
    fi
    kill "$daemon" 2>/dev/null || true
    reap "$daemon" 5 || true
else
    fail "daemon did not start: $(tail -n3 "$daemon_log")"
fi

echo "--- ignore.apps: no labels in the ignored fixture; \`nomouse reload\` applies a new config"
reload_config="$tmp/reload-config"
mkdir -p "$reload_config/$stem"
# The fixture is an unbundled program: its process name is what identifies it.
printf '[ignore]\napps = ["mac_app"]\n' >"$reload_config/$stem/config.toml"
if XDG_CONFIG_HOME="$reload_config" "$bin" doctor | grep -qE 'ignored apps: .*mac_app'; then pass "doctor lists the ignored app"; else fail "doctor does not list mac_app among the ignored apps"; fi
daemon_log="$tmp/daemon-reload.log"
XDG_CONFIG_HOME="$reload_config" NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon=$!
pids+=("$daemon")
if wait_for 10 grep -q 'daemon ready' "$daemon_log"; then
    front
    "$probe" key "$hints_key"
    if wait_for 5 grep -q 'the focused app is ignored' "$daemon_log" && (($(scans) == 0)); then pass "the hotkey in the ignored fixture showed no labels"; else fail "ignore.apps: $(scans) scan(s), log '$({ grep -E 'ignored|performed' "$daemon_log" || true; } | tail -n1)'"; fi
    printf '[hints]\nalphabet = "qwertyup"\n' >"$reload_config/$stem/config.toml"
    XDG_CONFIG_HOME="$reload_config" "$bin" reload
    if wait_for 5 grep -q 'config reloaded' "$daemon_log"; then
        front
        before_scans="$(scans)"
        before="$(value count.Two)"
        "$probe" key "$hints_key"
        if wait_for 5 scans_above "$before_scans"; then
            sleep 0.2
            label="$(label_for "$daemon_log" Two)"
            "$probe" type "$label"
            if [[ "$label" =~ ^[qwertyup]+$ ]] && wait_for 3 value_is count.Two $((before + 1)); then pass "after \`nomouse reload\` the labels use the new alphabet ('$label' clicked Two)"; else fail "after reload: label '$label', count.Two went from $before to '$(value count.Two)'"; fi
        else
            fail "after reload: no scan after $hints_key"
        fi
    else
        fail "\`nomouse reload\`: the daemon did not reload ($(grep -E 'reload' "$daemon_log" | tail -n1))"
    fi
    kill "$daemon" 2>/dev/null || true
    reap "$daemon" 5 || true
else
    fail "daemon did not start: $(tail -n3 "$daemon_log")"
fi

echo "--- menu bar and settings window: Settings…, a recorded hotkey and a picked theme reach config.toml and the daemon"
settings_config="$tmp/settings-config"
mkdir -p "$settings_config/$stem"
settings_file="$settings_config/$stem/config.toml"
daemon_log="$tmp/daemon-settings.log"
XDG_CONFIG_HOME="$settings_config" NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon=$!
pids+=("$daemon")
# has_line <line>: the settings window's config file has exactly that line.
has_line() { grep -qxF "$1" "$settings_file"; }
reloads() { grep -c 'config reloaded' "$daemon_log" || true; }
reloads_above() { (($(reloads) > $1)); }
if wait_for 10 grep -q 'daemon ready' "$daemon_log"; then
    if [[ "$("$probe" status-item "$daemon" || true)" == "$item_name" ]]; then pass "the menu bar shows the $item_name item"; else fail "the daemon has no $item_name item in the menu bar"; fi
    if "$probe" menu "$daemon" "Settings…" && wait_for 5 "$probe" window "$daemon" nomouse-settings; then
        pass "Settings… in the menu opens the settings window"
        before_reloads="$(reloads)"
        "$probe" press-title "$daemon" Hotkeys
        if "$probe" press "$daemon" hotkeys.hints && sleep 0.3 && "$probe" key ctrl+alt+j && wait_for 5 has_line 'hints = "ctrl+alt+j"'; then pass "a hotkey recorded in the window is in config.toml at once"; else fail "recorded hotkey: config.toml has '$(grep -E '^hints' "$settings_file" || true)'"; fi
        if wait_for 5 reloads_above "$before_reloads"; then pass "the daemon reloaded the recorded hotkey"; else fail "the daemon did not reload after the hotkey changed ($(grep -E 'reload|config file' "$daemon_log" | tail -n1))"; fi
        before_reloads="$(reloads)"
        "$probe" press-title "$daemon" Appearance
        if "$probe" choose "$daemon" overlay.theme Dark && wait_for 5 has_line 'theme = "dark"'; then pass "a theme picked in the window is in config.toml at once"; else fail "picked theme: config.toml has '$(grep -E '^theme' "$settings_file" || true)'"; fi
        if wait_for 5 reloads_above "$before_reloads"; then pass "the daemon reloaded the picked theme"; else fail "the daemon did not reload after the theme changed"; fi
        "$probe" close "$daemon" nomouse-settings || true
        front
        before_scans="$(scans)"
        "$probe" key ctrl+alt+j
        if wait_for 5 scans_above "$before_scans"; then pass "the recorded hotkey labels the fixture"; else fail "the recorded hotkey ctrl+alt+j started no scan"; fi
        "$probe" key escape
    else
        fail "Settings… opened no settings window"
    fi
    "$probe" menu "$daemon" "Quit nomouse" || true
    if wait_for 5 gone "$daemon"; then
        wait "$daemon" 2>/dev/null || true
        pass "Quit nomouse in the menu ends the daemon"
    else
        fail "Quit nomouse left the daemon running"
        kill "$daemon" 2>/dev/null || true
        reap "$daemon" 5 || true
    fi
else
    fail "daemon did not start: $(tail -n3 "$daemon_log")"
fi

echo "--- Dvorak: a letter hotkey triggers by character, labels type as shown"
# The hints hotkey is ctrl+alt+o: on Dvorak `o` is on the ANSI `s` key. The daemon starts on
# the user's layout, then the layout switches under it, so it must re-match the hotkey live.
dvorak=com.apple.keylayout.Dvorak
layout_before="$("$probe" layout)"
if "$probe" layout-enabled "$dvorak"; then dvorak_added=0; else dvorak_added=1; fi
mkdir -p "$tmp/dvorak-config/$stem"
printf '[hotkeys]\nhints = "ctrl+alt+o"\n' >"$tmp/dvorak-config/$stem/config.toml"
daemon_log="$tmp/daemon-dvorak.log"
XDG_CONFIG_HOME="$tmp/dvorak-config" NOMOUSE_LOG=debug "$bin" daemon 2>"$daemon_log" &
daemon=$!
pids+=("$daemon")
if wait_for 10 grep -q 'daemon ready' "$daemon_log"; then
    "$probe" select-layout "$dvorak"
    if wait_for 3 grep -q "keyboard layout $dvorak" "$daemon_log"; then
        front
        before_scans="$(scans)"
        before="$(value count.Three)"
        "$probe" key ctrl+alt+o
        if wait_for 5 scans_above "$before_scans"; then
            label="$(label_for "$daemon_log" Three)"
            "$probe" type "$label"
            if wait_for 3 value_is count.Three $((before + 1)); then pass "Dvorak: ctrl+alt+o and label '$label' clicked Three"; else fail "Dvorak: label '$label' typed, count.Three '$(value count.Three)'"; fi
        else
            fail "Dvorak: no scan after ctrl+alt+o (the key that types o)"
        fi
    else
        fail "Dvorak: the daemon did not re-match its hotkeys after the layout switch"
    fi
else
    fail "Dvorak daemon did not start: $(tail -n3 "$daemon_log")"
fi
kill "$daemon" 2>/dev/null || true
reap "$daemon" 5 || true
"$probe" select-layout "$layout_before"
if ((dvorak_added)); then "$probe" disable-layout "$dvorak"; fi
layout_before=""

echo "--- $passed passed, $failed failed"
((failed == 0))
