//! The ADR record checks of `cargo xtask adr-lint` (plan v9 section 5.6 task 1, V9-A15; README
//! sections 3.1-3.2): every amendment of an Accepted or Frozen ADR since `<base>` has its review
//! record and an approval, and the README Counts line equals the Status lines.
//!
//! **Review records.** For each `docs/adr/H*.md` whose Status line starts with `Accepted` or
//! `Frozen` (a Frozen ADR's part 2 included: the whole file is read):
//!
//! - The amendment rows are the rows of its `## Change log` table that the file at `<base>` does
//!   not have, dated on or after the day of the `<base>` commit (a changed older row is an edit
//!   that a newer row records). A row whose Change text starts with `Review record` only records
//!   record rows and needs no record of its own.
//! - Each amendment row names its kind first in the Change-kind cell: `clarify`, `fill`,
//!   `rewrite` or `reopen` (README section 3.3; V9-A15: every kind). Another word is a gap.
//! - The `## Review record` rows are `1 ...`, `2 ...`, `3 ...`, `4 ...` and `Approval`, each with a
//!   label in parentheses that names the date of the Change-log row it covers, for example
//!   `1 Reviewer (M4-P2 amendment: the 2026-10-27 ADRLINE clarify row)`. Only the rows that the
//!   file at `<base>` does not have count (a record already there covers an older row). A row
//!   whose result starts with `Pending` counts for nothing, and an Approval counts only when its
//!   result starts with `Approved` (after any `*` or `_` of markdown emphasis: `**Approved**`
//!   counts, `Not Approved` and `**Not approved**` do not).
//! - For each date: the complete records that name it (the smallest count of rows 1, 2, 3, 4 and
//!   Approval among the rows that name it) must be at least the amendment rows of that date. A
//!   Pending row that names the date is a gap, and so is a row whose Who or Result cell is empty
//!   (blank, or only markdown emphasis, code ticks or a dash): it counts for nothing.
//! - The approver of each approval names no writer of a row 1-4 of the same date (README section
//!   3.2: "The orchestrator never approves its own change"). The writer of a row is the text
//!   after `row text by ` in its Who cell, else the Who cell; in both cases up to the first ` (`,
//!   `,` or `;`, in lower case.
//!
//! - **Body changes.** When the text outside the `## Change log` and `## Review record` sections
//!   differs from the file at `<base>` (a file new since `<base>` is not compared), the ADR needs
//!   at least one amendment row (README section 3: an amendment adds a dated Change-log line).
//!   No amendment row is a gap.
//!
//! The date links a record to its Change-log rows: a label names no row id, so two rows of one
//! day share their records by count, and one amendment row covers every body change of the
//! range. That is the limit of this check.
//!
//! **Counts.** The README line `Counts on <date> (...): <n> Draft, <n> Not started, <n> Accepted,
//! <n> Frozen.` equals the Status lines: the first word of each `H<nn>-*.md` Status line (an ADR
//! with no file is Not started), and the index column (README section 5) of each row names the
//! same first word as its file.

use super::read;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// The amendment kinds (README section 3.3).
const KINDS: [&str; 4] = ["clarify", "fill", "rewrite", "reopen"];

/// The record parts (README section 3.1) and the approval.
const PARTS: [&str; 5] = ["1", "2", "3", "4", "Approval"];

/// The highest ADR id.
const MAX_ADR: u32 = 37;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The cells of a markdown table row.
fn cells(l: &str) -> Vec<String> {
    l.trim()
        .trim_matches('|')
        .split(" | ")
        .map(|c| c.trim().to_string())
        .collect()
}

/// A cell with no content: empty, or only markdown emphasis, code ticks or a dash.
fn blank(cell: &str) -> bool {
    cell.chars()
        .all(|c| c.is_whitespace() || "*_`-\u{2013}\u{2014}".contains(c))
}

/// The body rows of the first table under the `## <name>` heading (header and rule dropped).
fn section_rows(text: &str, name: &str) -> Vec<String> {
    let mut on = false;
    let mut rows = Vec::new();
    for l in text.lines() {
        if l.starts_with("## ") {
            on = l.trim() == format!("## {name}");
            continue;
        }
        if on && l.starts_with("| ") && !l.starts_with("|---") {
            rows.push(l.to_string());
        }
    }
    rows.into_iter().skip(1).collect()
}

