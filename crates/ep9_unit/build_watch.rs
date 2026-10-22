//! The git paths `build.rs` watches so it reruns when HEAD moves (H35-Q8). `build.rs` and
//! `tests/build_watch.rs` both include this file with `#[path]`.
//!
//! ```text
//! HEAD                         always
//! a branch HEAD, loose ref     refs/heads/<name>
//! a branch HEAD, packed ref    the nearest existing directory above refs/heads/<name> (inside
//!                              refs/), and packed-refs
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

/// The paths to pass to `cargo::rerun-if-changed`. Each one exists: a path that does not
/// exist would make cargo rerun the build script on every build.
pub fn head_watch_paths(dir: &Path) -> Vec<PathBuf> {
    let Some(head) = git_path(dir, "HEAD") else {
        return Vec::new();
    };
    let mut out = vec![head.clone()];
    let Ok(content) = std::fs::read_to_string(&head) else {
        return out;
    };
    // A detached HEAD holds the commit itself; a branch HEAD reads "ref: refs/heads/<name>".
    let Some(name) = content.trim().strip_prefix("ref: ") else {
        return out;
    };
    let Some(loose) = git_path(dir, name) else {
        return out;
    };
    let refs = git_path(dir, "refs");
    let packed = git_path(dir, "packed-refs");
    out.extend(ref_watch_paths(&loose, refs.as_deref(), packed.as_deref()));
    out
}

/// The paths that change when the branch ref at `loose` moves: the loose ref file if it
/// exists. Else the ref is packed: a commit writes the loose file and leaves packed-refs alone,
/// so watch the nearest existing directory above the loose file, inside `refs` (cargo scans a
/// watched directory recursively, so it sees the new file). `git pack-refs` also removes the
/// empty directories of a nested name (`refs/heads/review/` of `review/packed`), so that
/// directory can be `refs/heads` itself. Watch `packed-refs` too.
pub fn ref_watch_paths(loose: &Path, refs: Option<&Path>, packed: Option<&Path>) -> Vec<PathBuf> {
    if loose.is_file() {
        return vec![loose.to_path_buf()];
    }
    let mut out = Vec::new();
    if let Some(refs) = refs
        && let Some(dir) = loose
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(refs))
            .find(|p| p.is_dir())
    {
        out.push(dir.to_path_buf());
    }
    if let Some(packed) = packed.filter(|p| p.is_file()) {
        out.push(packed.to_path_buf());
    }
    out
}

/// `git rev-parse --git-path <path>`, absolute.
pub fn git_path(dir: &Path, path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(git(dir, &["rev-parse", "--git-path", path])?);
    Some(if p.is_absolute() { p } else { dir.join(p) })
}

/// The trimmed stdout of `git -C <dir> <args>`, or None if git is missing or fails.
pub fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !s.trim().is_empty()).then(|| s.trim().to_string())
}
