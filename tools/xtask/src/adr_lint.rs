//! `cargo xtask adr-lint` (M1 exit item 6: no H-row change without an approved ADR amendment;
//! plan v9 rule 26, V9-A15, V9-A16).
//!
//! ```text
//! cargo xtask adr-lint --base <rev> [--root <dir>]                  the full lint (M4-P3)
//! cargo xtask adr-lint --trailers-only --base <rev> [--root <dir>]  part 1 only (M4-P0)
//! ```
//!
//! The full lint runs five parts over `<rev>..HEAD` and passes only when all five pass (part 5
//! only while it gates):
//!
//! | Part | Passes when | Module |
//! |---|---|---|
//! | 1 trailers | every commit that touches an H-row crate has an `ADR:` trailer, or a line in `expectations/adr-trailer-exceptions.txt` (below) | this file |
//! | 2 review records | every amendment row of an Accepted or Frozen ADR since `<rev>` has review rows 1-4 and an approval by an agent that wrote none of them (V9-A15; README sections 3.1, 3.2), and such an ADR whose text outside its Change log and Review record changed has a new amendment row | `exit::records` |
//! | 3 Counts | the README Counts line and index column equal the Status lines | `exit::records` |
//! | 4 path:line drift | every `path:line` reference of `docs/adr` and `docs/plan` names the same line at HEAD as when it was written (no FIX and no UNFIXABLE line of the ADRLINE dry run) | `exit::drift` |
//! | 5 short cites | every short cite of `docs/plan/13-execution-modes.md` (`H31:66`, `AR:37`, ...) names the same line at HEAD as at the commit its header names (MC-w16-8). It gates from M4-P5 (`exit::cites::GATE` true); with `GATE` false it is report only: `note:` lines, no effect on the result | `exit::cites` |
//!
//! **Part 1.** Every commit in `<rev>..HEAD` that touches an H-row crate must carry a git trailer
//! `ADR: Hnn[, Hmm]` (ids H1-H37, written `H2` or `H02`) or `ADR: none (<reason>)`, or have a
//! line in `expectations/adr-trailer-exceptions.txt` (`<sha> <ADR ids> <decisions-log row>`,
//! append only, written by the orchestrator; a missing file has no lines).
//!
//! An H-row crate is a crate that the "Crates" row of an ADR names (docs/adr/README.md section 1:
//! "The ADR lint ... uses this line"): each `ep9_<name>` in backticks is `crates/ep9_<name>/`,
//! and a backticked path such as `tools/xtask` or `spikes/spike-b` counts when it holds a
//! `Cargo.toml` in the work tree or at `<rev>`, or a commit of the range adds, changes or deletes
//! its `Cargo.toml` (so a tool crate deleted in the range still counts). A commit touches one
//! when a path it adds, changes or deletes is inside it (a rename counts on both sides; paths
//! are read NUL-split from git, never in git's quoted form, by git calls on one commit at a
//! time, so no byte of a path or a message can move a path or a trailer to another commit). A
//! merge commit of 2 parents touches only the paths where its tree differs from `git merge-tree
//! --write-tree` of its parents (a clean merge touches nothing; `--cc` only when that git call
//! fails, with a `note:` line); an octopus merge touches every path that differs from its first
//! parent (`commit_paths`). The commits a merge brings in are checked one by one.
//!
//! `--root` (default: this workspace) is the git tree to check; the ADR files, the docs and the
//! exceptions file are read from its work tree. Exit 0: every part passes. Exit 1: the findings.
//! Exit 2: a usage error, an unknown `<rev>`, an exceptions line in another form, no Crates row
//! in `docs/adr/H*.md`, or a pin table line in another form.

use crate::exit::{cites, drift, read, records};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "usage: cargo xtask adr-lint [--trailers-only] --base <rev> [--root <dir>]";

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
    let base = base.ok_or_else(|| format!("--base <rev> is required\n{USAGE}"))?;
    let root = match root {
        Some(r) => r,
        None => {
            let r = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            std::fs::canonicalize(&r).unwrap_or(r)
        }
    };
    if trailers_only {
        let report = trailers(&root, &base)?;
        print!("{}", report.text);
        return Ok(report.missing == 0);
    }
    let lint = full(&root, &base)?;
    print!("{}", lint.text);
    Ok(lint.pass)
}

/// The result of the full lint.
#[derive(Debug)]
pub(crate) struct Lint {
    pub(crate) pass: bool,
    /// The findings of each part, then one summary line per part and the result line.
    pub(crate) text: String,
    /// The result line (the last line of `text`).
    pub(crate) summary: String,
}

