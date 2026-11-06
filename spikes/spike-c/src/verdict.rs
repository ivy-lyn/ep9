//! The Verdict row writer (M6-P2 SPIKEC-10; plan v11 §4.8 task 4, SCP §8; VERDICT-DRY):
//! `spike-c verdict-rows <dir>` reads what `gate.sh verdict` wrote into `<dir>` and prints
//! the SCP §8 table, one row per condition of SCP §0 (C4 one per gate kernel). It decides
//! nothing a person must decide: a marginal or void C4, a speed-only C5c miss and every
//! condition whose steps did not all run print a word m1-exit reads as OPEN.
//!
//! ```text
//!  <dir>/meta.txt              commit=<40 hex>  clean=yes|no  host=<host, CPU model>
//!                              load_before=<n n n>  load_after=<n n n>
//!  <dir>/steps.txt             the gate's result lines: `<step id> PASS|FAIL|SKIP [note]`
//!  <dir>/c4-crypto.txt         `spike-c time crypto` (protocol §3)
//!  <dir>/c4-navier-stokes.txt  `spike-c time navier-stokes`
//!  <dir>/c5c.txt               `spike-c sp-facts time crypto` (protocol §6.3)
//! ```
//!
//! | Row | Verdict `go` when | else |
//! |---|---|---|
//! | C1 | every am3, linsolve and navier step PASS | a FAIL: no-go; a missing or SKIP step: open |
//! | C2 | every deopt step PASS | as C1 |
//! | C3 | the diff step PASS | as C1 |
//! | C4 `<kernel>` | the `verdict/c4-<kernel>` step PASS, every run right, the R interval at or above 3.0 and the report's own words `interval on the pass side` | as C1 for the step; then a wrong run or the interval below 3.0: no-go; marginal, void, too few rounds: that word (OPEN) |
//! | C5a | the unit tests and the structural step PASS | as C1 |
//! | C5b | the unit tests, the structural and the deopt steps PASS | as C1 |
//! | C5c | every spfacts step and the `verdict/c5c` step PASS, every run right, the Q interval at or above 0.90 and the report's own words `interval on the pass side` | as C1 for the steps; then a wrong run: no-go; a speed-only miss: "speed-only miss" (H30 after the time box, OPEN) |
//!
//! A row reads only its own steps ([`C1_STEPS`] .. [`C5C_STEPS`], [`c4_step`]), each by its
//! full id, so a step that is missing reads open, not go (M6-P3 SC-FIX, Codex r1: C4 and C5c
//! read the report and not the step that wrote it). When a row's steps are not go, its
//! verdict is theirs, and a report that cannot be read gives the Condition cell `no report`
//! (not an error). The timing steps FAIL only on a wrong run, never on a speed miss
//! (`spike-c time`, `spike-c sp-facts time`). The report rounds its numbers, so a row reads
//! go only when the producer's words (`timing::Summary::side`, from the unrounded numbers)
//! agree; a ratio line with no such words reads marginal (M6-P3 Codex r2 T1).
//!
//! A tree that is not clean, or a commit that is not 40 hex, gives no table (an error): a row
//! must name the tree it measured (SCP §8, Commit).
//!
//! The hot functions config C (or P) left in T0 (SCP §3 config C: a refused function stays in
//! T0 and is named; crypto's `montReduce` and `bnpSquareTo` by V11-D30) are named in the
//! Condition cell after the numbers: `; stays in T0: <names>` in C4 `<kernel>` and C5c, and
//! `; stays in T0: <kernel> <names>` in C1, from the `stays in T0:` lines of the timing
//! reports (`timing::refused_lines`; M6-P3 SC-FIX, MC-P2-SCLEFT). The label m1-exit reads,
//! the text before the first `:`, does not change.

use std::path::Path;

/// The SCP §8 columns, in order.
pub const HEADER: &str = "| Spike | Condition | Verdict | Commit | Lowering | Host/CPU | ADRs |\n|---|---|---|---|---|---|---|";

/// The gate kernels of SCP §2, in C4 row order.
pub const C4_KERNELS: [&str; 2] = ["crypto", "navier-stokes"];

const SPIKE_C_ADRS: &str = "H2, H14, H16, H17, H29";
const SP_FACTS_ADRS: &str = "H30, H31, H32, H33, H34";

/// One timing report (`spike-c time`, `spike-c sp-facts time`): the ratio line's numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct Ratio {
    pub median: f64,
    pub interval: Option<(f64, f64)>,
    pub iqr_percent: f64,
    pub rounds: u32,
    /// "every run right" was printed (no wrong run).
    pub right: bool,
    /// The producer's verdict words after `; vs <bar>: ` (`timing::Summary::side`, decided on
    /// the unrounded numbers), or "" when the line has none.
    pub side: String,
}

