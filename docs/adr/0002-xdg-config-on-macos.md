# ADR 0002: the config file lives in ~/.config/nomouse on macOS too

- Status: accepted
- Date: 2026-11-09

## Context

nomouse first used each OS's native config directory: `$XDG_CONFIG_HOME` (else `~/.config`)
on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows. The owner wants one
place on both Unix-like systems: users of keyboard-driven tools keep their dotfiles in
`~/.config` and sync it across Macs and Linux machines; a path with a space under `~/Library`
is awkward in scripts and dotfile managers.

## Decision

On Linux and macOS the config file is `$XDG_CONFIG_HOME/nomouse/config.toml`, else
`~/.config/nomouse/config.toml`. An empty `XDG_CONFIG_HOME` counts as unset, as the XDG Base
Directory specification says. Windows keeps `%APPDATA%\nomouse\config.toml`. `--config`
overrides the path everywhere. `install.sh` uses the same rule for `--uninstall --purge`.

## Consequences

- One config file works unchanged on a Mac and a Linux machine.
- An existing `~/Library/Application Support/nomouse/config.toml` is no longer read; the
  change is listed in the changelog. No migration code: nomouse has not had a public release.
- Runtime files stay where each OS expects them (`$XDG_RUNTIME_DIR` or a private temp dir for
  the socket); only the config moved.