/// The five parts over `<base>..HEAD` in `root`.
pub(crate) fn full(root: &Path, base: &str) -> Result<Lint, String> {
    let t = trailers(root, base)?;
    let base_sha = base_commit(root, base)?;
    let rec = records::records(root, &base_sha)?;
    let (counts_line, from_files, count_findings) = records::counts(root)?;
    let d = drift::drift(root)?;
    let sc = cites::part(root)?;
    let mut text = String::new();
    // Part 1 prints its own summary line last.
    text.push_str(&t.text);
    for g in &rec.gaps {
        text.push_str(&format!("{g}\n"));
    }
    let rec_ok = rec.gaps.is_empty();
    text.push_str(&format!(
        "adr-lint review records: {base}..HEAD: {} Accepted or Frozen ADRs, {} amendment rows ({} rows need none: record-only rows and edits of older rows), {} gaps{}\n",
        rec.adrs,
        rec.rows,
        rec.skipped,
        rec.gaps.len(),
        pass_word(rec_ok)
    ));
    for f in &count_findings {
        text.push_str(&format!("{f}\n"));
    }
    let counts_ok = count_findings.is_empty();
    text.push_str(&format!(
        "adr-lint Counts: README {counts_line:?}; Status lines: {from_files}{}\n",
        pass_word(counts_ok)
    ));
    for l in &d.lines {
        text.push_str(&format!("{l}\n"));
    }
    text.push_str(&format!(
        "adr-lint path:line drift: {} (pins: {}){}\n",
        d.summary(),
        drift::PINS_FILE,
        pass_word(d.pass())
    ));
    for l in sc.lines() {
        text.push_str(&format!("{l}\n"));
    }
    text.push_str(&format!(
        "adr-lint short cites: {}{}\n",
        sc.summary(),
        pass_word(sc.pass())
    ));
    let pass = t.missing == 0 && rec_ok && counts_ok && d.pass() && sc.pass();
    // Part 5 joins the result line when it gates (exit::cites::GATE).
    let short = if cites::GATE {
        format!(", short cites {}", word(sc.pass()))
    } else {
        String::new()
    };
    let summary = format!(
        "adr-lint: {base}..HEAD: trailers {}, review records {}, Counts {}, path:line drift {}{short}: {}",
        word(t.missing == 0),
        word(rec_ok),
        word(counts_ok),
        word(d.pass()),
        word(pass)
    );
    text.push_str(&summary);
    text.push('\n');
    Ok(Lint {
        pass,
        text,
        summary,
    })
}

fn word(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

fn pass_word(ok: bool) -> &'static str {
    if ok { ": PASS" } else { ": FAIL" }
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

/// The H-row crate dirs (each ending in `/`) and the ADRs that name each one. A backticked path
/// counts when `<path>/Cargo.toml` is a file in the work tree, or `held(<path>/Cargo.toml)`
/// (the file is in the base tree or a commit of the range touches it).
fn hrow_crates(
    root: &Path,
    held: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, String> {
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
                } else if name.contains('/') && !name.contains("..") && {
                    let manifest = format!("{}/Cargo.toml", name.trim_end_matches('/'));
                    root.join(&manifest).is_file() || held(&manifest)
                } {
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
    /// A note line for the report: a merge whose paths came from `--cc`, or an octopus merge.
    note: Option<String>,
}

/// The full sha of the commit `base` names in `root`.
fn base_commit(root: &Path, base: &str) -> Result<String, String> {
    git_out(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
    )
    .map(|s| s.trim().to_string())
    .map_err(|_| format!("--base {base:?} is not a commit of {}", root.display()))
}

/// The commits of `<base_sha>..HEAD`, oldest first, with their trailers and the paths they touch.
///
/// The sha list comes from `git rev-list` (hex lines only). Each commit's subject, trailers and
/// paths come from git calls on that one sha, so no byte of a message or a path can start a new
/// record or move a path to another commit (a single `git log` stream split on a marker byte let a
/// file named 0x1e + an earlier sha take the paths after it to that commit).
fn commits(root: &Path, base_sha: &str) -> Result<Vec<Commit>, String> {
    let range = format!("{base_sha}..HEAD");
    let shas = git_out(root, &["rev-list", "--reverse", &range])?;
    let mut list: Vec<Commit> = Vec::new();
    for sha in shas.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("git rev-list {range}: not a sha: {sha:?}"));
        }
        let subject = git_out(root, &["show", "-s", "--format=%s", sha, "--"])?
            .trim_end_matches('\n')
            .to_string();
        // Trailer values split by NUL (a message given to `git commit` holds no NUL).
        let adr = git_out(
            root,
            &[
                "show",
                "-s",
                "--format=%(trailers:key=ADR,valueonly,unfold,separator=%x00)",
                sha,
                "--",
            ],
        )?
        .split('\0')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
        let (paths, note) = commit_paths(root, sha)?;
        list.push(Commit {
            sha: sha.to_string(),
            subject,
            adr,
            paths,
            note,
        });
    }
    Ok(list)
}