/// The ratio line that starts with `prefix` (`R = t(T0) / t(C) (gate unit):` or `Q = t(C) /
/// t(P) (C5c):`) and the rounds and correctness lines of the report `text`.
pub fn parse_ratio(text: &str, prefix: &str) -> Result<Ratio, String> {
    let line = text
        .lines()
        .find(|l| l.starts_with(prefix))
        .ok_or_else(|| format!("no line `{prefix}`"))?;
    let num = |s: &str| -> Result<f64, String> {
        s.trim()
            .parse::<f64>()
            .map_err(|_| format!("not a number: {s:?} in {line:?}"))
    };
    let after = |key: &str| -> Result<&str, String> {
        line.split_once(key)
            .map(|(_, r)| r)
            .ok_or_else(|| format!("no `{key}` in {line:?}"))
    };
    let median = num(after("median ")?.split(',').next().unwrap_or(""))?;
    let iv = after("interval ")?;
    let interval = if iv.starts_with('-') {
        None
    } else {
        let inner = iv
            .strip_prefix('[')
            .and_then(|r| r.split_once(']'))
            .map(|(a, _)| a)
            .ok_or_else(|| format!("bad interval in {line:?}"))?;
        let (lo, hi) = inner
            .split_once(',')
            .ok_or_else(|| format!("bad interval in {line:?}"))?;
        Some((num(lo)?, num(hi)?))
    };
    let iqr_percent = num(after("IQR ")?.split('%').next().unwrap_or(""))?;
    let side = line
        .split_once("; vs ")
        .and_then(|(_, r)| r.split_once(": "))
        .map_or(String::new(), |(_, w)| w.trim().to_string());
    let rounds = text
        .lines()
        .find_map(|l| l.strip_prefix("rounds: "))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or("no `rounds:` line")?;
    let right = text
        .lines()
        .any(|l| l.starts_with("correctness: every run right"));
    Ok(Ratio {
        median,
        interval,
        iqr_percent,
        rounds,
        right,
        side,
    })
}

/// The names of the hot functions a timing report says stay in T0 (its `stays in T0: <name>:
/// <refusal>` lines, `timing::refused_lines`), in report order.
pub fn refused_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix(crate::timing::REFUSED_PREFIX))
        .filter_map(|r| r.split(':').next())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// `; stays in T0: a, b` for `names`, or "" for none (the Condition cell's tail).
fn stays(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!("; stays in T0: {}", names.join(", "))
    }
}

/// The protocol §3 words for `r` against `bar`: `go`, `no-go (...)` or a word m1-exit reads
/// as OPEN (marginal, void, too few rounds).
pub fn ratio_verdict(r: &Ratio, bar: f64, speed_only_miss: &str) -> String {
    if !r.right {
        return "no-go (a wrong run: correctness)".into();
    }
    // The printed numbers are rounded (IQR to 0.1%, the interval to 0.001): the producer's
    // words, decided on the unrounded numbers, must agree before a row reads go.
    if r.iqr_percent > 5.0 || r.side.starts_with("void") {
        return "void (IQR > 5% of the median; run again)".into();
    }
    match r.interval {
        None => "too few rounds".into(),
        Some((_, hi)) if hi < bar || r.side == "interval on the fail side" => {
            speed_only_miss.into()
        }
        Some((lo, _)) if lo >= bar && r.side == "interval on the pass side" => "go".into(),
        Some(_) => "marginal (2x rounds; the median decides)".into(),
    }
}

/// The steps of one gate group a row needs: the group (a `gate.sh` function's id) and its
/// steps, recorded as `<group>/<step>`; a group with no steps is one step, recorded as
/// `<group>`. A group the gate could not run is one line `<group> SKIP|FAIL <note>` (no
/// checkout, a failed release build).
pub type Need<'a> = (&'a str, &'a [&'a str]);

const UNIT_TESTS: Need = ("unit-tests", &[]);
const STRUCTURAL: Need = ("structural", &["release"]);
const DEOPT: Need = ("deopt", &["each-guard", "seeded-stress"]);

/// C1: `gate.sh am3`, `linsolve` and `navier`.
pub const C1_STEPS: &[Need] = &[
    ("am3", &["compile", "tier-n1", "tier-n8", "stress"]),
    ("linsolve", &["graph", "tier-n1", "tier-n12"]),
    (
        "navier",
        &[
            "graph-lin_solve",
            "graph-project",
            "graph-advect",
            "set-n1",
            "set-n12",
        ],
    ),
];
/// C2: `gate.sh deopt`.
pub const C2_STEPS: &[Need] = &[DEOPT];
/// C3: `gate.sh diff`.
pub const C3_STEPS: &[Need] = &[("diff", &["kernels+corpus"])];
/// C5a: the unit tests and `gate.sh structural`.
pub const C5A_STEPS: &[Need] = &[UNIT_TESTS, STRUCTURAL];
/// C5b: as C5a, and `gate.sh deopt`.
pub const C5B_STEPS: &[Need] = &[UNIT_TESTS, STRUCTURAL, DEOPT];
/// C5c: `gate.sh spfacts` and the timing step of `gate.sh verdict` that wrote `c5c.txt`.
pub const C5C_STEPS: &[Need] = &[
    ("spfacts", &["write", "p-n1", "p-n8", "p-ics-only", "wrong"]),
    ("verdict/c5c", &[]),
];

