//! gates5 high 1: the dirty flag of the runner's compiled sources (`runner_build.rs`, which
//! build.rs runs) in real git repos: a tracked change under a compiled path is dirty, staged or
//! not; an untracked file or a change outside the compiled paths is not; no git tree gives
//! None. Each edit is under a path cargo watches, so build.rs reruns on it.
//!
//! FORK-CHECKOUT-ATTEST: the fork line of real fork checkouts that `sync_dep` made (temp git
//! repos, no network): each fork reads `pinned` after a sync, and `stale` when its checkout is
//! on an old patch set or edited (the negative control of each fork), with the other fork still
//! `pinned`.

#[path = "../runner_build.rs"]
mod runner_build;

use std::path::{Path, PathBuf};
use std::process::Command;

use runner_build::{
    COMPILED_PATHS, FORK_BUILT, compiled_dirty, dirty_watch_paths, fork_attest, fork_watch_paths,
    forks_pinned,
};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ep9-runner-build-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@test.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// `file` is a watched path or inside a watched directory.
fn watched(paths: &[PathBuf], file: &Path) -> bool {
    paths.iter().any(|w| file.starts_with(w))
}

#[test]
fn a_change_under_a_compiled_path_is_dirty() {
    let d = scratch("dirty");
    assert_eq!(compiled_dirty(&d), None, "no git tree");
    write(&d, "crates/ep9_vm/src/lib.rs", "// vm\n");
    write(&d, "tools/test262/src/main.rs", "// runner\n");
    write(&d, "Cargo.toml", "[workspace]\n");
    write(&d, "third_party/regress.commit", "b\n");
    write(&d, "third_party/patches/regress/README.md", "# p\n");
    write(&d, "third_party/icu4x.commit", "b\n");
    write(&d, "third_party/patches/icu4x/README.md", "# p\n");
    write(&d, "docs/x.md", "doc\n");
    git(&d, &["init", "-q"]);
    git(&d, &["add", "-A"]);
    git(&d, &["commit", "-q", "-m", "a"]);
    assert_eq!(compiled_dirty(&d), Some(false));
    // Outside the compiled paths, or untracked: not dirty.
    write(&d, "docs/x.md", "edited\n");
    write(&d, "crates/ep9_vm/src/new.rs", "// untracked\n");
    assert_eq!(compiled_dirty(&d), Some(false));
    let watch = dirty_watch_paths(&d);
    let index = d.join(".git/index");
    assert!(watch.iter().any(|p| p.ends_with(".git/index")), "{watch:?}");
    for (file, text) in [
        ("crates/ep9_vm/src/lib.rs", "// edited\n"),
        ("tools/test262/src/main.rs", "// edited\n"),
        ("Cargo.toml", "[workspace]\n# edited\n"),
        ("third_party/regress.commit", "c\n"),
        ("third_party/patches/regress/README.md", "# edited\n"),
        ("third_party/icu4x.commit", "c\n"),
        ("third_party/patches/icu4x/README.md", "# edited\n"),
    ] {
        assert!(watched(&watch, &d.join(file)), "{file}: {watch:?}");
        let old = std::fs::read_to_string(d.join(file)).unwrap();
        write(&d, file, text);
        assert_eq!(compiled_dirty(&d), Some(true), "{file}");
        // Staged: still dirty (the index differs from HEAD).
        git(&d, &["add", file]);
        assert_eq!(compiled_dirty(&d), Some(true), "{file} staged");
        // Undone: clean again.
        write(&d, file, &old);
        git(&d, &["add", file]);
        assert_eq!(compiled_dirty(&d), Some(false), "{file} undone");
    }
    assert!(index.is_file());
    std::fs::remove_dir_all(d).unwrap();
}

/// Each watched path exists (a missing one would rerun build.rs on every build), and the
/// compiled paths of this repository are all watched.
#[test]
fn the_watched_paths_exist() {
    let d = scratch("exist");
    write(&d, "Cargo.toml", "[workspace]\n");
    git(&d, &["init", "-q"]);
    git(&d, &["add", "-A"]);
    git(&d, &["commit", "-q", "-m", "a"]);
    let watch = dirty_watch_paths(&d);
    assert_eq!(watch.len(), 2, "the index and Cargo.toml: {watch:?}");
    assert!(watch.iter().all(|p| p.exists()), "{watch:?}");
    std::fs::remove_dir_all(&d).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if compiled_dirty(&repo).is_some() {
        let watch = dirty_watch_paths(&repo);
        for c in COMPILED_PATHS {
            assert!(
                watch.iter().any(|p| p.ends_with(c)),
                "{c} is not watched: {watch:?}"
            );
        }
    }
}

/// The repository's scripts/sync-lib.sh (the one shared check).
fn sync_lib() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/sync-lib.sh")
}

/// `sync_dep <fork> file://<upstream>` in `root`, as the sync scripts run it.
#[cfg(unix)]
fn sync(root: &Path, fork: &str, upstream: &Path) {
    let out = Command::new("bash")
        .arg("-c")
        .arg(r#"set -euo pipefail; root=$1; . "$root/scripts/sync-lib.sh"; sync_dep "$2" "$3""#)
        .arg("bash")
        .arg(root)
        .arg(fork)
        .arg(format!("file://{}", upstream.display()))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "sync {fork}: {out:?}");
}

