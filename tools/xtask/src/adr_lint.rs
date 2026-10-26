#![allow(dead_code)] // m4-stub
//! `cargo xtask adr-lint` (M1 exit item 6: no H-row change without an approved ADR amendment;
//! plan v9 rule 26, V9-A15, V9-A16).
//!
//! ```text
//! cargo xtask adr-lint --trailers-only --base <rev> [--root <dir>]
//! ```
//!
//! M4-P0 builds only the trailer part (V9-A16). Every commit in `<rev>..HEAD` that touches an
//! H-row crate must carry a git trailer `ADR: Hnn[, Hmm]` (ids H1-H37, written `H2` or `H02`) or
//! `ADR: none (<reason>)`, or have a line in `expectations/adr-trailer-exceptions.txt`
//! (`<sha> <ADR ids> <decisions-log row>`, append only, written by the orchestrator; a missing
//! file has no lines). Every other mode exits 2 "not implemented yet": the full lint (the review
//! records of V9-A15, README Counts against the Status lines, path:line drift) is M4-P3 TOOLS.
//!
//! An H-row crate is a crate that the "Crates" row of an ADR names (docs/adr/README.md section 1:
//! "The ADR lint ... uses this line"): each `ep9_<name>` in backticks is `crates/ep9_<name>/`,
//! and a backticked path such as `tools/xtask` or `spikes/spike-b` counts when it holds a
//! `Cargo.toml`. A commit touches one when a path it adds, changes or deletes is inside it (a
//! rename counts on both sides). A merge commit touches only what its own resolution changed
//! (`--cc`); the commits it merges are checked one by one.
//!
//! `--root` (default: this workspace) is the git tree to check; the ADR files and the
//! exceptions file are read from its work tree. Exit 0: every such commit has a good trailer or
//! an exceptions line. Exit 1: the list of commits without one. Exit 2: a usage error, an unknown
//! `<rev>`, an exceptions line in another form, or no Crates row in `docs/adr/H*.md`.

use crate::exit::read;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "usage: cargo xtask adr-lint --trailers-only --base <rev> [--root <dir>]";

/// The exceptions file of rule 26 (orchestrator only, append only).
const EXCEPTIONS: &str = "expectations/adr-trailer-exceptions.txt";

/// The highest ADR id (H1-H37, docs/adr/README.md section 5).
const MAX_ADR: u32 = 37;

/// Entry point.
pub fn main(args: &[String]) -> Result<bool, String> {
    let mut trailers_only = false;
    let mut base = None;
    let mut root = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--trailers-only" => trailers_only = true,
            "--base" => base = Some(val()?),
            "--root" => root = Some(PathBuf::from(val()?)),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if !trailers_only {
        return Err(format!(
            "not implemented yet: M4-P0 builds only `--trailers-only` (the review records, \
             Counts and path:line checks are M4-P3 TOOLS)\n{USAGE}"
        ));
    }
    let base = base.ok_or_else(|| format!("--base <rev> is required\n{USAGE}"))?;
    let root = match root {
        Some(r) => r,
        None => {
            let r = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            std::fs::canonicalize(&r).unwrap_or(r)
        }
    };
    let report = trailers(&root, &base)?;
    print!("{}", report.text);
    Ok(report.missing == 0)
}

/// The result of the trailer check.
#[derive(Debug)]
struct Trailers {
    /// Commits in the range.
    commits: usize,
    /// Commits that touch an H-row crate.
    touching: usize,
    /// Of those: with a good trailer, by an exceptions line, without either.
    trailed: usize,
    excepted: usize,
    missing: usize,
    /// The report: one line per commit without a trailer, then the summary line.
    text: String,
}

/// The H-row crate dirs (each ending in `/`) and the ADRs that name each one.
fn hrow_crates(root: &Path) -> Result<BTreeMap<String, Vec<String>>, String> {
    let dir = root.join("docs/adr");
    let rd = std::fs::read_dir(&dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('H') && n.ends_with(".md"))
        })
        .collect();
    files.sort();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut rows = 0;
    for f in &files {
        let adr = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('-').next())
            .unwrap_or("")
            .to_string();
        for line in read(f)?.lines().filter(|l| l.starts_with("| Crates |")) {
            rows += 1;
            // The backticked names: every other piece of a split on '`'.
            for name in line.split('`').skip(1).step_by(2) {
                let dir = if name.starts_with("ep9_")
                    && name[4..]
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    && name.len() > 4
                {
                    format!("crates/{name}/")
                } else if name.contains('/')
                    && !name.contains("..")
                    && root
                        .join(name.trim_end_matches('/'))
                        .join("Cargo.toml")
                        .is_file()
                {
                    format!("{}/", name.trim_end_matches('/'))
                } else {
                    continue;
                };
                let names = out.entry(dir).or_default();
                if !names.contains(&adr) {
                    names.push(adr.clone());
                }
            }
        }
    }
    if rows == 0 {
        return Err(format!(
            "no \"| Crates |\" row in {}/H*.md, so no H-row crate is known",
            dir.display()
        ));
    }
    Ok(out)
}

