//! Spike verdicts: markdown tables with a "Spike" and a "Verdict" column. Moved from m0_exit.rs
//! (M4-P0, no logic change).

use std::path::{Path, PathBuf};

/// The newest `docs/measurements/*-m0-spike-review.md` (file names start with the date).
pub(crate) fn newest_spike_review(root: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root.join("docs/measurements"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("-m0-spike-review.md"))
        })
        .collect();
    v.sort();
    v.pop()
}

/// (spike name in lower case, verdict text) for each row of each table that has a "Spike" and a
/// "Verdict" column. A cell "Spike B, SP-Unit" names two spikes.
pub(crate) fn verdict_table(md: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cols: Option<(usize, usize)> = None;
    for line in md.lines() {
        let l = line.trim();
        if !l.starts_with('|') {
            cols = None;
            continue;
        }
        let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
        if cols.is_none() {
            let s = cells.iter().position(|c| *c == "Spike");
            let v = cells.iter().position(|c| *c == "Verdict");
            if let (Some(s), Some(v)) = (s, v) {
                cols = Some((s, v));
            }
            continue;
        }
        let (s, v) = cols.unwrap_or((0, 0));
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        let (Some(name), Some(verdict)) = (cells.get(s), cells.get(v)) else {
            continue;
        };
        for n in name.split(',') {
            let n = n.trim().trim_matches('*').to_ascii_lowercase();
            if !n.is_empty() {
                out.push((n, verdict.trim_matches('*').to_string()));
            }
        }
    }
    out
}

/// One row of a Verdict table in the plan v10 form (V10-D37, MC-w17-9): the columns Spike,
/// Condition, Verdict, Commit, Lowering, Host/CPU and ADRs. Only Spike and Verdict must be there;
/// a missing column or an empty cell is `None` (the m1-exit reader then refuses the row).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VerdictRow {
    /// The spike, in lower case (a cell "Spike B, SP-Unit" gives two rows).
    pub(crate) spike: String,
    /// The protocol condition (`C1` … `C5c`), or "" when the table has no Condition column.
    pub(crate) condition: String,
    pub(crate) verdict: String,
    /// The commit the verdict measured (backticks and `*` removed).
    pub(crate) commit: Option<String>,
    /// The lowering label (`jit`; SP-Facts `jit, PROFILED input`), whitespace folded.
    pub(crate) lowering: Option<String>,
}

/// The rows of each table that has a "Spike" and a "Verdict" column, with the Condition, Commit
/// and Lowering cells when the table has those columns. A header cell names a column when it is
/// the name or starts with the name and a space ("Commit (40 hex)"). [`verdict_table`] stays the
/// M0 reader (m0-exit and the carried M0 item-5 cell read it), so their output does not change.
pub(crate) fn verdict_rows(md: &str) -> Vec<VerdictRow> {
    struct Cols {
        spike: usize,
        verdict: usize,
        condition: Option<usize>,
        commit: Option<usize>,
        lowering: Option<usize>,
    }
    let named = |c: &str, name: &str| c == name || c.starts_with(&format!("{name} "));
    let clean = |c: &str| {
        c.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_matches(|ch| ch == '*' || ch == '`')
            .trim()
            .to_string()
    };
    let mut out = Vec::new();
    let mut cols: Option<Cols> = None;
    for line in md.lines() {
        let l = line.trim();
        if !l.starts_with('|') {
            cols = None;
            continue;
        }
        let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
        let Some(k) = &cols else {
            let find = |name: &str| cells.iter().position(|c| named(c, name));
            if let (Some(spike), Some(verdict)) = (find("Spike"), find("Verdict")) {
                cols = Some(Cols {
                    spike,
                    verdict,
                    condition: find("Condition"),
                    commit: find("Commit"),
                    lowering: find("Lowering"),
                });
            }
            continue;
        };
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        let (Some(name), Some(verdict)) = (cells.get(k.spike), cells.get(k.verdict)) else {
            continue;
        };
        let opt = |i: Option<usize>| {
            i.and_then(|i| cells.get(i))
                .map(|c| clean(c))
                .filter(|c| !c.is_empty())
        };
        let condition = opt(k.condition).unwrap_or_default();
        let commit = opt(k.commit);
        let lowering = opt(k.lowering);
        for n in name.split(',') {
            let n = n.trim().trim_matches('*').to_ascii_lowercase();
            if !n.is_empty() {
                out.push(VerdictRow {
                    spike: n,
                    condition: condition.clone(),
                    verdict: verdict.trim_matches('*').to_string(),
                    commit: commit.clone(),
                    lowering: lowering.clone(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The V10-D37 columns: each cell read; a table without Commit or Lowering gives `None`; the
    /// M0 reader still reads the same (spike, verdict) pairs.
    #[test]
    fn verdict_rows_read_the_v10_columns() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let md = format!(
            "| Spike | Condition | Verdict | Commit (40 hex) | Lowering | Host/CPU | ADRs |\n\
             |---|---|---|---|---|---|---|\n\
             | Spike C | C1 | **GO** | `{sha}` | `jit` | m5max, Apple M5 Max | H2 |\n\
             | SP-Facts | C5c | GO, speed recorded | {sha} | jit,  PROFILED input | m5max | H30 |\n\
             | Spike C | C4 | GO |  |  | m5max | H2 |\n\
             \n\
             | Spike | Gate | Verdict | ADRs |\n\
             |---|---|---|---|\n\
             | Spike B, SP-Unit | g | GO | H5 |\n"
        );
        let rows = verdict_rows(&md);
        assert_eq!(rows.len(), 5, "{rows:?}");
        assert_eq!(
            rows[0],
            VerdictRow {
                spike: "spike c".into(),
                condition: "C1".into(),
                verdict: "GO".into(),
                commit: Some(sha.into()),
                lowering: Some("jit".into()),
            }
        );
        assert_eq!(rows[1].lowering.as_deref(), Some("jit, PROFILED input"));
        assert_eq!(rows[1].verdict, "GO, speed recorded");
        assert_eq!(
            (rows[2].commit.as_deref(), rows[2].lowering.as_deref()),
            (None, None)
        );
        for r in &rows[3..] {
            assert_eq!((r.condition.as_str(), r.commit.as_deref()), ("", None));
        }
        assert_eq!(
            (rows[3].spike.as_str(), rows[4].spike.as_str()),
            ("spike b", "sp-unit")
        );
        let old: Vec<(String, String)> = rows
            .iter()
            .map(|r| (r.spike.clone(), r.verdict.clone()))
            .collect();
        assert_eq!(verdict_table(&md), old);
    }
}