/// The text outside the `## Change log` and `## Review record` sections (each runs from its
/// heading to the next `## ` heading).
fn body(text: &str) -> String {
    let mut on = true;
    let mut out = String::new();
    for l in text.lines() {
        if l.starts_with("## ") {
            let t = l.trim();
            on = t != "## Change log" && t != "## Review record";
        }
        if on {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// Does a Result cell approve: does it start with `Approved` after markdown emphasis?
fn approves(result: &str) -> bool {
    result
        .trim_start_matches(|c: char| c == '*' || c == '_' || c.is_whitespace())
        .starts_with("Approved")
}

/// The value of the `| Status |` row.
pub(crate) fn status_of(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with("| Status |"))
        .and_then(|l| cells(l).get(1).cloned())
}

/// The first `YYYY-MM-DD` in a text.
fn first_date(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(9)).find_map(|i| {
        let d = &b[i..i + 10];
        let ok = d.iter().enumerate().all(|(k, c)| match k {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        });
        let before = i == 0 || !b[i - 1].is_ascii_digit();
        let after = b.get(i + 10).is_none_or(|c| !c.is_ascii_digit());
        (ok && before && after).then(|| &s[i..i + 10])
    })
}

/// The writer of a record row, from its Who cell.
fn writer(who: &str) -> String {
    let w = who.split_once("row text by ").map_or(who, |(_, w)| w);
    let end = [" (", ",", ";"]
        .iter()
        .filter_map(|p| w.find(p))
        .min()
        .unwrap_or(w.len());
    w[..end].trim().to_lowercase()
}

/// The ADR files `docs/adr/H<nn>-*.md`, sorted, with their ids (`H16`).
fn adr_files(root: &Path) -> Result<Vec<(String, String)>, String> {
    let dir = root.join("docs/adr");
    let rd = std::fs::read_dir(&dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    let mut out: Vec<(String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let num = name.strip_prefix('H')?.split('-').next()?.to_string();
            (name.ends_with(".md")
                && num.len() == 2
                && num.bytes().all(|b| b.is_ascii_digit())
                && e.path().is_file())
            .then(|| {
                (
                    format!("H{}", num.trim_start_matches('0')),
                    format!("docs/adr/{name}"),
                )
            })
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// The review-record check.
#[derive(Debug, Default)]
pub(crate) struct Records {
    /// Accepted or Frozen ADRs read.
    pub(crate) adrs: usize,
    /// Amendment rows since the base (needing a record).
    pub(crate) rows: usize,
    /// Rows that need none: record-only rows, and edits of rows dated before the base.
    pub(crate) skipped: usize,
    pub(crate) gaps: Vec<String>,
}

pub(crate) fn records(root: &Path, base_sha: &str) -> Result<Records, String> {
    let base_day = git(root, &["show", "-s", "--format=%cs", base_sha, "--"])
        .map(|s| s.trim().to_string())
        .filter(|s| first_date(s).is_some())
        .ok_or_else(|| format!("cannot read the date of {base_sha}"))?;
    let mut r = Records::default();
    for (_, rel) in adr_files(root)? {
        let text = read(&root.join(&rel))?;
        let status = status_of(&text).unwrap_or_default();
        if !(status.starts_with("Accepted") || status.starts_with("Frozen")) {
            continue;
        }
        r.adrs += 1;
        let base_text = git(root, &["show", &format!("{base_sha}:{rel}")]);
        let old: BTreeSet<String> = base_text
            .as_deref()
            .map(|t| section_rows(t, "Change log").into_iter().collect())
            .unwrap_or_default();
        // The record rows at the base, with their counts (two equal rows are two rows).
        let mut old_records: BTreeMap<String, usize> = BTreeMap::new();
        for row in base_text
            .as_deref()
            .map(|t| section_rows(t, "Review record"))
            .unwrap_or_default()
        {
            *old_records.entry(row).or_default() += 1;
        }
        let mut need: BTreeMap<String, usize> = BTreeMap::new();
        for row in section_rows(&text, "Change log") {
            if old.contains(&row) {
                continue;
            }
            let c = cells(&row);
            let (date, kind, change) = (
                c.first().map_or("", String::as_str),
                c.get(1).map_or("", String::as_str),
                c.get(2).map_or("", String::as_str),
            );
            let Some(d) = first_date(date).filter(|d| *d == date) else {
                r.gaps.push(format!(
                    "{rel}: a new Change-log row has no date in its first cell: {:?}",
                    row.chars().take(100).collect::<String>()
                ));
                continue;
            };
            if d < base_day.as_str() || change.starts_with("Review record") {
                r.skipped += 1;
                continue;
            }
            r.rows += 1;
            let word = kind
                .split(|ch: char| ch.is_whitespace() || "(;,".contains(ch))
                .next()
                .unwrap_or("");
            if !KINDS.contains(&word) {
                r.gaps.push(format!(
                    "{rel}: the {d} row of kind {kind:?} is not an amendment kind (clarify, fill, rewrite or reopen; README section 3.3): {:?}",
                    change.chars().take(80).collect::<String>()
                ));
            }
            *need.entry(d.to_string()).or_default() += 1;
        }
        if need.is_empty() {
            if base_text.as_deref().is_some_and(|b| body(b) != body(&text)) {
                r.gaps.push(format!(
                    "{rel}: the text outside `## Change log` and `## Review record` changed since the base, and no new Change-log row dated on or after {base_day} records it (README section 3: an amendment adds a dated Change-log line, then needs its review record)"
                ));
            }
            continue;
        }
        // Per date: complete rows per part, the writers of rows 1-4, the approvers.
        let mut have: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
        let mut writers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut approvers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for row in section_rows(&text, "Review record") {
            // A row the base file has covers an older Change-log row, not a new one.
            if let Some(n) = old_records.get_mut(&row)
                && *n > 0
            {
                *n -= 1;
                continue;
            }
            let c = cells(&row);
            let part_cell = c.first().map_or("", String::as_str);
            let Some(part) = PARTS
                .iter()
                .find(|p| {
                    part_cell
                        .strip_prefix(**p)
                        .is_some_and(|rest| rest.starts_with(' ') || rest.is_empty())
                })
                .copied()
            else {
                continue;
            };
            let label = match (part_cell.find('('), part_cell.ends_with(')')) {
                (Some(i), true) => &part_cell[i + 1..part_cell.len() - 1],
                _ => continue,
            };
            let Some(d) = first_date(label) else {
                continue;
            };
            if !need.contains_key(d) {
                continue;
            }
            let who = c.get(1).map_or("", String::as_str);
            let result = c.get(2).map_or("", String::as_str);
            if result.starts_with("Pending") || who.starts_with("Pending") {
                r.gaps.push(format!(
                    "{rel}: record row {part_cell:?} is Pending (the {d} amendment rows wait for it)"
                ));
                continue;
            }
            if blank(who) || blank(result) {
                r.gaps.push(format!(
                    "{rel}: record row {part_cell:?} has an empty Who or Result cell (no reviewer or no result; the {d} amendment rows need both)"
                ));
                continue;
            }
            if part == "Approval" {
                if !approves(result) {
                    continue;
                }
                approvers
                    .entry(d.to_string())
                    .or_default()
                    .insert(writer(who));
            } else {
                writers
                    .entry(d.to_string())
                    .or_default()
                    .insert(writer(who));
            }
            *have
                .entry(d.to_string())
                .or_default()
                .entry(part)
                .or_default() += 1;
        }
        for (d, n) in &need {
            let h = have.get(d);
            let count = |p: &str| h.and_then(|h| h.get(p)).copied().unwrap_or(0);
            let sets = PARTS.iter().map(|p| count(p)).min().unwrap_or(0);
            if sets < *n {
                let parts: Vec<String> =
                    PARTS.iter().map(|p| format!("{p}: {}", count(p))).collect();
                r.gaps.push(format!(
                    "{rel}: {n} amendment row(s) dated {d} since the base, {sets} complete review record(s) name that date (rows 1-4 and an approved Approval; README section 3.1; found {})",
                    parts.join(", ")
                ));
            }
            let w = writers.get(d).cloned().unwrap_or_default();
            for a in approvers.get(d).into_iter().flatten() {
                if w.contains(a) {
                    r.gaps.push(format!(
                        "{rel}: the {d} approval is by {a:?}, who also wrote a row 1-4 of that date (README section 3.2)"
                    ));
                }
            }
        }
    }
    Ok(r)
}

/// The Counts check: (the README Counts line, the counts from the files, findings).
pub(crate) fn counts(root: &Path) -> Result<(String, String, Vec<String>), String> {
    let readme = read(&root.join("docs/adr/README.md"))?;
    let line = readme
        .lines()
        .find(|l| l.starts_with("Counts on"))
        .ok_or("docs/adr/README.md has no `Counts on` line")?;
    let head = line.split(". ").next().unwrap_or(line);
    let said = head.split_once("): ").map_or("", |(_, rest)| rest);
    let names = ["Draft", "Not started", "Accepted", "Frozen"];
    let mut stated: BTreeMap<&str, Option<u64>> = BTreeMap::new();
    for part in said.trim_end_matches('.').split(", ") {
        if let Some((n, what)) = part.split_once(' ')
            && let Some(name) = names.iter().find(|w| **w == what.trim())
        {
            stated.insert(name, n.parse().ok());
        }
    }
    let files = adr_files(root)?;
    let mut found: BTreeMap<&str, u64> = names.iter().map(|n| (*n, 0)).collect();
    let mut first_words: BTreeMap<String, String> = BTreeMap::new();
    let mut findings = Vec::new();
    for id in 1..=MAX_ADR {
        let hid = format!("H{id}");
        let word = match files.iter().find(|(i, _)| *i == hid) {
            None => "Not started".to_string(),
            Some((_, rel)) => {
                let s = status_of(&read(&root.join(rel))?).unwrap_or_default();
                let w = s.split(|c: char| !c.is_alphabetic()).next().unwrap_or("");
                match names.iter().find(|n| **n == w) {
                    Some(n) => n.to_string(),
                    None => {
                        findings.push(format!("{rel}: Status {s:?} starts with no status word"));
                        continue;
                    }
                }
            }
        };
        if let Some(v) = found.get_mut(word.as_str()) {
            *v += 1;
        }
        first_words.insert(hid, word);
    }
    for n in names {
        match stated.get(n) {
            Some(Some(v)) if *v == found[n] => {}
            Some(Some(v)) => findings.push(format!(
                "README Counts line says {v} {n}, the Status lines give {}",
                found[n]
            )),
            _ => findings.push(format!("README Counts line names no count of {n}")),
        }
    }
    // The index column (README section 5) against each file's first word.
    for l in readme.lines().filter(|l| l.starts_with("| H")) {
        let c = cells(l);
        let (Some(id), Some(st)) = (c.first(), c.get(4)) else {
            continue;
        };
        if !id[1..].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let w = if st.starts_with("Not started") {
            "Not started"
        } else {
            st.split(|ch: char| !ch.is_alphabetic())
                .next()
                .unwrap_or("")
        };
        if let Some(fw) = first_words.get(id)
            && fw != w
        {
            findings.push(format!(
                "README index row {id} says {w:?}, its Status line says {fw:?}"
            ));
        }
    }
    let from_files = names
        .iter()
        .map(|n| format!("{} {n}", found[n]))
        .collect::<Vec<_>>()
        .join(", ");
    Ok((head.to_string(), from_files, findings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(first_date("the 2026-10-27 row"), Some("2026-10-27"));
        assert_eq!(first_date("12026-10-27 2026-10-26"), Some("2026-10-26"));
        assert_eq!(first_date("no date"), None);
        assert_eq!(
            writer(
                "orchestrator (Codex gpt-6.1-sol); row text by the M4-P2 records fixer, part 1 (Claude)"
            ),
            "the m4-p2 records fixer"
        );
        assert_eq!(
            writer("orchestrator (README §3.2); it wrote none"),
            "orchestrator"
        );
        assert_eq!(writer("M4-P1 Claude reviewer"), "m4-p1 claude reviewer");
        assert!(approves(
            "**Approved** (2026-10-27). The orchestrator wrote none"
        ));
        assert!(approves("**Approved.** Orchestrator approves"));
        assert!(approves("Approved"));
        assert!(!approves("Not Approved: row 3 is wrong"));
        assert!(!approves(
            "**Not approved**: the review record is not complete"
        ));
        assert!(!approves("**Pending** (not approved)"));
        assert_eq!(
            body(
                "# H\n\n## Decision\n\nA\n\n## Change log\n\n| r |\n## Review record\n| x |\n## Notes\nB\n"
            ),
            "# H\n\n## Decision\n\nA\n\n## Notes\nB\n"
        );
        assert_eq!(
            cells("| a | b \\| c | d |"),
            vec!["a".to_string(), "b \\| c".into(), "d".into()]
        );
        assert_eq!(
            section_rows(
                "## Change log\n\n| A | B |\n|---|---|\n| 1 | 2 |\n## Next\n| x | y |\n",
                "Change log"
            ),
            vec!["| 1 | 2 |".to_string()]
        );
    }
}
