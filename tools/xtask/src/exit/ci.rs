//! A CI run: `gh run view <id> --json jobs,headSha,databaseId,url`, its jobs and steps, and the
//! Windows CI names. Moved from m0_exit.rs (M4-P0, no logic change).

use super::json::{Json, parse_json};
use super::report::Status;
use std::process::Command;

/// The Windows triple; its column comes from CI.
pub(crate) const CI_TRIPLE: &str = "x86_64-pc-windows-msvc";
/// The CI matrix name of [`CI_TRIPLE`] (`.github/workflows/ci.yml`).
pub(crate) const CI_MATRIX: &str = "windows-x64";

// ---------------------------------------------------------------------------------------------
// CI run: `gh run view <id> --json jobs,headSha,databaseId,url`.

#[derive(Debug)]
pub(crate) struct Ci {
    pub(crate) id: Option<String>,
    pub(crate) head: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) jobs: Vec<Job>,
}

#[derive(Debug)]
pub(crate) struct Job {
    pub(crate) name: String,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug)]
pub(crate) struct Step {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) conclusion: String,
    pub(crate) started: Option<i64>,
    pub(crate) completed: Option<i64>,
}

impl Step {
    pub(crate) fn status(&self) -> Status {
        match (self.status.as_str(), self.conclusion.as_str()) {
            ("completed", "success") => Status::Pass,
            ("completed", "failure" | "timed_out" | "startup_failure") => Status::Fail,
            // skipped, cancelled, neutral; or queued, in_progress
            _ => Status::NotRun,
        }
    }

    pub(crate) fn seconds(&self) -> Option<i64> {
        Some(self.completed? - self.started?)
    }

    pub(crate) fn describe(&self) -> String {
        if self.status == "completed" {
            self.conclusion.clone()
        } else {
            format!("{} (not finished)", self.status)
        }
    }
}

/// A JSON string or number as text.
pub(crate) fn text(v: Option<&Json>) -> Option<String> {
    match v? {
        Json::Str(s) => Some(s.clone()),
        Json::Num(n) => Some(format!("{n:.0}")),
        _ => None,
    }
}

impl Ci {
    pub(crate) fn from_json(j: &Json) -> Result<Ci, String> {
        let jobs = j
            .get("jobs")
            .and_then(Json::arr)
            .ok_or("the CI run JSON has no \"jobs\" array (gh run view <id> --json jobs,...)")?;
        let step = |s: &Json| Step {
            name: text(s.get("name")).unwrap_or_default(),
            status: text(s.get("status")).unwrap_or_default(),
            conclusion: text(s.get("conclusion")).unwrap_or_default(),
            started: text(s.get("startedAt")).and_then(|t| iso_seconds(&t)),
            completed: text(s.get("completedAt")).and_then(|t| iso_seconds(&t)),
        };
        Ok(Ci {
            id: text(j.get("databaseId")),
            head: text(j.get("headSha")),
            url: text(j.get("url")),
            jobs: jobs
                .iter()
                .map(|job| Job {
                    name: text(job.get("name")).unwrap_or_default(),
                    steps: job
                        .get("steps")
                        .and_then(Json::arr)
                        .unwrap_or(&[])
                        .iter()
                        .map(step)
                        .collect(),
                })
                .collect(),
        })
    }

    /// The step named one of `names` in the job `<job> (windows-x64)`.
    pub(crate) fn step(&self, job: &str, names: &[&str]) -> Result<&Step, String> {
        let full = format!("{job} ({CI_MATRIX})");
        let j = self
            .jobs
            .iter()
            .find(|j| j.name == full)
            .ok_or_else(|| format!("the run has no job \"{full}\""))?;
        j.steps
            .iter()
            .find(|s| names.contains(&s.name.as_str()))
            .ok_or_else(|| format!("job \"{full}\" has no step {:?}", names[0]))
    }
}

/// `gh run view <id> --json jobs,headSha,databaseId,url`. Err (the Windows column reads
/// NOT-RUN) when gh is missing or the run does not exist.
pub(crate) fn gh_run(id: &str) -> Result<Ci, String> {
    let out = Command::new("gh")
        .args(["run", "view", id, "--json", "jobs,headSha,databaseId,url"])
        .output()
        .map_err(|e| format!("cannot run gh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh run view {id} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let j = parse_json(&String::from_utf8_lossy(&out.stdout))
        .map_err(|e| format!("gh run view {id}: {e}"))?;
    Ci::from_json(&j)
}

/// Seconds since 1970 of an RFC 3339 UTC time as GitHub writes it ("2026-10-22T01:02:03Z",
/// fractions allowed). None for anything else (GitHub writes "0001-01-01T00:00:00Z" for a step
/// that did not start; that parses, and the caller sees a nonsense duration only if both ends
/// are set).
pub(crate) fn iso_seconds(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    if !t.ends_with('Z') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    // Days from civil (H. Hinnant), proleptic Gregorian.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3_600 + mi * 60 + s)
}
