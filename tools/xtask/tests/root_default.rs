//! The `--root` default of the tree readers (plan v12 GATES10 task 4, PR-15): one xtask binary
//! (one build, as with one target dir shared by two worktrees) run in two worktrees of one
//! scratch repository reads the tree of the current dir, not the tree it was built from.
//!
//! adr-lint, m0-exit, m1-exit and m2-exit default to the git top dir of the current dir. The
//! scratch trees are small fixtures (plan rule 24): no live ADR or expectation file is read.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The exit checks with a `--host` and an `--out` and the git-top-dir default.
const EXIT_TASKS: [&str; 3] = ["m0-exit", "m1-exit", "m2-exit"];

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=root test",
            "-c",
            "user.email=root@test.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// `xtask <args>` with the current dir `cwd`.
fn xtask(cwd: &Path, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    eprintln!(
        "--- xtask {args:?} in {}: {}\n--- stdout\n{}--- stderr\n{}",
        cwd.display(),
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A scratch dir that is removed at the end.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Two worktrees `a` and `b` of one repository. Both hold `tools/xtask/Cargo.toml` (an ep9
/// tree) and an ADR whose Crates row names `ep9_vm`. In `a` the last commit touches
/// `crates/ep9_vm` with no `ADR:` trailer; in `b` it has one. Returns (scratch, a, b, base).
fn two_worktrees(name: &str) -> (Scratch, PathBuf, PathBuf, String) {
    let s = std::env::temp_dir().join(format!("ep9-root-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&s);
    let a = s.join("a");
    fs::create_dir_all(&a).unwrap();
    git(&a, &["init", "-q", "-b", "main"]);
    write(
        &a,
        "tools/xtask/Cargo.toml",
        "[package]\nname = \"xtask\"\n",
    );
    write(
        &a,
        "docs/adr/H01-test.md",
        "# H1 test\n\n| Crates | `ep9_vm` |\n",
    );
    write(&a, "crates/ep9_vm/src/lib.rs", "// base\n");
    git(&a, &["add", "-A"]);
    git(&a, &["commit", "-q", "-m", "base"]);
    let base = git(&a, &["rev-parse", "HEAD"]);
    let b = s.join("b");
    git(
        &a,
        &["worktree", "add", "-q", "-b", "b", b.to_str().unwrap()],
    );
    write(&a, "crates/ep9_vm/src/lib.rs", "// a\n");
    git(&a, &["commit", "-q", "-am", "vm: change a (no trailer)"]);
    write(&b, "crates/ep9_vm/src/lib.rs", "// b\n");
    git(
        &b,
        &["commit", "-q", "-am", "vm: change b\n\nADR: none (test)"],
    );
    (Scratch(s), a, b, base)
}

#[test]
fn m2_exit_reads_the_worktree_of_the_current_dir() {
    let (s, a, b, _) = two_worktrees("m2");
    for (wt, sub) in [(&a, ""), (&b, ""), (&a, "crates/ep9_vm")] {
        let out = s.0.join("out.md");
        let o = xtask(
            &wt.join(sub),
            &[
                "m2-exit",
                "--host",
                "aarch64-apple-darwin",
                "--out",
                out.to_str().unwrap(),
            ],
        );
        assert_eq!(o.status.code(), Some(1), "{}", text(&o));
        let head = git(wt, &["rev-parse", "HEAD"]);
        let md = fs::read_to_string(&out).unwrap();
        assert!(
            md.contains(&format!("tree at commit `{head}`")),
            "{} ({sub}): {md}",
            wt.display()
        );
    }
    // --root still wins over the current dir.
    let out = s.0.join("out.md");
    let o = xtask(
        &a,
        &[
            "m2-exit",
            "--host",
            "aarch64-apple-darwin",
            "--out",
            out.to_str().unwrap(),
            "--root",
            b.to_str().unwrap(),
        ],
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let head_b = git(&b, &["rev-parse", "HEAD"]);
    assert!(
        fs::read_to_string(&out)
            .unwrap()
            .contains(&format!("tree at commit `{head_b}`"))
    );
}

#[test]
fn adr_lint_reads_the_worktree_of_the_current_dir() {
    let (_s, a, b, base) = two_worktrees("adr");
    let lint = |wt: &Path| xtask(wt, &["adr-lint", "--trailers-only", "--base", &base]);
    let oa = lint(&a);
    assert_eq!(oa.status.code(), Some(1), "{}", text(&oa));
    assert!(text(&oa).contains("vm: change a"), "{}", text(&oa));
    let ob = lint(&b);
    assert_eq!(ob.status.code(), Some(0), "{}", text(&ob));
    assert!(!text(&ob).contains("vm: change a"), "{}", text(&ob));
}

#[test]
fn m1_exit_reads_the_worktree_of_the_current_dir() {
    let (s, a, b, _) = two_worktrees("m1");
    for wt in [&a, &b] {
        let out = s.0.join("m1.md");
        let o = xtask(
            wt,
            &[
                "m1-exit",
                "--host",
                "aarch64-apple-darwin",
                "--out",
                out.to_str().unwrap(),
                "--base",
                "HEAD",
            ],
        );
        assert_eq!(o.status.code(), Some(1), "{}", text(&o));
        let head = git(wt, &["rev-parse", "HEAD"]);
        assert!(
            fs::read_to_string(&out)
                .unwrap()
                .contains(&format!("tree at commit `{head}`")),
            "{}",
            wt.display()
        );
    }
}

#[test]
fn no_ep9_tree_needs_root() {
    let (s, a, _, _) = two_worktrees("none");
    // Outside any git work tree.
    let plain = s.0.join("plain");
    fs::create_dir_all(&plain).unwrap();
    // A git tree that is not an ep9 tree (a nested checkout such as third_party/test262).
    let other = a.join("third_party/other");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q"]);
    for (dir, want) in [
        (&plain, "is not in a git work tree"),
        (&other, "is not an ep9 tree"),
    ] {
        for task in EXIT_TASKS {
            let out = s.0.join("x.md");
            let o = xtask(
                dir,
                &[
                    task,
                    "--host",
                    "aarch64-apple-darwin",
                    "--out",
                    out.to_str().unwrap(),
                ],
            );
            assert_eq!(o.status.code(), Some(2), "{task}: {}", text(&o));
            assert!(text(&o).contains(want), "{task}: {}", text(&o));
            assert!(text(&o).contains("pass --root"), "{task}: {}", text(&o));
        }
        let o = xtask(dir, &["adr-lint", "--base", "HEAD"]);
        assert_eq!(o.status.code(), Some(2), "{}", text(&o));
        assert!(text(&o).contains(want), "{}", text(&o));
    }
}

#[test]
fn m0_exit_reads_the_worktree_of_the_current_dir() {
    let (s, a, b, _) = two_worktrees("m0");
    for (wt, sub) in [(&a, ""), (&b, ""), (&a, "crates/ep9_vm")] {
        let out = s.0.join("m0.md");
        let o = xtask(
            &wt.join(sub),
            &[
                "m0-exit",
                "--host",
                "aarch64-apple-darwin",
                "--out",
                out.to_str().unwrap(),
                "--no-build-check",
            ],
        );
        // The scratch tree has no ADR index, so m0-exit stops at its first read: a file of the
        // worktree of the current dir (the git top dir as git prints it), not of the build tree.
        let top = git(wt, &["rev-parse", "--show-toplevel"]);
        assert_eq!(o.status.code(), Some(2), "{}", text(&o));
        assert!(
            text(&o).contains(&format!("cannot read {top}")),
            "{}",
            text(&o)
        );
        assert!(text(&o).contains("README.md"), "{}", text(&o));
        assert!(!out.exists());
    }
}
