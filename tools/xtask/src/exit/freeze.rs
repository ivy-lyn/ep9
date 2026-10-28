//! The Frozen state of an ADR, part by part (m1-exit items 7 and 9; MODES-CHECK MC-w14-15: read
//! each Freeze-table part, not a whole-row Status word).
//!
//! An ADR with a Freeze table (README section 3.4: a `## Freeze table` or `### Freeze table`
//! heading, then a table with the columns `Part` and `Freeze point`) is read part by part:
//!
//! | Freeze point cell | The part at M1 exit |
//! |---|---|
//! | names `M0 exit`, `M1a` or `M1 exit` | due: it must be Frozen |
//! | `None` (an append-only list) | open: an append is a fill, never a freeze |
//! | anything else (a later gate) | not due yet |
//!
//! A due part is Frozen when the ADR's Status line starts with `Frozen` and the first field of
//! its parenthesis (up to the first `;` or `)`) names the part, as H20 does: `Frozen (core;
//! ...)`. Several parts are listed with `,` or ` and `. A part name is its Part cell without a
//! trailing `(append-only)` or `(gated)`, in any case. An ADR without a Freeze table freezes
//! whole: its Status line starts with `Frozen`.

use super::read;
use super::records::status_of;
use std::path::Path;

/// One row of a Freeze table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Part {
    pub(crate) name: String,
    pub(crate) freeze_point: String,
}

/// The parts of the first Freeze table of an ADR text (empty: no table).
pub(crate) fn freeze_table(text: &str) -> Vec<Part> {
    let mut lines = text.lines();
    if !lines.any(|l| {
        let t = l.trim_start_matches('#');
        l.starts_with("##") && t.trim_start().starts_with("Freeze table")
    }) {
        return Vec::new();
    }
    let cells = |l: &str| -> Vec<String> {
        l.trim()
            .trim_matches('|')
            .split(" | ")
            .map(|c| c.trim().to_string())
            .collect()
    };
    let mut cols: Option<(usize, usize)> = None;
    let mut out = Vec::new();
    for l in lines {
        if l.starts_with("#") {
            break;
        }
        if !l.starts_with('|') {
            if cols.is_some() {
                break;
            }
            continue;
        }
        let c = cells(l);
        match cols {
            None => {
                let p = c.iter().position(|x| x == "Part");
                let f = c.iter().position(|x| x == "Freeze point");
                if let (Some(p), Some(f)) = (p, f) {
                    cols = Some((p, f));
                }
            }
            Some((p, f)) => {
                if l.starts_with("|---") {
                    continue;
                }
                if let (Some(name), Some(fp)) = (c.get(p), c.get(f)) {
                    out.push(Part {
                        name: name.clone(),
                        freeze_point: fp.clone(),
                    });
                }
            }
        }
    }
    out
}

/// The part name without a trailing `(append-only)` or `(gated)`, in lower case.
fn base_name(name: &str) -> String {
    let n = name.trim();
    let n = n
        .strip_suffix("(append-only)")
        .or_else(|| n.strip_suffix("(gated)"))
        .unwrap_or(n);
    n.trim().to_lowercase()
}

/// When a part's freeze point falls, seen from M1 exit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Due {
    ByM1Exit,
    Open,
    Later,
}

pub(crate) fn due(freeze_point: &str) -> Due {
    let l = freeze_point.to_lowercase();
    if l.trim() == "none" {
        Due::Open
    } else if ["m0 exit", "m1a", "m1 exit"].iter().any(|k| l.contains(k)) {
        Due::ByM1Exit
    } else {
        Due::Later
    }
}

/// The part names that a Status line freezes (empty: not Frozen).
fn frozen_names(status: &str) -> Option<Vec<String>> {
    let rest = status.strip_prefix("Frozen")?;
    let inner = rest.trim_start().strip_prefix('(').unwrap_or("");
    let first = inner.split([';', ')']).next().unwrap_or("");
    Some(
        first
            .split(',')
            .flat_map(|x| x.split(" and "))
            .map(base_name)
            .filter(|x| !x.is_empty())
            .collect(),
    )
}

