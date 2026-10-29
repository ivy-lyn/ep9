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
//! 5. A cite on a doc 13 line that is new since the base, or on an edited line whose cite
//!    number changed (the base line the edit replaced, the removed line nearby that shares the
//!    most words with it, has no such cite; M4-P5 CX5-C7, verifier r1 F1 and r2 F1), was
//!    written for the files of the commit that added it, not for the base. A cite past the end
//!    of its file at HEAD is BAD AT HEAD. Else (CX6-3, plan v10 section 3.7) it gets the step 4
//!    check with that commit in place of the base: the newest commit after the base that
//!    changed how often the cite's text occurs in the doc (`git log -S`, a merge by its diff
//!    against its first parent) and whose doc holds more such cites than each of its parents'
//!    ([`Intro`]; a cite merged from a branch is found at its branch commit, and one that a
//!    merge resolution added, at the merge). The findings name that commit ("the commit that added the cite");
//!    a cite past the end of its file there is BAD AT INTRO (rule 31 moves a cite with its file
//!    in one commit). A cite that no commit added (uncommitted in the work tree) is checked at
//!    HEAD only. Step 4 is for the other cites, so an edit that keeps a stale number still gives
//!    MOVED. A range is decided per endpoint (CX6-5): its start (its end) keeps step 4 when the
//!    paired base line has a cite of that name with the same start (the same end).
//!
//! 6. A cited line that is the base line with a dated note appended at its end (the rest
//!    of the line at HEAD starts with `Note (YYYY-MM-DD` or `[YYYY-MM-DD`, as a dated
//!    addendum that moves no line is written, plan v10 section 3.8 WI-SC3) still names its
//!    line: a note (`short cite NOTE ADDED`), not a finding, as an edited table row is
//!    ([`appended_note`]; M5-P1 fixer). Text edited before the note stays CHANGED.
//!
//! Known limits (M4-P5 CX6-4; CX6-3 is closed by step 5 in M5-P1): the word-overlap pairing is
//! a heuristic: a reworded row that keeps a stale number can lose its pair to an inserted row,
//! and its cite is then checked at the commit that added that number. When one cite text is on
//! two doc lines added by different commits, both are checked at the newer commit (a move
//! between the two commits is not found). `git log -S` counts the text, not the cite: an edit
//! that turns `AR:45` into `AR:4` leaves the count of `AR:4` the same, so that commit is not a
//! candidate and an older one (or none: HEAD only) is used.
//!
//! The part gates from M4-P5 ([`GATE`] true; plan v9 rule 20, P5-ORCH-2: it guards the
//! doc 13 pointers such as the FR0 row H31:90): a finding fails adr-lint and so m1-exit item 6.
//! With [`GATE`] false the part is report only: its lines start with `note: `, an error is a
//! note, and the result does not change.
//!
//! To fix a MOVED finding, change the cite number: the cite is then new since the base and is
//! checked at HEAD (step 5). Setting `Cites at` to a later commit (one commit cannot
//! name itself, so a commit before the one that sets it) brings the other lines up to date.

use super::{git, read};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The doc whose short cites are checked.
pub(crate) const DOC: &str = "docs/plan/13-execution-modes.md";

