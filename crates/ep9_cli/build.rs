//! The freshness line of `ep9 --version` (route P0-BUILDID; M6-ORCH-148, -150): the commit the
//! binary was built from and whether the engine paths had uncommitted changes, so a bench tool
//! can name the binary it measured, not the checkout it runs in. It sets `EP9_FRESHNESS`
//! for `src/main.rs`:
//!
//! ```text
//! commit <git rev-parse HEAD> clean    no change in ENGINE_PATHS (git status, untracked too)
//! commit <git rev-parse HEAD> dirty    a change in ENGINE_PATHS
//! commit unknown                       no git, or not a checkout
//! ```
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
            match dirty(&root) {
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