/// The Frozen check of one ADR at M1 exit: (every due part is Frozen, the evidence).
pub(crate) fn frozen_by_m1(root: &Path, id: &str) -> (bool, String) {
    let num = id.trim_start_matches('H');
    let prefix = format!("H{num:0>2}-");
    let file = std::fs::read_dir(root.join("docs/adr"))
        .ok()
        .and_then(|rd| {
            let mut v: Vec<String> = rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(&prefix) && n.ends_with(".md"))
                .collect();
            v.sort();
            v.into_iter().next()
        });
    let Some(file) = file else {
        return (false, format!("{id}: no docs/adr/{prefix}*.md"));
    };
    let text = match read(&root.join("docs/adr").join(&file)) {
        Ok(t) => t,
        Err(e) => return (false, format!("{id}: {e}")),
    };
    let status = status_of(&text).unwrap_or_default();
    let word = status
        .split(|c: char| !c.is_alphabetic())
        .next()
        .unwrap_or("")
        .to_string();
    let names = frozen_names(&status);
    let parts = freeze_table(&text);
    if parts.is_empty() {
        let ok = names.is_some();
        return (
            ok,
            format!(
                "{id}: {} (no Freeze table: the whole row{})",
                if ok { "Frozen" } else { word.as_str() },
                if ok { "" } else { " is not Frozen" }
            ),
        );
    }
    let names = names.unwrap_or_default();
    let mut missing = Vec::new();
    let mut held = Vec::new();
    let mut open = Vec::new();
    let mut later = Vec::new();
    for p in &parts {
        match due(&p.freeze_point) {
            Due::ByM1Exit => {
                if names.contains(&base_name(&p.name)) {
                    held.push(p.name.clone());
                } else {
                    missing.push(format!("{} ({})", p.name, p.freeze_point));
                }
            }
            Due::Open => open.push(p.name.clone()),
            Due::Later => later.push(format!("{} ({})", p.name, p.freeze_point)),
        }
    }
    let ok = missing.is_empty() && !held.is_empty();
    let mut t = format!("{id} (Status {word}): ");
    if held.is_empty() && missing.is_empty() {
        t.push_str("no part of its Freeze table freezes by M1 exit");
    } else {
        if !held.is_empty() {
            t.push_str(&format!("Frozen: {}", held.join(", ")));
        }
        if !missing.is_empty() {
            if !held.is_empty() {
                t.push_str("; ");
            }
            t.push_str(&format!(
                "due by M1 exit, not Frozen: {}",
                missing.join(", ")
            ));
        }
    }
    if !open.is_empty() {
        t.push_str(&format!(
            "; open lists (an append is a fill): {}",
            open.join(", ")
        ));
    }
    if !later.is_empty() {
        t.push_str(&format!("; later parts: {}", later.join(", ")));
    }
    (ok, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADR: &str = "# H29\n\n| Field | Value |\n|---|---|\n| Status | STATUS |\n\n## Freeze table\n\nText.\n\n\
        | Part | What it holds | Freeze point | After |\n|---|---|---|---|\n\
        | Contract | x | M1 exit | rules |\n\
        | Record tags (append-only) | x | None | append |\n\
        | A4-built (gated) | x | The A4 gates (M8 exit) | rules |\n\n## Change log\n";

    fn tree(status: &str, adr: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let k = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ep9-freeze-{}-{k}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/adr")).unwrap();
        std::fs::write(
            root.join("docs/adr/H29-deopt.md"),
            adr.replace("STATUS", status),
        )
        .unwrap();
        root
    }

    #[test]
    fn parts_are_read_one_by_one() {
        let parts = freeze_table(ADR);
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts
                .iter()
                .map(|p| due(&p.freeze_point))
                .collect::<Vec<_>>(),
            [Due::ByM1Exit, Due::Open, Due::Later]
        );
        // A Draft is not Frozen; a whole-row Frozen word that names no part does not freeze it.
        for (status, ok) in [
            ("Draft (2026-10-22)", false),
            ("Frozen (2026-11-30, M1 exit)", false),
            ("Frozen", false),
            ("Frozen (contract; 2026-11-30, M1 exit)", true),
            ("Frozen (Contract and record tags; M1 exit)", true),
            ("Accepted (contract)", false),
        ] {
            let root = tree(status, ADR);
            let (got, text) = frozen_by_m1(&root, "H29");
            assert_eq!(got, ok, "{status}: {text}");
            assert!(
                text.contains("open lists (an append is a fill): Record tags (append-only)"),
                "{text}"
            );
            assert!(text.contains("later parts: A4-built (gated)"), "{text}");
            std::fs::remove_dir_all(&root).unwrap();
        }
        // No Freeze table: the whole row.
        let plain = "# H16\n\n| Status | STATUS |\n";
        for (status, ok) in [("Frozen (2026-12-01)", true), ("Accepted", false)] {
            let root = tree(status, plain);
            assert_eq!(frozen_by_m1(&root, "H29").0, ok, "{status}");
            std::fs::remove_dir_all(&root).unwrap();
        }
        let root = tree("Frozen", plain);
        assert!(!frozen_by_m1(&root, "H30").0);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
