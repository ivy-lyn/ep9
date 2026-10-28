//! Where a JSON summary came from (plan v9 section 7 step 3, P3-ORCH-14 and P3-ORCH-21): the
//! commit the ep9 tree is at when the runner runs, whether that tree had uncommitted changes,
//! and the host or CI job that ran it. The fields are read at run time, not at build time: a
//! binary built at one commit and run after a checkout of another names the other one. Run the
//! runner with `cargo run` (it rebuilds on a change) to make an m1-exit input. `cargo xtask m1-exit` items 3 and 4 read the three fields
//! and give OPEN "stale input" when the commit is missing, the tree was dirty, or the paths the
//! run depends on changed after the commit.
//!
//! - `"commit"`: `git rev-parse --verify HEAD` in the repo root (found at build time, as the
//!   default paths are). Left out when git cannot answer.
//! - `"dirty"`: `git status --porcelain --untracked-files=no` prints a line (a tracked file has
//!   an uncommitted change). Left out when git cannot answer.
//! - `"commit_source"`: `"git"`, or `"env"` when the repo root is no git tree (a copy of the
//!   tree, as on a sandbox box) and `EP9_SOURCE_COMMIT` (40 hex digits) and `EP9_SOURCE_DIRTY`
//!   (`0` or `1`) name them; the caller that copied the tree sets both.
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
pub const TREE_PATHS: [&str; 6] = [
    "crates",
    "tools/bench",
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
    /// "git" or "env".
    pub from: &'static str,
    pub made_by: String,
    /// (path, id at HEAD) for each of [`TREE_PATHS`] that HEAD has (git only).
    pub tree: Vec<(&'static str, String)>,
}

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

/// The facts of the tree at `root`; `env` reads an environment variable (a parameter, so the
/// tests do not touch the process environment).
pub fn source(root: &Path, env: &dyn Fn(&str) -> Option<String>) -> Source {
    let made_by = made_by(env);
    // A work tree of its own: `--show-toplevel` must be `root`, so a copy of the tree inside
    // another repository does not take that repository's commit.
    let top = git(root, &["rev-parse", "--show-toplevel"]);
    let same = top.as_deref().is_some_and(|t| {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        canon(Path::new(t)) == canon(root)
    });
    if same {
        let commit = git(root, &["rev-parse", "--verify", "HEAD^{commit}"]).filter(|c| full_sha(c));
        let dirty =
            git(root, &["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty());
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
        };
    }
    let commit = env("EP9_SOURCE_COMMIT")
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| full_sha(c));
    let dirty = match env("EP9_SOURCE_DIRTY").as_deref().map(str::trim) {
        Some("0") => Some(false),
        Some("1") => Some(true),
        _ => None,
    };
    Source {
        commit,
        dirty,
        from: "env",
        made_by,
        tree: Vec::new(),
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
/// dirty flag is left out, so m1-exit reads "no commit" and gives OPEN).
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
        // The temp tree has none of the six paths; with one, its id is named.
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
}