/// C4 `<kernel>`: the timing step of `gate.sh verdict` that wrote `c4-<kernel>.txt`.
pub fn c4_step(kernel: &str) -> String {
    format!("verdict/c4-{kernel}")
}

/// The verdict of the steps `need`: go if each of them ran and passed; no-go if a step of one
/// of the groups FAILed; else open, naming what did not run (a SKIP, or no line).
pub fn steps_verdict(steps: &[(String, String)], need: &[Need]) -> String {
    let in_group =
        |id: &str, g: &str| id == g || id.strip_prefix(g).is_some_and(|r| r.starts_with('/'));
    let failed: Vec<&str> = steps
        .iter()
        .filter(|(id, s)| s == "FAIL" && need.iter().any(|(g, _)| in_group(id, g)))
        .map(|(id, _)| id.as_str())
        .collect();
    if !failed.is_empty() {
        return format!("no-go (FAIL: {})", failed.join(" "));
    }
    // PASS: at least one line with this id, and every line with it PASS.
    let passed = |id: &str| {
        let mut lines = steps.iter().filter(|(i, _)| i == id).peekable();
        lines.peek().is_some() && lines.all(|(_, s)| s == "PASS")
    };
    let mut not_run: Vec<String> = Vec::new();
    for (g, subs) in need {
        if subs.is_empty() {
            if !passed(g) {
                not_run.push((*g).to_string());
            }
            continue;
        }
        let bad: Vec<String> = subs
            .iter()
            .map(|s| format!("{g}/{s}"))
            .filter(|id| !passed(id))
            .collect();
        let group_line = steps.iter().any(|(id, s)| id == g && s != "PASS");
        if group_line || (!bad.is_empty() && bad.len() == subs.len()) {
            not_run.push((*g).to_string());
        } else {
            not_run.extend(bad);
        }
    }
    if not_run.is_empty() {
        "go".into()
    } else {
        format!("open (not run: {})", not_run.join(" "))
    }
}

/// The gate's result lines (`<id> PASS|FAIL|SKIP [note]`, `gate.sh` `record`).
pub fn parse_steps(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (id, status) = (it.next()?, it.next()?);
            matches!(status, "PASS" | "FAIL" | "SKIP").then(|| (id.to_string(), status.to_string()))
        })
        .collect()
}

/// `key=value` lines.
fn meta_value<'a>(meta: &'a str, key: &str) -> Option<&'a str> {
    meta.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(str::trim)
}

fn read(dir: &Path, name: &str) -> Result<String, String> {
    std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"))
}

