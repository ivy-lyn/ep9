//! Where a JSON summary came from (plan v9 section 7 step 3, P3-ORCH-14 and P3-ORCH-21): the
//! commit the ep9 tree is at when the runner runs, whether that tree had uncommitted changes,
//! and the host or CI job that ran it. The fields are read at run time, not at build time: a
//! binary built at one commit and run after a checkout of another names the other one. Run the
//! runner with `cargo run` (it rebuilds on a change) to make an m1-exit input.
//! `cargo xtask m1-exit` items 3 and 4 read the three fields and give OPEN "stale input" when
//! the commit is missing, the tree was dirty, or the paths the run depends on changed after the
//! commit.
//!
//! The runner reads the facts twice (CX5-C1): before the run starts and when it writes the
//! summary. The JSON names the first read; when the two differ in the commit, the dirty flag,
//! the source kind or a tree id ([`settle`]), it writes `"dirty": true` and
//! `"source_changed_during_run": true`, so m1-exit gives OPEN.
//!
//! - `"commit"`: `git rev-parse --verify HEAD` in the repo root (found at build time, as the
//!   default paths are). Left out when git cannot answer.
//! - `"dirty"`: `git status --porcelain --untracked-files=no` prints a line (a tracked file has
//!   an uncommitted change). Left out when git cannot answer.
//! - `"commit_source"`: `"git"`, or `"env"` when the repo root is no git tree (a copy of the
//!   tree, as on a sandbox box) and `EP9_SOURCE_COMMIT` (40 hex digits) and `EP9_SOURCE_DIRTY`
//!   (`0` or `1`) name them; the caller that copied the tree sets both. With no git tree and no
//!   `EP9_SOURCE_COMMIT`: `"sbx-commit"` when `<root>/.sbx-commit` holds one line of 40 hex
//!   digits (CX5-C6; `infra/x86-sandbox/sbx push-git` writes it after it extracts `git archive`
//!   of that commit, committed files only, so `"dirty": false`). Such a tree is taken as clean
//!   with no file check; the month-5 provenance work adds one.
//! - An explicit `EP9_SOURCE_DIRTY` other than `0` makes `"dirty": true` on every path (git, env,
//!   `.sbx-commit`), also with no `EP9_SOURCE_COMMIT` (CX6-2). `0` does not hide a dirty git tree.
//! - `"made_by"`: `host:<hostname>`, plus ` gh-run:<GITHUB_RUN_ID>/<GITHUB_JOB>` in a GitHub job.
//! - `"tree"`: `{"<path>": "<git tree or blob id at HEAD>", ...}` for [`TREE_PATHS`] (from git
//!   only). A pull_request CI job runs on GitHub's merge commit, which the local repository does
//!   not have: m1-exit then compares these ids with HEAD's.
//!
//! The nesting log of m1-exit item 8 prints the same facts in one line (NESTING_CMD in
//! tools/xtask/src/m1_exit.rs).

use crate::json;
use std::path::Path;
use std::process::Command;

/// The paths whose ids the JSON names: the paths whose change makes an m1-exit item 3 or 4 input
/// stale (`T262_FRESH_PATHS` in tools/xtask/src/m1_exit.rs).
pub const TREE_PATHS: [&str; 7] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "tools/test262",
    "third_party/test262.commit",
];

/// The source facts of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub commit: Option<String>,
    pub dirty: Option<bool>,
    /// "git", "env" or "sbx-commit".
    pub from: &'static str,
    pub made_by: String,
    /// (path, id at HEAD) for each of [`TREE_PATHS`] that HEAD has (git only).
    pub tree: Vec<(&'static str, String)>,
    /// The facts read when the summary was written differ from the ones read before the run
    /// ([`settle`]).
    pub changed: bool,
}

/// The file `infra/x86-sandbox/sbx push-git` writes in the root of the tree it extracts.
pub const SBX_COMMIT: &str = ".sbx-commit";

/// The trimmed stdout of `git -C <root> <args>`, or None when git fails.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `EP9_SOURCE_DIRTY`: unset is None, `0` is `false`, any other value is `true` (CX6-2).
fn dirty_env(env: &dyn Fn(&str) -> Option<String>) -> Option<bool> {
    env("EP9_SOURCE_DIRTY").map(|d| d.trim() != "0")
}

