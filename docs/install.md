# Installing nomouse

- [The one-line install](#the-one-line-install)
- [macOS](#macos)
- [Linux X11](#linux-x11)
- [Linux Wayland](#linux-wayland)
- [Debian and Ubuntu package](#debian-and-ubuntu-package)
- [Windows](#windows)
- [Installer options](#installer-options)
- [Updating](#updating)
- [Uninstalling](#uninstalling)
- [Building from source](#building-from-source)

## The one-line install

```sh
curl -fsSL --retry 5 --retry-all-errors https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash
```

The installer needs no sudo. It picks the build for your OS and CPU, downloads it from the latest
[GitHub release](https://github.com/yuyudhan/nomouse/releases), checks it against the
release's `.sha256` and refuses to install on a mismatch, then prints the next steps for your
system. `--retry-all-errors` makes curl retry a connection the network cuts; the installer's
own downloads retry the same way. curl older than 7.71 (Ubuntu 20.04, RHEL 8) does not know it:
drop `--retry-all-errors` there.

| System | What you get |
|--------|--------------|
| macOS 11+, Apple silicon and Intel | `nomouse.app` in `/Applications` (`~/Applications` when `/Applications` is not writable), and a `nomouse` command in `~/.local/bin` that runs the app's binary |
| Linux x86_64 and aarch64 | the `nomouse` and `nomouse-settings` (the settings window) binaries in `~/.local/bin`, and nomouse in the app launcher: the desktop entry `io.github.yuyudhan.nomouse.desktop` in `~/.local/share/applications` with its icons, and the tray icons, in `~/.local/share/icons/hicolor` |

If `~/.local/bin` is not on your `PATH`, the installer says so and gives the line to add to
`~/.profile` or `~/.zshrc`. The settings window needs GTK 4 (4.6 or later) and libadwaita (1.1
or later), which Ubuntu 22.04 and later, Fedora and Arch desktops have; when they are missing the
installer prints the command that adds them (for example `sudo apt install libgtk-4-1
libadwaita-1-0`). nomouse itself runs without them.

Offline, or behind a proxy that blocks the script: download the release assets and install from
the folder (`gh release download --repo yuyudhan/nomouse --pattern '*' --dir nomouse-release`
does it in one step), then:

```sh
bash nomouse-release/install.sh --from nomouse-release
```

## macOS

1. Run the one-line install above.
2. Open the app: `open /Applications/nomouse.app`. It has no Dock icon; its keycap icon sits
   in the menu bar, and it listens for the hotkey.
3. Grant Accessibility. nomouse reads buttons and links through the macOS accessibility API and
   clicks through it, which needs one permission:
   1. On its first start a Welcome window explains this; **Open System Settings** opens
      **Privacy & Security › Accessibility**.
   2. Switch on **nomouse** in the list (unlock with your password if asked).
   3. nomouse notices the switch within a second and restarts by itself. No logout needed.

   Input Monitoring and Screen Recording are not needed.
4. Press `ctrl+space` in any app. Labels appear; type one to click.
5. Start at login: **Start at login** in the menu bar icon's menu (or on the settings
   window's General page); it appears under **System Settings › General › Login Items**.

The release builds are not notarized by Apple yet. The installer removes the quarantine flag so
Gatekeeper does not block the first start. If you downloaded `nomouse.app` or the `.dmg` in a
browser instead, remove it yourself once:

```sh
xattr -dr com.apple.quarantine /Applications/nomouse.app
```

You can run `nomouse setup` in a terminal instead of opening the app: it walks through the same
permission. A permission granted to the app belongs to the app; when you run the `nomouse`
command from a terminal, macOS asks on behalf of the terminal instead, so grant it there too or
start the app.

## Linux X11

1. Run the one-line install above.
2. `nomouse setup` turns on the accessibility bus (`org.a11y.Status.IsEnabled`) if your desktop
   left it off and explains anything else that is missing.
3. Open nomouse from the app launcher: press Super, type `nomouse` and pick it. That starts
   the daemon unless it is running already and opens the settings window; `ctrl+space` shows
   the labels. (`nomouse` in a terminal starts the daemon too.) Right-click the entry for
   **nomouse doctor**, which opens the doctor's report in a terminal.
4. Start at login: the settings window's **General › Start at login**, or the tray menu's
   **Start at login** (an XDG autostart entry, `~/.config/autostart/nomouse.desktop`). In i3,
   `exec nomouse daemon` in its config works too.

The tray icon (the nomouse label in the panel) has the menu: on/off, Pause, each mode, Settings…,
Run doctor…, Start at login, Check for updates…, About and Quit. KDE, Xfce, waybar and most
panels show it; on GNOME it needs the AppIndicator extension, which Ubuntu turns on by default
(elsewhere: `gnome-extensions enable ubuntu-appindicators@ubuntu.com`, or install "AppIndicator
and KStatusNotifierItem Support"). Without a tray, use the app launcher.

Most GTK and KDE apps expose their accessibility tree by default. Two kinds need a switch:

| Apps | Switch |
|------|--------|
| Qt apps | `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` in your session environment (e.g. `~/.profile`) |
| Chromium, Chrome, Electron apps (VS Code, Slack, Discord) | start them with `--force-renderer-accessibility` |

## Linux Wayland

Wayland gives programs no global hotkeys, so your compositor binds them and runs
`nomouse hints` (and the other modes), which hand the mode to the running daemon.

**Sway** (`~/.config/sway/config`):

```text
exec nomouse daemon
bindsym Ctrl+space exec nomouse hints
bindsym Ctrl+Shift+space exec nomouse continuous
bindsym Ctrl+Mod1+space exec nomouse scroll
bindsym Ctrl+Mod1+slash exec nomouse search
```

**Hyprland** (`~/.config/hypr/hyprland.conf`):

```text
exec-once = nomouse daemon
bind = CTRL, space, exec, nomouse hints
bind = CTRL SHIFT, space, exec, nomouse continuous
bind = CTRL ALT, space, exec, nomouse scroll
bind = CTRL ALT, slash, exec, nomouse search
```

**KDE Plasma**: in System Settings › Keyboard › Shortcuts, add a custom shortcut running
`nomouse hints` (and one for each other mode), and add `nomouse daemon` to Autostart. KWin has no
virtual pointer for programs, so nomouse clicks through `/dev/uinput`, which your user must be
allowed to write. Check it:

```sh
ls -l /dev/uinput      # a "+" after the permissions means an ACL is set
```

If it is not writable, add this udev rule once (needs root), then log out and in:

```sh
echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' \
    | sudo tee /etc/udev/rules.d/70-nomouse-uinput.rules \
    && sudo udevadm control --reload && sudo udevadm trigger
```

`uaccess` gives the logged-in seat user access without adding anyone to a group.

**GNOME on Wayland** is not supported yet: Mutter offers no overlay layer, no virtual pointer and
no client hotkeys. Choose **GNOME on Xorg** (**Ubuntu on Xorg** on Ubuntu) under the gear on the
login screen; the installer, `nomouse doctor` and the app launcher's nomouse entry all say so
when they run on GNOME Wayland. See
[platform-support.md](platform-support.md) for each compositor.

## Debian and Ubuntu package

Every release also has `nomouse_amd64.deb` and `nomouse_arm64.deb`, which install
`/usr/bin/nomouse`, `/usr/bin/nomouse-settings` and the same app launcher entry and icons as
the installer, under `/usr/share/applications` and `/usr/share/icons/hicolor`; apt installs GTK 4
and libadwaita for the settings window (the package recommends them):

```sh
sudo apt install ./nomouse_amd64.deb
```

## Windows

The Windows backend (UI Automation) is earlier than the macOS and Linux ones. A build
(`nomouse-x86_64-pc-windows-msvc.zip`) and a PowerShell installer that needs no administrator
ship with every release:

```powershell
irm https://github.com/yuyudhan/nomouse/releases/latest/download/install.ps1 | iex
```

## Installer options

Options go after `bash -s --` when the script is piped:

```sh
curl -fsSL https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash -s -- --version 0.1.0
```

| Option | Effect |
|--------|--------|
| `--version X.Y.Z` | install that release instead of the latest |
| `--from DIR` | install from a folder holding the release assets (no network) |
| `--uninstall` | remove nomouse, keep the config |
| `--uninstall --purge` | also delete `~/.config/nomouse` |
| `-h`, `--help` | show the options |

| Environment | Effect |
|-------------|--------|
| `NOMOUSE_INSTALL_DIR` | where the `nomouse` command goes (default `~/.local/bin`) |
| `NOMOUSE_APP_DIR` | macOS: where `nomouse.app` goes (default `/Applications`) |
| `GH_TOKEN` or `GITHUB_TOKEN` | download through the GitHub API with this token (avoids anonymous rate limits) |
| `NOMOUSE_INSTALL_BASE` | a URL holding the release assets, for mirrors |
| `NOMOUSE_API_BASE` | the GitHub API base for token downloads (default `https://api.github.com`) |

## Updating

Run the one-line install again. It replaces the binary (or the app) in place; a running daemon
keeps working until you restart it. Your config is never touched.

## Uninstalling

```sh
curl -fsSL https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash -s -- --uninstall
```

This removes `~/.local/bin/nomouse`, on Linux `nomouse-settings`, the app launcher entry and its
icons, the tray icons and the start-at-login entry, and on macOS
`nomouse.app` (only when it is nomouse's bundle, `com.yuyudhan.nomouse`). Add `--purge` to
delete `~/.config/nomouse` too. On macOS, also remove
nomouse from **Privacy & Security › Accessibility** and from **Login Items**.

## Building from source

```sh
git clone https://github.com/yuyudhan/nomouse && cd nomouse
cargo build --release -p nomouse     # the Rust toolchain from rust-toolchain.toml
./target/release/nomouse doctor
```

`just app` builds `target/app/nomouse.app` on macOS. The build and test commands are in
[CONTRIBUTING.md](../CONTRIBUTING.md).
