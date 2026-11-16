//! The freshness line of `ep9 --version` (route P0-BUILDID; M6-ORCH-148, -150): the commit the
//! binary was built from and whether the engine paths had uncommitted changes, so a bench tool
//! can name the binary it measured, not the checkout it runs in. It sets `EP9_FRESHNESS`
//! for `src/main.rs`:
//!
//! ```text
//! commit <git rev-parse HEAD> clean    no change in ENGINE_PATHS (git status, untracked too)
//!                                      and each fork checkout is its pin plus its patches
//! commit <git rev-parse HEAD> dirty    a change in ENGINE_PATHS, or a fork checkout that is
//!                                      not its pin plus its patches (or cannot be checked)
//! commit unknown                       no git, or not a checkout
//! ```
//!
//! FORK-CHECKOUT-ATTEST: the regress and icu4x pins and patch sets are tree paths, but cargo
//! builds the git-ignored checkouts. The fork check is `fork_attest` of `scripts/sync-lib.sh`
//! through `tools/test262/runner_build.rs` (the runner's build script uses the same file): a
//! checkout left on an old patch set, edited, or on another HEAD is an uncommitted engine
//! change, so the line says `dirty` and the bench tools refuse or flag the binary. The script
//! reruns when a fork's stamp, HEAD or built dir changes (`fork_watch_paths`).
//!
//! It is a freshness id, not the C-19 build id (`crates/ep9_unit/build.rs`, `ENGINE_BUILD_ID`):
//! that id names no tree state by design (H35 "Build id"; MV-1 of MODES-CHECK-w30). It reruns
//! when HEAD moves (the HEAD and branch-ref paths of `crates/ep9_unit/build_watch.rs`, which
//! this file includes) and when a file under ENGINE_PATHS changes (cargo scans a watched
//! directory).

use std::path::Path;
use std::process::Command;

#[path = "../ep9_unit/build_watch.rs"]
mod build_watch;

#[path = "../../tools/test262/runner_build.rs"]
#[allow(dead_code)] // the runner's dirty paths; this script reads only the fork check
mod runner_build;

/// The paths whose state is the engine's: `tools/bench/cross/kernels.mjs` `TREE_PATHS`
/// (FRESH_PATHS, REGRESS_PATHS, ICU4X_PATHS) without `tools/bench` (the bench tools are not in
/// the binary). The regress and icu_calendar forks are built from git-ignored checkouts of
/// their pins and patch sets, so a pin or patch change is an engine change. The xtask test
/// `m2_exit::tests::the_build_line_engine_paths_are_the_bench_tree_paths` keeps the lists equal.
const ENGINE_PATHS: &[&str] = &[
    "crates",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
    "third_party/icu4x.commit",
    "third_party/patches/icu4x",
];

fn main() {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it");
    let root = Path::new(&dir).join("..").join("..");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../../tools/test262/runner_build.rs");
    for p in runner_build::fork_watch_paths(&root) {
        println!("cargo::rerun-if-changed={}", p.display());
    }
    for p in ENGINE_PATHS {
        let p = root.join(p);
        if p.exists() {
            println!("cargo::rerun-if-changed={}", p.display());
        }
    }
    let line = match build_watch::git(&root, &["rev-parse", "HEAD"]) {
        Some(commit) => {
            for p in build_watch::head_watch_paths(&root) {
                println!("cargo::rerun-if-changed={}", p.display());
            }
            // A fork checkout that is not its pin plus its patches is an engine change.
            let forks = runner_build::forks_pinned(runner_build::fork_attest(&root).as_deref());
            match dirty(&root).map(|d| d || forks != Some(true)) {
                Some(true) => format!("commit {commit} dirty"),
                Some(false) => format!("commit {commit} clean"),
                None => "commit unknown".to_string(),
            }
        }
        None => "commit unknown".to_string(),
    };
    println!("cargo::rustc-env=EP9_FRESHNESS={line}");
}

/// `git status --porcelain -- ENGINE_PATHS` has a line (untracked files too: a new module file
/// is built), or None if git fails.
fn dirty(root: &Path) -> Option<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--"])
        .args(ENGINE_PATHS)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
}
