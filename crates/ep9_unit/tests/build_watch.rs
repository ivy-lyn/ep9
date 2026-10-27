//! The git paths `build.rs` watches (H35-Q8), in real git repos: a commit on the checked-out
//! branch must change one of them, also while the branch ref is packed and its name is
//! nested (`review/packed`: `git pack-refs` removes `refs/heads/review/` too).

#[path = "../build_watch.rs"]
mod build_watch;

use std::path::{Path, PathBuf};
use std::process::Command;

use build_watch::{git_path, head_watch_paths};

/// A fresh directory under the temp dir.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ep9-build-watch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// False when no `git` program can be started (a minimal image such as rust:*-slim has none;
/// `build.rs` then watches nothing and has nothing to test here). In CI git must be there.
fn have_git() -> bool {
    let ok = Command::new("git").arg("--version").output().is_ok();
    assert!(
        ok || std::env::var_os("CI").is_none(),
        "git not found, and CI is set"
    );
    if !ok {
        eprintln!("skipped: no git program");
    }
    ok
}

/// `git -C <dir> <args>` without the user's or the system's git config; panics on failure.
fn run_git(dir: &Path, args: &[&str]) {
    let empty = dir.parent().unwrap().join("empty.gitconfig");
    std::fs::write(&empty, "").unwrap();
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=ep9",
            "-c",
            "user.email=ep9@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Best effort: git writes its object files read-only, which some platforms refuse to delete.
fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

fn commit(dir: &Path, msg: &str) {
    run_git(dir, &["commit", "-q", "--allow-empty", "-m", msg]);
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// True if a change to `file` (which exists) is seen through `watched`: `file` is one of them,
/// or it is inside a watched directory (cargo scans a watched directory recursively).
fn seen(watched: &[PathBuf], file: &Path) -> bool {
    let file = canon(file);
    watched.iter().any(|w| {
        let w = canon(w);
        file == w || (w.is_dir() && file.starts_with(&w))
    })
}

/// Checks the packed nested branch of `dir` (checked out there): every watched path exists, the
/// next commit writes a loose ref that a watched path covers, and then the loose ref is watched.
fn check_packed_nested_branch(dir: &Path, packer: &Path, branch: &str) {
    run_git(packer, &["pack-refs", "--all"]);
    let loose = git_path(dir, &format!("refs/heads/{branch}")).unwrap();
    assert!(!loose.exists(), "pack-refs left {}", loose.display());
    let watched = head_watch_paths(dir);
    for w in &watched {
        assert!(w.exists(), "watches a missing path {}", w.display());
    }
    let packed = git_path(dir, "packed-refs").unwrap();
    assert!(
        seen(&watched, &packed),
        "packed-refs not watched: {watched:?}"
    );
    assert!(
        seen(&watched, &git_path(dir, "HEAD").unwrap()),
        "HEAD not watched: {watched:?}"
    );
    commit(dir, "moves the packed branch");
    assert!(loose.is_file(), "the commit wrote no loose ref");
    assert!(
        seen(&watched, &loose),
        "the commit wrote {} outside every watched path: {watched:?}",
        loose.display()
    );
    // Loose again: the ref file itself is watched.
    assert!(seen(&head_watch_paths(dir), &loose));
}

#[test]
fn a_commit_on_a_packed_nested_branch_is_seen() {
    if !have_git() {
        return;
    }
    let root = scratch("repo");
    let dir = root.join("r");
    std::fs::create_dir(&dir).unwrap();
    run_git(&dir, &["init", "-q"]);
    commit(&dir, "a");
    run_git(&dir, &["checkout", "-q", "-b", "review/packed"]);
    commit(&dir, "b");
    check_packed_nested_branch(&dir, &dir, "review/packed");
    cleanup(&root);
}

#[test]
fn a_commit_on_a_packed_nested_branch_in_a_worktree_is_seen() {
    if !have_git() {
        return;
    }
    let root = scratch("worktree");
    let main = root.join("main");
    std::fs::create_dir(&main).unwrap();
    run_git(&main, &["init", "-q"]);
    commit(&main, "a");
    run_git(
        &main,
        &["worktree", "add", "-q", "-b", "wt/nested/x", "../wt"],
    );
    let wt = root.join("wt");
    commit(&wt, "b");
    // The worktree's HEAD is its own; its refs and packed-refs are the main repo's.
    check_packed_nested_branch(&wt, &main, "wt/nested/x");
    cleanup(&root);
}

#[test]
fn a_detached_head_watches_head_only() {
    if !have_git() {
        return;
    }
    let root = scratch("detached");
    let dir = root.join("r");
    std::fs::create_dir(&dir).unwrap();
    run_git(&dir, &["init", "-q"]);
    commit(&dir, "a");
    run_git(&dir, &["checkout", "-q", "--detach"]);
    let watched = head_watch_paths(&dir);
    assert_eq!(watched.len(), 1, "{watched:?}");
    assert_eq!(canon(&watched[0]), canon(&git_path(&dir, "HEAD").unwrap()));
    cleanup(&root);
}
