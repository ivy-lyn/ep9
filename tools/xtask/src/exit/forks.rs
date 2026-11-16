//! FORK-CHECKOUT-ATTEST: the fork line of an input. The regress and icu4x pins and patch sets
//! are tree paths, so the fresh-path checks prove which fork code a tree names; cargo builds the
//! git-ignored checkouts. The one check of a checkout, `fork_attest` of `scripts/sync-lib.sh`
//! (the `.ep9-stamp` and `pinned_tree` rule), prints a line that the producers keep:
//!
//! ```text
//! regress=pinned icu4x=pinned     one <fork>=<state> per fork of FORKS, in that order;
//!                                 state pinned, stale or absent
//! ```
//!
//! | Input | Field | Who writes it |
//! |---|---|---|
//! | ep9-test262 exec JSON | `"runner_forks"` | the runner's build.rs (`EP9_RUNNER_FORKS`), a build fact |
//! | ep9-kernels/v1 | `"forks"` | kernels.mjs at the run |
//!
//! [`check`]: every fork reads `pinned`, else OPEN (also for a missing field or a line in
//! another form). m1-exit reads the field only on a tree whose `scripts/sync-lib.sh` has the
//! check ([`required`]): an older tree has no producer of it, so its m1-exit report keeps its
//! cells (the m1-exit identity check). m2-exit always reads it.

use std::path::Path;

use super::git;

/// The forks the engine builds from git-ignored checkouts (`tools/test262/runner_build.rs`
/// `FORK_BUILT`, `tools/bench/cross/kernels.mjs` `FORKS`; a test checks both).
pub(crate) const FORKS: [&str; 2] = ["regress", "icu4x"];

/// The text whose presence in `scripts/sync-lib.sh` says that the tree has the check.
const MARK: &str = "fork_attest() {";

/// Whether the tree at `root` has the shared fork check: HEAD's `scripts/sync-lib.sh` (or the
/// file, outside a git tree) defines `fork_attest`.
pub(crate) fn required(root: &Path) -> bool {
    git(root, &["show", "HEAD:scripts/sync-lib.sh"])
        .or_else(|| std::fs::read_to_string(root.join("scripts/sync-lib.sh")).ok())
        .is_some_and(|t| t.contains(MARK))
}

/// Ok when `line` names each fork of [`FORKS`], in order, as `pinned`; else the OPEN reason.
/// `field` is the JSON field name, for the reason.
pub(crate) fn check(line: Option<&str>, field: &str) -> Result<(), String> {
    let Some(line) = line.map(str::trim).filter(|l| !l.is_empty()) else {
        return Err(format!(
            "it names no fork checkout state ({field}; make it again with this tree's tools)"
        ));
    };
    let pairs: Vec<Option<(&str, &str)>> = line.split(' ').map(|p| p.split_once('=')).collect();
    let form = pairs.len() == FORKS.len()
        && pairs.iter().zip(FORKS).all(|(p, f)| {
            p.is_some_and(|(n, s)| n == f && matches!(s, "pinned" | "stale" | "absent"))
        });
    if !form {
        return Err(format!(
            "its fork line {line:?} is in another form ({field})"
        ));
    }
    let bad: Vec<&str> = pairs
        .iter()
        .flatten()
        .filter(|(_, s)| *s != "pinned")
        .map(|(n, _)| *n)
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the {} fork checkout was not its pin plus its patches ({field}: {line})",
            bad.join(" and ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_every_fork_pinned_passes() {
        assert_eq!(check(Some("regress=pinned icu4x=pinned"), "f"), Ok(()));
        assert_eq!(
            check(Some("regress=stale icu4x=pinned"), "f"),
            Err("the regress fork checkout was not its pin plus its patches (f: regress=stale icu4x=pinned)".into())
        );
        assert_eq!(
            check(Some("regress=pinned icu4x=absent"), "f").unwrap_err(),
            "the icu4x fork checkout was not its pin plus its patches (f: regress=pinned icu4x=absent)"
        );
        assert!(
            check(Some("regress=stale icu4x=stale"), "f")
                .unwrap_err()
                .starts_with("the regress and icu4x fork checkout")
        );
        for missing in [None, Some(""), Some("  ")] {
            assert!(
                check(missing, "f")
                    .unwrap_err()
                    .contains("no fork checkout state (f;")
            );
        }
        for other in [
            "regress=pinned",
            "icu4x=pinned regress=pinned",
            "regress=pinned icu4x=pinned tsrs=pinned",
            "regress=ok icu4x=pinned",
            "regress pinned icu4x pinned",
        ] {
            assert!(
                check(Some(other), "f")
                    .unwrap_err()
                    .contains("another form"),
                "{other}"
            );
        }
    }

    /// The producers name the same forks, in the same order.
    #[test]
    fn the_producers_name_these_forks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rb = std::fs::read_to_string(root.join("tools/test262/runner_build.rs")).unwrap();
        let k = rb.find("pub const FORK_BUILT").unwrap();
        let body = &rb[k..k + rb[k..].find("];").unwrap()];
        let names: Vec<&str> = body
            .match_indices("(\"")
            .map(|(i, _)| &body[i + 2..i + 2 + body[i + 2..].find('"').unwrap()])
            .collect();
        assert_eq!(names, FORKS);
        let km = std::fs::read_to_string(root.join("tools/bench/cross/kernels.mjs")).unwrap();
        assert!(
            km.contains(r#"export const FORKS = ["regress", "icu4x"];"#),
            "kernels.mjs FORKS"
        );
        assert!(required(&root), "this tree has the shared check");
        let d = std::env::temp_dir().join(format!("ep9-forks-{}", std::process::id()));
        std::fs::create_dir_all(d.join("scripts")).unwrap();
        assert!(!required(&d));
        std::fs::write(d.join("scripts/sync-lib.sh"), "sync_dep() { :; }\n").unwrap();
        assert!(!required(&d), "a sync-lib.sh before the check");
        std::fs::write(d.join("scripts/sync-lib.sh"), "fork_attest() {\n}\n").unwrap();
        assert!(required(&d));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
