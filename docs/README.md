# nomouse documentation

## Using nomouse

| Page | What is in it |
|------|---------------|
| [install.md](install.md) | installing on macOS, Linux (X11 and each Wayland compositor) and Windows; the permissions step by step; updating and uninstalling |
| [usage.md](usage.md) | every mode and every key: hints, other clicks, modifier clicks, continuous, scroll, search, the commands |
| [configuration.md](configuration.md) | every config key with its default, checked against the code by a test |
| [troubleshooting.md](troubleshooting.md) | when labels, clicks or hotkeys do not work, by platform |
| [platform-support.md](platform-support.md) | what runs on macOS, X11 and each Wayland compositor, and how each is tested |

## How it is built

| Page | What is in it |
|------|---------------|
| [features.md](features.md) | every feature with its status on each OS, compared with Homerow |
| [architecture.md](architecture.md) | the crates, the platform traits, the activation loop, the config schema |
| [wayland.md](wayland.md) | the Wayland backend: protocols, hotkeys, coordinates, compositor limits |
| [testing.md](testing.md) | the test layers and how to run each |
| [plan.md](plan.md) | the slices still to build, each with its acceptance test |
| [roadmap.md](roadmap.md) | the milestones |
| [adr/](adr/) | architecture decisions: [platform crates behind traits](adr/0001-platform-crates-behind-traits.md), [XDG config on macOS](adr/0002-xdg-config-on-macos.md), [licence and identity](adr/0003-licence-and-identity.md) |
