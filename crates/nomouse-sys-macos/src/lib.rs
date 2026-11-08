//! Safe wrappers over the Accessibility (AXUIElement), CGEvent and AppKit FFI that nomouse needs on `target_os = "macos"`.
//!
//! This is one of the two crates where `unsafe` is permitted (see `docs/architecture.md`,
//! "Unsafe policy"). Rules: every public item is safe to call; every `unsafe` block carries a
//! `// SAFETY:` comment (enforced by `clippy::undocumented_unsafe_blocks`); unsafe functions
//! use explicit inner `unsafe` blocks (`unsafe_op_in_unsafe_fn`). Wrappers land with the
//! backend milestone (macOS M2, Windows M3).
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
