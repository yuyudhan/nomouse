# Wayland

nomouse runs natively on wlroots compositors (Sway 1.7+, Hyprland). `nomouse doctor` lists what
the compositor offers and what does not work, and why.

- **Overlay**: one `zwlr_layer_shell_v1` surface per output on the overlay layer, click-through
  (empty input region) and keyboard-exclusive while labels are up.
- **Input**: `zwlr_virtual_pointer_v1` plus `zwp_virtual_keyboard_v1` (for modifier clicks) when
  the compositor offers them; otherwise an absolute pointer on `/dev/uinput`. Your user needs
  write access to `/dev/uinput` for that; check with `ls -l /dev/uinput` (a `+` means an ACL is
  set, as `uaccess` does) and, if needed, add the udev rule (needs root, once):

      echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' \
          | sudo tee /etc/udev/rules.d/70-nomouse-uinput.rules \
          && sudo udevadm control --reload && sudo udevadm trigger

- **Hotkeys**: Wayland gives clients no global hotkeys, so the compositor binds the commands, and
  `nomouse hints|continuous|scroll|search` hand the mode to the running `nomouse daemon` over
  `$XDG_RUNTIME_DIR/nomouse.sock` (without a daemon they run in-process). Sway:

      exec nomouse daemon
      bindsym Ctrl+space exec nomouse hints
      bindsym Ctrl+Shift+space exec nomouse continuous
      bindsym Ctrl+Mod1+space exec nomouse scroll
      bindsym Ctrl+Mod1+slash exec nomouse search

  Hyprland: `exec-once = nomouse daemon` and `bind = CTRL, space, exec, nomouse hints`. The same
  commands work on X11, where the daemon also grabs its own hotkeys; `--standalone` runs a mode
  in the calling process even when a daemon runs.
- **Coordinates**: AT-SPI reports native Wayland windows' elements relative to the window (a
  Wayland client does not know where its window is). XWayland apps report global coordinates
  and work fully. For native GTK/Qt windows nomouse adds the window's position from the
  compositor's IPC (Sway `get_tree`, Hyprland `j/clients`). Limits: a native app's popup menus
  are placed relative to their own surface, and with several windows of one native app on
  screen the focused one is the one placed correctly.
- **GNOME Wayland**: Mutter implements neither layer-shell nor virtual pointer, and Shell offers
  clients no hotkeys: nomouse refuses with that explanation; use "GNOME on Xorg".
  **KDE Plasma Wayland**: layer-shell works, input goes through `/dev/uinput`, the hotkey is a
  custom shortcut running `nomouse hints`; KWin has no window-position IPC, so only XWayland
  apps are placed correctly (doctor reports it).

`scripts/e2e-wayland.sh` runs the Wayland end-to-end test on a private headless Sway.
