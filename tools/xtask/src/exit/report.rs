//! Cells and their status, and small text helpers for the evidence. Moved from m0_exit.rs
//! (M4-P0, no logic change), plus the stub-marker grep that m0-exit and m1-exit share.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Cell status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Status {
    Pass,
    /// N/A by design: the item has no reading on this column, and that counts as a pass. Only
    /// the Windows cell of item 4 has it (a gate-host speed reading; plan-v8 V8-A22).
    NotApplicable,
    Open,
    NotRun,
    Fail,
}

impl Status {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::NotApplicable => "N/A",
            Status::Fail => "FAIL",
            Status::Open => "OPEN",
            Status::NotRun => "NOT-RUN",
        }
    }

    /// PASS, or N/A by design.
    pub(crate) fn ok(self) -> bool {
        matches!(self, Status::Pass | Status::NotApplicable)
    }
}

/// One cell: a status and why.
#[derive(Clone, Debug)]
pub(crate) struct Cell {
    pub(crate) status: Status,
    pub(crate) evidence: String,
}

pub(crate) fn cell(status: Status, evidence: impl Into<String>) -> Cell {
    Cell {
        status,
        evidence: evidence.into(),
    }
}

pub(crate) fn all_ok(cells: &[(Cell, Cell)]) -> bool {
    cells.iter().all(|(h, w)| h.status.ok() && w.status.ok())
}

pub(crate) fn not_given(flag: &str, how: &str) -> Cell {
    cell(
        Status::NotRun,
        format!("no {flag} input; make it with `{how}`"),
    )
}

/// Up to 5 paths, for the evidence.
pub(crate) fn some_paths(paths: &[String]) -> String {
    let mut t = paths.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if paths.len() > 5 {
        let _ = write!(t, " and {} more", paths.len() - 5);
    }
    t
}

pub(crate) fn short(sha: &str) -> String {
    if sha.len() > 12 {
        format!("{}…{}", &sha[..8], &sha[sha.len() - 5..])
    } else {
        sha.to_string()
    }
}

/// The stub-marker tree check (plan rule 4): every line of a file under one of `dirs` that holds
/// one of `markers` (`git grep -F`, tracked and untracked files, not ignored ones; outside a git
/// work tree, every file; a dir that does not exist matches nothing). PASS when there is none,
/// OPEN with the lines when there are some, NOT-RUN when git fails. Cut out of m0-exit's
/// `tree_checks` (M4-P0): same text for its two markers and its dirs crates, spikes and tools.
pub(crate) fn stub_markers(root: &Path, markers: &[&str], dirs: &[&str]) -> Cell {
    let grep = |extra: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(root).args(["grep", "-n", "-I", "-F"]);
        cmd.args(extra);
        for &m in markers {
            cmd.args(["-e", m]);
        }
        cmd.arg("--").args(dirs).output()
    };
    // Tracked and untracked files (not ignored ones: build output), as grep -r over the tree;
    // outside a git work tree, every file.
    let out = grep(&["--untracked", "--exclude-standard"]).and_then(|o| {
        if o.status.code() == Some(128) {
            grep(&["--no-index", "--exclude-standard"])
        } else {
            Ok(o)
        }
    });
    match out {
        Ok(o) if o.status.code() == Some(1) => {
            cell(Status::Pass, format!("no {} line", or_list(markers)))
        }
        Ok(o) if o.status.success() => {
            let lines: Vec<String> = String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            cell(
                Status::Open,
                format!(
                    "{} stub line(s); each one's owner closes it (W12 if its role slipped): {}",
                    lines.len(),
                    some_paths(&lines)
                ),
            )
        }
        Ok(o) => cell(
            Status::NotRun,
            format!(
                "git grep failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        ),
        Err(e) => cell(Status::NotRun, format!("cannot run git: {e}")),
    }
}

/// `` `a` ``, `` `a` or `b` ``, `` `a`, `b` or `c` ``.
fn or_list(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|i| format!("`{i}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}
