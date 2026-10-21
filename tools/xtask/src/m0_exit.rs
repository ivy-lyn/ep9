#![allow(dead_code)] // m2-stub
//! `cargo xtask m0-exit` (M0 exit check, docs/plan/09-milestones.md): the table of the 12 M0
//! exit items for one host, PASS, FAIL, OPEN or NOT-RUN per item, with the evidence of each.
//!
//! ```text
//! cargo xtask m0-exit --host <triple> --out m0-exit.md [--ci-run <id>]
//! ```
//!
//! Stub (M0 month 2, P0): the task is dispatched from `main.rs` but not built; it exits 2. The
//! P2 INFRA2 task builds it: items 1-12 per host from the test262 JSONs, the corpus check, the
//! README counts, the spike verdict files and the nesting output; the `x86_64-pc-windows-msvc`
//! column from the jobs of a CI run (`--ci-run`, `gh run view <id> --json jobs`).

/// Entry point. Today: always an error, so the task exits 2 ("not implemented yet").
pub fn main(_args: &[String]) -> Result<bool, String> {
    Err("not built yet (M0 month 2 P0 stub; the P2 INFRA2 task builds it)".into())
}