/// A fake repo root with the real sync-lib.sh and both forks (each with an upstream of one
/// commit, a pin and one patch), synced. Returns the root and the upstream dirs.
#[cfg(unix)]
fn fork_root(name: &str) -> (PathBuf, PathBuf) {
    let d = scratch(name);
    let root = d.join("root");
    write(
        &root,
        "scripts/sync-lib.sh",
        &std::fs::read_to_string(sync_lib()).unwrap(),
    );
    for (fork, built) in FORK_BUILT {
        let up = d.join(format!("up-{fork}"));
        // The upstream holds the built dir of the fork (relative to the checkout).
        let rel = Path::new(built)
            .strip_prefix(format!("third_party/{fork}"))
            .unwrap();
        write(&up.join(rel), "src/lib.rs", "// one\n");
        git(&up, &["init", "-q"]);
        git(&up, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
        git(&up, &["add", "-A"]);
        git(&up, &["commit", "-q", "-m", "one"]);
        let rev = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&up)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        write(&root, &format!("third_party/{fork}.commit"), &rev);
        let lib = rel.join("src/lib.rs");
        let lib = lib.to_str().unwrap();
        write(
            &root,
            &format!("third_party/patches/{fork}/0001-a.patch"),
            &format!(
                "diff --git a/{lib} b/{lib}\n--- a/{lib}\n+++ b/{lib}\n@@ -1 +1 @@\n-// one\n+// one patched\n"
            ),
        );
        sync(&root, fork, &up);
    }
    (root, d)
}

/// The fork line of synced checkouts reads pinned for each fork; per fork, a checkout on an
/// old patch set (a patch joined the set after the sync) and an edited built file read stale,
/// while the other fork stays pinned; a fork with no checkout reads absent. These fail on the
/// old runner build, which had no fork check (only the pin and patch files were watched).
#[cfg(unix)]
#[test]
fn the_fork_line_is_pinned_only_for_a_synced_checkout() {
    let (root, d) = fork_root("forks");
    let line = fork_attest(&root);
    assert_eq!(line.as_deref(), Some("regress=pinned icu4x=pinned"));
    assert_eq!(forks_pinned(line.as_deref()), Some(true));
    for (k, (fork, built)) in FORK_BUILT.iter().enumerate() {
        let other = FORK_BUILT[1 - k].0;
        let want = |state: &str| {
            let mut v = [String::new(), String::new()];
            v[k] = format!("{fork}={state}");
            v[1 - k] = format!("{other}=pinned");
            v.join(" ")
        };
        // The negative control of the route: a checkout left on an old patch set.
        let rel = Path::new(built)
            .strip_prefix(format!("third_party/{fork}"))
            .unwrap();
        let lib = rel.join("src/lib.rs");
        let lib = lib.to_str().unwrap();
        let second = format!("third_party/patches/{fork}/0002-b.patch");
        write(
            &root,
            &second,
            &format!(
                "diff --git a/{lib} b/{lib}\n--- a/{lib}\n+++ b/{lib}\n@@ -1 +1 @@\n-// one patched\n+// one patched twice\n"
            ),
        );
        let line = fork_attest(&root);
        assert_eq!(
            line.as_deref(),
            Some(want("stale").as_str()),
            "{fork} old patch set"
        );
        assert_eq!(forks_pinned(line.as_deref()), Some(false));
        sync(&root, fork, &d.join(format!("up-{fork}")));
        assert_eq!(
            fork_attest(&root).as_deref(),
            Some("regress=pinned icu4x=pinned"),
            "{fork} synced again"
        );
        // An edit of a built file (a stale build input with the stamp in place).
        write(&root, &format!("{built}/src/lib.rs"), "// edited\n");
        assert_eq!(
            fork_attest(&root).as_deref(),
            Some(want("stale").as_str()),
            "{fork} edited"
        );
        std::fs::remove_file(root.join(&second)).unwrap();
        sync(&root, fork, &d.join(format!("up-{fork}")));
    }
    assert_eq!(
        fork_attest(&root).as_deref(),
        Some("regress=pinned icu4x=pinned")
    );
    std::fs::remove_dir_all(root.join("third_party/icu4x")).unwrap();
    let line = fork_attest(&root);
    assert_eq!(line.as_deref(), Some("regress=pinned icu4x=absent"));
    assert_eq!(forks_pinned(line.as_deref()), Some(false));
    std::fs::remove_dir_all(&d).unwrap();
}

/// No scripts/sync-lib.sh (a tree before the check): no line, so the exec JSON names no fork
/// state and m1-exit and m2-exit read it as missing. The watched fork paths exist and name each
/// fork's stamp, HEAD and built dir; the repository's own checkouts read pinned when the tree
/// is synced (the sync scripts run before the first cargo command).
#[test]
fn the_fork_check_needs_the_shared_script_and_watches_each_fork() {
    let d = scratch("nolib");
    assert_eq!(fork_attest(&d), None);
    assert_eq!(forks_pinned(None), None);
    assert_eq!(fork_watch_paths(&d), Vec::<PathBuf>::new());
    std::fs::remove_dir_all(&d).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let watch = fork_watch_paths(&repo);
    assert!(watch.iter().all(|p| p.exists()), "{watch:?}");
    assert!(
        watch.iter().any(|p| p.ends_with("scripts/sync-lib.sh")),
        "{watch:?}"
    );
    for (fork, built) in FORK_BUILT {
        if repo
            .join("third_party")
            .join(fork)
            .join(".ep9-stamp")
            .exists()
        {
            for p in [
                Path::new("third_party").join(fork).join(".ep9-stamp"),
                Path::new("third_party").join(fork).join(".git/HEAD"),
                PathBuf::from(built),
            ] {
                assert!(watch.iter().any(|w| w.ends_with(&p)), "{p:?}: {watch:?}");
            }
        }
    }
}
