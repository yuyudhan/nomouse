# Installing nomouse

- [The one-line install](#the-one-line-install)
- [Requirements](#requirements)
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
curl -fsSLO --retry 5 --retry-all-errors https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh && bash install.sh
```

It saves `install.sh` in the current folder and runs it only when the download worked. Download
first rather than piping curl into bash: a failed download piped into bash (a 404, a cut
connection) runs an empty script and exits 0, so it looks like success with nothing installed.
Saved to a file, a failed download stops the command with curl's error.

The installer needs no sudo. It picks the build for your OS and CPU, downloads it from the latest
[GitHub release](https://github.com/yuyudhan/nomouse/releases), checks it against the
release's `.sha256` and refuses to install on a mismatch, then prints the next steps for your
system. `--retry-all-errors` makes curl retry a connection the network cuts; the installer's
own downloads retry the same way. curl older than 7.71 (Ubuntu 20.04, RHEL 8) does not know it:
drop `--retry-all-errors` there.

| System | What you get |
|--------|--------------|
| macOS 11+, Apple silicon and Intel | `nomouse.app` in `/Applications` (`~/Applications` when `/Applications` is not writable), and a `nomouse` command in `~/.local/bin` that runs the app's binary |
| Linux x86_64 and aarch64, glibc 2.35+ | the `nomouse` and `nomouse-settings` (the settings window) binaries in `~/.local/bin`, and nomouse in the app launcher: the desktop entry `io.github.yuyudhan.nomouse.desktop` in `~/.local/share/applications` with its icons, and the tray icons, in `~/.local/share/icons/hicolor` |

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

## Requirements

| System | Needs |
|--------|-------|
| macOS | macOS 11 or later, Apple silicon or Intel (one universal build). Every release runs it on macOS 15 on Apple silicon and on Intel; macOS 11 to 14 are built for but not tested |
| Linux | x86_64 or aarch64 with glibc 2.35 or later: Ubuntu 22.04, Debian 12, Fedora 36, RHEL 10 or newer. X11, or Wayland on Sway, Hyprland or KDE Plasma ([GNOME on Wayland is not supported](#linux-wayland)) |
| Windows | Windows 10 or 11 on x86_64, or Windows 11 on ARM (it runs the x86_64 build under x64 emulation). No Visual C++ runtime needed |

The Linux installer checks the C library before it downloads anything. It stops, and points
at [Building from source](#building-from-source), on:

- glibc older than 2.35 (Ubuntu 20.04, Debian 11, RHEL 8 and 9, Amazon Linux 2);
- musl systems (Alpine, Void musl);
- NixOS and Guix, which keep no glibc loader at `/lib64/ld-linux-x86-64.so.2`
  (`/lib/ld-linux-aarch64.so.1` on aarch64). On NixOS, turning on nix-ld
  (`programs.nix-ld.enable = true;` in `configuration.nix`, then `sudo nixos-rebuild switch`)
  provides that loader and lets the installer go ahead; if the installer's test run of
  `nomouse --version` then names a missing library, add it to `programs.nix-ld.libraries`.

It also runs the downloaded `nomouse --version` once before it replaces anything, so a build
that cannot start on your system fails the install with the loader's error and leaves the old
copy as it was.

Not supported: Windows 10 on ARM (it has no x64 emulation), 32-bit Windows, 32-bit ARM and
other CPUs on Linux. Build from source there if Rust supports the system.

## macOS

1. Run the one-line install above.
2. Open the app: `open /Applications/nomouse.app`. It has no Dock icon; its keycap icon sits
   in the menu bar, and it listens for the hotkey.
3. Grant Accessibility. nomouse reads buttons and links through the macOS accessibility API and
   clicks through it, which needs one permission:
   1. On its first start a Welcome window explains this; **Open System Settings** opens
      **System Settings › Privacy & Security › Accessibility**. macOS 27 and later call this
      pane **Device Control and Data Access**; it is the same list.
   2. Switch on **nomouse** in the list (unlock with your password if asked).
   3. nomouse notices the switch within a second and restarts by itself. No logout needed.

   Input Monitoring and Screen Recording are not needed. After every update macOS asks for
   this once more, see [Updating](#updating).
4. Press `ctrl+space` in any app. Labels appear; type one to click.
5. Start at login: **Start at login** in the menu bar icon's menu (or on the settings
   window's General page); it appears under **System Settings › General › Login Items**.

The release builds are ad-hoc signed, not notarized by Apple yet. The one-line install
removes the quarantine flag, so Gatekeeper does not block the first start. If you downloaded
`nomouse-universal-apple-darwin.app.zip` or the `.dmg` in a browser instead, macOS says
"Apple could not verify nomouse is free of malware" (older versions: "nomouse.app is damaged"
or "the developer cannot be verified"). Clear the flag once:

```sh
xattr -dr com.apple.quarantine /Applications/nomouse.app
```

or open it once, then click **Open Anyway** in System Settings › Privacy & Security (macOS 15
and later have no right-click › Open for this). The one-line install is the easier way.

You can run `nomouse setup` in a terminal instead of opening the app: it walks through the same
permission. A permission granted to the app belongs to the app; when you run the `nomouse`
command from a terminal, macOS asks on behalf of the terminal instead. So `nomouse doctor` run
in Terminal checks the terminal's grant and says so ("this terminal (Apple_Terminal) is not
trusted … nomouse.app has its own grant"); the app's own status is in its menu, and **Run
doctor…** there checks the app.

## Linux X11

1. Run the one-line install above.
2. `nomouse setup` turns on the accessibility bus (`org.a11y.Status.IsEnabled`) if your desktop
   left it off and explains anything else that is missing.
3. Open nomouse from the app launcher: press Super, type `nomouse` and pick it. That starts
   the daemon unless it is running already and opens the settings window; `ctrl+space` shows
   the labels. (`nomouse` in a terminal starts the daemon too.) Right-click the entry for
   **nomouse doctor**, which opens the doctor's report in a terminal.
4. Start at login: the settings window's **General › Start at login**, or the tray menu's
   **Start at login** (an XDG autostart entry, `~/.config/autostart/nomouse.desktop`). Desktops
   such as GNOME, KDE, Xfce, Cinnamon and MATE read that entry; Sway, Hyprland and i3 do not,
   so there it takes one line in the compositor's config (the settings window and the tray
   show the line with the full path of your `nomouse`):

   | Compositor | Config file | Line |
   |------------|-------------|------|
   | Sway | `~/.config/sway/config` | `exec nomouse daemon` |
   | Hyprland | `~/.config/hypr/hyprland.conf` | `exec-once = nomouse daemon` |
   | i3 | `~/.config/i3/config` | `exec --no-startup-id nomouse daemon` |

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

- **Windows 10 and 11 on x86_64**: supported. `nomouse.exe` links the C runtime statically
  (0.1.8 and later), so it needs no Visual C++ Redistributable.
- **Windows 11 on ARM**: the installer installs the x86_64 build and says
  `Windows on ARM: nomouse runs under x64 emulation`.
- **Windows 10 on ARM and 32-bit Windows**: refused by the installer; Windows 10 on ARM cannot
  run x64 programs.

The installer runs the new `nomouse.exe --version` before it replaces an existing install, so a
build that cannot start leaves the old one in place.

## Installer options

Options go after the script, once it is downloaded:

```sh
curl -fsSLO https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh && bash install.sh --version 0.1.0
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

Run the one-line install again. Your config is never touched.

- **macOS**: the installer quits the running nomouse, replaces the app and starts the new
  one. Releases are not Developer ID signed yet, so macOS ties the Accessibility grant to each
  version's own signature: the installer removes the old entry, and you switch nomouse on once
  more (the Welcome window appears and opens the pane; see the steps below).
- **Linux**: the installer replaces the binaries; a running daemon keeps working until you
  restart it with `nomouse quit && nomouse launch`.

`nomouse doctor`'s `daemon` line shows what runs: `running (pid 4242, nomouse 0.1.8)`, or a
FAIL with the restart command while a daemon from before the update still runs.

The menu's **Check for updates…** says when a newer release is out. On macOS its window shows
the one-line install and **Copy Install Command** puts it on the clipboard: paste it into
Terminal. **Open Release Page** opens the release, but install with the one-line install rather
than the `.dmg` or `.app.zip` there, which a browser quarantines (see [macOS](#macos)).

### Allowing nomouse again after an update (macOS)

Until releases are signed with an Apple Developer ID, every new version needs the grant once
more, also after installing a source build of `nomouse.app` of another version:

1. Open the new app: `open /Applications/nomouse.app` (the installer already did if nomouse
   was running). The Welcome window, or the dimmed menu bar icon's
   **Accessibility: not allowed. Allow…**, offers **Open System Settings**.
2. Click it. nomouse removes the old entry, macOS adds the new copy switched off, and the pane
   opens: **System Settings › Privacy & Security › Accessibility** (macOS 27 and later:
   **Device Control and Data Access**).
3. Switch **nomouse** on. nomouse restarts by itself within a second or two.

If the list still shows nomouse switched on and nomouse keeps waiting, that switch belongs to
the old version. Remove it by hand in Terminal, then start the app and switch it on again:

```sh
nomouse quit
tccutil reset Accessibility com.yuyudhan.nomouse
open /Applications/nomouse.app
```

Or, in the pane, select nomouse, click **−**, then **+** and pick `/Applications/nomouse.app`.

## Uninstalling

```sh
curl -fsSLO https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh && bash install.sh --uninstall
```

This removes `~/.local/bin/nomouse`, on Linux `nomouse-settings`, the app launcher entry and its
icons, the tray icons and the start-at-login entry, and on macOS
`nomouse.app` (only when it is nomouse's bundle, `com.yuyudhan.nomouse`). Add `--purge` to
delete `~/.config/nomouse` too. On macOS, also remove nomouse from **Login Items** and its
Accessibility entry, in the pane or with `tccutil reset Accessibility com.yuyudhan.nomouse`.

## Building from source

For systems the release builds do not cover (older glibc, musl, NixOS without nix-ld, other
CPUs) or to run the latest `nightly`. The one-line install is easier where it works: a source
build has no app launcher entry or icons on Linux and, from `cargo install`, no `nomouse.app`
on macOS.

### Rust 1.95 or newer, from rustup

nomouse needs Rust 1.95 or newer. Distribution packages of Rust are often older (check with
`rustc --version`); they fail with `nomouse requires rustc 1.95` or
``feature `edition2024` is required``. Use [rustup](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o rustup-init.sh
sh rustup-init.sh -y
. "$HOME/.cargo/env"
```

Already have rustup? Update first:

```sh
rustup update stable
```

### Build prerequisites

A C compiler and linker, and on Linux the GTK 4 and libadwaita development files for the
settings window (`nomouse-settings`):

| System | Command |
|--------|---------|
| Debian, Ubuntu | `sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel` |
| Arch | `sudo pacman -S --needed base-devel gtk4 libadwaita` |
| openSUSE | `sudo zypper install gcc pkgconf gtk4-devel libadwaita-devel` |
| macOS | `xcode-select --install` (the Command Line Tools) |
| Windows | the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with "Desktop development with C++", which rustup offers to install |

Without them the build stops with ``linker `cc` not found`` (no C compiler),
`xcrun: error: invalid active developer path` (no Command Line Tools on macOS) or a
`pkg-config` error naming `gtk4` or `libadwaita-1` (Linux, the settings window).

### With `cargo install`

```sh
cargo install --locked --git https://github.com/yuyudhan/nomouse nomouse
```

On Linux, add the settings window:

```sh
cargo install --locked --git https://github.com/yuyudhan/nomouse nomouse nomouse-settings
```

- `--locked` builds with the repository's `Cargo.lock`. Without it cargo picks the newest
  version of every dependency, which may need a newer Rust than yours.
- The package name is required: the repository has two programs, and without one cargo stops
  with `multiple packages with binaries found: nomouse, nomouse-settings`.
- `cargo install` builds with your default toolchain (the repository's pinned one applies only
  inside a clone). Below 1.95 it stops at once with `cannot install package nomouse …, it
  requires rustc 1.95 or newer`; run `rustup update stable`.
- It builds the tip of `nightly`. For a release, add its tag, for example `--tag v0.1.7`
  (every tag is on the [releases](https://github.com/yuyudhan/nomouse/releases) page).
- It installs to `~/.cargo/bin`. On Linux run `nomouse setup` next, as for the
  [X11](#linux-x11) or [Wayland](#linux-wayland) install. On macOS this is the bare command
  without the menu bar app, so the Accessibility grant goes to the terminal you run it from;
  build the app as below instead.

### From a clone

```sh
git clone https://github.com/yuyudhan/nomouse && cd nomouse
rustup toolchain install
```

`rustup toolchain install` installs the toolchain pinned in `rust-toolchain.toml` (rustup 1.28
and later no longer do that in `rustup show`), with the standard libraries for the cross
targets `just check` lints. To skip that download, build with your own stable Rust (1.95 or
newer) by adding `+stable` after `cargo`:
`cargo +stable build --release --locked -p nomouse`.

Linux, with the app launcher entry, icons and settings window installed as the one-line install
does, from the build:

```sh
cargo build --release --locked -p nomouse -p nomouse-settings
target="$(rustc -vV | sed -n 's/^host: //p')"
scripts/release/package.sh "$target" target/release/nomouse target/dist
bash scripts/install.sh --from target/dist
```

macOS, as `nomouse.app` in `/Applications` (an update over an installed copy needs the grant
again, see [Updating](#updating)). `NOMOUSE_FLAVOUR=release` makes it the release build; without
it `app.sh` gets whatever flavour the build was:

```sh
NOMOUSE_FLAVOUR=release cargo build --release --locked -p nomouse
scripts/release/app.sh target/release/nomouse target/release-app
bash scripts/install.sh --from target/release-app
```

`just app` is for working on nomouse: it builds **nomouse Dev.app**
(`com.yuyudhan.nomouse.dev`) in `target/app/`, a development build that runs beside the
installed nomouse with its own config, Accessibility grant and hotkeys
([CONTRIBUTING.md](../CONTRIBUTING.md#development-builds)).

Windows: `cargo build --release --locked -p nomouse`, then run `target\release\nomouse.exe`.

Check the result with `nomouse doctor`. The build and test commands are in
[CONTRIBUTING.md](../CONTRIBUTING.md).