/// Is `v` a good value of an `ADR:` trailer: `none (<reason>)`, or ADR ids `Hnn` (1-37) split
/// by commas?
fn good_value(v: &str) -> bool {
    let v = v.trim();
    if let Some(reason) = v.strip_prefix("none (").and_then(|r| r.strip_suffix(')')) {
        return !reason.trim().is_empty();
    }
    !v.is_empty()
        && v.split(',').all(|id| {
            let id = id.trim();
            id.strip_prefix('H').is_some_and(|n| {
                (1..=2).contains(&n.len())
                    && n.bytes().all(|b| b.is_ascii_digit())
                    && n.parse::<u32>().is_ok_and(|n| (1..=MAX_ADR).contains(&n))
            })
        })
}

/// The sha prefixes of the exceptions file (each line `<sha> <ADR ids> <decisions-log row>`).
fn exceptions(root: &Path) -> Result<Vec<String>, String> {
    let p = root.join(EXCEPTIONS);
    if !p.is_file() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (i, line) in read(&p)?.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = t.split_whitespace().collect();
        let sha = fields[0];
        if fields.len() < 3
            || !(7..=40).contains(&sha.len())
            || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(format!(
                "{EXCEPTIONS}:{}: not `<sha> <ADR ids> <decisions-log row>`: {t:?}",
                i + 1
            ));
        }
        out.push(sha.to_ascii_lowercase());
    }
    Ok(out)
}

/// Up to 5 ADR ids, for a finding line.
fn named_by(ids: &[String]) -> String {
    let mut t = ids.iter().take(5).cloned().collect::<Vec<_>>().join(" ");
    if ids.len() > 5 {
        t.push_str(&format!(" and {} more", ids.len() - 5));
    }
    t
}

/// Runs git in `root`; Err with its stderr on a failure.
fn git_out(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One commit of the range.
struct Commit {
    sha: String,
    subject: String,
    /// The values of its `ADR:` trailers.
    adr: Vec<String>,
    paths: Vec<String>,
}

/// The commits of `<base>..HEAD`, oldest first, with their trailers and the paths they touch.
fn commits(root: &Path, base: &str) -> Result<Vec<Commit>, String> {
    let base_sha = git_out(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
    )
    .map_err(|_| format!("--base {base:?} is not a commit of {}", root.display()))?;
    let range = format!("{}..HEAD", base_sha.trim());
    // Records split by 0x1e, fields by 0x1f, trailer values by 0x1d.
    let log = git_out(
        root,
        &[
            "log",
            "--reverse",
            "--format=%x1e%H%x1f%s%x1f%(trailers:key=ADR,valueonly,unfold,separator=%x1d)",
            &range,
        ],
    )?;
    let mut list: Vec<Commit> = Vec::new();
    for rec in log.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut f = rec.trim_end_matches('\n').splitn(3, '\u{1f}');
        let sha = f.next().unwrap_or("").to_string();
        let subject = f.next().unwrap_or("").to_string();
        let adr = f
            .next()
            .unwrap_or("")
            .split('\u{1d}')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect();
        list.push(Commit {
            sha,
            subject,
            adr,
            paths: Vec::new(),
        });
    }
    let names = git_out(
        root,
        &[
            "log",
            "--reverse",
            "--format=%x1e%H",
            "--name-only",
            "--no-renames",
            "--cc",
            &range,
        ],
    )?;
    let mut paths: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for rec in names.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let sha = lines.next().unwrap_or("").trim();
        paths.insert(
            sha,
            lines
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        );
    }
    for c in &mut list {
        c.paths = paths.remove(c.sha.as_str()).unwrap_or_default();
    }
    Ok(list)
}

