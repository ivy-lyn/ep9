//! The cargo logs that the exit inputs hold (nesting and corpus logs): the start and exit-status
//! lines of each cargo run, cargo failure lines, and the corpus digest lines. Moved from
//! m0_exit.rs (M4-P0, no logic change).

/// The start line that CORPUS_CMD and NESTING_CMD write before each cargo run (M3-P3 fix
/// round 2, C-G2): `m0-exit start: <label>`. With [`SENTINEL`] after the run, it makes each
/// run of a log one pair, so a run that was killed (a start, no status) cannot hide between
/// two good runs.
pub(crate) const START: &str = "m0-exit start: ";

/// The exit-status line that CORPUS_CMD and NESTING_CMD write after each cargo run
/// (M2-P2-OPEN-I9-EXIT): `m0-exit status: <code>`. A run that is killed writes no status, or a
/// code that is not 0, so its log cannot pass.
pub(crate) const SENTINEL: &str = "m0-exit status: ";

/// One cargo run of a log: the lines from its start line ([`START`]) to its exit-status line
/// ([`SENTINEL`]).
pub(crate) struct Invocation<'a> {
    pub(crate) label: &'a str,
    /// The lines between the start line and the status line, trimmed.
    pub(crate) body: Vec<&'a str>,
    /// The code of the status line: None if the run has none (killed, or the log stops), Err
    /// if the line holds no number.
    pub(crate) exit: Option<Result<i64, String>>,
}

impl Invocation<'_> {
    pub(crate) fn ok(&self) -> bool {
        self.exit == Some(Ok(0))
    }
}

/// The cargo runs of a log, and the lines that break the pairing: a status line with no start
/// line before it, and a non-blank line outside every run (output that no run owns). A run
/// with no status line ends at the next start line or at the end of the log.
pub(crate) fn invocations(log: &str) -> (Vec<Invocation<'_>>, Vec<String>) {
    let mut runs: Vec<Invocation> = Vec::new();
    let mut problems = Vec::new();
    let mut open = false;
    for t in log.lines().map(str::trim) {
        if let Some(label) = t.strip_prefix(START) {
            runs.push(Invocation {
                label: label.trim(),
                body: Vec::new(),
                exit: None,
            });
            open = true;
        } else if let Some(code) = t.strip_prefix(SENTINEL) {
            match runs.last_mut() {
                Some(r) if open => {
                    r.exit = Some(code.parse::<i64>().map_err(|_| code.to_string()));
                    open = false;
                }
                _ => problems.push(format!(
                    "an exit-status line \"{t}\" with no start line (\"{START}<label>\") before it"
                )),
            }
        } else if open {
            if let Some(r) = runs.last_mut() {
                r.body.push(t);
            }
        } else if !t.is_empty() {
            problems.push(format!("output outside every cargo run: {t:?}"));
        }
    }
    (runs, problems)
}

/// The pairing rules of a whole log: at least `min` cargo runs, each with its start line and
/// its exit-status line, each status `0`, and no line outside a run (so nothing after the last
/// status). Err(why).
pub(crate) fn invocation_check(log: &str, min: usize) -> Result<(), String> {
    let (runs, problems) = invocations(log);
    if let Some(p) = problems.first() {
        return Err(format!(
            "{p}{}",
            if problems.len() > 1 {
                format!(" (and {} more such lines)", problems.len() - 1)
            } else {
                String::new()
            }
        ));
    }
    if let Some(r) = runs.iter().find(|r| r.exit.is_none()) {
        return Err(format!(
            "the cargo run \"{START}{}\" has no exit status (killed?)",
            r.label
        ));
    }
    if runs.len() < min {
        return Err(format!(
            "{} cargo runs with their start and exit-status lines, {min} needed: a killed run, or a log not made with the command below",
            runs.len()
        ));
    }
    if let Some(r) = runs.iter().find(|r| !r.ok()) {
        return Err(match &r.exit {
            Some(Ok(code)) => format!("the cargo run \"{START}{}\" exited {code}", r.label),
            _ => format!(
                "the cargo run \"{START}{}\" has a bad exit-status line",
                r.label
            ),
        });
    }
    Ok(())
}

/// The first cargo or rustc failure in a log (items 7 and 12): a line that starts with
/// `error[E` or `error:` (a compile error, `error: could not compile`, `error: test failed`, a
/// crashed test binary, a usage or lockfile error) or a `thread '<name>' (<id>) panicked at`
/// line (older Rust prints no id). Such an
/// invocation can stop before its "Finished" line or its test result, so the log parsers would
/// count its lines in an earlier good run. A good corpus or nesting run prints none: the report
/// lines of the corpus test are indented counts and `e.g.` lines. A log that holds a failed
/// attempt and then a good run is rejected too (CORPUS_CMD writes one invocation per file).
pub(crate) fn cargo_failure(log: &str) -> Option<&str> {
    log.lines().map(str::trim).find(|t| {
        t.starts_with("error[E")
            || t.starts_with("error:")
            || (t.starts_with("thread '") && t.contains(" panicked at "))
    })
}

/// One cargo test run in a corpus log: one start line to its exit-status line
/// ([`invocations`]; a log can hold several runs, appended).
pub(crate) struct CorpusRun {
    pub(crate) profile: &'static str,
    /// (digest, units) of each `corpus digest <hex> over <n> units` line.
    pub(crate) digests: Vec<(String, String)>,
    /// "test result: ok. 1 passed" lines.
    pub(crate) ok: usize,
    /// A "test result: FAILED" line.
    pub(crate) failed: bool,
    /// The code of the run's exit-status line ([`SENTINEL`]), if it has one (-1: a line with
    /// no number).
    pub(crate) exit: Option<i64>,
}

/// The runs of a corpus log, one per cargo run ([`invocations`]): the digests and the test
/// result of a run are counted only in it. The profile is that of the run's cargo "Finished
/// `<profile>` profile" line. Lines outside every run belong to none ([`invocation_check`]
/// rejects the log).
pub(crate) fn corpus_runs(log: &str) -> Vec<CorpusRun> {
    let (invs, _) = invocations(log);
    invs.iter()
        .map(|inv| {
            let mut r = CorpusRun {
                profile: "unknown profile",
                digests: Vec::new(),
                ok: 0,
                failed: false,
                exit: inv.exit.as_ref().map(|e| e.as_ref().map_or(-1, |c| *c)),
            };
            for t in &inv.body {
                if let Some(rest) = t.strip_prefix("Finished `") {
                    r.profile = match rest.split('`').next() {
                        Some("release") => "release",
                        Some("test" | "dev") => "debug",
                        _ => "unknown profile",
                    };
                } else if let Some((d, units)) = t
                    .strip_prefix("corpus digest ")
                    .and_then(|rest| rest.split_once(" over "))
                {
                    r.digests.push((
                        d.to_string(),
                        units.split(' ').next().unwrap_or("").to_string(),
                    ));
                } else if t.starts_with("test result: FAILED") {
                    r.failed = true;
                } else if t.starts_with("test result: ok. 1 passed") {
                    r.ok += 1;
                }
            }
            r
        })
        .collect()
}
