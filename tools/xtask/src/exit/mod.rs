//! The parts that the exit checks share (`cargo xtask m0-exit`, `cargo xtask m1-exit`): input
//! readers, the cell type and the report helpers. M4-P0 (plan v9 V9-D36) moved them out of
//! m0_exit.rs with no logic change, so the m0-exit report stays byte-identical.
//!
//! | Module | What it reads or holds |
//! |---|---|
//! | `json` | the small JSON reader (xtask has no JSON crate) |
//! | `ci` | a CI run (`gh run view <id> --json jobs,headSha,databaseId,url`), its jobs and steps |
//! | `logs` | cargo logs with start and exit-status lines (nesting, corpus) |
//! | `t262` | test262 runner JSONs, committed expectation files, the recomputed diff |
//! | `gc_shards` | the GC-stress shard split and the merge of shard JSONs |
//! | `adr` | the ADR index of docs/adr/README.md and its Counts line |
//! | `verdict` | spike Verdict tables |
//! | `cross` | ep9-cross/v1 runs: freshness and the M0 canary |
//! | `report` | `Status`, `Cell`, text helpers, the stub-marker grep |

pub(crate) mod adr;
pub(crate) mod ci;
pub(crate) mod cross;
pub(crate) mod gc_shards;
pub(crate) mod json;
pub(crate) mod logs;
pub(crate) mod report;
pub(crate) mod t262;
pub(crate) mod verdict;

use std::path::Path;
use std::process::Command;

pub(crate) fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))
}

/// The trimmed stdout of `git -C <dir> <args>`, or None.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !s.trim().is_empty()).then(|| s.trim().to_string())
}
