//! The short-cite check of `cargo xtask adr-lint` (MODES-CHECK-w16 MC-w16-8): the cites of
//! `docs/plan/13-execution-modes.md` in its short forms (`H31:66`, `AR:37`, `DL:2743`,
//! `SCP:28`, `golden:334`, ...), which the `path:line` drift check ([`super::drift`]) does not
//! read, because they name no path.
//!
//! 1. The short forms are the ones doc 13 defines in its header line `- Short forms: P02 =
//!    `docs/plan/02-architecture.md`, ..., Hnn = `docs/adr/Hnn-*.md`, golden = ...`: each
//!    `` <name> = `<path>` `` pair. A name that ends in `nn` with a `*` path (`Hnn`) is a family:
//!    `H31` is the one file of the family's dir whose name starts with `H31-`.
//! 2. The base is the commit the header names right after `Cites at ` (`- Cites at <sha> ...`).
//!    A header that names none there (the text "the M4-P4 merge head"), or names one that is not
//!    a commit here, falls back to the last commit that changed doc 13, and a BASE finding says
//!    so: that base moves with every doc 13 edit, so it can hide an older move.
//! 3. The cites are those above the `## 12.` heading: the header puts section 12 (the critique
//!    record) outside the "names the line there" rule, and section 13 is the history of earlier
//!    cite moves (old numbers on purpose).
//! 4. Each cited line (both ends of a range) at the base is compared with the same line at HEAD.
//!    A different line is MOVED when the base text (trimmed, not blank) is exactly one line of
//!    the file at HEAD (the report names the new number), else CHANGED (the text was edited or
//!    removed). A cite past the end of its file at the base is BAD AT BASE. With a fallback base
//!    a MOVED line is NOT AT BASE LINE and names no new number: the cite may have been wrong at
//!    that base already (verifier r1 F2: `golden:334` named another line there). A cite whose
//!    file is missing at HEAD (a family name with no file) is NO FILE.
//!
//! The part gates from M4-P5 ([`GATE`] true; plan v9 rule 20, P5-ORCH-2: it guards the
//! doc 13 pointers such as the FR0 row H31:90): a finding fails adr-lint and so m1-exit item 6.
//! With [`GATE`] false the part is report only: its lines start with `note: `, an error is a
//! note, and the result does not change.

use super::{git, read};
use std::collections::HashMap;
use std::path::Path;

/// The doc whose short cites are checked.
pub(crate) const DOC: &str = "docs/plan/13-execution-modes.md";

/// True (from M4-P5, after the MC-w16-2 doc 13 fix): a finding (MOVED, NOT AT BASE LINE,
/// CHANGED, BAD AT BASE, NO FILE, BASE) fails adr-lint. False: report only (MC-w16-8).
pub(crate) const GATE: bool = true;

/// The result of the short-cite check.
#[derive(Debug, Default)]
pub(crate) struct Cites {
    /// The cites read (both ends of a range count).
    pub(crate) total: usize,
    /// One line per cite that does not name the same line at HEAD, and a BASE line when the
    /// header names no base commit.
    pub(crate) findings: Vec<String>,
    /// Notes: the base and where it came from, unknown names.
    pub(crate) notes: Vec<String>,
}

impl Cites {
    pub(crate) fn pass(&self) -> bool {
        !GATE || self.findings.is_empty()
    }

    pub(crate) fn summary(&self) -> String {
        format!(
            "{DOC}: {} short cites, {} findings{}",
            self.total,
            self.findings.len(),
            if GATE { "" } else { " (report only, MC-w16-8)" }
        )
    }

    /// The report lines: the notes, then the findings (each `note: ` first while report only).
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut v: Vec<String> = self.notes.iter().map(|n| format!("note: {n}")).collect();
        v.extend(self.findings.iter().map(|f| {
            if GATE {
                f.clone()
            } else {
                format!("note: {f}")
            }
        }));
        v
    }
}

/// A short form: a single file, or a family (`Hnn` = the file of `dir` named `H<dd>-*`).
#[derive(Debug, PartialEq)]
enum Form {
    File(String),
    Family { dir: String, prefix: String },
}

