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
//!
//! FORK-CHECKOUT-ATTEST: the pins and patch sets above say which fork code the tree names, not
//! what the git-ignored checkouts hold. [`fork_attest`] asks the one shared check,
//! `fork_attest` of `scripts/sync-lib.sh` (the `.ep9-stamp` and `pinned_tree` rule), whether each
//! checkout that cargo builds is its pin plus its patches (`EP9_RUNNER_FORKS` of build.rs, and
//! the `dirty` word of `ep9 --version`, `crates/ep9_cli/build.rs`, which includes this file too).
//!
//! ```text
//! fork line   sh -c '. scripts/sync-lib.sh; root=<root>; fork_attest regress icu4x'
//!             -> "regress=pinned icu4x=pinned" (stale or absent in place of pinned)
//! watched     per fork: its .ep9-stamp, its .git/HEAD and the dir cargo builds from it
//!             ([`FORK_BUILT`]); a sync rewrites the stamp, so cargo reruns the script
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

/// The paths, from the repo root, whose files build the runner: the engine crates, the runner,
/// the root manifest and lock file, the toolchain, the regress fork (pin and patch set;
/// ep9_vm builds regress through `[patch.crates-io]`) and the icu4x fork (pin and patch set;
/// ep9_temporal and ep9_intl build icu_calendar through `[patch.crates-io]`).
pub const COMPILED_PATHS: [&str; 9] = [
    "crates",
    "tools/test262",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
    "third_party/icu4x.commit",
    "third_party/patches/icu4x",
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

/// The forks the engine builds from git-ignored checkouts (`[patch.crates-io]` of the root
/// `Cargo.toml`), each with the dir that the patch entry names: regress (ep9_vm) and
/// icu_calendar of icu4x (ep9_temporal, ep9_intl).
pub const FORK_BUILT: [(&str, &str); 2] = [
    ("regress", "third_party/regress"),
    ("icu4x", "third_party/icu4x/components/calendar"),
];

/// The fork line of the checkouts under `root` (module doc): `"regress=<state> icu4x=<state>"`
/// as `fork_attest` of `scripts/sync-lib.sh` prints it. None when the check cannot run (no `sh`,
/// no `scripts/sync-lib.sh`) or prints a line in another form.
pub fn fork_attest(root: &Path) -> Option<String> {
    let lib = root.join("scripts/sync-lib.sh");
    if !lib.is_file() {
        return None;
    }
    let out = Command::new("sh")
        .arg("-c")
        .arg(r#". "$1"; root=$2; shift 2; fork_attest "$@""#)
        .arg("sh")
        .arg(&lib)
        .arg(root)
        .args(FORK_BUILT.map(|(f, _)| f))
        .output()
        .ok()?;
    let line = String::from_utf8(out.stdout).ok()?.trim().to_string();
    // The form: one `<fork>=<state>` per fork, in FORK_BUILT order; status 0 iff all pinned.
    let pairs: Vec<(&str, &str)> = line.split(' ').filter_map(|p| p.split_once('=')).collect();
    let form = pairs.len() == FORK_BUILT.len()
        && pairs
            .iter()
            .zip(FORK_BUILT)
            .all(|((f, s), (want, _))| *f == want && matches!(*s, "pinned" | "stale" | "absent"));
    let all = pairs.iter().all(|(_, s)| *s == "pinned");
    let status = out.status.code();
    (form && status == Some(if all { 0 } else { 1 })).then_some(line)
}

/// Some(true) when every fork of the line reads `pinned`, Some(false) when one does not, None
/// for no line. (`crates/ep9_cli/build.rs` and the tests read it; the runner's build.rs passes
/// the line on as it is.)
#[allow(dead_code)]
pub fn forks_pinned(line: Option<&str>) -> Option<bool> {
    line.map(|l| {
        l.split(' ')
            .all(|p| p.split_once('=').is_some_and(|(_, s)| s == "pinned"))
    })
}

/// The paths to pass to `cargo::rerun-if-changed` so the fork line follows the checkouts: per
/// fork its `.ep9-stamp`, its `.git/HEAD` and the dir cargo builds (each one that exists).
pub fn fork_watch_paths(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.join("scripts/sync-lib.sh")];
    for (fork, built) in FORK_BUILT {
        let dir = root.join("third_party").join(fork);
        out.push(dir.join(".ep9-stamp"));
        out.push(dir.join(".git/HEAD"));
        out.push(root.join(built));
    }
    out.retain(|p| p.exists());
    out
}
