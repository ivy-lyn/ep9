//! `cargo xtask fp-check` (H36, docs/plan/08-repo.md section 7): no banned libm symbol, no FMA
//! instruction outside `ep9_math` and no second math crate in the post-LTO object of each
//! target; exceptions come from `expectations/fp-check-allow.toml`.
//!
//! Not implemented yet (stub from M0 week 4 P0): it exits 2.
//!
//! Fixture tests go in `tools/xtask/tests/fpcheck.rs` and `tools/xtask/tests/fpcheck/<case>/`.
//! Each fixture has its own `[workspace]` table and no crates.io dependency, there is no
//! `main.rs` directly under `tools/xtask/tests/fpcheck/`, a test copies its fixture to a temp
//! dir before it builds it, and a test that needs llvm-tools or extra rustup targets is
//! `#[ignore]`.

pub fn main(_args: &[String]) -> Result<bool, String> {
    Err("not implemented".into())
}