/// The `` <name> = `<path>` `` pairs of the doc's `- Short forms:` header line.
fn short_forms(doc: &str) -> Vec<(String, Form)> {
    let Some(line) = doc.lines().find(|l| l.starts_with("- Short forms:")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find(" = `") {
        let before = &rest[..i];
        let name: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let after = &rest[i + 4..];
        let Some(j) = after.find('`') else { break };
        let path = &after[..j];
        rest = &after[j + 1..];
        if name.is_empty() {
            continue;
        }
        let form = match (name.strip_suffix("nn"), path.rsplit_once('/')) {
            (Some(prefix), Some((dir, file)))
                if !prefix.is_empty() && file.starts_with(&name) && file.contains('*') =>
            {
                Form::Family {
                    dir: dir.to_string(),
                    prefix: prefix.to_string(),
                }
            }
            _ if path.contains('*') => continue,
            _ => Form::File(path.to_string()),
        };
        out.push((name, form));
    }
    out
}

/// A cite: (the name as written, e.g. `H31`; the line; the end of a range).
type Cite = (String, u64, Option<u64>);

/// The short cites of one doc line, for the given forms. A cite starts at a word boundary.
fn cites_in(line: &str, forms: &[(String, Form)]) -> Vec<Cite> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        if (i > 0 && word(b[i - 1])) || !word(b[i]) {
            i += 1;
            continue;
        }
        let mut matched = None;
        for (name, form) in forms {
            let len = match form {
                Form::File(_) => line[i..].starts_with(name.as_str()).then_some(name.len()),
                Form::Family { prefix, .. } => {
                    let r = &b[i..];
                    (r.len() >= prefix.len() + 2
                        && line[i..].starts_with(prefix.as_str())
                        && r[prefix.len()].is_ascii_digit()
                        && r[prefix.len() + 1].is_ascii_digit())
                    .then_some(prefix.len() + 2)
                }
            };
            if let Some(len) = len
                && b.get(i + len) == Some(&b':')
            {
                matched = Some(len);
                break;
            }
        }
        let Some(len) = matched else {
            i += 1;
            continue;
        };
        let num = |mut j: usize| {
            let s = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            (j > s).then(|| (line[s..j].parse::<u64>().ok(), j))
        };
        let Some((Some(n), mut j)) = num(i + len + 1) else {
            i += len;
            continue;
        };
        let mut end = None;
        if b.get(j) == Some(&b'-')
            && let Some((Some(m), k)) = num(j + 1)
        {
            end = Some(m);
            j = k;
        }
        // `H31:66x` or `AR:37.5` is not a cite of a line.
        if j < b.len()
            && (word(b[j]) || (b[j] == b'.' && b.get(j + 1).is_some_and(u8::is_ascii_digit)))
        {
            i = j;
            continue;
        }
        out.push((line[i..i + len].to_string(), n, end));
        i = j;
    }
    out
}

/// The base of the check: (commit, named). `named` is true for the commit of the doc header's
/// `Cites at <sha>`; else the base is the last commit that changed the doc, and a BASE finding
/// says why (no sha there, or one that is not a commit here).
fn base_commit(root: &Path, doc: &str, out: &mut Cites) -> Result<(String, bool), String> {
    let named = doc
        .lines()
        .find_map(|l| l.strip_prefix("- Cites at "))
        .and_then(|r| r.split(|c: char| !c.is_ascii_hexdigit()).next())
        .filter(|t| (7..=40).contains(&t.len()));
    if let Some(t) = named
        && let Some(full) = git(root, &["rev-parse", "--verify", &format!("{t}^{{commit}}")])
    {
        out.notes.push(format!(
            "short cites: base {} (the doc 13 header: \"Cites at {t}\")",
            &full[..full.len().min(12)]
        ));
        return Ok((full, true));
    }
    let last = git(root, &["log", "-1", "--format=%H", "--", DOC])
        .ok_or_else(|| format!("no commit changed {DOC}"))?;
    let why = match named {
        Some(t) => {
            format!("names `{t}` after \"Cites at \", which is not a commit in this repository")
        }
        None => "names no commit after \"Cites at \"".to_string(),
    };
    out.findings.push(format!(
        "SHORT CITE BASE: the {DOC} header {why}; base = the last commit that changed it, {} (it moves with every doc edit and can hide an older move: name the base in the header)",
        &last[..last.len().min(12)]
    ));
    Ok((last, false))
}