/// True (from M4-P5, after the MC-w16-2 doc 13 fix): a finding (MOVED, NOT AT BASE LINE,
/// CHANGED, BAD AT BASE, BAD AT HEAD, NO FILE, BASE) fails adr-lint. False: report only (MC-w16-8).
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
    // The doc 13 lines at the base: a cite on a line that is not one of them (new or edited
    // since) was written for the files at HEAD, so it is checked at HEAD only (CX5-C7). An
    // edited line keeps the move check for each cite that the base line it replaced also has
    // ([`paired`]; verifier r1 F1): only a new cite number is checked at HEAD only.
    let base_doc = git_raw(root, &format!("{base}:{DOC}")).unwrap_or_default();
    let old_lines: HashSet<&str> = base_doc.lines().collect();
    let base_v: Vec<&str> = base_doc.lines().collect();
    let head_v: Vec<&str> = doc.lines().collect();
    let pair = paired(&base_v, &head_v);
    // The files of each family dir at HEAD.
    let mut dirs: HashMap<String, Vec<String>> = HashMap::new();
    let mut files: HashMap<(String, String), Option<Vec<String>>> = HashMap::new();
    let mut show = |rev: &str, p: &str| -> Option<Vec<String>> {
        files
            .entry((p.to_string(), rev.to_string()))
            .or_insert_with(|| {
                git_raw(root, &format!("{rev}:{p}"))
                    .map(|t| t.lines().map(str::to_string).collect())
            })
            .clone()
    };
    // CX6-3: the commit that added each new cite ([`Intro`]).
    let mut intro = Intro::new(root, &base, &forms, &doc);
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
            // CX6-5: "carried" per endpoint. The start (the end) keeps the move check when the
            // paired base line has a cite of this name with the same start (the same end).
            let base_cites = pair[i].map_or_else(Vec::new, |b| cites_in(base_v[b], &forms));
            let carried = |k: u64, is_end: bool| {
                old_lines.contains(l)
                    || base_cites.iter().any(|(bn, bs, be)| {
                        *bn == name && if is_end { *be == Some(k) } else { *bs == k }
                    })
            };
            for (k, is_end) in std::iter::once((n, false)).chain(end.map(|e| (e, true))) {
                out.total += 1;
                // The lines the cite is checked against: the base, or for a new cite the commit
                // that added it (CX6-3: its lines and the text that names it).
                let mut from: Option<(Option<Vec<String>>, String)> = None;
                if !carried(k, is_end) {
                    let lines = h.as_ref().map_or(0, Vec::len);
                    if k == 0 || k as usize > lines {
                        out.findings.push(format!(
                            "SHORT CITE BAD AT HEAD doc13:{} {name}:{k}: {file} has no line {k} at HEAD",
                            i + 1
                        ));
                        continue;
                    }
                    // CX6-3: checked at the commit that added it; at HEAD only when no commit
                    // after the base added it (an uncommitted cite).
                    let Some(c) = intro.commit(&(name.clone(), n, end)) else {
                        continue;
                    };
                    let label = format!("{}, the commit that added the cite", &c[..7]);
                    from = Some((show(&c, &file), label));
                }
                let (b, short_base, named, is_intro) = match &from {
                    Some((lines, label)) => (lines, label.as_str(), true, true),
                    None => (&b, short_base, named, false),
                };
                let at = |v: &Option<Vec<String>>| {
                    v.as_ref()
                        .and_then(|v| v.get((k as usize).wrapping_sub(1)).cloned())
                };
                let (lb, lh) = (at(b), at(&h));
                if lb.is_some() && lb == lh {
                    continue;
                }
                let cite = format!("doc13:{} {name}:{k}", i + 1);
                let Some(lb) = lb else {
                    out.findings.push(if is_intro {
                        format!(
                            "SHORT CITE BAD AT INTRO {cite}: {file} has no line {k} at {short_base}"
                        )
                    } else {
                        format!(
                            "SHORT CITE BAD AT BASE {cite}: {file} has no line {k} at {short_base}"
                        )
                    });
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
                // The same line with a dated note appended (step 6): a note, not a finding.
                if hits.is_empty()
                    && let Some(lh) = &lh
                    && appended_note(text, lh.trim())
                {
                    out.notes.push(format!(
                        "short cite NOTE ADDED {cite}: the line at {short_base} with a dated note appended (read the note once): {clip}"
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

/// CX6-3 (M4-P5; plan v10 section 3.7): the commit that added a cite that is new since the base
/// (step 5 of the module doc). The candidates are the commits after the base that changed the
/// number of times the cite's text occurs in the doc (`git log -S<cite> <base>..HEAD -- <doc>`,
/// newest first; a merge by its diff against its first parent); the cite was added by the
/// newest one whose doc holds more cites of that name and number (above `## 12.`, as
/// [`cites_in`] reads them) than each of its parents' (so a merge resolution that adds a cite
/// in no parent is its adder, and a cite merged from a branch keeps its branch commit). None when no
/// such commit exists, or when the doc in the work tree holds more of the cite than HEAD's (an
/// uncommitted cite: no commit wrote it).
struct Intro<'a> {
    root: &'a Path,
    base: &'a str,
    forms: &'a [(String, Form)],
    /// The cites of the doc at a rev (the doc's cites above `## 12.`); None: no doc there.
    docs: HashMap<String, Option<Vec<Cite>>>,
    /// The cites of the work-tree doc.
    work: Vec<Cite>,
}

impl<'a> Intro<'a> {
    fn new(root: &'a Path, base: &'a str, forms: &'a [(String, Form)], doc: &str) -> Self {
        Intro {
            root,
            base,
            forms,
            docs: HashMap::new(),
            work: doc_cites(doc, forms),
        }
    }

    /// How many cites equal to `cite` the doc at `rev` holds (0 when it has no doc).
    fn count(&mut self, rev: &str, cite: &Cite) -> usize {
        let (root, forms) = (self.root, self.forms);
        self.docs
            .entry(rev.to_string())
            .or_insert_with(|| git_raw(root, &format!("{rev}:{DOC}")).map(|d| doc_cites(&d, forms)))
            .as_ref()
            .map_or(0, |v| v.iter().filter(|c| *c == cite).count())
    }

    fn commit(&mut self, cite: &Cite) -> Option<String> {
        let at_work = self.work.iter().filter(|c| *c == cite).count();
        if at_work > self.count("HEAD", cite) {
            return None;
        }
        let (name, n, end) = cite;
        let text = match end {
            Some(e) => format!("{name}:{n}-{e}"),
            None => format!("{name}:{n}"),
        };
        // `--diff-merges=first-parent`: a merge is searched by its diff against its first
        // parent, so a cite that a merge resolution added (in no parent) is a candidate too
        // (Codex r1 gates6). `%P`: the parents, so a merge counts as the adder only when its
        // doc holds more of the cite than every parent's; a cite merged from a branch is then
        // found at its branch commit, as before.
        let log = git(
            self.root,
            &[
                "log",
                "--diff-merges=first-parent",
                "--format=%H %P",
                &format!("-S{text}"),
                &format!("{}..HEAD", self.base),
                "--",
                DOC,
            ],
        )?;
        for l in log.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let mut ids = l.split(' ');
            let Some(c) = ids.next() else { continue };
            let here = self.count(c, cite);
            let parents: Vec<&str> = ids.filter(|p| !p.is_empty()).collect();
            let most = parents
                .iter()
                .map(|p| self.count(p, cite))
                .max()
                .unwrap_or(0);
            if here > most {
                return Some(c.to_string());
            }
        }
        None
    }
}

/// The cites of a doc text above its `## 12.` heading (step 3 of the module doc).
fn doc_cites(doc: &str, forms: &[(String, Form)]) -> Vec<Cite> {
    doc.lines()
        .take_while(|l| !l.starts_with("## 12."))
        .flat_map(|l| cites_in(l, forms))
        .collect()
}

/// For each line of `head`, the line of `base` it replaced, if any: the lines of a longest
/// common subsequence are kept (no pair: they are base lines); between two kept lines, an added
/// line pairs with the removed line it shares the most words with ([`match_gap`]; CX5 verifier
/// r2 F1), so a row inserted above an edited row does not take that row's pair.
fn paired(base: &[&str], head: &[&str]) -> Vec<Option<usize>> {
    let (n, m) = (base.len(), head.len());
    // t[i][j]: the length of a longest common subsequence of base[i..] and head[j..].
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if base[i] == head[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let mut out = vec![None; m];
    let (mut i, mut j) = (0, 0);
    let (mut gone, mut added) = (Vec::new(), Vec::new());
    loop {
        let kept = i < n && j < m && base[i] == head[j];
        if kept || (i == n && j == m) {
            // In each gap, an added line pairs with the removed line that shares the most words
            // (cite numbers masked), not with the one at its place: a row inserted above an
            // edited row, or two edited rows that swap, still pair right.
            match_gap(base, head, &gone, &added, &mut out);
            gone.clear();
            added.clear();
            if !kept {
                return out;
            }
            i += 1;
            j += 1;
        } else if j < m && (i == n || t[i][j + 1] >= t[i + 1][j]) {
            added.push(j);
            j += 1;
        } else {
            gone.push(i);
            i += 1;
        }
    }
}

/// The words of a line, each digit after a `:` masked, so a line whose cite numbers changed
/// has the same words.
fn shape(l: &str) -> HashSet<String> {
    l.split_whitespace()
        .map(|w| match w.split_once(':') {
            Some((a, b)) => format!(
                "{a}:{}",
                b.chars()
                    .map(|c| if c.is_ascii_digit() { '#' } else { c })
                    .collect::<String>()
            ),
            None => w.to_string(),
        })
        .collect()
}

/// Pairs the added lines of one gap with its removed lines: the pair with the most shared words
/// first (then the nearer one in the gap), each line once; a pair with no shared word is none.
fn match_gap(
    base: &[&str],
    head: &[&str],
    gone: &[usize],
    added: &[usize],
    out: &mut [Option<usize>],
) {
    let gs: Vec<HashSet<String>> = gone.iter().map(|&b| shape(base[b])).collect();
    let mut cand = Vec::new();
    for (x, &h) in added.iter().enumerate() {
        let hs = shape(head[h]);
        for (y, g) in gs.iter().enumerate() {
            let n = hs.intersection(g).count();
            if n > 0 {
                cand.push((std::cmp::Reverse(n), x.abs_diff(y), x, y));
            }
        }
    }
    cand.sort();
    let (mut used_a, mut used_g) = (vec![false; added.len()], vec![false; gone.len()]);
    for (_, _, x, y) in cand {
        if !used_a[x] && !used_g[y] {
            used_a[x] = true;
            used_g[y] = true;
            out[added[x]] = Some(gone[y]);
        }
    }
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

/// True if `head` is the line `base` (not blank) with a dated note appended (step 6 of the
/// module doc): after `base` come blanks, then `Note (YYYY-MM-DD` or `[YYYY-MM-DD`.
fn appended_note(base: &str, head: &str) -> bool {
    let Some(rest) = head.strip_prefix(base) else {
        return false;
    };
    if base.is_empty() || !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let rest = rest.trim_start();
    let Some(date) = rest
        .strip_prefix("Note (")
        .or_else(|| rest.strip_prefix('['))
    else {
        return false;
    };
    let d = date.as_bytes();
    d.len() >= 10
        && d[..10].iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
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

    /// Step 6 (M5-P1 fixer): a cited line with a dated note appended at its end is a note, not
    /// a finding; text after the line that is not a dated note, or an edit before the note,
    /// stays CHANGED.
    #[test]
    fn a_dated_note_appended_to_a_cited_line_is_a_note() {
        let root = repo("note");
        write(&root, "docs/adr/README.md", "a\n- Entry in vm.\nc\nd\n");
        let body = "\nAR:2 and AR:3 and AR:4.\n";
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
        write(
            &root,
            "docs/adr/README.md",
            "a\n- Entry in vm. Note (2026-10-29, M5-P1): it is split.\nc [2026-10-29, x:] done\nd and more\n",
        );
        commit(&root, "notes");
        let c = short_cites(&root).unwrap();
        assert_eq!(c.total, 3);
        for k in [2, 3] {
            assert!(
                c.notes
                    .iter()
                    .any(|n| n.starts_with(&format!("short cite NOTE ADDED doc13:6 AR:{k}:"))),
                "{k}: {:?}",
                c.notes
            );
        }
        assert_eq!(
            c.findings,
            vec![format!(
                "SHORT CITE CHANGED doc13:6 AR:4: the line at {} is not in the file at HEAD: d",
                &base[..7]
            )]
        );
        assert!(appended_note("x", "x Note (2026-10-29): y"));
        assert!(appended_note("x", "x  [2026-10-29, r:] y"));
        assert!(!appended_note("x", "xNote (2026-10-29)"));
        assert!(!appended_note("x", "x Note (soon)"));
        assert!(!appended_note("x", "x [2026-1-29]"));
        assert!(!appended_note("x", "y x Note (2026-10-29)"));
        assert!(!appended_note("", " Note (2026-10-29)"));
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

    /// M4-P5 CX5-C7 (Codex round 1): a doc 13 line that is new since the base names the line
    /// at HEAD it was written for, so it is checked at HEAD only. Codex's fixture: the base
    /// README has 3 lines; one commit adds a 4th and a new doc 13 line with a correct AR:4.
    /// A new cite past the end at HEAD is BAD AT HEAD; an unchanged doc 13 line keeps the
    /// base-vs-HEAD move check.
    #[test]
    fn a_new_doc_line_is_checked_at_head() {
        let root = repo("newline");
        write(&root, "docs/adr/README.md", "a\nb\nc\n");
        let body = "\nAR:2.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        let head = |extra: &str| format!("{}{body}{extra}", HEADER.replace("{BASE}", &base[..7]));
        write(&root, DOC, &head(""));
        commit(&root, "header");
        // One commit: a 4th README paragraph and a new doc 13 line that cites it.
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nc\nd, the new paragraph\n",
        );
        write(&root, DOC, &head("\nThe new rule: AR:4.\n"));
        commit(&root, "new paragraph");
        let c = short_cites(&root).unwrap();
        assert_eq!((c.total, c.findings.len()), (2, 0), "{:?}", c.findings);
        assert!(c.pass());
        // A new cite past the end of its file at HEAD.
        write(&root, DOC, &head("\nThe new rule: AR:4, AR:9.\n"));
        let c = short_cites(&root).unwrap();
        assert_eq!(
            c.findings,
            vec!["SHORT CITE BAD AT HEAD doc13:8 AR:9: docs/adr/README.md has no line 9 at HEAD"]
        );
        // The unchanged doc 13 line (AR:2) keeps the move check against the base.
        write(
            &root,
            "docs/adr/README.md",
            "a0\na\nb\nc\nd, the new paragraph\n",
        );
        write(&root, DOC, &head("\nThe new rule: AR:5.\n"));
        commit(&root, "move");
        let c = short_cites(&root).unwrap();
        assert_eq!(
            c.findings,
            vec![format!(
                "SHORT CITE MOVED doc13:6 AR:2 -> AR:3 (same text as at {}): b",
                &base[..7]
            )]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// CX6-3 (M4-P5; plan v10 section 3.7): a cite added after `Cites at` is checked at the
    /// commit that added it, not at HEAD only: a later move of its line is MOVED, also when the
    /// cite came in on a merged branch. A cite whose file had no such line at that commit is BAD
    /// AT INTRO. An uncommitted cite is checked at HEAD only.
    #[test]
    fn a_new_cite_is_checked_at_its_introduction_commit() {
        let root = repo("intro");
        write(&root, "docs/adr/README.md", "a\nb\nc\n");
        let body = "\nAR:2.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        let head = |extra: &str| format!("{}{body}{extra}", HEADER.replace("{BASE}", &base[..7]));
        write(&root, DOC, &head(""));
        commit(&root, "header");
        // One commit: a 4th README paragraph and a new doc 13 line that cites it.
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nc\nd, the new paragraph\n",
        );
        write(&root, DOC, &head("\nThe new rule: AR:4.\n"));
        let added = commit(&root, "new paragraph");
        assert_eq!(short_cites(&root).unwrap().findings, Vec::<String>::new());
        // A later commit moves both cited lines; doc 13 is not touched. The base cite (AR:2) and
        // the new one (AR:4, checked at the commit that added it) are both MOVED.
        write(
            &root,
            "docs/adr/README.md",
            "z\na\nb\nc\nd, the new paragraph\n",
        );
        commit(&root, "insert");
        let c = short_cites(&root).unwrap();
        assert_eq!(
            c.findings,
            vec![
                format!(
                    "SHORT CITE MOVED doc13:6 AR:2 -> AR:3 (same text as at {}): b",
                    &base[..7]
                ),
                format!(
                    "SHORT CITE MOVED doc13:8 AR:4 -> AR:5 (same text as at {}, the commit that added the cite): d, the new paragraph",
                    &added[..7]
                ),
            ]
        );
        assert!(!c.pass());
        // An uncommitted cite is checked at HEAD only (no commit wrote it).
        write(&root, DOC, &head("\nThe new rule: AR:4.\n\nMore: AR:1.\n"));
        let c = short_cites(&root).unwrap();
        assert_eq!(c.findings.len(), 2, "{:?}", c.findings);
        // The fix commit moves the numbers: the fixed cites were added there, so no finding.
        write(
            &root,
            "docs/adr/README.md",
            "z\na\nb\nc\nd, the new paragraph\ne\nf\n",
        );
        let fixed = |extra: &str| {
            format!(
                "{}\nAR:3.\n\nThe new rule: AR:5.\n{extra}",
                HEADER.replace("{BASE}", &base[..7])
            )
        };
        write(&root, DOC, &fixed(""));
        commit(&root, "fix");
        assert_eq!(short_cites(&root).unwrap().findings, Vec::<String>::new());
        // A cite written ahead of its line: no line 8 at the commit that added it (rule 31
        // moves a cite with its file in one commit).
        write(&root, DOC, &fixed("\nAhead: AR:8.\n"));
        let ahead = commit(&root, "ahead");
        write(
            &root,
            "docs/adr/README.md",
            "z\na\nb\nc\nd, the new paragraph\ne\nf\ng\n",
        );
        commit(&root, "g");
        assert_eq!(
            short_cites(&root).unwrap().findings,
            vec![format!(
                "SHORT CITE BAD AT INTRO doc13:10 AR:8: docs/adr/README.md has no line 8 at {}, the commit that added the cite",
                &ahead[..7]
            )]
        );
        write(&root, DOC, &fixed(""));
        let rows: String = (1..=20).map(|i| format!("row {i}\n")).collect();
        write(&root, "docs/adr/H31-fuses.md", &rows);
        commit(&root, "drop ahead");
        // A cite added on a branch (a role branch) and merged: checked at the branch commit.
        // The main line inserts a row at the top meanwhile; the merge moves the cited row from
        // 21 to 22.
        let main = git_in(&root, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_in(&root, &["checkout", "-q", "-b", "role"]);
        write(
            &root,
            "docs/adr/H31-fuses.md",
            &format!("{rows}row 21, the role's row\n"),
        );
        write(&root, DOC, &fixed("\nRole: H31:21.\n"));
        let role = commit(&root, "role");
        git_in(&root, &["checkout", "-q", &main]);
        write(&root, "docs/adr/H31-fuses.md", &format!("row 0\n{rows}"));
        commit(&root, "main moves");
        git_in(
            &root,
            &["merge", "-q", "--no-ff", "-m", "merge role", "role"],
        );
        assert_eq!(
            short_cites(&root).unwrap().findings,
            vec![format!(
                "SHORT CITE MOVED doc13:10 H31:21 -> H31:22 (same text as at {}, the commit that added the cite): row 21, the role's row",
                &role[..7]
            )]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A cite added in a merge resolution, in neither parent (Codex r1 gates6 [medium], M5-P1
    /// fixer round 3): it is checked at the merge commit. A later commit moves its line: MOVED.
    /// Without the merge diff in the `-S` search no commit added it, and the move was missed.
    #[test]
    fn a_cite_added_in_a_merge_resolution_is_checked_at_the_merge() {
        let root = repo("merge-intro");
        write(&root, "docs/adr/README.md", "a\nb\nc\n");
        let body = "\nAR:2.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        let head = |extra: &str| format!("{}{body}{extra}", HEADER.replace("{BASE}", &base[..7]));
        write(&root, DOC, &head(""));
        commit(&root, "header");
        let main = git_in(&root, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_in(&root, &["checkout", "-q", "-b", "side"]);
        write(&root, "side.txt", "side\n");
        commit(&root, "side");
        git_in(&root, &["checkout", "-q", &main]);
        write(&root, "main.txt", "main\n");
        commit(&root, "main");
        // The merge itself adds the README paragraph and the cite (a resolution edit).
        git_in(&root, &["merge", "-q", "--no-ff", "--no-commit", "side"]);
        write(
            &root,
            "docs/adr/README.md",
            "a\nb\nc\nd, the merge's paragraph\n",
        );
        write(&root, DOC, &head("\nMerge rule: AR:4.\n"));
        let merge = commit(&root, "merge side");
        assert_eq!(short_cites(&root).unwrap().findings, Vec::<String>::new());
        write(
            &root,
            "docs/adr/README.md",
            "z\na\nb\nc\nd, the merge's paragraph\n",
        );
        commit(&root, "insert");
        assert_eq!(
            short_cites(&root).unwrap().findings,
            vec![
                format!(
                    "SHORT CITE MOVED doc13:6 AR:2 -> AR:3 (same text as at {}): b",
                    &base[..7]
                ),
                format!(
                    "SHORT CITE MOVED doc13:8 AR:4 -> AR:5 (same text as at {}, the commit that added the cite): d, the merge's paragraph",
                    &merge[..7]
                ),
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The pairing of edited lines: the n-th added line in a gap pairs with the n-th removed
    /// one; an added line beyond the removed ones, and a kept line, pair with none.
    #[test]
    fn edited_lines_pair_with_the_lines_they_replaced() {
        let base = ["h", "row a AR:2", "row b AR:3", "t"];
        let head = [
            "h",
            "new AR:9",
            "row b AR:4 (see)",
            "row a AR:2 x",
            "t",
            "end",
        ];
        assert_eq!(
            paired(&base, &head),
            vec![None, None, Some(2), Some(1), None, None]
        );
        assert_eq!(paired(&[], &["x"]), vec![None]);
        assert_eq!(paired(&["x", "y"], &[]), Vec::<Option<usize>>::new());
    }

    /// M4-P5 CX5 verifier r1 F1: an edited doc 13 line (its text differs from the base) keeps
    /// the move check for each cite its base line had; only a cite whose number changed (or a
    /// line that replaced none) is checked at HEAD only. Else an edit that keeps a stale number
    /// hides a MOVED cite for good (after the next `Cites at` bump it is the same at both).
    #[test]
    fn an_edited_line_keeps_the_move_check() {
        let root = repo("edited");
        write(&root, "docs/adr/README.md", "a\nb\nc\nd\n");
        let body = "\nRule AR:2.\n\nOther AR:3.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        let head = |b: &str| format!("{}{b}", HEADER.replace("{BASE}", &base[..7]));
        write(&root, DOC, &head(body));
        commit(&root, "header");
        write(&root, "docs/adr/README.md", "z\na\nb\nc\nd\n");
        commit(&root, "move");
        // Both lines edited, the stale numbers kept: both still MOVED.
        write(
            &root,
            DOC,
            &head("\nRule AR:2 (see).\n\nOther AR:3 (see).\n"),
        );
        commit(&root, "edit");
        let c = short_cites(&root).unwrap();
        let short = &base[..7];
        assert_eq!(
            c.findings,
            vec![
                format!("SHORT CITE MOVED doc13:6 AR:2 -> AR:3 (same text as at {short}): b"),
                format!("SHORT CITE MOVED doc13:8 AR:3 -> AR:4 (same text as at {short}): c"),
            ]
        );
        // One line fixed (its number changed: checked at HEAD), the other still stale.
        write(
            &root,
            DOC,
            &head("\nRule AR:3 (see).\n\nOther AR:3, AR:5.\n"),
        );
        commit(&root, "half");
        let c = short_cites(&root).unwrap();
        assert_eq!(
            c.findings,
            vec![format!(
                "SHORT CITE MOVED doc13:8 AR:3 -> AR:4 (same text as at {short}): c"
            )]
        );
        // Every number +1 in one commit: no finding.
        write(&root, DOC, &head("\nRule AR:3.\n\nOther AR:4.\n"));
        commit(&root, "renumber");
        let c = short_cites(&root).unwrap();
        assert_eq!(c.findings, Vec::<String>::new());
        assert_eq!(c.total, 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// M4-P5 CX6-5 (Codex round 2): "carried" is decided per endpoint of a range. A range with
    /// one changed endpoint keeps the move check for the other one: AR:2-4 -> AR:2-5 after a
    /// line insert above line 2 gives MOVED for the start; AR:2-4 -> AR:3-4 for the end.
    #[test]
    fn a_range_keeps_the_move_check_per_endpoint() {
        let root = repo("range");
        write(&root, "docs/adr/README.md", "a\nb\nc\nd\ne\n");
        let body = "\nRule AR:2-4.\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        let short = base[..7].to_string();
        let head = |b: &str| format!("{}{b}", HEADER.replace("{BASE}", &short));
        write(&root, DOC, &head(body));
        commit(&root, "header");
        write(&root, "docs/adr/README.md", "z\na\nb\nc\nd\ne\n");
        commit(&root, "move");
        let check = |text: &str, want: Vec<String>| {
            write(&root, DOC, &head(text));
            commit(&root, "edit");
            let c = short_cites(&root).unwrap();
            assert_eq!(c.findings, want, "{text}");
            assert_eq!(c.total, 2, "{text}");
        };
        // The end fixed, the start stale: MOVED for the start.
        check(
            "\nRule AR:2-5.\n",
            vec![format!(
                "SHORT CITE MOVED doc13:6 AR:2 -> AR:3 (same text as at {short}): b"
            )],
        );
        // The start fixed, the end stale: MOVED for the end.
        check(
            "\nRule AR:3-4.\n",
            vec![format!(
                "SHORT CITE MOVED doc13:6 AR:4 -> AR:5 (same text as at {short}): d"
            )],
        );
        // Both fixed: none.
        check("\nRule AR:3-5.\n", vec![]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// CX5 verifier r2 F1: an edited line pairs with the removed line it shares the most words
    /// with, not with the one at its place in the gap. A row inserted above an edited row, two
    /// edited rows that swap, and a new line above an edited row keep the move check; a
    /// contiguous +1 renumber gives none.
    #[test]
    fn an_inserted_or_swapped_row_keeps_the_move_check() {
        let root = repo("gap");
        write(&root, "docs/adr/README.md", "a\nb\nc\nd\n");
        let body = "\n| r1 | AR:2 |\n| r2 | AR:3 |\n";
        write(
            &root,
            DOC,
            &format!("{}{body}", HEADER.replace("{BASE}", "0000000")),
        );
        let base = commit(&root, "base");
        let short = base[..7].to_string();
        let head = |b: &str| format!("{}{b}", HEADER.replace("{BASE}", &short));
        write(&root, DOC, &head(body));
        commit(&root, "header");
        write(&root, "docs/adr/README.md", "z\na\nb\nc\nd\n");
        commit(&root, "move");
        let r1 = |l: usize| {
            format!("SHORT CITE MOVED doc13:{l} AR:2 -> AR:3 (same text as at {short}): b")
        };
        let r2 = |l: usize| {
            format!("SHORT CITE MOVED doc13:{l} AR:3 -> AR:4 (same text as at {short}): c")
        };
        let check = |text: &str, want: Vec<String>| {
            write(&root, DOC, &head(text));
            commit(&root, "edit");
            assert_eq!(short_cites(&root).unwrap().findings, want, "{text}");
        };
        // A new row above r1, r1 edited with its stale AR:2 kept.
        check(
            "\n| r0 | new AR:5 |\n| r1 | AR:2 (see) |\n| r2 | AR:3 |\n",
            vec![r1(7), r2(8)],
        );
        // r1 and r2 both edited and swapped.
        check(
            "\n| r2 | AR:3 (see) |\n| r1 | AR:2 (see) |\n",
            vec![r2(6), r1(7)],
        );
        // A new line above an edited r1, no blank line between; r2 renumbered.
        check(
            "\nA new rule for the loop AR:1.\n| r1 | AR:2 (see) |\n| r2 | AR:4 |\n",
            vec![r1(7)],
        );
        // The contiguous table renumbered +1: none.
        check("\n| r1 | AR:3 |\n| r2 | AR:4 |\n", vec![]);
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