/// The facts of the tree at `root`; `env` reads an environment variable (a parameter, so the
/// tests do not touch the process environment).
pub fn source(root: &Path, env: &dyn Fn(&str) -> Option<String>) -> Source {
    let made_by = made_by(env);
    // An explicit dirty declaration wins on every path (CX6-2).
    let declared_dirty = dirty_env(env) == Some(true);
    // A work tree of its own: `--show-toplevel` must be `root`, so a copy of the tree inside
    // another repository does not take that repository's commit.
    let top = git(root, &["rev-parse", "--show-toplevel"]);
    let same = top.as_deref().is_some_and(|t| {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        canon(Path::new(t)) == canon(root)
    });
    if same {
        let commit = git(root, &["rev-parse", "--verify", "HEAD^{commit}"]).filter(|c| full_sha(c));
        let dirty = if declared_dirty {
            Some(true)
        } else {
            git(root, &["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty())
        };
        let tree = if commit.is_some() {
            TREE_PATHS
                .iter()
                .filter_map(|p| Some((*p, git(root, &["rev-parse", &format!("HEAD:{p}")])?)))
                .collect()
        } else {
            Vec::new()
        };
        return Source {
            commit,
            dirty,
            from: "git",
            made_by,
            tree,
            changed: false,
        };
    }
    if env("EP9_SOURCE_COMMIT").is_none()
        && let Ok(text) = std::fs::read_to_string(root.join(SBX_COMMIT))
    {
        // One commit's committed files (`git archive`): not dirty unless the caller says so. A
        // bad line names nothing.
        let commit = Some(text.trim().to_ascii_lowercase()).filter(|c| full_sha(c));
        return Source {
            dirty: if declared_dirty {
                Some(true)
            } else {
                commit.as_ref().map(|_| false)
            },
            commit,
            from: "sbx-commit",
            made_by,
            tree: Vec::new(),
            changed: false,
        };
    }
    let commit = env("EP9_SOURCE_COMMIT")
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| full_sha(c));
    Source {
        commit,
        dirty: dirty_env(env),
        from: "env",
        made_by,
        tree: Vec::new(),
        changed: false,
    }
}

/// The facts of a run (CX5-C1): `before` (read before the run started) when `after` (read when
/// the summary is written) names the same commit, dirty flag, source kind and tree ids; else
/// `before` with `dirty` true and `changed` set, because the tests may have run on either tree.
pub fn settle(before: Source, after: &Source) -> Source {
    let same = before.commit == after.commit
        && before.dirty == after.dirty
        && before.from == after.from
        && before.tree == after.tree;
    if same {
        before
    } else {
        Source {
            dirty: Some(true),
            changed: true,
            ..before
        }
    }
}

/// `host:<hostname>[ gh-run:<run id>/<job>]`.
pub fn made_by(env: &dyn Fn(&str) -> Option<String>) -> String {
    let host = Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| env("COMPUTERNAME"))
        .or_else(|| env("HOSTNAME"))
        .unwrap_or_else(|| "unknown".into());
    let mut s = format!("host:{host}");
    if let Some(run) = env("GITHUB_RUN_ID").filter(|r| !r.is_empty()) {
        let job = env("GITHUB_JOB").unwrap_or_default();
        s.push_str(&format!(" gh-run:{run}/{job}"));
    }
    s
}

