//! `cargo xtask feature-matrix` (H27, docs/plan/08-repo.md section 7): build, clippy
//! `-D warnings` and test for each feature set, one package at a time, never with
//! `--workspace` (a workspace build unifies features and would hide a jitless leak):
//!
//! | Set | Packages and flags |
//! |---|---|
//! | default (`jit`) | `-p ep9_cli` |
//! | jitless | `-p ep9_cli --no-default-features` |
//! | aot | `-p ep9_unit --features aot` |
//!
//! Not implemented yet (stub from M0 week 4 P0): it exits 2.

pub fn main(_args: &[String]) -> Result<bool, String> {
    Err("not implemented".into())
}