/// The short-cite part of adr-lint: [`short_cites`]; while report only ([`GATE`] false) an error
/// is a note, so it cannot stop the other parts (verifier r1 F6).
pub(crate) fn part(root: &Path) -> Result<Cites, String> {
    match short_cites(root) {
        Err(e) if !GATE => Ok(Cites {
            notes: vec![format!("short cites: not checked: {e}")],
            ..Cites::default()
        }),
        r => r,
    }
}

/// The short-cite check over the work tree's doc 13 (module doc). A tree without doc 13 has no
/// cites.
pub(crate) fn short_cites(root: &Path) -> Result<Cites, String> {
    let mut out = Cites::default();
    let path = root.join(DOC);
    if !path.is_file() {
        out.notes
            .push(format!("short cites: no {DOC} in this tree"));
        return Ok(out);
    }
    let doc = read(&path)?;
    let forms = short_forms(&doc);
    if forms.is_empty() {
        out.notes
            .push(format!("short cites: {DOC} has no \"- Short forms:\" line"));
        return Ok(out);
    }
    let (base, named) = base_commit(root, &doc, &mut out)?;
    let short_base = &base[..base.len().min(7)];
    // The files of each family dir at HEAD.
    let mut dirs: HashMap<String, Vec<String>> = HashMap::new();
    let mut files: HashMap<(String, &str), Option<Vec<String>>> = HashMap::new();
    let mut show = |rev: &str, p: &str| -> Option<Vec<String>> {
        files
            .entry((p.to_string(), if rev == "HEAD" { "HEAD" } else { "base" }))
            .or_insert_with(|| {
                git_raw(root, &format!("{rev}:{p}"))
                    .map(|t| t.lines().map(str::to_string).collect())
            })
            .clone()
    };
    for (i, l) in doc.lines().enumerate() {
        if l.starts_with("## 12.") {
            break;
        }
        for (name, n, end) in cites_in(l, &forms) {
            let file = match forms.iter().find(|(f, form)| match form {
                Form::File(_) => *f == name,
                Form::Family { prefix, .. } => {
                    name.starts_with(prefix.as_str()) && name.len() == prefix.len() + 2
                }
            }) {
                Some((_, Form::File(p))) => Some(p.clone()),
                Some((_, Form::Family { dir, .. })) => {
                    let listing = dirs.entry(dir.clone()).or_insert_with(|| {
                        git(
                            root,
                            &["ls-tree", "--name-only", "HEAD", &format!("{dir}/")],
                        )
                        .map(|t| t.lines().map(str::to_string).collect())
                        .unwrap_or_default()
                    });
                    let want = format!("{dir}/{name}-");
                    listing.iter().find(|f| f.starts_with(&want)).cloned()
                }
                None => None,
            };
            let Some(file) = file else {
                // A renamed or deleted ADR: the cite names nothing (verifier r1 F6).
                for k in std::iter::once(n).chain(end) {
                    out.total += 1;
                    out.findings.push(format!(
                        "SHORT CITE NO FILE doc13:{} {name}:{k}: no file for {name} at HEAD",
                        i + 1
                    ));
                }
                continue;
            };
            let (b, h) = (show(&base, &file), show("HEAD", &file));
            for k in std::iter::once(n).chain(end) {
                out.total += 1;
                let at = |v: &Option<Vec<String>>| {
                    v.as_ref()
                        .and_then(|v| v.get((k as usize).wrapping_sub(1)).cloned())
                };
                let (lb, lh) = (at(&b), at(&h));
                if lb.is_some() && lb == lh {
                    continue;
                }
                let cite = format!("doc13:{} {name}:{k}", i + 1);
                let Some(lb) = lb else {
                    out.findings.push(format!(
                        "SHORT CITE BAD AT BASE {cite}: {file} has no line {k} at {short_base}"
                    ));
                    continue;
                };
                let text = lb.trim();
                let hits: Vec<usize> = h
                    .as_ref()
                    .map(|v| {
                        v.iter()
                            .enumerate()
                            .filter(|(_, l)| !text.is_empty() && l.trim() == text)
                            .map(|(j, _)| j + 1)
                            .collect()
                    })
                    .unwrap_or_default();
                let clip: String = text.chars().take(80).collect();
                // The same row, its text edited in place (a table row with the same first cell):
                // the cite still names its row, so this is a note, not a finding.
                if hits.is_empty()
                    && let Some(lh) = &lh
                    && same_row(text, lh.trim(), b.as_deref(), h.as_deref())
                {
                    out.notes.push(format!(
                        "short cite EDITED {cite}: the same table row, its text changed since {short_base} (read it once): {clip}"
                    ));
                    continue;
                }
                out.findings.push(match hits.as_slice() {
                    [m] if named => format!("SHORT CITE MOVED {cite} -> {name}:{m} (same text as at {short_base}): {clip}"),
                    // A fallback base may not be the commit the cite was written for: its line
                    // there proves nothing, so name no target.
                    [_] => format!("SHORT CITE NOT AT BASE LINE {cite}: its text at {short_base} is on another line at HEAD (fallback base: the cite may have been wrong already; find the line by hand): {clip}"),
                    _ => format!(
                        "SHORT CITE CHANGED {cite}: the line at {short_base} is {} at HEAD: {clip}",
                        if hits.is_empty() { "not in the file".to_string() } else { format!("on {} lines", hits.len()) }
                    ),
                });
            }
        }
    }
    Ok(out)
}

