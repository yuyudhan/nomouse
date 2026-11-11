# Contributing

Work lands on `nightly` (the default branch). CI runs on Ubuntu, macOS and Windows for every push
and pull request to it.

## Setup

    rustup toolchain install          # the toolchain and targets pinned in rust-toolchain.toml
    cargo install just taplo-cli typos-cli cargo-deny --locked
    # plus shellcheck (apt install shellcheck / brew install shellcheck)
    just install-hooks                # pre-commit, commit-msg and pre-push hooks; safe to rerun

`rust-toolchain.toml` pins the toolchain CI uses and the standard libraries of the three
targets `just check` lints (`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`,
`aarch64-apple-darwin`). rustup 1.28 and later install it with `rustup toolchain install`
(`rustup show` only reports it). The declared minimum, `rust-version` in `Cargo.toml`, is 1.95:
kbvm, the Linux keymap reader, needs it. CI's `msrv` job checks the workspace with exactly that
version on Linux, macOS and Windows; raise both together when a dependency needs more. Building
prerequisites per OS are in [docs/install.md](docs/install.md#build-prerequisites).

## Development builds

A development build runs beside the installed nomouse, so you can work on nomouse while using
it. The flavour is fixed at build time: `NOMOUSE_FLAVOUR=dev` or `release`, else debug builds
are dev and `--release` builds are release (`crates/nomouse-core/src/flavour.rs`).

| | Release | Dev |
|---|---|---|
| Config | `~/.config/nomouse/` | `~/.config/nomouse-dev/` |
| Socket, lock, log | `nomouse.sock`, `.lock`, `.log` | `nomouse-dev.sock`, `.lock`, `.log` |
| macOS app | `nomouse.app`, `com.yuyudhan.nomouse` | `nomouse Dev.app`, `com.yuyudhan.nomouse.dev`, with its own Accessibility grant and login item |
| Linux ids | `io.github.yuyudhan.nomouse`, autostart `nomouse.desktop` | `io.github.yuyudhan.nomouse.dev`, autostart `nomouse-dev.desktop` |
| Default hotkeys | `ctrl+space`, `ctrl+shift+space`, `ctrl+alt+space`, `ctrl+alt+/` | `ctrl+alt+shift+space`, `ctrl+alt+shift+.`, `ctrl+alt+shift+,`, `ctrl+alt+shift+/` |
| `--version` | `nomouse 0.1.7` | `nomouse 0.1.7 (dev)` |

- `just dev <args>` runs a debug (dev) build, e.g. `just dev doctor`; with no arguments it runs
  the daemon. `nomouse doctor`'s first line names the flavour and its config directory
  (`flavour: dev build, config in ~/.config/nomouse-dev`).
- `just dev-config` installs `config/dev.example.toml` as the dev config: every key at its dev
  default, with comments. It keeps an existing file unless `--force`, which backs it up as
  `config.toml.bak-<time>` first. It never touches `~/.config/nomouse/`.
- `just app` builds an optimised **nomouse Dev.app** in `target/app/`: open it once and allow
  *nomouse Dev* in Accessibility. Rebuilding it never touches the installed `nomouse.app`'s
  grant. `just test-app-dev` checks that bundle; `just test-app` checks the release one.
- `just e2e-macos --bin <binary>` tests a development build as well as a release one.
- A dev build looks for `nomouse-settings` and `nomouse` only beside itself, never on `PATH`.
- The release `nomouse.app` comes only from `scripts/release/build.sh` and `app.sh`, which pin
  `NOMOUSE_FLAVOUR=release`. Release paths and ids never change: users' configs, Accessibility
  grants, login items and autostart entries depend on them.

## Git hooks

`just install-hooks` points `core.hooksPath` at `.githooks/`:

| Hook | Runs |
|------|------|
| pre-commit | `just pre-commit`: format check (rustfmt, taplo), line-count and unsafe guards, typos, shellcheck, host clippy |
| commit-msg | subject is `<gitmoji> [scope: ]summary`, one line of at most 100 characters, no trailing period, a blank line before any body, no tool or assistant credits |
| pre-push | `just verify` on every pushed commit: everything above, clippy for all three OS targets, tests, cargo-deny |

The pre-commit and pre-push hooks check exactly what is committed, not the working tree:
`scripts/hooks/in-snapshot.sh` exports the staged index (or the pushed commit) to
`target/hook-tree/<hook>/` and runs the checks there, building into `target/hook-target/<hook>/`
(never the main `target/`, whose artifacts it would otherwise overwrite). So
someone else's uncommitted files can neither break nor sneak past your commit. The snapshot is
updated in place, so repeated runs rebuild only what changed.

## Commands

| Command | Does |
|---------|------|
| `just fmt` | `cargo fmt` and `taplo fmt` in place |
| `just pre-commit` | the pre-commit subset: format check, guards, typos, shellcheck, host clippy |
| `just check` | format check, guards, typos, shellcheck, clippy `-D warnings` for `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`, `aarch64-apple-darwin` (from macOS or Windows the Linux target skips `nomouse-settings`, whose GTK build scripts need Linux's libraries; Linux hosts and CI lint it) |
| `just test` | all unit, integration, property and backend-contract tests, the check that `docs/configuration.md` lists every config key with its default, `scripts/test-installer.sh` (`install.sh` in `sh` against a fake release for this OS, from a folder, through a fake GitHub API, and through the documented `curl … \| sh` one-liner piped into `sh`, `dash`, `bash` and `zsh` from an empty folder, which must leave that folder and `TMPDIR` empty after an install, a 404, a checksum mismatch and Ctrl-C/HUP/TERM mid-download; colour only on a terminal), `scripts/test-install-command.sh` (the documented one-liner against a local server in each shell: a 404 and an `install.sh` cut off mid-download run nothing, and nothing is left behind) and `scripts/test-dev-config.sh` (`just dev-config` in a throwaway HOME) |
| `just e2e` | Linux X11 end to end on a private Xvfb/i3/AT-SPI session: GTK, Chromium, Qt, the real-backend contract |
| `just coverage` | line coverage of `nomouse-core` and the binary's non-OS logic; fails below `scripts/coverage-floors.txt` |
| `just deny` | cargo-deny: advisories, licences, duplicate versions, sources |
| `just verify` | check, test, deny |
| `just release-dry-run` | every Linux release asset into `target/dist/`, nothing published, then `just test-install` |
| `just test-install` | `install.sh` end to end against `target/dist/` in an `env -i` sandbox (temp HOME, PATH, TERM only); it asserts the sandbox and that decoy `XDG_CONFIG_HOME`/`NOMOUSE_INSTALL_DIR` stay untouched |
| `just release X.Y.Z` | cut a release (see below); only on the owner's word |
| `just dev`, `just dev-config`, `just app`, `just test-app-dev` | development builds, see [above](#development-builds) |

## Rules

- **No unsafe code** outside `nomouse-sys-macos` and `nomouse-sys-windows`. Every other crate root
  has `#![forbid(unsafe_code)]`; `scripts/check-unsafe.sh` enforces both. In the sys crates every
  `unsafe` block has a `// SAFETY:` comment and every exported item is safe to call.
- **Files stay under 3000 lines** (`scripts/check-line-count.sh`). Split modules before then.
- **4-space indentation everywhere**, no tabs: Rust (rustfmt.toml), TOML (taplo), YAML, Markdown
  and scripts (.editorconfig).
- **Lints** (workspace `[lints]`): clippy pedantic; `unwrap_used`, `expect_used` and `panic` denied
  outside tests; `undocumented_unsafe_blocks`; `missing_docs` on every public item; `todo!` and
  `unimplemented!` denied. Fix the code instead of adding `#[allow]`.
- **Tests** accompany every behaviour change; property tests (proptest) for invariants. The layers,
  the backend contract every `Platform` must pass, the e2e rules and the coverage floors are in
  [docs/testing.md](docs/testing.md).
- **Platform code** lives in its platform crate and compiles on every OS (OS-specific modules are
  `cfg`-gated), so `cargo clippy --target` works from any machine.
- **Dependencies** must pass cargo-deny: permissive licences only, one version per crate.

## Releases

The version lives in one place, `version` under `[workspace.package]` in the root `Cargo.toml`;
every crate inherits it. Changes are listed under `## [Unreleased]` in `CHANGELOG.md` as they
land.

`just release X.Y.Z` (`scripts/release/release.sh`), on a clean `nightly` that is not behind
origin: sets the version, refreshes `Cargo.lock`, dates the Unreleased changelog section, runs
`just verify`, commits `🔖 Release vX.Y.Z`, tags `vX.Y.Z` and pushes both. It publishes a
release, so it runs only when the owner says so. Any failure before the commit restores the
files.

The tag push runs `.github/workflows/release.yml`:

| Job | Builds |
|-----|--------|
| Linux (ubuntu-22.04, glibc 2.35) | `nomouse-x86_64-unknown-linux-gnu.tar.gz`, `nomouse-aarch64-unknown-linux-gnu.tar.gz` (built on an arm64 runner: the settings window links GTK 4), `nomouse_amd64.deb`, `nomouse_arm64.deb` (cargo-deb) |
| macOS | arm64 and x86_64 built for macOS 11.0 (`MACOSX_DEPLOYMENT_TARGET`, minos checked), joined with `lipo` into `nomouse-universal-apple-darwin.tar.gz`; the same binary in `nomouse.app` (`scripts/release/app.sh`) as `nomouse-universal-apple-darwin.app.zip` and in the disk image `nomouse-universal-apple-darwin.dmg` (`scripts/release/dmg.sh`); `scripts/test-app.sh` checks the bundle, the image and the cask; `scripts/release/signature.sh` reports `developer-id` or `ad-hoc` |
| macOS smoke | the universal tarball (`--version`, `doctor`) and the `.app.zip` bundle (codesign verify, `--version`) on `macos-15` (arm64) and `macos-15-intel` |
| Windows | `nomouse-x86_64-pc-windows-msvc.zip`, with the C runtime linked statically (`+crt-static`; `build.sh` fails if the exe still imports `VCRUNTIME140.dll`) |
| Assemble | `install.sh`, `install.ps1`, `nomouse.rb` (Homebrew formula from `packaging/homebrew/`), `nomouse-app.rb` (the cask, when the disk image is there), a `.sha256` per asset and `SHA256SUMS`; tests `install.sh` against the built archive, then creates the GitHub release with the changelog section as notes |
| Homebrew | on a tag, `scripts/release/publish-homebrew.sh` writes `Formula/nomouse.rb` and `Casks/nomouse.rb` to `yuyudhan/homebrew-tap`, checks them (`brew style`, `brew audit --strict --online`, installs and uninstalls both) and pushes; a dry run publishes to a local copy of the tap instead |

Asset names carry no version, so `releases/latest/download/<asset>` always resolves. Each build
job smoke-tests `nomouse --version` against the workspace version, and a tag that does not match
it fails the run.

Running the workflow by hand is a dry run: `gh workflow run release.yml --ref nightly` builds
and packs every asset and uploads them as the `release-X.Y.Z` workflow artifact, with no tag and
no release. `just release-dry-run` does this machine's Linux architecture locally (needs
`cargo-deb`, `libgtk-4-dev` and `libadwaita-1-dev`).

Homebrew: `nomouse.rb` (the formula, the bare command line) and `nomouse-app.rb` (the cask,
`nomouse.app` from the disk image) are attached to every release, and the `homebrew` job
publishes them to the tap `yuyudhan/homebrew-tap` as `Formula/nomouse.rb` and
`Casks/nomouse.rb`. It needs the repository secret `HOMEBREW_TAP_TOKEN`, a fine-grained token
with Contents read and write on `yuyudhan/homebrew-tap`; without it the job skips with a notice
saying so and ends green, and the GitHub release itself is unaffected. From a machine where `gh` is logged in,
`scripts/release/publish-homebrew.sh X.Y.Z <release assets dir>` publishes the same files (add
`--no-install` where nomouse is installed and running: the cask's install test would quit it).
Homebrew 7 loads casks only from a trusted tap, and the formula and the cask share the name
`nomouse`, so a check by hand is:

    brew tap yuyudhan/tap
    brew trust --tap yuyudhan/tap
    brew install --cask yuyudhan/tap/nomouse      # nomouse.app
    brew install --formula yuyudhan/tap/nomouse   # the command line only

While releases are ad-hoc signed, the cask clears the quarantine flag, and macOS asks for
Accessibility again after each `brew upgrade`.

### macOS signing and notarization

Without signing secrets the macOS binary, `nomouse.app` and the disk image carry ad-hoc
signatures only. That is enough for the installer: curl sets no quarantine flag, so Gatekeeper
does not stop it. A download through a browser is quarantined and blocked, and macOS ties the
Accessibility grant to the code signature: an ad-hoc signature's designated requirement is its
code hash, so the grant survives rebuilding the same binary (`scripts/test-app.sh` checks that)
but not an update. Until the secrets below exist, every release run posts an "ad-hoc signed
release" warning annotation, the installer resets the stale Accessibility entry on update, and
users switch nomouse on again after each update.

Developer ID signing needs the owner's Apple Developer Program membership and these repository
secrets:

| Secret | Value |
|--------|-------|
| `MACOS_CERTIFICATE` | base64 of the "Developer ID Application" certificate with its private key (`.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | the `.p12` password |
| `MACOS_SIGN_IDENTITY` | the identity, `Developer ID Application: <name> (<team id>)` |
| `NOTARY_KEY` | base64 of an App Store Connect API key (`AuthKey_<id>.p8`, Developer role) |
| `NOTARY_KEY_ID`, `NOTARY_ISSUER` | that key's id and the team's issuer id |

With them the macOS job (`scripts/release/signing.sh`) imports the certificate into a temporary
keychain; `app.sh` signs `nomouse.app` with the hardened runtime and a secure timestamp, then
`scripts/release/notarize.sh` notarizes and staples it before it is zipped; `dmg.sh` signs,
notarizes and staples the disk image; and the job fails unless
`spctl --assess --type open --context context:primary-signature` accepts the image. A manual
run (`gh workflow run release.yml --ref nightly`) exercises all of it without a release. The
bare binary in the tarball stays ad-hoc signed (a bare binary cannot be stapled).
`signing.sh` refuses a `MACOS_SIGN_IDENTITY` that is not `Developer ID Application: …`, and with
one `signature.sh` requires a team-based designated requirement (`anchor apple generic` and
`certificate leaf[subject.OU]`), which is what keeps the grant across updates.
