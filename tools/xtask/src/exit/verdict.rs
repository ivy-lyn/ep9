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