/// The NUL-split paths of a `git diff-tree -z -r --name-only --no-renames <args> --` call.
///
/// `-z`: every path ends in NUL and is never quoted (without it git writes a path with a
/// non-ASCII byte, a tab, `"` or `\\` as a C string in quotes).
fn diff_paths(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let mut all = vec![
        "diff-tree",
        "-z",
        "-r",
        "--no-commit-id",
        "--name-only",
        "--no-renames",
    ];
    all.extend_from_slice(args);
    all.push("--");
    Ok(git_out(root, &all)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// The paths that `sha` itself adds, changes or deletes, and a note for the report when a merge
/// fell back to `--cc` or is an octopus merge.
///
/// - A commit with 0 or 1 parents: its diff (`--root`: a root commit lists every path it adds).
/// - A merge of 2 parents: the paths where its tree differs from `git merge-tree --write-tree
///   <p1> <p2>` (git 2.38 or later), the tree a clean automatic merge gives. A clean merge whose
///   tree equals that tree touches nothing; a conflict resolution or any other change of the
///   merge lists the paths it changed (a conflicted path holds the conflict markers in that
///   tree, so its resolution always differs). `--cc --name-only` is not used: it lists every
///   path that both parents changed, even when git merged it with no conflict. Only when
///   `git merge-tree` fails (an older git, or no tree on its first line) do the paths come from
///   `--cc`, and the note says so.
/// - An octopus merge (3 or more parents): `git merge-tree` merges only 2, so the lint does not
///   split its own changes from what it merges in. It counts every path where its tree differs
///   from its first parent (all it brings in), and the note says so: an octopus merge that
///   brings in an H-row path needs its own trailer.
fn commit_paths(root: &Path, sha: &str) -> Result<(Vec<String>, Option<String>), String> {
    let parents_line = git_out(root, &["show", "-s", "--format=%P", sha, "--"])?;
    let parents: Vec<&str> = parents_line.split_whitespace().collect();
    let short = &sha[..sha.len().min(12)];
    match parents.as_slice() {
        [] | [_] => Ok((diff_paths(root, &["--root", sha])?, None)),
        [p1, p2] => match merge_tree(root, p1, p2) {
            Ok(tree) => Ok((diff_paths(root, &[&tree, sha])?, None)),
            Err(e) => Ok((
                diff_paths(root, &["--cc", sha])?,
                Some(format!(
                    "note: {short}: `git merge-tree --write-tree` failed ({e}); this merge's paths are the `--cc --name-only` list, which also names paths both parents changed with no conflict"
                )),
            )),
        },
        [p1, ..] => Ok((
            diff_paths(root, &[p1, sha])?,
            Some(format!(
                "note: {short}: an octopus merge of {} parents; its paths are every path that differs from its first parent",
                parents.len()
            )),
        )),
    }
}

/// The tree of a clean automatic merge of `p1` and `p2` (conflicted paths hold the conflict
/// markers): the first line of `git merge-tree --write-tree`. Exit 0 is a clean merge, exit 1 a
/// merge with conflicts (the tree is still printed); any other exit is a failure.
fn merge_tree(root: &Path, p1: &str, p2: &str) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "merge-tree",
            "--write-tree",
            "--no-messages",
            "--allow-unrelated-histories",
            p1,
            p2,
        ])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let tree = stdout.lines().next().unwrap_or("").trim();
    let ok_exit = matches!(out.status.code(), Some(0 | 1));
    if ok_exit && matches!(tree.len(), 40 | 64) && tree.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(tree.to_string())
    } else {
        Err(format!(
            "{}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The trailer check over `<base>..HEAD` in `root`.
fn trailers(root: &Path, base: &str) -> Result<Trailers, String> {
    let base_sha = base_commit(root, base)?;
    let list = commits(root, &base_sha)?;
    let touched: std::collections::BTreeSet<&str> = list
        .iter()
        .flat_map(|c| c.paths.iter().map(String::as_str))
        .collect();
    let crates = hrow_crates(root, |manifest| {
        touched.contains(manifest)
            || git_out(root, &["cat-file", "-e", &format!("{base_sha}:{manifest}")]).is_ok()
    })?;
    let exc = exceptions(root)?;
    let mut r = Trailers {
        commits: list.len(),
        touching: 0,
        trailed: 0,
        excepted: 0,
        missing: 0,
        text: String::new(),
    };
    for c in &list {
        if let Some(n) = &c.note {
            r.text.push_str(&format!("{n}\n"));
        }
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

    const ADR_PATH: &str = "docs/adr/H16-ics.md";

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn git(root: &Path, args: &[&str]) -> String {
        git_in(root, args, b"")
    }

    /// Runs git in `root` with `input` on its stdin; its stdout, trimmed.
    fn git_in(root: &Path, args: &[&str], input: &[u8]) -> String {
        let (ok, out) = git_run(root, args, input);
        assert!(ok, "git {args:?}: {out}");
        out
    }

    /// Runs git in `root` with `input` on its stdin: (exit 0, stdout trimmed or the whole output).
    fn git_run(root: &Path, args: &[&str], input: &[u8]) -> (bool, String) {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new("git")
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
                "-c",
                "rerere.enabled=false",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Dropping stdin after the write closes it.
        child.stdin.take().unwrap().write_all(input).unwrap();
        let out = child.wait_with_output().unwrap();
        if out.status.success() {
            (
                true,
                String::from_utf8(out.stdout).unwrap().trim().to_string(),
            )
        } else {
            (false, format!("{out:?}"))
        }
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

    /// Like `commit`, but puts `files` into the git index only, never into the work tree: Windows
    /// allows no file name with `"`, a tab or a byte below 0x20, and takes `\` as a separator.
    /// Blobs come from `hash-object -w --stdin`; paths and the message go on stdin
    /// (`update-index -z --index-info`, `commit -F -`), not as arguments. Git for Windows also
    /// refuses such a path in the index unless `core.protectNTFS` is off. The work tree lacks
    /// these paths, so a later `commit` (`git add -A`) would stage them as deleted: a test that
    /// uses this keeps to it after its first call.
    fn commit_index(root: &Path, files: &[(&str, &str)], msg: &str) -> String {
        const NTFS: [&str; 2] = ["-c", "core.protectNTFS=false"];
        let mut info = String::new();
        for (p, t) in files {
            let blob = git_in(root, &["hash-object", "-w", "--stdin"], t.as_bytes());
            info.push_str(&format!("100644 blob {blob}\t{p}\0"));
        }
        git_in(
            root,
            &[NTFS[0], NTFS[1], "update-index", "-z", "--index-info"],
            info.as_bytes(),
        );
        git_in(
            root,
            &[
                NTFS[0],
                NTFS[1],
                "commit",
                "-q",
                "--no-verify",
                "--allow-empty",
                "-F",
                "-",
            ],
            msg.as_bytes(),
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
                (ADR_PATH, ADR),
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
        let c = hrow_crates(&root, |_| false).unwrap();
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

    /// Git quotes a path with a byte outside printable ASCII (`core.quotePath`) or with a tab,
    /// quote or backslash; such a path inside an H-row crate still needs the trailer.
    #[test]
    fn quoted_paths_still_count() {
        for file in [
            "crates/ep9_vm/src/\u{3c0}.rs",
            "crates/ep9_vm/src/a\tb.rs",
            "crates/ep9_vm/src/a\"b.rs",
            "crates/ep9_vm/src/a\\b.rs",
            "tools/xtask/src/\u{e9}t\u{e9}.rs",
        ] {
            let (root, base) = repo("quoted");
            commit_index(&root, &[(file, "// x\n")], "no trailer");
            let r = trailers(&root, &base).unwrap();
            assert_eq!((r.touching, r.missing), (1, 1), "{file:?}: {}", r.text);
            assert!(r.text.contains(file), "{file:?}: {}", r.text);
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    /// A tool crate (a Crates-row path with a `Cargo.toml`) deleted in the range stays an H-row
    /// crate for every commit of the range, the deleting one included.
    #[test]
    fn a_deleted_tool_crate_still_counts() {
        let (root, base) = repo("deleted");
        commit(
            &root,
            &[("tools/xtask/src/main.rs", "// 2\n")],
            "xtask: no trailer",
        );
        git(&root, &["rm", "-q", "-r", "tools/xtask"]);
        git(
            &root,
            &[
                "commit",
                "-q",
                "--no-verify",
                "-m",
                "xtask: delete, no trailer",
            ],
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (2, 2, 2), "{}", r.text);
        assert!(r.text.contains("tools/xtask/Cargo.toml"), "{}", r.text);
        std::fs::remove_dir_all(&root).unwrap();
        // A tool crate in the base tree counts while the work tree lacks it (not committed).
        let (root, base) = repo("worktree-gone");
        commit(
            &root,
            &[("tools/xtask/src/main.rs", "// 2\n")],
            "xtask: no trailer",
        );
        std::fs::remove_dir_all(root.join("tools/xtask")).unwrap();
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.touching, r.missing), (1, 1), "{}", r.text);
        std::fs::remove_dir_all(&root).unwrap();
        // A tool crate added in the range and deleted again in it counts too.
        let (root, _) = repo("added-deleted");
        write(
            &root,
            ADR_PATH,
            &format!("{ADR}| Crates | `tools/later` |\n"),
        );
        let b2 = commit(&root, &[], "docs: name tools/later\n\nADR: H16");
        commit(
            &root,
            &[("tools/later/Cargo.toml", "[package]\n")],
            "later: add, no trailer",
        );
        git(&root, &["rm", "-q", "-r", "tools/later"]);
        git(
            &root,
            &[
                "commit",
                "-q",
                "--no-verify",
                "-m",
                "later: delete, no trailer",
            ],
        );
        let r = trailers(&root, &b2).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (2, 2, 2), "{}", r.text);
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

    /// A crate file of 3 lines, each set apart by a blank line so that git merges edits of
    /// different lines with no conflict.
    fn three(a: &str, b: &str, c: &str) -> String {
        format!("// {a}\n\n// keep 1\n\n// keep 2\n\n// {b}\n\n// keep 3\n\n// keep 4\n\n// {c}\n")
    }

    /// M4-P3 review (jit-tools finding 2): a clean merge where both sides edited the same crate
    /// file touches nothing (`--cc --name-only` listed that file, so merge 55c4965 failed); a
    /// conflict resolution in a crate file needs a trailer, and so does a merge that drops a
    /// side's change (`-s ours`).
    #[test]
    fn a_merge_touches_only_what_differs_from_the_automatic_merge() {
        let (root, base) = repo("merge-tree");
        let lib = "crates/ep9_vm/src/lib.rs";
        commit(
            &root,
            &[(lib, &three("a", "b", "c"))],
            "vm: three\n\nADR: H16",
        );
        let main = git(&root, &["symbolic-ref", "--short", "HEAD"]);
        git(&root, &["checkout", "-q", "-b", "side"]);
        commit(
            &root,
            &[(lib, &three("a", "b", "side"))],
            "vm: side\n\nADR: H16",
        );
        git(&root, &["checkout", "-q", &main]);
        commit(
            &root,
            &[(lib, &three("main", "b", "c"))],
            "vm: main\n\nADR: H16",
        );
        // The clean merge, no trailer: both sides touched lib.rs, the merge itself nothing.
        git(
            &root,
            &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        );
        let clean = git(&root, &["rev-parse", "HEAD"]);
        assert_eq!(
            read(&root.join(lib)).unwrap(),
            three("main", "b", "side"),
            "the fixture merge is not clean"
        );
        let cc = git(
            &root,
            &[
                "diff-tree",
                "-r",
                "--no-commit-id",
                "--name-only",
                "--cc",
                &clean,
            ],
        );
        assert_eq!(cc, lib, "--cc --name-only lists the file of a clean merge");
        let (paths, note) = commit_paths(&root, &clean).unwrap();
        assert_eq!((paths.len(), note), (0, None));
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (4, 3, 0), "{}", r.text);
        // A real conflict: both sides change the middle line; the merge writes a third text.
        git(&root, &["checkout", "-q", "-b", "side2"]);
        commit(
            &root,
            &[(lib, &three("main", "s2", "side"))],
            "vm: s2\n\nADR: H16",
        );
        git(&root, &["checkout", "-q", &main]);
        commit(
            &root,
            &[(lib, &three("main", "m2", "side"))],
            "vm: m2\n\nADR: H16",
        );
        let (ok, _) = git_run(
            &root,
            &["merge", "-q", "--no-ff", "--no-edit", "side2"],
            b"",
        );
        assert!(!ok, "the fixture merge has no conflict");
        let resolved = three("main", "both", "side");
        let fixed = commit(&root, &[(lib, &resolved)], "merge side2, resolved");
        assert_eq!(
            git(&root, &["show", "-s", "--format=%P", &fixed])
                .split(' ')
                .count(),
            2
        );
        assert_eq!(
            commit_paths(&root, &fixed).unwrap(),
            (vec![lib.to_string()], None)
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (7, 6, 1), "{}", r.text);
        assert!(
            r.text.contains(&format!(
                "{} merge side2, resolved: touches {lib}",
                &fixed[..12]
            )),
            "{}",
            r.text
        );
        // The same resolution with a trailer passes.
        git(
            &root,
            &[
                "commit",
                "-q",
                "--amend",
                "--no-verify",
                "-m",
                "merge side2, resolved\n\nADR: H16",
            ],
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (7, 6, 0), "{}", r.text);
        // `-s ours` drops the side's change: that is the merge's own change of lib.rs.
        git(&root, &["checkout", "-q", "-b", "side3"]);
        commit(
            &root,
            &[(lib, &three("s3", "both", "side"))],
            "vm: s3\n\nADR: H16",
        );
        git(&root, &["checkout", "-q", &main]);
        git(
            &root,
            &[
                "merge",
                "-q",
                "--no-ff",
                "-s",
                "ours",
                "-m",
                "merge side3, ours",
                "side3",
            ],
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (9, 8, 1), "{}", r.text);
        assert!(r.text.contains("merge side3, ours: touches"), "{}", r.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An octopus merge counts every path that differs from its first parent, with a note.
    #[test]
    fn an_octopus_merge_counts_all_it_brings_in() {
        let (root, base) = repo("octopus");
        let main = git(&root, &["symbolic-ref", "--short", "HEAD"]);
        for (b, f) in [("o1", "crates/ep9_vm/src/o1.rs"), ("o2", "README.md")] {
            git(&root, &["checkout", "-q", "-b", b, &main]);
            commit(&root, &[(f, "// o\n")], &format!("{b}\n\nADR: H16"));
        }
        git(&root, &["checkout", "-q", &main]);
        git(
            &root,
            &["merge", "-q", "--no-ff", "-m", "octopus", "o1", "o2"],
        );
        let head = git(&root, &["rev-parse", "HEAD"]);
        assert_eq!(
            git(&root, &["show", "-s", "--format=%P", &head])
                .split(' ')
                .count(),
            3
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (3, 2, 1), "{}", r.text);
        assert!(
            r.text.contains("an octopus merge of 3 parents"),
            "{}",
            r.text
        );
        assert!(
            r.text.contains("octopus: touches crates/ep9_vm/src/o1.rs"),
            "{}",
            r.text
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Codex r2: a file named 0x1e + an earlier sha (after a file that sorts before it) made the
    /// paths after it count for that earlier commit, so a commit without a trailer passed. Bytes
    /// of a path or a subject never change which commit a path or a trailer belongs to.
    #[test]
    fn names_cannot_move_paths_or_trailers() {
        let (root, base) = repo("spoof");
        let a = commit(
            &root,
            &[("crates/ep9_vm/src/lib.rs", "// a\n")],
            "vm: one\n\nADR: H16",
        );
        let spoof = format!("\u{1e}{a}");
        let b = commit_index(
            &root,
            &[
                ("\u{1}", "x"),
                (spoof.as_str(), "x"),
                ("crates/ep9_vm/src/lib.rs", "// b\n"),
            ],
            "vm: two, no trailer",
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (2, 2, 1), "{}", r.text);
        assert!(r.text.contains(&b[..12]), "{}", r.text);
        assert!(r.text.ends_with(": FAIL\n"), "{}", r.text);
        // Marker bytes in a subject do not make a trailer.
        let c = commit_index(
            &root,
            &[("crates/ep9_vm/src/lib.rs", "// c\n")],
            &format!("vm: three\u{1f}H16\u{1e}{a}\u{1f}x\u{1f}H16"),
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (3, 3, 2), "{}", r.text);
        assert!(r.text.contains(&c[..12]), "{}", r.text);
        std::fs::remove_dir_all(&root).unwrap();
        // A root commit merged in from an unrelated history lists every path it adds.
        let (root, base) = repo("root");
        let main = git(&root, &["symbolic-ref", "--short", "HEAD"]);
        git(&root, &["checkout", "-q", "--orphan", "other"]);
        git(&root, &["rm", "-q", "-r", "--cached", "."]);
        for e in std::fs::read_dir(&root).unwrap() {
            let p = e.unwrap().path();
            if p.file_name().unwrap() != ".git" {
                if p.is_dir() {
                    std::fs::remove_dir_all(&p).unwrap();
                } else {
                    std::fs::remove_file(&p).unwrap();
                }
            }
        }
        commit(
            &root,
            &[("crates/ep9_vm/src/root.rs", "// root\n")],
            "vm: a root, no trailer",
        );
        git(&root, &["checkout", "-q", &main]);
        git(
            &root,
            &[
                "merge",
                "-q",
                "--no-ff",
                "--no-edit",
                "--allow-unrelated-histories",
                "other",
            ],
        );
        let r = trailers(&root, &base).unwrap();
        assert_eq!((r.commits, r.touching, r.missing), (2, 1, 1), "{}", r.text);
        assert!(r.text.contains("crates/ep9_vm/src/root.rs"), "{}", r.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn modes_and_usage() {
        let a = |s: &[&str]| main(&s.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(a(&[]).unwrap_err().contains("--base"));
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
        // The full lint needs the README (part 3): the trailer fixture has none.
        assert!(
            a(&["--base", "HEAD", "--root", r])
                .unwrap_err()
                .contains("README")
        );
        assert!(
            a(&["--base", "no-such-rev", "--root", r])
                .unwrap_err()
                .contains("not a commit")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The full-lint fixtures (plan v9 rule 24).
    fn fixture(name: &str) -> String {
        read(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/adr_lint")
                .join(name),
        )
        .unwrap()
    }

    const H16: &str = "docs/adr/H16-ics.md";
    const H29: &str = "docs/adr/H29-deopt.md";
    const README: &str = "docs/adr/README.md";
    const LIB: &str = "crates/ep9_vm/src/lib.rs";

    /// A fixture tree for the full lint: (root, base sha, the base commit's day).
    fn full_repo(name: &str) -> (PathBuf, String, String) {
        let root = std::env::temp_dir().join(format!("ep9-adr-lint-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        let base = commit(
            &root,
            &[
                (README, &fixture("adr-README.md")),
                (H16, &fixture("H16-accepted.md")),
                (H29, &fixture("H29-draft.md")),
                (LIB, &fixture("vm-lib.rs.txt")),
            ],
            "base",
        );
        let day = git(&root, &["show", "-s", "--format=%cs", &base]);
        (root, base, day)
    }

    /// Appends `rows` to the table under `## <section>` of the file `rel` (before the next
    /// heading, or at the end).
    fn add_rows(root: &Path, rel: &str, section: &str, rows: &[String]) {
        let text = read(&root.join(rel)).unwrap();
        let mut out = Vec::new();
        let mut on = false;
        let mut done = false;
        for l in text.lines() {
            if l.starts_with("## ") {
                if on && !done {
                    out.extend(rows.iter().cloned());
                    out.push(String::new());
                    done = true;
                }
                on = l.trim() == format!("## {section}");
            }
            if on
                && !done
                && l.trim().is_empty()
                && out.last().is_some_and(|p: &String| p.starts_with('|'))
            {
                out.extend(rows.iter().cloned());
                done = true;
            }
            out.push(l.to_string());
        }
        if !done {
            out.extend(rows.iter().cloned());
        }
        write(root, rel, &(out.join("\n") + "\n"));
    }

    /// The record rows 1-4 and an approval for `label`.
    fn record(label: &str, approver: &str, approval: &str) -> Vec<String> {
        vec![
            format!("| 1 Reviewer ({label}) | reviewer C (Claude) | Checked | |"),
            format!(
                "| 2 Codex ({label}) | orchestrator (Codex); row text by fixer D (Claude) | Filled | |"
            ),
            format!("| 3 Verifier ({label}) | fixer D (Claude) | Checked | |"),
            format!("| 4 Data ({label}) | fixer D (copied) | Runs pass | |"),
            format!("| Approval ({label}) | {approver} (README §3.2) | {approval} | |"),
        ]
    }

    /// Plan v9 section 5.6 acceptance: a clarify row on an Accepted ADR without rows 1-4 fails;
    /// with rows 1-4 and an approval by an agent that wrote none of them, it passes.
    #[test]
    fn a_clarify_row_needs_its_review_record() {
        let (root, base, day) = full_repo("records");
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        assert!(lint.summary.ends_with(": PASS"), "{}", lint.summary);
        // The clarify row, no record: FAIL.
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!(
                "| {day} | clarify (proposed; Accepted path) | A word fixed | docs agent |"
            )],
        );
        commit(&root, &[], "docs/adr: H16 clarify");
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains(&format!(
                "docs/adr/H16-ics.md: 1 amendment row(s) dated {day} since the base, 0 complete"
            )),
            "{}",
            lint.text
        );
        assert!(
            lint.summary.contains("review records FAIL"),
            "{}",
            lint.summary
        );
        assert!(lint.summary.contains("trailers PASS"), "{}", lint.summary);
        // Rows 1-4 with a Pending approval: still FAIL.
        let label = format!("M4-P3 amendment: the {day} clarify row");
        add_rows(
            &root,
            H16,
            "Review record",
            &record(&label, "Pending (orchestrator", "Pending"),
        );
        commit(&root, &[], "docs/adr: H16 record rows");
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(lint.text.contains("is Pending"), "{}", lint.text);
        // The approval by an agent that wrote row 3: FAIL.
        let text = read(&root.join(H16)).unwrap().replace(
            &format!("| Approval ({label}) | Pending (orchestrator (README §3.2) | Pending | |"),
            &format!("| Approval ({label}) | fixer D (README §3.2) | **Approved** | |"),
        );
        write(&root, H16, &text);
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains("approval is by \"fixer d\""),
            "{}",
            lint.text
        );
        // The approval by the orchestrator, which wrote none of rows 1-4: PASS.
        write(
            &root,
            H16,
            &text.replace(
                &format!("| Approval ({label}) | fixer D"),
                &format!("| Approval ({label}) | orchestrator"),
            ),
        );
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        // A second row of the same day needs a second record.
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | fill | Q1 filled | role |")],
        );
        let lint = full(&root, &base).unwrap();
        assert!(lint.text.contains("2 amendment row(s)"), "{}", lint.text);
        assert!(!lint.pass);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// M4-P3 review (jit-tools finding 4): only a result that starts with `Approved` approves.
    #[test]
    fn not_approved_is_no_approval() {
        let (root, base, day) = full_repo("not-approved");
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | clarify | A word fixed | docs agent |")],
        );
        let label = format!("the {day} clarify row");
        add_rows(
            &root,
            H16,
            "Review record",
            &record(&label, "orchestrator", "Not Approved: row 3 is wrong"),
        );
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(lint.text.contains("Approval: 0)"), "{}", lint.text);
        let text = read(&root.join(H16)).unwrap();
        write(
            &root,
            H16,
            &text.replace(
                "Not Approved: row 3 is wrong",
                "**Approved** (orchestrator)",
            ),
        );
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// M4-P3 Codex round 1 (tools, empty cells): a record row 1-4 or an Approval with an empty
    /// Who or Result cell (blank, or only markdown or a dash) is no record: a gap that names it.
    #[test]
    fn an_empty_who_or_result_is_no_record() {
        let (root, base, day) = full_repo("empty-cells");
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | clarify | A word fixed | docs agent |")],
        );
        let label = format!("the {day} clarify row");
        let good = record(&label, "orchestrator", "**Approved**");
        // Codex's case: rows 1-4 with no identity and no result, and an approved Approval.
        let mut rows: Vec<String> = ["1 Reviewer", "2 Codex", "3 Verifier", "4 Data"]
            .iter()
            .map(|p| format!("| {p} ({label}) |  |  | |"))
            .collect();
        rows.push(good[4].clone());
        add_rows(&root, H16, "Review record", &rows);
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains(&format!(
                "record row \"1 Reviewer ({label})\" has an empty Who or Result cell"
            )),
            "{}",
            lint.text
        );
        assert!(
            lint.text
                .contains("found 1: 0, 2: 0, 3: 0, 4: 0, Approval: 1)"),
            "{}",
            lint.text
        );
        // Each of the 5 rows with one empty cell (Who or Result), the other 4 rows complete.
        let clean = read(&root.join(H16)).unwrap();
        let rows_text = rows.join("\n");
        for (i, row) in good.iter().enumerate() {
            let c: Vec<String> = row
                .trim()
                .trim_matches('|')
                .split(" | ")
                .map(|c| c.trim().to_string())
                .collect();
            for (cell, blank) in [(1, ""), (2, ""), (1, "**"), (2, "—")] {
                let mut bad = c.clone();
                bad[cell] = blank.to_string();
                let mut set = good.clone();
                set[i] = format!("| {} |", bad.join(" | "));
                write(&root, H16, &clean.replace(&rows_text, &set.join("\n")));
                let lint = full(&root, &base).unwrap();
                assert!(!lint.pass, "row {i} cell {cell} {blank:?}: {}", lint.text);
                assert!(
                    lint.text.contains("has an empty Who or Result cell"),
                    "row {i} cell {cell} {blank:?}: {}",
                    lint.text
                );
            }
        }
        // The complete record: PASS.
        write(&root, H16, &clean.replace(&rows_text, &good.join("\n")));
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// M4-P3 Codex round 2 (tools, empty cells): a Who or Result cell of one space (`| |`) is an
    /// empty cell, not a part of the next one: the gap still names the row.
    #[test]
    fn a_one_space_empty_cell_is_no_record() {
        let (root, base, day) = full_repo("one-space-cells");
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | clarify | A word fixed | docs agent |")],
        );
        let label = format!("the {day} clarify row");
        let good = record(&label, "orchestrator", "**Approved**");
        // Codex's case: rows 1-4 with empty Who and Result, an Evidence cell, an approval.
        let mut rows: Vec<String> = ["1 Reviewer", "2 Codex", "3 Verifier", "4 Data"]
            .iter()
            .map(|p| format!("| {p} ({label}) | | | evidence |"))
            .collect();
        rows.push(good[4].clone());
        add_rows(&root, H16, "Review record", &rows);
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        for p in ["1 Reviewer", "2 Codex", "3 Verifier", "4 Data"] {
            assert!(
                lint.text.contains(&format!(
                    "record row \"{p} ({label})\" has an empty Who or Result cell"
                )),
                "{p}: {}",
                lint.text
            );
        }
        assert!(
            lint.text
                .contains("found 1: 0, 2: 0, 3: 0, 4: 0, Approval: 1)"),
            "{}",
            lint.text
        );
        // Each of the 5 rows with a one-space Who or Result (with and without the last
        // delimiter), the other 4 rows complete.
        let clean = read(&root.join(H16)).unwrap();
        let rows_text = rows.join("\n");
        let part = |row: &str| row.split(" | ").next().unwrap().to_string();
        for (i, row) in good.iter().enumerate() {
            let p = part(row);
            for bad in [
                format!("{p} | | Checked | evidence |"),
                format!("{p} | reviewer C (Claude) | | evidence |"),
                format!("{p} | | | evidence"),
                format!("{p} | reviewer C (Claude) | |"),
            ] {
                let mut set = good.clone();
                set[i] = bad.clone();
                write(&root, H16, &clean.replace(&rows_text, &set.join("\n")));
                let lint = full(&root, &base).unwrap();
                assert!(!lint.pass, "{bad}: {}", lint.text);
                assert!(
                    lint.text.contains("has an empty Who or Result cell"),
                    "{bad}: {}",
                    lint.text
                );
            }
        }
        // The complete record: PASS.
        write(&root, H16, &clean.replace(&rows_text, &good.join("\n")));
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// M4-P3 review (jit-tools finding 5): a record that the base file already has does not
    /// cover a new Change-log row of the same date.
    #[test]
    fn a_base_record_covers_no_new_row() {
        let (root, _, day) = full_repo("base-record");
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | clarify | A word fixed | docs agent |")],
        );
        add_rows(
            &root,
            H16,
            "Review record",
            &record(
                &format!("the {day} clarify row"),
                "orchestrator",
                "Approved",
            ),
        );
        let base = commit(&root, &[], "docs/adr: H16 clarify, recorded");
        assert!(full(&root, &base).unwrap().pass);
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | clarify | Another word | docs agent |")],
        );
        commit(&root, &[], "docs/adr: H16 second clarify, no record");
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains(&format!(
                "docs/adr/H16-ics.md: 1 amendment row(s) dated {day} since the base, 0 complete"
            )),
            "{}",
            lint.text
        );
        // Its own record (new since the base) passes; a copy of the base rows counts too.
        add_rows(
            &root,
            H16,
            "Review record",
            &record(
                &format!("the {day} clarify row"),
                "orchestrator",
                "Approved",
            ),
        );
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// M4-P3 review (jit-tools finding 3): a change of an Accepted ADR outside its Change log and
    /// Review record needs a new Change-log row.
    #[test]
    fn a_body_change_needs_a_change_log_row() {
        let (root, base, day) = full_repo("body");
        let text = read(&root.join(H16)).unwrap();
        commit(
            &root,
            &[(
                H16,
                &text.replace("The IC entry point", "The one IC entry point"),
            )],
            "docs/adr: H16 body, no row",
        );
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains(&format!(
                "docs/adr/H16-ics.md: the text outside `## Change log` and `## Review record` changed since the base, and no new Change-log row dated on or after {day}"
            )),
            "{}",
            lint.text
        );
        assert!(
            lint.summary.contains("review records FAIL"),
            "{}",
            lint.summary
        );
        // A Draft ADR's body is free; a row with its record makes H16 pass.
        let h29 = read(&root.join(H29)).unwrap();
        write(&root, H29, &format!("{h29}\nMore draft text.\n"));
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | clarify | One word | docs agent |")],
        );
        add_rows(
            &root,
            H16,
            "Review record",
            &record(
                &format!("the {day} clarify row"),
                "orchestrator",
                "Approved",
            ),
        );
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rows_that_need_no_record_and_bad_kinds() {
        let (root, base, day) = full_repo("kinds");
        // A Draft ADR's row, a record-only row, an edit of an older row: no record needed.
        add_rows(
            &root,
            H29,
            "Change log",
            &[format!("| {day} | fill | Q1 | role |")],
        );
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!(
                "| {day} | clarify | Review record: rows 1-4 of the fill row | fixer |"
            )],
        );
        let t = read(&root.join(H16))
            .unwrap()
            .replace("The first text", "The first text, edited");
        write(&root, H16, &t);
        let lint = full(&root, &base).unwrap();
        assert!(lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains("0 amendment rows (2 rows need none"),
            "{}",
            lint.text
        );
        // A kind that is not an amendment kind on an Accepted ADR.
        add_rows(
            &root,
            H16,
            "Change log",
            &[format!("| {day} | as built | K9 built | role |")],
        );
        let lint = full(&root, &base).unwrap();
        assert!(
            lint.text.contains("is not an amendment kind"),
            "{}",
            lint.text
        );
        assert!(!lint.pass);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn counts_equal_the_status_lines() {
        let (root, base, _) = full_repo("counts");
        let t = read(&root.join(H29))
            .unwrap()
            .replace("| Status | Draft", "| Status | Accepted");
        write(&root, H29, &t);
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text
                .contains("README Counts line says 1 Draft, the Status lines give 0"),
            "{}",
            lint.text
        );
        assert!(
            lint.text
                .contains("README index row H29 says \"Draft\", its Status line says \"Accepted\""),
            "{}",
            lint.text
        );
        assert!(lint.summary.contains("Counts FAIL"), "{}", lint.summary);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_moved_line_is_drift() {
        let (root, base, _) = full_repo("drift");
        // A line added above the referenced one (with the trailer the crate needs).
        commit(
            &root,
            &[(
                LIB,
                &format!("// new first line\n{}", fixture("vm-lib.rs.txt")),
            )],
            "vm: a line\n\nADR: H16",
        );
        let lint = full(&root, &base).unwrap();
        assert!(!lint.pass, "{}", lint.text);
        assert!(
            lint.text.contains("FIX       docs/adr/H16-ics.md:10 crates/ep9_vm/src/lib.rs:2 -> crates/ep9_vm/src/lib.rs:3"),
            "{}",
            lint.text
        );
        assert!(
            lint.summary.contains("path:line drift FAIL"),
            "{}",
            lint.summary
        );
        // The doc names the new line: PASS (the new token's intro commit maps it to itself).
        let t = read(&root.join(H16))
            .unwrap()
            .replace("lib.rs:2", "lib.rs:3");
        commit(&root, &[(H16, &t)], "docs/adr: H16 ref");
        // (The body edit has no Change-log row, so the review-record part fails: drift only.)
        let drift_pass = |l: &Lint| l.summary.contains("path:line drift PASS");
        assert!(drift_pass(&full(&root, &base).unwrap()));
        // A token new in the work tree must lie inside the file.
        write(&root, H16, &t.replace("lib.rs:3", "lib.rs:99"));
        let lint = full(&root, &base).unwrap();
        assert!(
            lint.text.contains("new token past the end"),
            "{}",
            lint.text
        );
        write(&root, H16, &t);
        // A pin and a history pin, from the pin table of the tree.
        let pins = format!(
            "# pins\npin\tH16-ics.md\tcrates/ep9_vm/src/lib.rs:3\t{LIB}\t^pub fn ic_entry\\(\\) \\{{\\}}$\t0\n"
        );
        write(&root, drift::PINS_FILE, &pins);
        let lint = full(&root, &base).unwrap();
        assert!(drift_pass(&lint), "{}", lint.text);
        assert!(lint.text.contains("pinned=1"), "{}", lint.text);
        write(
            &root,
            drift::PINS_FILE,
            &format!(
                "{pins}history\tH16-ics.md\tcrates/ep9_vm/src/lib.rs:7\tno such line\tHEAD\tx\n"
            ),
        );
        let lint = full(&root, &base).unwrap();
        assert!(
            lint.text.contains("UNFIXABLE UNUSED HISTORY PIN"),
            "{}",
            lint.text
        );
        write(
            &root,
            drift::PINS_FILE,
            "pin\tH16-ics.md\tx.rs:1\tx.rs\ta.b\t0\n",
        );
        assert!(full(&root, &base).unwrap_err().contains("regex syntax"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
