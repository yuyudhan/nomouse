# ADR 0003: MIT OR Apache-2.0, and the name nomouse with bundle id com.yuyudhan.nomouse

- Status: accepted
- Date: 2026-11-09

## Context

Until now the repository had no licence (`LICENSE` said "to be decided"), and the name and the
macOS bundle id `com.yuyudhan.nomouse` used by `scripts/release/app.sh` and the Homebrew cask
were provisional. Before the first public release the owner had to settle both: the bundle id is
what macOS ties the Accessibility grant to, so changing it later would make every user grant the
permission again.

## Decision

- The licence is MIT OR Apache-2.0, at the user's option (the owner's answer to question 55,
  "MIT + Apache-2.0"). The texts are in `LICENSE-MIT` and `LICENSE-APACHE`; the workspace sets
  `license = "MIT OR Apache-2.0"` and every crate inherits it. Contributions are dual licensed
  the same way unless the contributor says otherwise.
- The project keeps the name nomouse and the macOS app keeps the bundle id
  `com.yuyudhan.nomouse` (question 56, "yes").

## Consequences

- The dual licence is the Rust ecosystem norm and is compatible with every dependency licence
  cargo-deny allows (`deny.toml`).
- Release archives ship both licence files; the Homebrew formula declares
  `license any_of: ["MIT", "Apache-2.0"]`.
- `com.yuyudhan.nomouse` stays the default of `NOMOUSE_BUNDLE_ID` in the release scripts.
