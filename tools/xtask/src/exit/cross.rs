//! Cross-engine runs (`tools/bench/cross`, ep9-cross/v1): the freshness of a run and the M0
//! canary cell. Moved from m0_exit.rs (M4-P0, no logic change).

use super::git;
use super::json::Json;
use super::report::{Cell, Status, cell, short};
use std::path::Path;
use std::process::Command;

/// Item 4 freshness (plan-v8 V8-A14): the cross run measured this tree, or an ancestor of HEAD
/// with no change under `crates/` or `tools/bench/` since. Err(why) gives OPEN.
pub(crate) fn cross_fresh(root: &Path, commit: &str) -> Result<String, String> {
    let head = git(root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .ok_or("cannot read HEAD (not a git tree?)")?;
    if commit.is_empty() || commit == "unknown" {
        return Err("the run names no ep9 commit".into());
    }
    let full = git(
        root,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .ok_or_else(|| format!("its commit `{commit}` is not in this repository"))?;
    if full == head {
        return Ok("this tree (HEAD)".into());
    }
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", &full, &head])
        .status()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !ancestor.success() {
        return Err(format!(
            "its commit `{}` is not an ancestor of HEAD `{}`",
            short(&full),
            short(&head)
        ));
    }
    let diff = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "diff",
            "--quiet",
            &full,
            &head,
            "--",
            "crates",
            "tools/bench",
        ])
        .status()
        .map_err(|e| format!("cannot run git: {e}"))?;
    match diff.code() {
        Some(0) => Ok(format!(
            "an ancestor of HEAD `{}` with no change under crates/ or tools/bench/ since",
            short(&head)
        )),
        Some(1) => Err(format!(
            "not this tree: crates/ or tools/bench/ changed between its commit `{}` and HEAD `{}` (a fresh gate-host run closes it)",
            short(&full),
            short(&head)
        )),
        _ => Err("git diff failed".into()),
    }
}

pub(crate) fn cross_canary(p: &Path, j: &Json, root: &Path) -> Cell {
    if j.get("schema").and_then(Json::str) != Some("ep9-cross/v1") {
        return cell(
            Status::Fail,
            format!("{}: not an ep9-cross/v1 run", p.display()),
        );
    }
    let commit = j
        .path(&["commit", "ep9"])
        .and_then(Json::str)
        .unwrap_or("unknown");
    let provisional = j.get("provisional").and_then(Json::bool).unwrap_or(true);
    let gate_host = j
        .path(&["host", "gate_host"])
        .and_then(Json::bool)
        .unwrap_or(false);
    let rows = j.path(&["M", "ratios"]).and_then(Json::arr).unwrap_or(&[]);
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    for w in ["richards", "deltablue"] {
        let row = rows.iter().find(|r| {
            r.get("workload").and_then(Json::str) == Some(w)
                && r.get("comparator").and_then(Json::str) == Some("qjs")
        });
        let Some(row) = row else {
            status = Status::Fail;
            parts.push(format!("{w}: no row vs QuickJS (did it run?)"));
            continue;
        };
        // The gate reads net ratios only (run.mjs label()): a gross fallback row, written when
        // net time is <= 0 in a round, passes or fails nothing.
        let metric = row.get("metric").and_then(Json::str).unwrap_or("");
        let invalid = row.get("net_invalid").and_then(Json::str);
        let Some(net) = row.get("net").filter(|n| **n != Json::Null) else {
            status = status.max(Status::Open);
            parts.push(format!(
                "{w}: net invalid ({}): no gate verdict (a {metric} ratio is shown, not gated)",
                invalid.unwrap_or("no net ratio")
            ));
            continue;
        };
        if metric != "net" || invalid.is_some() {
            status = status.max(Status::Open);
            parts.push(format!(
                "{w}: metric {metric:?}, net invalid ({}): no gate verdict",
                invalid.unwrap_or("none given")
            ));
            continue;
        }
        let median = net.get("median").and_then(Json::num).unwrap_or(0.0);
        let low = net
            .get("interval")
            .and_then(Json::arr)
            .and_then(|a| a.first())
            .and_then(Json::num)
            .unwrap_or(0.0);
        let void = net
            .get("void")
            .or_else(|| row.get("void"))
            .and_then(Json::bool)
            .unwrap_or(true);
        if low < 0.6 {
            status = Status::Fail;
        } else if void {
            status = status.max(Status::Open);
        }
        parts.push(format!(
            "{w} net {median:.3}x (interval low end {low:.3}{})",
            if void { ", void row" } else { "" }
        ));
    }
    if provisional || !gate_host {
        status = status.max(Status::Open);
        parts.push(format!("provisional {provisional}, gate host {gate_host}"));
    }
    let tree = match cross_fresh(root, commit) {
        Ok(t) => t,
        Err(why) => {
            status = status.max(Status::Open);
            why
        }
    };
    cell(
        status,
        format!(
            "{}: {}; measured at ep9 commit `{}`: {tree}",
            p.display(),
            parts.join("; "),
            short(commit),
        ),
    )
}
