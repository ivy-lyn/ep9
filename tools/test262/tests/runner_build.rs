//! gates5 high 1: the dirty flag of the runner's compiled sources (`runner_build.rs`, which
//! build.rs runs) in real git repos: a tracked change under a compiled path is dirty, staged or
//! not; an untracked file or a change outside the compiled paths is not; no git tree gives
//! None. Each edit is under a path cargo watches, so build.rs reruns on it.

#[path = "../runner_build.rs"]
mod runner_build;

use std::path::{Path, PathBuf};
use std::process::Command;

use runner_build::{COMPILED_PATHS, compiled_dirty, dirty_watch_paths};

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