/// The SCP §8 table of the gate run in `dir` (module docs).
pub fn rows(dir: &Path) -> Result<String, String> {
    let meta = read(dir, "meta.txt")?;
    let commit = meta_value(&meta, "commit").ok_or("meta.txt: no commit")?;
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("meta.txt: commit {commit:?} is not 40 hex"));
    }
    if meta_value(&meta, "clean") != Some("yes") {
        return Err("meta.txt: the tree is not clean (a row must name the tree it ran on)".into());
    }
    let host = meta_value(&meta, "host").ok_or("meta.txt: no host")?;
    let load = format!(
        "load {} before, {} after",
        meta_value(&meta, "load_before").ok_or("meta.txt: no load_before")?,
        meta_value(&meta, "load_after").ok_or("meta.txt: no load_after")?
    );
    let host = format!("{host}, {load}");
    let steps = parse_steps(&read(dir, "steps.txt")?);
    let row = |spike: &str, cond: String, verdict: String, lowering: &str, adrs: &str| {
        format!("| {spike} | {cond} | {verdict} | {commit} | {lowering} | {host} | {adrs} |")
    };
    let mut out = vec![HEADER.to_string()];
    let c4: Vec<(&str, Result<String, String>)> = C4_KERNELS
        .iter()
        .map(|k| (*k, read(dir, &format!("c4-{k}.txt"))))
        .collect();
    let spike_c = |id: &str, what: &str, need: &[Need]| {
        row(
            "Spike C",
            format!("{id}: {what}"),
            steps_verdict(&steps, need),
            "jit",
            SPIKE_C_ADRS,
        )
    };
    let in_t0: Vec<String> = c4
        .iter()
        .filter_map(|(k, text)| {
            let names = refused_names(text.as_deref().ok()?);
            (!names.is_empty()).then(|| format!("{k} {}", names.join(", ")))
        })
        .collect();
    let c1 = format!(
        "the hot sets of crypto and navier-stokes compile from live facts, output equal to T0{}",
        if in_t0.is_empty() {
            String::new()
        } else {
            format!("; stays in T0: {}", in_t0.join("; "))
        }
    );
    out.push(spike_c("C1", &c1, C1_STEPS));
    out.push(spike_c(
        "C2",
        "each guard of am3 forced alone deopts to T0, output equal, one counted exit",
        C2_STEPS,
    ));
    out.push(spike_c(
        "C3",
        "the tier differential, kernels and corpus, every guard forced, 0 mismatches",
        C3_STEPS,
    ));
    for (k, text) in &c4 {
        let step = c4_step(k);
        let step = steps_verdict(&steps, &[(step.as_str(), &[])]);
        let report = text.as_ref().map_err(String::clone).and_then(|t| {
            parse_ratio(t, "R = t(T0) / t(C) (gate unit):")
                .map(|r| (t, r))
                .map_err(|e| format!("c4-{k}.txt: {e}"))
        });
        let (cond, verdict) = match report {
            Ok((text, r)) => {
                let iv = r
                    .interval
                    .map_or("-".into(), |(lo, hi)| format!("[{lo:.3}, {hi:.3}]"));
                let cond = format!(
                    "C4 {k}: R {:.3} {iv}, {} rounds, IQR {:.1}%{}",
                    r.median,
                    r.rounds,
                    r.iqr_percent,
                    stays(&refused_names(text))
                );
                let verdict = if step == "go" {
                    ratio_verdict(&r, 3.0, "no-go (the interval is below 3.0)")
                } else {
                    step
                };
                (cond, verdict)
            }
            // The step did not pass: its verdict stands, with no numbers.
            Err(_) if step != "go" => (format!("C4 {k}: no report"), step),
            Err(e) => return Err(e),
        };
        out.push(row("Spike C", cond, verdict, "jit", SPIKE_C_ADRS));
    }
    let sp = |id: &str, what: String, verdict: String| {
        row(
            "SP-Facts",
            format!("{id}: {what}"),
            verdict,
            "jit, PROFILED input",
            SP_FACTS_ADRS,
        )
    };
    out.push(sp(
        "C5a",
        "the lattice and ref-node checks on every compiled graph".into(),
        steps_verdict(&steps, C5A_STEPS),
    ));
    out.push(sp(
        "C5b",
        "the metadata holds offsets only; a moved copy deopts the same".into(),
        steps_verdict(&steps, C5B_STEPS),
    ));
    let steps_c5c = steps_verdict(&steps, C5C_STEPS);
    let report = read(dir, "c5c.txt").and_then(|t| {
        parse_ratio(&t, "Q = t(C) / t(P) (C5c):")
            .map(|q| (t, q))
            .map_err(|e| format!("c5c.txt: {e}"))
    });
    let (cond, verdict) = match report {
        Ok((text, q)) => {
            let iv = q
                .interval
                .map_or("-".into(), |(lo, hi)| format!("[{lo:.3}, {hi:.3}]"));
            let cond = format!(
                "crypto from a PROFILED file, output equal; Q = t(C) / t(P) {:.3} {iv}, {} rounds, IQR {:.1}%{}",
                q.median,
                q.rounds,
                q.iqr_percent,
                stays(&refused_names(&text))
            );
            let verdict = if steps_c5c == "go" {
                ratio_verdict(&q, 0.90, "speed-only miss (an H30 row after the time box)")
            } else {
                steps_c5c
            };
            (cond, verdict)
        }
        // The steps did not pass: their verdict stands, with no numbers.
        Err(_) if steps_c5c != "go" => ("no report".to_string(), steps_c5c),
        Err(e) => return Err(e),
    };
    out.push(sp("C5c", cond, verdict));
    Ok(out.join("\n"))
}

