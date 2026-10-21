//! `cargo xtask gc-lint` (H10): parses `crates/ep9_vm/src` and `crates/ep9_builtins/src` with
//! `syn` and flags a stack-only GC type (`Local` and friends, or a struct that holds one) inside
//! a heap container, a `static` or a `thread_local`. Findings print `file:line` with `/`
//! separators on every OS.
//!
//! Not implemented yet (stub from M0 week 4 P0): it exits 2.
//!
//! Fixture tests go in `tools/xtask/tests/gc_lint.rs` and `tools/xtask/tests/gc_lint/<case>/`,
//! with no `main.rs` directly under `tools/xtask/tests/gc_lint/`.

pub fn main(_args: &[String]) -> Result<bool, String> {
    Err("not implemented".into())
}
