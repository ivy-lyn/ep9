//! Whether the sources the runner is built from have uncommitted changes (`EP9_RUNNER_DIRTY`,
//! gates5 high 1; m1-exit X4). `build.rs` and `tests/runner_build.rs` both include this file
//! with `#[path]`.
//!
//! ```text
//! dirty     git status --porcelain --untracked-files=no -- <COMPILED_PATHS> prints a line
//!           (a tracked file differs from HEAD, staged or not), run in the repo root
//! watched   the git index (git add, commit, stash, checkout) and each compiled path that
//!           exists (cargo scans a watched directory recursively: an edit that is not staged
//!           changes no git file, so the index alone would keep a stale flag)
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

/// The paths, from the repo root, whose files build the runner: the engine crates, the runner,
/// the root manifest and lock file, the toolchain, and the regress fork (pin and patch set;
/// ep9_vm builds regress through `[patch.crates-io]`).
pub const COMPILED_PATHS: [&str; 7] = [
    "crates",
    "tools/test262",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
];

/// Some(true) when a tracked file under [`COMPILED_PATHS`] differs from HEAD, Some(false) when
/// none does, None when git cannot answer (no git, or `root` is no work tree).
pub fn compiled_dirty(root: &Path) -> Option<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=no", "--"])
        .args(COMPILED_PATHS)
        .output()
        .ok()?;
    out.status.success().then_some(!out.stdout.is_empty())
}

/// The paths to pass to `cargo::rerun-if-changed` so the flag follows the compiled sources:
/// the git index of `root` and each of [`COMPILED_PATHS`] that exists (a path that does not
/// exist would make cargo rerun the build script on every build).
pub fn dirty_watch_paths(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(o) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-path", "index"])
        .output()
        && o.status.success()
    {
        let p = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        let p = if p.is_absolute() { p } else { root.join(p) };
        if p.is_file() {
            out.push(p);
        }
    }
    out.extend(
        COMPILED_PATHS
            .iter()
            .map(|c| root.join(c))
            .filter(|p| p.exists()),
    );
    out
}
