# Troubleshooting

Start with `nomouse doctor`. It checks the session, the accessibility API, input, the hotkeys,
the running daemon and the config file, marks each `ok` or `FAIL`, and prints the fix for every
failure. The longer `hint:` paragraph about the system's accessibility switch follows only when
an accessibility check failed.

For more detail, run a mode in the foreground with logs:

```sh
NOMOUSE_LOG=debug nomouse hints --standalone
```

## The install

**`curl: (35) error:0A000126:SSL routines::unexpected eof while reading`**
The network cut the connection during the TLS handshake, usually to GitHub's download host
`release-assets.githubusercontent.com`; on some networks this happens to a third of
connections. Run the one-line install with `--retry 5 --retry-all-errors` as
[install.md](install.md#the-one-line-install) shows: curl retries the cut connection, and the
installer (0.1.2 and later) retries its own downloads the same way.

**`curl: option --retry-all-errors: is unknown`**
Your curl is older than 7.71. Drop `--retry-all-errors` from the command, or install from a
folder with `install.sh --from`.

**`curl: (22)` or `(56) The requested URL returned error`**
The download URL could not be reached, often through a proxy. Download the release assets from
the [releases page](https://github.com/yuyudhan/nomouse/releases) and install from the folder
with `install.sh --from`, as [install.md](install.md#the-one-line-install) shows.

**`checksum mismatch`**
The download was cut off or changed on the way. Nothing was installed; run the installer
again. If it keeps failing, a proxy is rewriting downloads.

**`nomouse: command not found` after installing**
`~/.local/bin` is not on your `PATH`. Add `export PATH="$HOME/.local/bin:$PATH"` to
`~/.zshrc` (macOS) or `~/.profile` (Linux) and open a new terminal.

**`nomouse's Linux build needs glibc 2.35 or later … this system has glibc 2.31`**
The release is built on Ubuntu 22.04 and needs its C library or a newer one (Ubuntu 22.04,
Debian 12, Fedora 36, RHEL 10). Nothing was installed. Upgrade the system or
[build from source](install.md#building-from-source).

**`this system uses musl libc`** (Alpine, Void musl)
The release builds link glibc. [Build from source](install.md#building-from-source).

**`this system has no glibc program loader at /lib64/ld-linux-x86-64.so.2`** (NixOS, Guix)
On NixOS add `programs.nix-ld.enable = true;` to `configuration.nix`, run
`sudo nixos-rebuild switch` and install again; or
[build from source](install.md#building-from-source).

**`nomouse.exe from … does not start`, or `nomouse.exe needs the Microsoft Visual C++
2015-2022 Redistributable (x64)`** (Windows)
Releases up to 0.1.7 need the Visual C++ runtime; 0.1.8 and later link it in and need nothing.
Install the current release, or the runtime from <https://aka.ms/vs/17/release/vc_redist.x64.exe>
and run the installer again. Nothing was installed.

**`nomouse ships an x86_64 build, which Windows on ARM runs under x64 emulation from Windows 11
on`** or **`nomouse ships a 64-bit x86_64 Windows build only`**
Windows 10 on ARM and 32-bit Windows cannot run nomouse. Windows 11 on ARM installs it and
prints `Windows on ARM: nomouse runs under x64 emulation`.

## Building from source

**`cannot install package nomouse …, it requires rustc 1.95 or newer`**
Your Rust is too old. `rustup update stable`, then run the command again. A distribution's own
`rustc` (from apt, dnf) is usually older: install Rust with rustup as
[install.md](install.md#rust-195-or-newer-from-rustup) shows.

**``feature `edition2024` is required``**
The same, with a cargo too old to read nomouse's manifest: install or update Rust with rustup.

**`multiple packages with binaries found: nomouse, nomouse-settings`**
Name the package: `cargo install --locked --git https://github.com/yuyudhan/nomouse nomouse`.

**``linker `cc` not found``**, **`xcrun: error: invalid active developer path`**, or a
`pkg-config` error naming `gtk4` or `libadwaita-1`
A build prerequisite is missing: a C compiler (`build-essential` on Debian and Ubuntu, `gcc` on
Fedora), the Command Line Tools on macOS (`xcode-select --install`), or the GTK 4 and
libadwaita development files for the Linux settings window. The command for each system is in
[install.md](install.md#build-prerequisites).

## macOS

**Nothing happens on `ctrl+space`**
1. Is nomouse running? `pgrep -l nomouse`. If not, `open /Applications/nomouse.app`.
2. Is it allowed? **System Settings › Privacy & Security › Accessibility** (**Device Control
   and Data Access** on macOS 27 and later) must list **nomouse** switched on. Without it,
   macOS delivers no keys to nomouse and no clicks from it.
3. Does another app own `ctrl+space`? macOS uses it for "Select the previous input source" when
   you have more than one keyboard layout (System Settings › Keyboard › Keyboard Shortcuts ›
   Input Sources). Turn that off or pick another hotkey in the
   [config](configuration.md#hotkeys-the-global-shortcuts).
4. Is secure input on? See below.

**nomouse is switched on in Accessibility but still not trusted** (usually right after an
update)
macOS ties the grant to the signature that was allowed. Releases are ad-hoc signed until they
are Developer ID signed, so every update, reinstall or rebuild of another version has a new
signature, and the switch belongs to the old copy; switching it off and on does not help.
Click **Open System Settings** in nomouse's Welcome window or menu: it removes the old entry
and macOS lists the running copy, switched off; switch it on. By hand:

```sh
nomouse quit
tccutil reset Accessibility com.yuyudhan.nomouse
open /Applications/nomouse.app
```

then switch nomouse on in the pane (or select nomouse there, click **−**, then **+** and pick
`/Applications/nomouse.app`). The full steps are in
[install.md](install.md#allowing-nomouse-again-after-an-update-macos).

**`nomouse doctor` says the terminal is not trusted**
Run from a terminal, the doctor checks the terminal's grant, because macOS attributes the
process to the terminal that started it; the line says so (`this terminal (Apple_Terminal) is
not trusted …`). nomouse.app has its own grant: its menu shows it, and **Run doctor…** in the
menu checks the app. Allow the terminal too only if you run nomouse from it.

**The hotkeys stop working while a password field or Terminal's Secure Keyboard Entry is
active**
While secure input is on, macOS sends no keys to other apps, nomouse included. `nomouse
doctor`'s `Secure input` line names the app holding it (`on, held by Terminal (pid 4242)`).
Leave the password field, or turn off **Secure Keyboard Entry** in Terminal's or iTerm2's
menu. The hotkeys work again as soon as it is off.

**"Apple could not verify nomouse is free of malware", "nomouse.app is damaged" or "cannot be
opened because the developer cannot be verified"**
The app was downloaded in a browser (also from the release page **Check for updates…** opens),
which adds a quarantine flag, and the release is not notarized yet. Clear the flag once:
`xattr -dr com.apple.quarantine /Applications/nomouse.app`, or click **Open Anyway** in
System Settings › Privacy & Security. The one-line install sets no flag.

**No labels in Chrome, Edge, Brave, Slack or VS Code**
Chromium and Electron apps build their accessibility tree only when asked. List them in the
config:

```toml
[macos]
enhanced_ui_apps = ["com.google.Chrome", "com.microsoft.VSCode"]
```

## Linux

**Some apps get no labels**
- Qt apps: set `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` in your session environment.
- Chromium, Chrome and Electron apps: start them with `--force-renderer-accessibility`.
- Terminals and games draw their own text and expose no elements; there is nothing to label.

**`doctor` reports the accessibility bus is off**
Run `nomouse setup`, which switches `org.a11y.Status.IsEnabled` on, then restart the apps you
want labels in (they read the switch when they start).

**The hotkey does nothing on X11**
Another program grabbed it; `doctor` names the hotkey that could not be registered. Pick
another in the config.

**No tray icon**
`doctor`'s "tray icon" line says why. GNOME shows tray icons only with the AppIndicator
extension: Ubuntu ships it on (`gnome-extensions list --enabled` lists
`ubuntu-appindicators@ubuntu.com`); elsewhere install "AppIndicator and KStatusNotifierItem
Support". Sway and Hyprland need a bar with a tray (waybar's `tray` module); i3bar shows only
the older XEmbed tray icons, so run `snixembed` to bridge them.
Without a tray, open the settings from the app launcher; the hotkeys work either way.

**Picking nomouse in the launcher shows a notification, not the settings window**
The window is missing or cannot start; the notification says why, and
`$XDG_RUNTIME_DIR/nomouse-settings.log` has its errors. Usually GTK 4 or libadwaita is not
installed: `sudo apt install libgtk-4-1 libadwaita-1-0` (Fedora: `gtk4 libadwaita`, Arch:
`gtk4 libadwaita`). The window needs GTK 4.6 and libadwaita 1.1 or later.

**`label font: no TrueType/OpenType font found`**
nomouse draws its labels with a bold sans font. It looks at the DejaVu, Liberation and Noto
files at their usual paths, then asks fontconfig (`fc-match sans:bold`), then searches `fonts`
under every `$XDG_DATA_DIRS` entry, `~/.local/share/fonts` and `~/.fonts`. The message lists
every folder it searched and the package for your system; DejaVu is enough:

| System | Package |
|--------|---------|
| Debian, Ubuntu | `sudo apt install fonts-dejavu-core` |
| Fedora | `sudo dnf install dejavu-sans-fonts` |
| Arch | `sudo pacman -S ttf-dejavu` |
| openSUSE | `sudo zypper install dejavu-fonts` |
| Alpine | `sudo apk add font-dejavu` |
| Void | `sudo xbps-install dejavu-fonts-ttf` |
| Gentoo | `sudo emerge media-fonts/dejavu` |
| NixOS | `pkgs.dejavu_fonts` in `fonts.packages` |

A `.ttf` or `.otf` copied into `~/.local/share/fonts` works too.

**Start at login does nothing on Sway, Hyprland or i3**
Those compositors do not read login autostart entries
(`~/.config/autostart/nomouse.desktop`). The settings window's Start at login row and the tray
show the line to add instead: `exec nomouse daemon` in `~/.config/sway/config`,
`exec-once = nomouse daemon` in `~/.config/hypr/hyprland.conf`, or
`exec --no-startup-id nomouse daemon` in `~/.config/i3/config`.

## Wayland

**`ctrl+space` does nothing**
Expected: Wayland programs cannot grab global keys. Bind `nomouse hints` in your compositor; see
[install.md](install.md#linux-wayland).

**GNOME says nomouse is not supported**
GNOME's Wayland compositor offers no overlay layer, no virtual pointer and no hotkeys to
programs. Log in with **GNOME on Xorg** (**Ubuntu on Xorg** on Ubuntu; the gear on the login
screen). The choice sticks for later logins.

**KDE Plasma: labels appear but clicks do nothing**
KWin offers no virtual pointer, so nomouse clicks through `/dev/uinput`. Give your user access
with the udev rule in [install.md](install.md#linux-wayland) and log in again.

**Labels sit in the wrong place in a native Wayland app**
nomouse asks the compositor where the window is (Sway and Hyprland tell it; KWin does not).
Apps running on XWayland always place correctly. A native app's popup menus, and all but the
focused window of an app with several windows open, can be off.

## The daemon

**`daemon: the running daemon (pid 4242) is nomouse 0.1.7, this command is nomouse 0.1.8`**
A daemon started before an update still runs the old version. Restart it with the command the
line ends with: `nomouse quit && open -a nomouse` on macOS, `nomouse quit && nomouse launch` on
Linux.

**`daemon: a daemon … did not report its version`**
A daemon from 0.1.7 or older, which cannot answer the question yet. Restart it the same way.

**`daemon: a daemon (pid 4242) holds the single-instance lock but does not answer on the
command socket`**
It is still in its setup (on macOS, the Welcome window waiting for Accessibility: finish it)
or stuck. Stop it with `kill 4242` and start it again as the line says.

**`daemon: not running`**
Start it: `open -a nomouse` on macOS, `nomouse launch` (or pick nomouse in the app launcher)
on Linux.

**`flavour: dev build, config in ~/.config/nomouse-dev`**, or my settings and hotkeys are
different
That `nomouse` is a development build (`cargo run`, `just dev`, "nomouse Dev.app"), which has
its own config, hotkeys (`ctrl+alt+shift+space` and so on) and, on macOS, its own
Accessibility entry. The installed nomouse says `flavour: release build, config in
~/.config/nomouse`. See [CONTRIBUTING.md](../CONTRIBUTING.md#development-builds).

## The config

**My change does nothing**
`nomouse config check` validates the file and names the first bad key. The daemon keeps its
previous settings when the new file is invalid, and logs why with `NOMOUSE_LOG=info`.

**Two modes cannot share a hotkey**
Each of `hotkeys.hints`, `continuous`, `scroll` and `search` needs its own chord.

## Reporting a bug

Open an issue at <https://github.com/yuyudhan/nomouse/issues> with the output of
`nomouse --version` and `nomouse doctor`, the app you were in, and the log of
`NOMOUSE_LOG=debug nomouse hints --standalone`.