/// The fixture run of `gate.sh verdict --fixtures` (VERDICT-DRY on fixtures): the inputs of a
/// gate run, written to `dir`: every step PASS (the timing steps too), a clean tree, C4 crypto
/// on the pass side, C4 navier-stokes marginal, C5c on the pass side.
pub fn write_fixtures(dir: &Path) -> Result<(), String> {
    let files = [
        (
            "meta.txt",
            "commit=0123456789abcdef0123456789abcdef01234567\nclean=yes\nhost=aarch64-apple-darwin, Apple M5 Max (fixture)\nload_before=1.10 1.20 1.30\nload_after=1.40 1.50 1.60\n",
        ),
        (
            "steps.txt",
            "unit-tests               PASS\nbuild-release            PASS\nstructural/release       PASS\nam3/compile              PASS\nam3/tier-n1              PASS\nam3/tier-n8              PASS\nam3/stress               PASS\ndeopt/each-guard         PASS\ndeopt/seeded-stress      PASS\nlinsolve/graph           PASS\nlinsolve/tier-n1         PASS\nlinsolve/tier-n12        PASS\nnavier/graph-lin_solve   PASS\nnavier/graph-project     PASS\nnavier/graph-advect      PASS\nnavier/set-n1            PASS\nnavier/set-n12           PASS\ndiff/kernels+corpus      PASS\nspfacts/write            PASS\nspfacts/p-n1             PASS\nspfacts/p-n8             PASS\nspfacts/p-ics-only       PASS\nspfacts/wrong            PASS\nverdict/c4-crypto        PASS\nverdict/c4-navier-stokes PASS\nverdict/c5c              PASS\n",
        ),
        (
            "c4-crypto.txt",
            "Spike C class S timing, crypto (report only, never a reading: protocol §3)\nhost: aarch64 on Apple M5 Max, load { 1.10 1.20 1.30 }\nrounds: 15 + 1 warm-up (dropped); n = 8 run() per benchmark; hot calls k = 10\nR = t(T0) / t(C) (gate unit): median 3.930, interval [3.812, 4.055], IQR 4.0% of the median; vs 3: interval on the pass side\ncompiled hot set of C: 2776 bytes, guards 16 fact + 13 unbacked\nstays in T0: montReduce: opcode SetNamedProperty at offset 43 has no native code\nstays in T0: bnpSquareTo: opcode CallProperty0 at offset 10 has no native code\ncorrectness: every run right (T0 a right driver run, C = T0, H1 = H0)\n",
        ),
        (
            "c4-navier-stokes.txt",
            "Spike C class S timing, navier-stokes (report only, never a reading: protocol §3)\nhost: aarch64 on Apple M5 Max, load { 1.10 1.20 1.30 }\nrounds: 15 + 1 warm-up (dropped); n = 12 run() per benchmark; hot calls k = 10\nR = t(T0) / t(C) (gate unit): median 3.010, interval [2.950, 3.120], IQR 2.0% of the median; vs 3: marginal\ncorrectness: every run right (T0 a right driver run, C = T0, H1 = H0)\n",
        ),
        (
            "c5c.txt",
            "rounds: 15 + 1 warm-up (dropped)\nQ = t(C) / t(P) (C5c): median 1.002, interval [0.980, 1.020], IQR 1.5% of the median; vs 0.9: interval on the pass side\nstays in T0: montReduce: opcode SetNamedProperty at offset 43 has no native code\nstays in T0: bnpSquareTo: opcode CallProperty0 at offset 10 has no native code\ncorrectness: every run right (T0 a right driver run, C = T0, P = T0)\n",
        ),
    ];
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, text) in files {
        std::fs::write(dir.join(name), text).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("spike-c-verdict-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        write_fixtures(&d).expect("fixtures");
        d
    }

    /// The fixture rows: the SCP §8 columns, one row per condition (C4 per gate kernel), the
    /// labels m1-exit reads (the text before the first `:`), go where the inputs pass and
    /// `marginal` for the marginal C4.
    #[test]
    fn the_fixture_rows_have_the_scp_8_form() {
        let d = fixture_dir("form");
        let t = rows(&d).expect("rows");
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], HEADER.lines().next().unwrap());
        let body = &lines[2..];
        let labels: Vec<(&str, &str)> = body
            .iter()
            .map(|l| {
                let c: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
                assert_eq!(c.len(), 7, "{l}");
                assert_eq!(c[3], "0123456789abcdef0123456789abcdef01234567");
                let lowering = if c[0] == "Spike C" {
                    "jit"
                } else {
                    "jit, PROFILED input"
                };
                assert_eq!(c[4], lowering, "{l}");
                (c[1].split(':').next().unwrap(), c[2])
            })
            .collect();
        assert_eq!(
            labels,
            [
                ("C1", "go"),
                ("C2", "go"),
                ("C3", "go"),
                ("C4 crypto", "go"),
                (
                    "C4 navier-stokes",
                    "marginal (2x rounds; the median decides)"
                ),
                ("C5a", "go"),
                ("C5b", "go"),
                ("C5c", "go"),
            ]
        );
        assert!(
            t.contains(
                "| C4 crypto: R 3.930 [3.812, 4.055], 15 rounds, IQR 4.0%; stays in T0: montReduce, bnpSquareTo |"
            ),
            "{t}"
        );
        // MC-P2-SCLEFT: the hot functions left in T0 are named (C1, C4 crypto, C5c); the
        // navier-stokes row names none (its whole hot set compiles).
        assert!(
            t.contains("output equal to T0; stays in T0: crypto montReduce, bnpSquareTo |"),
            "{t}"
        );
        assert!(
            t.contains("IQR 2.0% |")
                && t.contains("IQR 1.5%; stays in T0: montReduce, bnpSquareTo |"),
            "{t}"
        );
        // SCP §8 Host/CPU: the host, its CPU model and the load.
        assert!(
            t.contains("| aarch64-apple-darwin, Apple M5 Max (fixture), load 1.10 1.20 1.30 before, 1.40 1.50 1.60 after |"),
            "{t}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The names come from the `stays in T0:` lines the timing reports print (the same words).
    #[test]
    fn the_writer_reads_the_refused_lines_of_timing() {
        let refused = vec![
            (
                "montReduce".to_string(),
                "montReduce: opcode SetNamedProperty at offset 43 has no native code".to_string(),
            ),
            ("bnpSquareTo".to_string(), "limit: a: b".to_string()),
        ];
        let text = format!(
            "rounds: 15\n{}correctness: x\n",
            crate::timing::refused_lines(&refused)
        );
        assert_eq!(refused_names(&text), ["montReduce", "bnpSquareTo"]);
        assert!(refused_names("rounds: 15\n").is_empty());
    }

    /// A dirty tree or a short commit gives no table; a FAIL step is no-go and a missing one
    /// open; a wrong timing run is no-go; a void C4 says so; a C5c speed-only miss is not go.
    #[test]
    fn the_writer_refuses_and_reads_each_case() {
        let d = fixture_dir("cases");
        let meta = std::fs::read_to_string(d.join("meta.txt")).unwrap();
        std::fs::write(d.join("meta.txt"), meta.replace("clean=yes", "clean=no")).unwrap();
        assert!(rows(&d).unwrap_err().contains("not clean"));
        std::fs::write(d.join("meta.txt"), meta.replace("01234567\n", "0123456\n")).unwrap();
        assert!(rows(&d).unwrap_err().contains("not 40 hex"));
        std::fs::write(d.join("meta.txt"), &meta).unwrap();
        let steps = std::fs::read_to_string(d.join("steps.txt")).unwrap();
        let s = steps
            .replace(
                "deopt/each-guard         PASS",
                "deopt/each-guard         FAIL exit 1",
            )
            .replace("diff/kernels+corpus      PASS\n", "");
        std::fs::write(d.join("steps.txt"), s).unwrap();
        let t = rows(&d).expect("rows");
        assert!(t.contains("| no-go (FAIL: deopt/each-guard) |"), "{t}");
        assert!(t.contains("| open (not run: diff) |"), "{t}");
        std::fs::write(d.join("steps.txt"), &steps).unwrap();
        let c4 = std::fs::read_to_string(d.join("c4-crypto.txt")).unwrap();
        std::fs::write(
            d.join("c4-crypto.txt"),
            c4.replace(
                "correctness: every run right",
                "correctness FAIL: 1 wrong runs",
            ),
        )
        .unwrap();
        assert!(
            rows(&d)
                .unwrap()
                .contains("| no-go (a wrong run: correctness) |")
        );
        std::fs::write(d.join("c4-crypto.txt"), c4.replace("IQR 4.0%", "IQR 6.0%")).unwrap();
        assert!(
            rows(&d)
                .unwrap()
                .contains("| void (IQR > 5% of the median; run again) |")
        );
        std::fs::write(
            d.join("c4-crypto.txt"),
            c4.replace("[3.812, 4.055]", "[2.812, 2.955]"),
        )
        .unwrap();
        assert!(
            rows(&d)
                .unwrap()
                .contains("| no-go (the interval is below 3.0) |")
        );
        std::fs::write(d.join("c4-crypto.txt"), &c4).unwrap();
        let c5c = std::fs::read_to_string(d.join("c5c.txt")).unwrap();
        std::fs::write(
            d.join("c5c.txt"),
            c5c.replace("[0.980, 1.020]", "[0.700, 0.850]"),
        )
        .unwrap();
        assert!(
            rows(&d)
                .unwrap()
                .contains("| speed-only miss (an H30 row after the time box) |")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The Verdict cell of the row labelled `label` (the Condition text before its first `:`).
    fn verdict_of(t: &str, label: &str) -> String {
        t.lines()
            .find_map(|l| {
                let c: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
                (c.len() == 7 && c[1].split(':').next() == Some(label)).then(|| c[2].to_string())
            })
            .unwrap_or_else(|| panic!("no row {label} in {t}"))
    }

    /// The fixture's steps.txt with the line of step `id` replaced by `line` ("" drops it).
    fn with_step(steps: &str, id: &str, line: &str) -> String {
        let mut found = false;
        let out: String = steps
            .lines()
            .filter_map(|l| {
                if l.split_whitespace().next() == Some(id) {
                    found = true;
                    (!line.is_empty()).then(|| format!("{line}\n"))
                } else {
                    Some(format!("{l}\n"))
                }
            })
            .collect();
        assert!(found, "no step {id}");
        out
    }

    /// Codex r1 on M6-P3 SC-FIX: a C4 or C5c row needs its own timing step to pass, not only
    /// a passing report. For each timing step: FAIL is no-go, SKIP or missing is open, and
    /// only that step's row changes (C4 navier-stokes stays marginal when its step passes).
    #[test]
    fn a_timing_row_needs_its_own_step() {
        let d = fixture_dir("timing-steps");
        let steps = std::fs::read_to_string(d.join("steps.txt")).unwrap();
        let good = rows(&d).expect("rows");
        let cases = [
            ("verdict/c4-crypto", "C4 crypto"),
            ("verdict/c4-navier-stokes", "C4 navier-stokes"),
            ("verdict/c5c", "C5c"),
        ];
        for (id, label) in cases {
            for (line, want) in [
                (
                    format!("{id:<24} FAIL  exit 1"),
                    format!("no-go (FAIL: {id})"),
                ),
                (
                    format!("{id:<24} SKIP  no v8-v7 checkout"),
                    format!("open (not run: {id})"),
                ),
                (String::new(), format!("open (not run: {id})")),
            ] {
                std::fs::write(d.join("steps.txt"), with_step(&steps, id, &line)).unwrap();
                let t = rows(&d).expect("rows");
                assert_eq!(verdict_of(&t, label), want, "{line:?}\n{t}");
                for (_, other) in cases.iter().filter(|(_, l)| *l != label) {
                    assert_eq!(verdict_of(&t, other), verdict_of(&good, other), "{t}");
                }
                for other in ["C1", "C2", "C3", "C5a", "C5b"] {
                    assert_eq!(verdict_of(&t, other), "go", "{t}");
                }
            }
        }
        // A step that failed may leave no report: the row keeps its verdict, with no numbers.
        // A passing step with no report still gives no table.
        std::fs::write(
            d.join("steps.txt"),
            with_step(&steps, "verdict/c4-crypto", "verdict/c4-crypto FAIL exit 2"),
        )
        .unwrap();
        std::fs::write(d.join("c4-crypto.txt"), "").unwrap();
        let t = rows(&d).expect("rows");
        assert!(
            t.contains("| C4 crypto: no report | no-go (FAIL: verdict/c4-crypto) |"),
            "{t}"
        );
        // C1 names no T0 function of crypto then (its report is gone).
        assert!(t.contains("output equal to T0 | go |"), "{t}");
        std::fs::write(d.join("steps.txt"), &steps).unwrap();
        assert!(rows(&d).unwrap_err().contains("c4-crypto.txt: no line"));
        std::fs::remove_file(d.join("c4-crypto.txt")).unwrap();
        assert!(rows(&d).unwrap_err().starts_with("c4-crypto.txt: "));
        std::fs::write(
            d.join("steps.txt"),
            with_step(&steps, "verdict/c4-crypto", "verdict/c4-crypto SKIP"),
        )
        .unwrap();
        let t = rows(&d).expect("rows");
        assert!(
            t.contains("| C4 crypto: no report | open (not run: verdict/c4-crypto) |"),
            "{t}"
        );
        write_fixtures(&d).expect("fixtures");
        std::fs::write(d.join("steps.txt"), with_step(&steps, "verdict/c5c", "")).unwrap();
        std::fs::remove_file(d.join("c5c.txt")).unwrap();
        let t = rows(&d).expect("rows");
        assert!(
            t.contains("| C5c: no report | open (not run: verdict/c5c) |"),
            "{t}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every row needs each of its steps by its full id: one step missing or SKIP is open (it
    /// names that step), a group the gate skipped or failed as one line names the group.
    #[test]
    fn a_row_needs_each_of_its_steps() {
        let d = fixture_dir("each-step");
        let steps = std::fs::read_to_string(d.join("steps.txt")).unwrap();
        let am3: String = ["am3/compile", "am3/tier-n1", "am3/tier-n8"]
            .iter()
            .fold(steps.clone(), |s, id| with_step(&s, id, ""));
        for (s, label, want) in [
            (
                with_step(&steps, "am3/stress", ""),
                "C1",
                "open (not run: am3/stress)",
            ),
            (
                with_step(&steps, "navier/set-n12", "navier/set-n12 SKIP"),
                "C1",
                "open (not run: navier/set-n12)",
            ),
            (
                with_step(&am3, "am3/stress", "am3 SKIP  no v8-v7 checkout"),
                "C1",
                "open (not run: am3)",
            ),
            (
                with_step(&am3, "am3/stress", "am3 FAIL  release build failed"),
                "C1",
                "no-go (FAIL: am3)",
            ),
            (
                with_step(&steps, "deopt/seeded-stress", ""),
                "C2",
                "open (not run: deopt/seeded-stress)",
            ),
            (
                with_step(&steps, "deopt/seeded-stress", ""),
                "C5b",
                "open (not run: deopt/seeded-stress)",
            ),
            (
                with_step(&steps, "unit-tests", ""),
                "C5a",
                "open (not run: unit-tests)",
            ),
            (
                with_step(&steps, "structural/release", "structural/release FAIL"),
                "C5a",
                "no-go (FAIL: structural/release)",
            ),
            (
                with_step(&steps, "spfacts/p-ics-only", ""),
                "C5c",
                "open (not run: spfacts/p-ics-only)",
            ),
            (
                format!("{steps}spfacts/wrong            SKIP\n"),
                "C5c",
                "open (not run: spfacts/wrong)",
            ),
        ] {
            std::fs::write(d.join("steps.txt"), &s).unwrap();
            let t = rows(&d).expect("rows");
            assert_eq!(verdict_of(&t, label), want, "{t}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The step ids the rows need are the ids `gate.sh` records: every `run` step of the
    /// groups `all` runs and of `gate.sh verdict` is read by a row (the report-only steps and
    /// the fixture step are not), and every id a row needs is one the gate records.
    #[test]
    fn the_row_steps_are_the_gate_steps() {
        let gate = include_str!("../gate.sh");
        let mut ran: Vec<String> = gate
            .lines()
            .filter_map(|l| l.trim().strip_prefix("run \""))
            .filter_map(|r| r.split('"').next())
            .filter(|id| !id.ends_with(" (report)") && *id != "verdict/fixtures")
            .map(String::from)
            .collect();
        let c4 = format!("for k in {}; do", C4_KERNELS.join(" "));
        assert!(gate.contains(&c4), "gate.sh: no `{c4}`");
        let at = ran.iter().position(|id| id == "verdict/c4-$k").expect("c4");
        ran.splice(at..=at, C4_KERNELS.iter().map(|k| c4_step(k)));
        ran.sort();
        let mut needed: Vec<String> = [
            C1_STEPS, C2_STEPS, C3_STEPS, C5A_STEPS, C5B_STEPS, C5C_STEPS,
        ]
        .iter()
        .flat_map(|n| n.iter())
        .flat_map(|(g, subs)| {
            if subs.is_empty() {
                vec![(*g).to_string()]
            } else {
                subs.iter().map(|s| format!("{g}/{s}")).collect()
            }
        })
        .chain(C4_KERNELS.iter().map(|k| c4_step(k)))
        .collect();
        needed.sort();
        needed.dedup();
        assert_eq!(needed, ran);
    }

    /// The writer reads the real report lines of `timing.rs` (the same words).
    #[test]
    fn the_writer_reads_the_timing_report_lines() {
        let s = crate::timing::Summary::of(&[
            3.1, 3.2, 3.3, 3.2, 3.15, 3.25, 3.2, 3.1, 3.3, 3.2, 3.2, 3.15, 3.25, 3.2, 3.2,
        ]);
        let text = format!(
            "rounds: 15 + 1 warm-up (dropped)\nR = t(T0) / t(C) (gate unit): {}\ncorrectness: every run right\n",
            s.line(3.0)
        );
        let r = parse_ratio(&text, "R = t(T0) / t(C) (gate unit):").expect("parse");
        assert_eq!(r.rounds, 15);
        assert!((r.median - s.median).abs() < 5e-4);
        assert_eq!(ratio_verdict(&r, 3.0, "miss"), "go");
    }

    /// Codex r2 T1: the report rounds the IQR to 0.1% and the interval to 0.001, so a void or
    /// marginal run can print numbers that pass. The producer's words decide those cases.
    #[test]
    fn a_rounded_report_keeps_the_producer_verdict() {
        use crate::timing::Summary;
        let read = |s: &Summary, bar: f64| {
            let p = "R = t(T0) / t(C) (gate unit):";
            let text = format!(
                "rounds: 15 + 1 warm-up (dropped)\n{p} {}\ncorrectness: every run right\n",
                s.line(bar)
            );
            ratio_verdict(&parse_ratio(&text, p).expect("parse"), bar, "miss")
        };
        let void = Summary {
            median: 3.9,
            interval: Some((3.5, 4.2)),
            iqr_share: 0.0504,
        };
        assert!(void.line(3.0).contains("IQR 5.0%"));
        assert!(read(&void, 3.0).starts_with("void"));
        let edge = Summary {
            median: 3.2,
            interval: Some((2.9996, 3.4)),
            iqr_share: 0.02,
        };
        assert!(edge.line(3.0).contains("[3.000, 3.400]"));
        assert!(read(&edge, 3.0).starts_with("marginal"));
        let low = Summary {
            median: 0.8,
            interval: Some((0.7, 0.8996)),
            iqr_share: 0.02,
        };
        assert!(low.line(0.9).contains("0.900]"));
        assert_eq!(read(&low, 0.9), "miss");
    }
}