/// The first cell of a table row (not blank).
fn first_cell(l: &str) -> Option<&str> {
    l.trim()
        .strip_prefix('|')
        .and_then(|r| r.split_once('|'))
        .map(|(c, _)| c.trim())
        .filter(|c| !c.is_empty())
}

/// Two table rows with the same first cell (not blank), and that first cell names one row only
/// in the file at the base and at HEAD: with two rows of that cell, the cited row may have moved
/// and the other taken its line (verifier r1 F6).
fn same_row(a: &str, b: &str, base: Option<&[String]>, head: Option<&[String]>) -> bool {
    let Some(key) = first_cell(a) else {
        return false;
    };
    let once = |v: Option<&[String]>| {
        v.is_some_and(|v| v.iter().filter(|l| first_cell(l) == Some(key)).count() == 1)
    };
    first_cell(b) == Some(key) && once(base) && once(head)
}

/// The raw stdout of `git show <spec>` (blank lines kept), or None.
fn git_raw(root: &Path, spec: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", spec])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    const HEADER: &str = "# doc\n\n- Cites at {BASE} (the test head).\n- Short forms: AR = `docs/adr/README.md`, DL = `docs/decisions-log.md`, Hnn = `docs/adr/Hnn-*.md`, golden = `g.txt`. MC-w13-n = finding n.\n";

    fn git_in(root: &Path, args: &[&str]) -> String {
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
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn commit(root: &Path, msg: &str) -> String {
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-q", "--allow-empty", "-m", msg]);
        git_in(root, &["rev-parse", "HEAD"])
    }

    fn forms() -> Vec<(String, Form)> {
        short_forms(HEADER)
    }

    #[test]
    fn the_header_defines_the_forms() {
        assert_eq!(
            forms(),
            vec![
                ("AR".into(), Form::File("docs/adr/README.md".into())),
                ("DL".into(), Form::File("docs/decisions-log.md".into())),
                (
                    "Hnn".into(),
                    Form::Family {
                        dir: "docs/adr".into(),
                        prefix: "H".into()
                    }
                ),
                ("golden".into(), Form::File("g.txt".into())),
            ]
        );
    }

    #[test]
    fn cites_are_found_at_word_boundaries() {
        let f = forms();
        let c = |l: &str| cites_in(l, &f);
        assert_eq!(
            c("see H31:66 and H31:90-92, (AR:37; DL:2743), golden:3-5."),
            vec![
                ("H31".into(), 66, None),
                ("H31".into(), 90, Some(92)),
                ("AR".into(), 37, None),
                ("DL".into(), 2743, None),
                ("golden".into(), 3, Some(5)),
            ]
        );
        // Not cites: inside a word, a three-digit id, no number, a decimal, an undefined name.
        assert!(
            c("PAR:3 XH31:2 H310:4 H3:5 AR: x AR:3.5 P02:4 MC-w13-2").is_empty(),
            "{:?}",
            c("PAR:3 XH31:2 H310:4 H3:5 AR: x AR:3.5 P02:4")
        );
        assert_eq!(c("`H07:58`"), vec![("H07".into(), 58, None)]);
    }

    fn repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ep9-cites-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git_in(&root, &["init", "-q"]);
        root
    }

    /// The base is the header's commit; a line that moves, one that changes, and the cites of
    /// section 12 and later (not read).
    #[test]
    fn moved_and_changed_lines_are_reported() {
        let root = repo("moves");
        let adr: String = (1..=10).map(|i| format!("row {i}\n")).collect();
        write(&root, "docs/adr/H31-fuses.md", &adr);
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nc\n| Poll | one unit |\n",
        );
        write(&root, "g.txt", "g1\ng2\n");
        let body =
            "\nH31:2 and H31:4-5, AR:3, golden:2, AR:4.\n\n## 12. Review log\n\nH31:9 is old.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", &base[..7])),
        );
        commit(&root, "header");
        let c = short_cites(&root).unwrap();
        assert_eq!((c.total, c.findings.len()), (6, 0), "{c:?}");
        assert!(
            c.notes[0].contains("the doc 13 header: \"Cites at "),
            "{:?}",
            c.notes
        );
        assert!(c.pass());
        // Two rows above row 2: rows 2, 4, 5 move by 2; AR line 3 changes in place.
        let moved: String = std::iter::once("new a\nnew b\n".to_string())
            .chain((1..=10).map(|i| format!("row {i}\n")))
            .collect();
        write(&root, "docs/adr/H31-fuses.md", &moved);
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nC changed\n| Poll | one unit, edited |\n",
        );
        // golden:2 ("g2") is on 2 lines at HEAD and not on line 2: CHANGED, no target (N1).
        write(&root, "g.txt", "g1\ng1b\ng2\ng2\n");
        commit(&root, "move");
        let c = short_cites(&root).unwrap();
        assert_eq!(c.total, 6);
        assert!(
            c.notes
                .iter()
                .any(|n| n.starts_with("short cite EDITED doc13:6 AR:4: the same table row")),
            "{:?}",
            c.notes
        );
        assert_eq!(
            c.findings,
            vec![
                format!(
                    "SHORT CITE MOVED doc13:6 H31:2 -> H31:4 (same text as at {}): row 2",
                    &base[..7]
                ),
                format!(
                    "SHORT CITE MOVED doc13:6 H31:4 -> H31:6 (same text as at {}): row 4",
                    &base[..7]
                ),
                format!(
                    "SHORT CITE MOVED doc13:6 H31:5 -> H31:7 (same text as at {}): row 5",
                    &base[..7]
                ),
                format!(
                    "SHORT CITE CHANGED doc13:6 AR:3: the line at {} is not in the file at HEAD: c",
                    &base[..7]
                ),
                format!(
                    "SHORT CITE CHANGED doc13:6 golden:2: the line at {} is on 2 lines at HEAD: g2",
                    &base[..7]
                ),
            ]
        );
        // The part gates (GATE): a moved cite fails it and prints as a finding, not a note.
        assert!(!c.pass());
        assert!(
            c.lines()
                .iter()
                .any(|l| l.starts_with("SHORT CITE MOVED doc13:6 H31:2"))
        );
        assert!(!c.lines().iter().any(|l| l.starts_with("note: SHORT CITE")));
        assert!(c.summary().contains("6 short cites, 5 findings"));
        assert!(!c.summary().contains("report only"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// No commit after "Cites at": the base is the last commit that changed the doc; a cite past
    /// the end of its file at the base is BAD AT BASE.
    #[test]
    fn a_header_without_a_commit_uses_the_last_doc_change() {
        let root = repo("nobase");
        write(&root, "docs/adr/H02-x.md", "one\n");
        write(
            &root,
            DOC,
            &format!(
                "{}\nH02:1, H02:7, H09:1.\n",
                HEADER.replace("{BASE}", "the M4-P4 merge head")
            ),
        );
        let doc_commit = commit(&root, "doc");
        write(&root, "docs/adr/H02-x.md", "zero\none\n");
        commit(&root, "later");
        let c = short_cites(&root).unwrap();
        assert_eq!(c.findings.len(), 4, "{:?}", c.findings);
        // The fallback base is a finding (verifier r1 F6).
        assert!(
            c.findings[0].starts_with(&format!(
                "SHORT CITE BASE: the {DOC} header names no commit after \"Cites at \"; base = the last commit that changed it, {}",
                &doc_commit[..12]
            )),
            "{:?}",
            c.findings
        );
        // With a fallback base, a moved line names no target: the cite may have been wrong at
        // that base already (verifier r1 F2).
        assert!(
            c.findings[1].starts_with("SHORT CITE NOT AT BASE LINE doc13:6 H02:1: its text at "),
            "{:?}",
            c.findings
        );
        assert!(!c.findings[1].contains("->"), "{:?}", c.findings);
        assert!(
            c.findings[2].starts_with("SHORT CITE BAD AT BASE doc13:6 H02:7"),
            "{:?}",
            c.findings
        );
        // A family name with no file at HEAD is a finding, not a note (verifier r1 F6).
        assert_eq!(
            c.findings[3],
            "SHORT CITE NO FILE doc13:6 H09:1: no file for H09 at HEAD"
        );
        assert_eq!(c.total, 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A header sha that is not a commit here is a BASE finding, not a silent fallback; a
    /// header sha that is one gives MOVED with its target (verifier r1 F2, F6).
    #[test]
    fn a_header_sha_must_resolve() {
        let root = repo("badsha");
        write(&root, "docs/adr/H02-x.md", "one\n");
        let body = "\nH02:1.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "abcdef1")),
        );
        let doc_commit = commit(&root, "doc");
        let c = short_cites(&root).unwrap();
        assert_eq!(c.findings.len(), 1, "{:?}", c.findings);
        assert!(
            c.findings[0].starts_with(&format!(
                "SHORT CITE BASE: the {DOC} header names `abcdef1` after \"Cites at \", which is not a commit in this repository; base = the last commit that changed it, {}",
                &doc_commit[..12]
            )),
            "{:?}",
            c.findings
        );
        // Named and resolved: the move names its target.
        write(&root, "docs/adr/H02-x.md", "zero\none\n");
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", &doc_commit[..7])),
        );
        commit(&root, "move");
        let c = short_cites(&root).unwrap();
        assert_eq!(c.findings.len(), 1, "{:?}", c.findings);
        assert!(
            c.findings[0].starts_with("SHORT CITE MOVED doc13:6 H02:1 -> H02:2"),
            "{:?}",
            c.findings
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Two rows with one first cell: an edit at the cited line is not EDITED, because the cited
    /// row may have moved and the other taken its line (verifier r1 F6).
    #[test]
    fn edited_needs_a_unique_first_cell() {
        let root = repo("rows");
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nc\n| Poll | one unit |\n| Poll | other |\n",
        );
        write(
            &root,
            DOC,
            &format!("{}\nAR:4.\n", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        write(
            &root,
            DOC,
            &format!("{}\nAR:4.\n", HEADER.replace("{BASE}", &base[..7])),
        );
        // The cited row moves below the other one and is edited.
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nc\n| Poll | other |\n| Poll | one unit, edited |\n",
        );
        commit(&root, "swap");
        let c = short_cites(&root).unwrap();
        assert!(
            !c.notes.iter().any(|n| n.contains("EDITED")),
            "{:?}",
            c.notes
        );
        assert_eq!(c.findings.len(), 1, "{:?}", c.findings);
        assert!(
            c.findings[0].starts_with("SHORT CITE CHANGED doc13:6 AR:4"),
            "{:?}",
            c.findings
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// While report only, an error of the check is a note, so adr-lint's other parts still run
    /// (verifier r1 F6); while it gates, the error stops adr-lint: here doc 13 is in the tree
    /// but no commit has it.
    #[test]
    fn an_error_is_a_note_while_report_only() {
        let root = repo("err");
        write(&root, "x.txt", "x\n");
        commit(&root, "first");
        write(&root, DOC, &HEADER.replace("{BASE}", "no sha"));
        assert!(short_cites(&root).is_err());
        let p = part(&root);
        if GATE {
            assert!(p.is_err(), "{p:?}");
        } else {
            let c = p.unwrap();
            assert!(c.pass());
            assert!(
                c.notes[0].starts_with("short cites: not checked: no commit changed"),
                "{:?}",
                c.notes
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_tree_without_doc_13_has_no_cites() {
        let root = repo("nodoc");
        let c = short_cites(&root).unwrap();
        assert_eq!((c.total, c.findings.len()), (0, 0));
        assert!(c.pass());
        std::fs::remove_dir_all(root).unwrap();
    }
}
