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

/// X16 (plan v11; the 825702d6 class): the findings of the README index as m0-exit reads it
/// ([`adr_index`]): it parses, and its rows are H1-H37, each once, in order. Empty: no finding.
/// adr-lint part 3 prints them, so an index that m0-exit cannot read fails adr-lint too.
pub(crate) fn index_findings(readme: &str) -> Vec<String> {
    let rows = match adr_index(readme) {
        Ok(r) => r,
        Err(e) => {
            return vec![format!(
                "{e} (the m0-exit index parse: rows `| H<n> | ... |` with 6 cells split by ` | `)"
            )];
        }
    };
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let want: Vec<String> = (1..=37).map(|n| format!("H{n}")).collect();
    if ids == want {
        Vec::new()
    } else {
        let missing: Vec<&str> = want
            .iter()
            .filter(|w| !ids.contains(&w.as_str()))
            .map(String::as_str)
            .collect();
        vec![format!(
            "docs/adr/README.md: the index rows are not H1-H37, each once, in order (read {} rows: {}{})",
            ids.len(),
            ids.join(" "),
            if missing.is_empty() {
                String::new()
            } else {
                format!("; missing {}", missing.join(" "))
            }
        )]
    }
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