/// The process environment, for [`source`].
pub fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Appends `,"commit":...,"dirty":...,"commit_source":...,"made_by":...` (a missing commit or
/// dirty flag is left out, so m1-exit reads "no commit" and gives OPEN), then
/// `,"source_changed_during_run":true` when [`settle`] saw a change, and `,"tree":{...}`.
pub fn write_fields(out: &mut String, s: &Source) {
    if let Some(c) = &s.commit {
        out.push_str(",\"commit\":");
        json::string(out, c);
    }
    if let Some(d) = s.dirty {
        out.push_str(if d {
            ",\"dirty\":true"
        } else {
            ",\"dirty\":false"
        });
    }
    out.push_str(",\"commit_source\":");
    json::string(out, s.from);
    out.push_str(",\"made_by\":");
    json::string(out, &s.made_by);
    if s.changed {
        out.push_str(",\"source_changed_during_run\":true");
    }
    if !s.tree.is_empty() {
        out.push_str(",\"tree\":{");
        for (i, (p, id)) in s.tree.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json::string(out, p);
            out.push(':');
            json::string(out, id);
        }
        out.push('}');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git_in(root: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
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

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ep9-t262-src-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// A clean tree: its HEAD, not dirty; a changed tracked file makes it dirty; an untracked
    /// file does not.
    #[test]
    fn a_git_tree_names_its_head_and_dirty_state() {
        let d = tmp("git");
        std::fs::write(d.join("a.txt"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        let s = source(&d, &no_env);
        assert_eq!(s.from, "git");
        assert_eq!(s.commit.as_deref().map(str::len), Some(40), "{s:?}");
        assert_eq!(s.dirty, Some(false));
        std::fs::write(d.join("new.txt"), "untracked\n").unwrap();
        assert_eq!(source(&d, &no_env).dirty, Some(false));
        std::fs::write(d.join("a.txt"), "changed\n").unwrap();
        assert_eq!(source(&d, &no_env).dirty, Some(true));
        let mut out = String::new();
        write_fields(&mut out, &source(&d, &no_env));
        assert!(out.starts_with(",\"commit\":\""), "{out}");
        assert!(out.contains(",\"dirty\":true,\"commit_source\":\"git\",\"made_by\":\"host:"));
        // The temp tree has none of the seven paths; with one, its id is named.
        assert!(!out.contains("\"tree\""), "{out}");
        std::fs::write(d.join("Cargo.lock"), "lock\n").unwrap();
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "lock"]);
        let s = source(&d, &no_env);
        assert_eq!(s.tree.len(), 1);
        assert_eq!(s.tree[0].0, "Cargo.lock");
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(
            out.ends_with(&format!(",\"tree\":{{\"Cargo.lock\":\"{}\"}}", s.tree[0].1)),
            "{out}"
        );
        // The root Cargo.toml is a tree path too (M4-P5 C1: its `[profile.release]`).
        std::fs::write(d.join("Cargo.toml"), "[workspace]\n").unwrap();
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "manifest"]);
        let s = source(&d, &no_env);
        let names: Vec<&str> = s.tree.iter().map(|(p, _)| *p).collect();
        assert_eq!(names, ["Cargo.toml", "Cargo.lock"], "{s:?}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// No git tree: the commit comes only from EP9_SOURCE_COMMIT and EP9_SOURCE_DIRTY; without
    /// them both fields are left out of the JSON. A subdir of another repository is not that
    /// repository's tree.
    #[test]
    fn a_copy_without_git_uses_the_env_or_names_nothing() {
        let d = tmp("copy");
        let s = source(&d, &no_env);
        assert_eq!((s.commit.clone(), s.dirty, s.from), (None, None, "env"));
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(!out.contains("\"commit\""), "{out}");
        assert!(!out.contains("\"dirty\""), "{out}");
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let env = |k: &str| match k {
            "EP9_SOURCE_COMMIT" => Some(sha.to_string()),
            "EP9_SOURCE_DIRTY" => Some("0".to_string()),
            "GITHUB_RUN_ID" => Some("42".to_string()),
            "GITHUB_JOB" => Some("test".to_string()),
            _ => None,
        };
        let s = source(&d, &env);
        assert_eq!(s.commit.as_deref(), Some(sha));
        assert_eq!(s.dirty, Some(false));
        assert!(s.made_by.ends_with(" gh-run:42/test"), "{}", s.made_by);
        // A short or bad sha is no commit.
        let bad = |k: &str| (k == "EP9_SOURCE_COMMIT").then(|| "abc123".to_string());
        assert_eq!(source(&d, &bad).commit, None);
        // Inside another repository: not its commit.
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["commit", "-q", "--allow-empty", "-m", "outer"]);
        let sub = d.join("copy");
        std::fs::create_dir_all(&sub).unwrap();
        let s = source(&sub, &no_env);
        assert_eq!((s.commit, s.from), (None, "env"));
        std::fs::remove_dir_all(d).unwrap();
    }

    /// CX5-C1 (Codex round 1): the facts are read before the run and again when the summary
    /// is written. A commit made between the two reads (here, in place of a long run), or
    /// another env commit, gives the first commit with dirty true and the change flag; the
    /// same facts give the first read unchanged.
    #[test]
    fn a_source_change_during_the_run_is_dirty() {
        let d = tmp("during");
        std::fs::write(d.join("Cargo.lock"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        let before = source(&d, &no_env);
        let same = settle(before.clone(), &source(&d, &no_env));
        assert_eq!(same, before);
        let mut out = String::new();
        write_fields(&mut out, &same);
        assert!(!out.contains("source_changed_during_run"), "{out}");
        // The checkout moves on while the tests run.
        std::fs::write(d.join("Cargo.lock"), "b\n").unwrap();
        git_in(&d, &["commit", "-q", "-am", "b"]);
        let after = source(&d, &no_env);
        assert_ne!(after.commit, before.commit);
        let s = settle(before.clone(), &after);
        assert_eq!(
            (s.commit.clone(), s.dirty, s.changed),
            (before.commit.clone(), Some(true), true)
        );
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(out.contains(",\"dirty\":true,"), "{out}");
        assert!(out.contains(",\"source_changed_during_run\":true"), "{out}");
        // Only the dirty flag changes (an edit that is not committed): a change too.
        std::fs::write(d.join("Cargo.lock"), "c\n").unwrap();
        let s = settle(after.clone(), &source(&d, &no_env));
        assert!(s.changed, "{s:?}");
        // A git-free copy: the env seam names another commit at the end.
        let copy = tmp("during-copy");
        let at = |sha: &'static str| {
            move |k: &str| match k {
                "EP9_SOURCE_COMMIT" => Some(sha.to_string()),
                "EP9_SOURCE_DIRTY" => Some("0".to_string()),
                _ => None,
            }
        };
        let a = "0123456789abcdef0123456789abcdef01234567";
        let b = "89abcdef0123456789abcdef0123456789abcdef";
        let first = source(&copy, &at(a));
        assert!(!settle(first.clone(), &source(&copy, &at(a))).changed);
        let s = settle(first, &source(&copy, &at(b)));
        assert_eq!(
            (s.commit.as_deref(), s.dirty, s.changed),
            (Some(a), Some(true), true)
        );
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(copy).unwrap();
    }

    /// CX6-2 (Codex round 2): an explicit `EP9_SOURCE_DIRTY` other than `0` marks the run dirty
    /// on every path: git, env and `.sbx-commit`, also with no `EP9_SOURCE_COMMIT`. `0` does not
    /// hide a dirty git tree.
    #[test]
    fn an_explicit_dirty_env_is_dirty_on_every_path() {
        let dirty =
            |v: &'static str| move |k: &str| (k == "EP9_SOURCE_DIRTY").then(|| v.to_string());
        // git: a clean tree.
        let d = tmp("dirty-env-git");
        std::fs::write(d.join("a.txt"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        for (v, want) in [("1", true), ("yes", true), ("0", false)] {
            let s = source(&d, &dirty(v));
            assert_eq!((s.from, s.dirty), ("git", Some(want)), "{v}: {s:?}");
        }
        std::fs::write(d.join("a.txt"), "changed\n").unwrap();
        assert_eq!(source(&d, &dirty("0")).dirty, Some(true));
        // .sbx-commit, no EP9_SOURCE_COMMIT.
        let sbx = tmp("dirty-env-sbx");
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(sbx.join(".sbx-commit"), format!("{sha}\n")).unwrap();
        for (v, want) in [("1", true), ("2", true), ("0", false)] {
            let s = source(&sbx, &dirty(v));
            assert_eq!(
                (s.commit.as_deref(), s.from, s.dirty),
                (Some(sha), "sbx-commit", Some(want)),
                "{v}: {s:?}"
            );
        }
        // env: no git tree, no .sbx-commit, no EP9_SOURCE_COMMIT.
        let copy = tmp("dirty-env-copy");
        for (v, want) in [("1", Some(true)), ("true", Some(true)), ("0", Some(false))] {
            let s = source(&copy, &dirty(v));
            assert_eq!(
                (s.commit.as_deref(), s.from, s.dirty),
                (None, "env", want),
                "{v}"
            );
        }
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(sbx).unwrap();
        std::fs::remove_dir_all(copy).unwrap();
    }

    /// CX5-C6 (Codex round 1): `sbx push-git` extracts `git archive` of one commit (committed
    /// files only) and writes that commit to `<root>/.sbx-commit`. With no git tree and no
    /// EP9_SOURCE_COMMIT, the runner names it: not dirty, `commit_source` "sbx-commit". A
    /// line that is not 40 hex digits names nothing; the env goes first.
    #[test]
    fn a_sbx_push_git_tree_names_its_sbx_commit() {
        let d = tmp("sbx");
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(d.join(".sbx-commit"), format!("{sha}\n")).unwrap();
        let s = source(&d, &no_env);
        assert_eq!(
            (s.commit.as_deref(), s.dirty, s.from),
            (Some(sha), Some(false), "sbx-commit"),
            "{s:?}"
        );
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(
            out.starts_with(&format!(
                ",\"commit\":\"{sha}\",\"dirty\":false,\"commit_source\":\"sbx-commit\""
            )),
            "{out}"
        );
        // The env goes first.
        let other = "0123456789abcdef0123456789abcdef01234567";
        let env = |k: &str| match k {
            "EP9_SOURCE_COMMIT" => Some(other.to_string()),
            "EP9_SOURCE_DIRTY" => Some("1".to_string()),
            _ => None,
        };
        let s = source(&d, &env);
        assert_eq!(
            (s.commit.as_deref(), s.dirty, s.from),
            (Some(other), Some(true), "env")
        );
        // Not a full id: no commit, no dirty flag.
        std::fs::write(d.join(".sbx-commit"), "abc123\n").unwrap();
        let s = source(&d, &no_env);
        assert_eq!((s.commit.as_deref(), s.dirty), (None, None), "{s:?}");
        std::fs::remove_dir_all(d).unwrap();
    }
}