/// The trailer check over `<base>..HEAD` in `root`.
fn trailers(root: &Path, base: &str) -> Result<Trailers, String> {
    let crates = hrow_crates(root)?;
    let exc = exceptions(root)?;
    let list = commits(root, base)?;
    let mut r = Trailers {
        commits: list.len(),
        touching: 0,
        trailed: 0,
        excepted: 0,
        missing: 0,
        text: String::new(),
    };
    for c in &list {
        let hits: Vec<(&String, &String)> = c
            .paths
            .iter()
            .filter_map(|p| {
                crates
                    .keys()
                    .find(|d| p.starts_with(d.as_str()))
                    .map(|d| (p, d))
            })
            .collect();
        let Some(&(first, dir)) = hits.first() else {
            continue;
        };
        r.touching += 1;
        let bad: Vec<&String> = c.adr.iter().filter(|v| !good_value(v)).collect();
        if !c.adr.is_empty() && bad.is_empty() {
            r.trailed += 1;
            continue;
        }
        if exc.iter().any(|e| c.sha.starts_with(e.as_str())) {
            r.excepted += 1;
            continue;
        }
        r.missing += 1;
        let why = if c.adr.is_empty() {
            "no `ADR:` trailer".to_string()
        } else {
            format!(
                "a bad `ADR:` trailer {:?} (want `ADR: Hnn[, Hmm]` or `ADR: none (<reason>)`)",
                bad[0]
            )
        };
        r.text.push_str(&format!(
            "{} {}: touches {first}{} (H-row crate {dir}, named by {}); {why}\n",
            &c.sha[..c.sha.len().min(12)],
            c.subject,
            if hits.len() > 1 {
                format!(" and {} more H-row path(s)", hits.len() - 1)
            } else {
                String::new()
            },
            named_by(&crates[dir])
        ));
    }
    r.text.push_str(&format!(
        "adr-lint --trailers-only: {base}..HEAD: {} commits, {} touch an H-row crate ({} crate dirs): \
         {} with an ADR trailer, {} by an {EXCEPTIONS} line, {} without{}\n",
        r.commits,
        r.touching,
        crates.len(),
        r.trailed,
        r.excepted,
        r.missing,
        if r.missing == 0 { ": PASS" } else { ": FAIL" }
    ));
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture ADR with a Crates row (no live ADR file is read, plan rule 24).
    const ADR: &str = "# H16\n\n| Field | Value |\n|---|---|\n\
        | Crates | `ep9_vm` (ICs), `ep9_unit`; `tools/xtask` (`disasm-check`); `crates/`, `tools/none` |\n";

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=adr-lint test",
                "-c",
                "user.email=adr-lint@test.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Writes `files` and commits them with `msg`; the new HEAD.
    fn commit(root: &Path, files: &[(&str, &str)], msg: &str) -> String {
        for (p, t) in files {
            write(root, p, t);
        }
        git(root, &["add", "-A"]);
        git(
            root,
            &["commit", "-q", "--no-verify", "--allow-empty", "-m", msg],
        );
        git(root, &["rev-parse", "HEAD"])
    }

    /// A fixture repo: the ADR, an H-row crate, a tool crate, a doc. Returns (root, base).
    fn repo(name: &str) -> (PathBuf, String) {
        let root = std::env::temp_dir().join(format!("ep9-adr-lint-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        let base = commit(
            &root,
            &[
                ("docs/adr/H16-ics.md", ADR),
                ("crates/ep9_vm/src/lib.rs", "// vm\n"),
                ("crates/ep9_other/src/lib.rs", "// not named\n"),
                ("tools/xtask/Cargo.toml", "[package]\n"),
                ("tools/bench/x.mjs", "// bench\n"),
                ("README.md", "# r\n"),
            ],
            "base",
        );
        (root, base)
    }

    #[test]
    fn crates_rows_name_the_hrow_dirs() {
        let (root, _) = repo("crates");
        let c = hrow_crates(&root).unwrap();
        // `crates/` and `tools/none` hold no Cargo.toml; `ep9_unit` is named but not built yet.
        assert_eq!(
            c.keys().map(String::as_str).collect::<Vec<_>>(),
            ["crates/ep9_unit/", "crates/ep9_vm/", "tools/xtask/"]
        );
        assert_eq!(c["crates/ep9_vm/"], ["H16"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn trailer_values() {
        for ok in ["H16", "H02, H20", "H2,H37", "none (tooling only)", " H1 "] {
            assert!(good_value(ok), "{ok}");
        }
        for bad in [
            "",
            "H",
            "H38",
            "H0",
            "H123",
            "h16",
            "16",
            "none",
            "none ()",
            "none ( )",
            "H16 part 2",
            "H16;H20",
            "maybe",
        ] {
            assert!(!good_value(bad), "{bad}");
        }
    }

    /// V9-A16 acceptance: a fixture commit without a trailer fails; with one, or with an
    /// exceptions line, it passes.
    #[test]
    fn a_commit_without_a_trailer_fails() {
        let (root, base) = repo("missing");
        commit(&root, &[("README.md", "# r2\n")], "docs: no trailer needed");
        commit(
            &root,
            &[("crates/ep9_other/src/lib.rs", "// 2\n")],
            "other: a crate no Crates row names",
        );
        commit(
            &root,
            &[("tools/bench/x.mjs", "// 2\n")],
            "bench: not a crate any row names",
        );
        commit(
            &root,
            &[("crates/ep9_vm/src/lib.rs", "// 2\n")],
            "vm: a change\n\nBody.\n\nADR: H16\nCo-Authored-By: X <x@test.invalid>",
        );
        commit(
            &root,
            &[("tools/xtask/src/main.rs", "// 2\n")],
            "xtask: a change\n\nADR: none (tooling)\nCo-Authored-By: X <x@test.invalid>",
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!(
            (r.commits, r.touching, r.trailed, r.missing),
            (5, 2, 2, 0),
            "{}",
            r.text
        );
        assert!(r.text.ends_with(": PASS\n"), "{}", r.text);
        // The fixture commit without a trailer.
        let bad = commit(
            &root,
            &[("crates/ep9_vm/src/new.rs", "// new\n")],
            "vm: no trailer\n\nCo-Authored-By: X <x@test.invalid>",
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.touching, r.missing), (3, 1), "{}", r.text);
        assert!(r.text.contains(&bad[..12]), "{}", r.text);
        assert!(r.text.contains("crates/ep9_vm/src/new.rs"), "{}", r.text);
        assert!(r.text.contains("no `ADR:` trailer"), "{}", r.text);
        assert!(r.text.ends_with(": FAIL\n"), "{}", r.text);
        // An exceptions line covers it (orchestrator, rule 26).
        write(
            &root,
            EXCEPTIONS,
            &format!("# sha ids row\n{} H16 M4-P0-TRAILER-1\n", &bad[..10]),
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.excepted, r.missing), (1, 0), "{}", r.text);
        // A line in another form is a usage error.
        write(&root, EXCEPTIONS, &format!("{}\n", &bad[..10]));
        assert!(trailers(&root, &base).unwrap_err().contains(EXCEPTIONS));
        std::fs::remove_file(root.join(EXCEPTIONS)).unwrap();
        // A bad value, a trailer in the body only, a rename out of an H-row crate.
        for (file, msg) in [
            ("crates/ep9_vm/src/a.rs", "vm: bad\n\nADR: H99"),
            (
                "crates/ep9_vm/src/b.rs",
                "vm: body only\n\nADR: H16 is named here.\nBut this paragraph is no trailer block.",
            ),
        ] {
            let (root, base) = repo("bad-value");
            commit(&root, &[(file, "// x\n")], msg);
            let r = trailers(&root, &base).unwrap();
            assert_eq!(r.missing, 1, "{msg}: {}", r.text);
            std::fs::remove_dir_all(&root).unwrap();
        }
        let (root2, base2) = repo("rename");
        git(&root2, &["mv", "crates/ep9_vm/src/lib.rs", "moved.rs"]);
        git(&root2, &["commit", "-q", "--no-verify", "-m", "move out"]);
        let r = trailers(&root2, &base2).unwrap();
        assert_eq!(r.missing, 1, "{}", r.text);
        std::fs::remove_dir_all(&root2).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A merge commit is checked only for what its own resolution changed.
    #[test]
    fn merges_are_checked_by_their_own_changes() {
        let (root, base) = repo("merge");
        git(&root, &["checkout", "-q", "-b", "side"]);
        commit(
            &root,
            &[("crates/ep9_vm/src/lib.rs", "// side\n")],
            "vm: side\n\nADR: H16",
        );
        git(&root, &["checkout", "-q", "-"]);
        commit(&root, &[("README.md", "# main\n")], "docs: main");
        git(&root, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (3, 1, 0), "{}", r.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn modes_and_usage() {
        let a = |s: &[&str]| main(&s.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(a(&[]).unwrap_err().contains("not implemented yet"));
        assert!(
            a(&["--base", "x"])
                .unwrap_err()
                .contains("not implemented yet")
        );
        assert!(a(&["--trailers-only"]).unwrap_err().contains("--base"));
        assert!(
            a(&["--trailers-only", "--what"])
                .unwrap_err()
                .contains("unknown")
        );
        let (root, _) = repo("usage");
        let r = root.to_str().unwrap();
        assert!(
            a(&["--trailers-only", "--base", "no-such-rev", "--root", r])
                .unwrap_err()
                .contains("not a commit")
        );
        assert!(a(&["--trailers-only", "--base", "HEAD", "--root", r]).unwrap());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
