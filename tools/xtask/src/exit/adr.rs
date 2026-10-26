//! The ADR index (docs/adr/README.md section 5) and its Counts line. Moved from m0_exit.rs
//! (M4-P0, no logic change).

use super::read;
use std::path::Path;

/// One row of the ADR index (docs/adr/README.md section 5).
#[derive(Debug, Clone)]
pub(crate) struct AdrRow {
    pub(crate) id: String,
    pub(crate) fix_by: String,
    pub(crate) status: String,
}

pub(crate) fn adr_index(text: &str) -> Result<Vec<AdrRow>, String> {
    let rows: Vec<AdrRow> = text
        .lines()
        .filter(|l| l.starts_with("| H"))
        .filter_map(|l| {
            let cols: Vec<&str> = l
                .split(" | ")
                .map(|c| c.trim_matches(|ch| ch == '|' || ch == ' '))
                .collect();
            (cols.len() >= 6 && cols[0][1..].bytes().all(|b| b.is_ascii_digit())).then(|| AdrRow {
                id: cols[0].to_string(),
                fix_by: cols[2].to_string(),
                status: cols[4].to_string(),
            })
        })
        .collect();
    if rows.len() < 37 {
        return Err(format!(
            "docs/adr/README.md: found {} index rows, expected H1-H37",
            rows.len()
        ));
    }
    Ok(rows)
}

pub(crate) fn merged(status: &str) -> bool {
    status.starts_with("Accepted") || status.starts_with("Frozen")
}

/// The README counts line, for the evidence.
pub(crate) fn counts_line(root: &Path) -> String {
    read(&root.join("docs/adr/README.md"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("Counts on"))
                .map(|l| l.split(". ").next().unwrap_or(l).to_string())
        })
        .unwrap_or_default()
}
