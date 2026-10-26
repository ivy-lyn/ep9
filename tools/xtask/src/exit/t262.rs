//! test262 runner JSONs: the committed expectation files, counts, and the expectation diff
//! recomputed from the results. Moved from m0_exit.rs (M4-P0, no logic change).

use super::json::Json;
use super::read;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The committed expectation file `expectations/<name>`: Ok(path) if it exists and git tracks it.
pub(crate) fn committed(root: &Path, name: &str) -> Result<PathBuf, String> {
    let p = root.join("expectations").join(name);
    if !p.is_file() {
        return Err(format!("expectations/{name} does not exist"));
    }
    match Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(&p)
        .output()
    {
        Ok(out) if out.status.success() => Ok(p),
        Ok(out) if String::from_utf8_lossy(&out.stderr).contains("not a git repository") => Ok(p),
        Ok(_) => Err(format!("expectations/{name} is not committed")),
        Err(_) => Ok(p), // no git: the file exists
    }
}

/// The paths of an expectation file (comments and blank lines dropped).
pub(crate) fn expectation_set(p: &Path) -> Result<BTreeSet<String>, String> {
    Ok(read(p)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

pub(crate) fn count(j: &Json, path: &[&str]) -> Option<(u64, u64)> {
    let c = j.path(path)?;
    Some((
        c.get("passed")?.num()? as u64,
        c.get("total")?.num()? as u64,
    ))
}

pub(crate) fn pct(p: u64, t: u64) -> f64 {
    if t == 0 {
        0.0
    } else {
        100.0 * p as f64 / t as f64
    }
}

/// Infra results and the expectation diff of an exec JSON.
pub(crate) fn exec_health(j: &Json) -> (usize, usize, usize) {
    let infra = j.get("results").and_then(Json::arr).map_or(0, |r| {
        r.iter()
            .filter(|r| r.get("status").and_then(Json::str) == Some("infra"))
            .count()
    });
    let len = |k: &str| {
        j.path(&["expectations", k])
            .and_then(Json::arr)
            .map_or(0, <[Json]>::len)
    };
    (infra, len("new_failures"), len("now_passing"))
}

/// The expectation diff of an exec JSON, recomputed here: its "fail" results against the
/// committed `expectations/<file>` without `skip` (the tests the run did not run). The runner's
/// own diff is not enough: it stays empty when the runner cannot read its expectation file, and
/// `--expect` can name any file. So the run must name the committed file of this tree, or the
/// same name in a checkout that is not on this host (a CI job's JSON, M3-P3-DRYRUN-CI): such a
/// path does not exist here and ends in `/expectations/<file>` (`\` read as `/`).
#[derive(Debug)]
pub(crate) struct Recheck {
    /// Failing, and not in the file.
    pub(crate) new: Vec<String>,
    /// In the file, and not failing.
    pub(crate) stale: Vec<String>,
    /// The run names its file in another checkout (a CI job): that path, for the evidence.
    pub(crate) foreign: Option<String>,
}

pub(crate) fn recheck(
    root: &Path,
    j: &Json,
    file: &str,
    skip: &BTreeSet<String>,
) -> Result<Recheck, String> {
    let want = committed(root, file)?;
    let used = j
        .path(&["expectations", "file"])
        .and_then(Json::str)
        .unwrap_or("");
    let same = match (std::fs::canonicalize(used), std::fs::canonicalize(&want)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    let foreign = !same
        && !used.is_empty()
        && !Path::new(used).exists()
        && used
            .replace('\\', "/")
            .ends_with(&format!("/expectations/{file}"));
    if !same && !foreign {
        return Err(format!(
            "the run compared with {used:?}, not expectations/{file} of this tree ({}){}",
            want.display(),
            if Path::new(used).is_file() {
                ""
            } else {
                ": that file does not exist, so the runner could not read it and compared nothing"
            }
        ));
    }
    let expected: BTreeSet<String> = expectation_set(&want)?.difference(skip).cloned().collect();
    let results = j
        .get("results")
        .and_then(Json::arr)
        .ok_or("the run has no \"results\" array")?;
    let failing: BTreeSet<String> = results
        .iter()
        .filter(|r| r.get("status").and_then(Json::str) == Some("fail"))
        .filter_map(|r| r.get("path").and_then(Json::str).map(str::to_string))
        .collect();
    Ok(Recheck {
        new: failing.difference(&expected).cloned().collect(),
        stale: expected.difference(&failing).cloned().collect(),
        foreign: foreign.then(|| used.to_string()),
    })
}

impl Recheck {
    /// "recomputed against `expectations/<file>`", plus the other checkout's path if any.
    pub(crate) fn basis(&self, file: &str) -> String {
        match &self.foreign {
            None => format!("recomputed against expectations/{file}"),
            Some(p) => format!(
                "recomputed against expectations/{file} of this tree; the run named {p:?}, the same file in another checkout (a CI job)"
            ),
        }
    }
}
