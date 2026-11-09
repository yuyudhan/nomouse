# Contributing

Work lands on `nightly` (the default branch). CI runs on Ubuntu, macOS and Windows for every push
and pull request to it.

## Setup

    rustup show                       # installs the pinned toolchain and targets
    cargo install just taplo-cli typos-cli cargo-deny --locked
    # plus shellcheck (apt install shellcheck / brew install shellcheck)
    just install-hooks                # pre-commit, commit-msg and pre-push hooks; safe to rerun

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
| `just check` | format check, guards, typos, shellcheck, clippy `-D warnings` for `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`, `aarch64-apple-darwin` |
| `just test` | all unit, integration, property and backend-contract tests, the check that `docs/configuration.md` lists every config key with its default, and `scripts/test-installer.sh` (`install.sh` against a fake release for this OS, from a folder and through a fake GitHub API) |
| `just e2e` | Linux X11 end to end on a private Xvfb/i3/AT-SPI session: GTK, Chromium, Qt, the real-backend contract |
| `just coverage` | line coverage of `nomouse-core` and the binary's non-OS logic; fails below `scripts/coverage-floors.txt` |
| `just deny` | cargo-deny: advisories, licences, duplicate versions, sources |
| `just verify` | check, test, deny |
| `just release-dry-run` | every Linux release asset into `target/dist/`, nothing published, then `just test-install` |
| `just test-install` | `install.sh` end to end against `target/dist/` in an `env -i` sandbox (temp HOME, PATH, TERM only); it asserts the sandbox and that decoy `XDG_CONFIG_HOME`/`NOMOUSE_INSTALL_DIR` stay untouched |
| `just release X.Y.Z` | cut a release (see below); only on the owner's word |

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
| Linux (ubuntu-22.04, glibc 2.35) | `nomouse-x86_64-unknown-linux-gnu.tar.gz`, `nomouse-aarch64-unknown-linux-gnu.tar.gz` (cross-linked with gcc-aarch64-linux-gnu), `nomouse_amd64.deb`, `nomouse_arm64.deb` (cargo-deb) |
| macOS | arm64 and x86_64 joined with `lipo` into `nomouse-universal-apple-darwin.tar.gz`; the same binary in `nomouse.app` (`scripts/release/app.sh`) as `nomouse-universal-apple-darwin.app.zip` and in the disk image `nomouse-universal-apple-darwin.dmg` (`scripts/release/dmg.sh`); `scripts/test-app.sh` checks the bundle, the image and the cask |
| Windows | `nomouse-x86_64-pc-windows-msvc.zip` |
| Assemble | `install.sh`, `install.ps1`, `nomouse.rb` (Homebrew formula from `packaging/homebrew/`), `nomouse-app.rb` (the cask, when the disk image is there), a `.sha256` per asset and `SHA256SUMS`; tests `install.sh` against the built archive, then creates the GitHub release with the changelog section as notes |

Asset names carry no version, so `releases/latest/download/<asset>` always resolves. Each build
job smoke-tests `nomouse --version` against the workspace version, and a tag that does not match
it fails the run.

Running the workflow by hand is a dry run: `gh workflow run release.yml --ref nightly` builds
and packs every asset and uploads them as the `release-X.Y.Z` workflow artifact, with no tag and
no release. `just release-dry-run` does the Linux half locally (needs `cargo-deb`, and for
aarch64 either `aarch64-linux-gnu-gcc` or `cargo-zigbuild` with zig).

Homebrew: `nomouse.rb` (the formula, the bare command line) and `nomouse-app.rb` (the cask,
`nomouse.app` from the disk image) are attached to every release; publishing them means copying
them to the tap repository `yuyudhan/homebrew-tap` as `Formula/nomouse.rb` and
`Casks/nomouse.rb`. Homebrew reads casks only from a tap, so check one there: `brew style
--cask` and `brew audit --cask --strict` on `<tap>/nomouse`, then `brew install --cask` and
`brew uninstall --cask`.

### macOS signing and notarization

Without signing secrets the macOS binary, `nomouse.app` and the disk image carry ad-hoc
signatures only. That is enough for the installer: curl sets no quarantine flag, so Gatekeeper
does not stop it. A download through a browser is quarantined and blocked, and macOS ties the
Accessibility grant to the code signature: an ad-hoc signature's designated requirement is its
code hash, so the grant survives rebuilding the same binary (`scripts/test-app.sh` checks that)
but not an update.

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
