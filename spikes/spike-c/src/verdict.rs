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
//! | C4 `<kernel>` | every run right and the R interval at or above 3.0 | a wrong run or the interval below 3.0: no-go; marginal, void, too few rounds: that word (OPEN) |
//! | C5a | the unit tests and the structural step PASS | as C1 |
//! | C5b | the unit tests, the structural and the deopt steps PASS | as C1 |
//! | C5c | every spfacts step PASS, every run right, the Q interval at or above 0.90 | a FAIL or a wrong run: no-go; a speed-only miss: "speed-only miss" (H30 after the time box, OPEN) |
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
    if r.iqr_percent > 5.0 {
        return "void (IQR > 5% of the median; run again)".into();
    }
    match r.interval {
        None => "too few rounds".into(),
        Some((lo, _)) if lo >= bar => "go".into(),
        Some((_, hi)) if hi < bar => speed_only_miss.into(),
        Some(_) => "marginal (2x rounds; the median decides)".into(),
    }
}

/// The verdict of the steps whose ids start with one of `prefixes`: go if each of them ran
/// and passed (at least one), no-go on a FAIL, open on a SKIP or none.
pub fn steps_verdict(steps: &[(String, String)], prefixes: &[&str]) -> String {
    let mine: Vec<&(String, String)> = steps
        .iter()
        .filter(|(id, _)| {
            prefixes
                .iter()
                .any(|p| id == p || id.starts_with(&format!("{p}/")))
        })
        .collect();
    if mine.iter().any(|(_, s)| s == "FAIL") {
        let failed: Vec<&str> = mine
            .iter()
            .filter(|(_, s)| s == "FAIL")
            .map(|(id, _)| id.as_str())
            .collect();
        return format!("no-go (FAIL: {})", failed.join(" "));
    }
    let missing: Vec<&str> = prefixes
        .iter()
        .copied()
        .filter(|p| {
            !mine
                .iter()
                .any(|(id, _)| id == p || id.starts_with(&format!("{p}/")))
        })
        .collect();
    if !missing.is_empty() || mine.iter().any(|(_, s)| s != "PASS") {
        return format!("open (not run: {})", missing.join(" "));
    }
    "go".into()
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
    let c4: Vec<(&str, String)> = C4_KERNELS
        .iter()
        .map(|k| Ok((*k, read(dir, &format!("c4-{k}.txt"))?)))
        .collect::<Result<_, String>>()?;
    let spike_c = |id: &str, what: &str, prefixes: &[&str]| {
        row(
            "Spike C",
            format!("{id}: {what}"),
            steps_verdict(&steps, prefixes),
            "jit",
            SPIKE_C_ADRS,
        )
    };
    let in_t0: Vec<String> = c4
        .iter()
        .filter_map(|(k, text)| {
            let names = refused_names(text);
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
    out.push(spike_c("C1", &c1, &["am3", "linsolve", "navier"]));
    out.push(spike_c(
        "C2",
        "each guard of am3 forced alone deopts to T0, output equal, one counted exit",
        &["deopt"],
    ));
    out.push(spike_c(
        "C3",
        "the tier differential, kernels and corpus, every guard forced, 0 mismatches",
        &["diff"],
    ));
    for (k, text) in &c4 {
        let r = parse_ratio(text, "R = t(T0) / t(C) (gate unit):")
            .map_err(|e| format!("c4-{k}.txt: {e}"))?;
        let iv = r
            .interval
            .map_or("-".into(), |(lo, hi)| format!("[{lo:.3}, {hi:.3}]"));
        out.push(row(
            "Spike C",
            format!(
                "C4 {k}: R {:.3} {iv}, {} rounds, IQR {:.1}%{}",
                r.median,
                r.rounds,
                r.iqr_percent,
                stays(&refused_names(text))
            ),
            ratio_verdict(&r, 3.0, "no-go (the interval is below 3.0)"),
            "jit",
            SPIKE_C_ADRS,
        ));
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
        steps_verdict(&steps, &["unit-tests", "structural"]),
    ));
    out.push(sp(
        "C5b",
        "the metadata holds offsets only; a moved copy deopts the same".into(),
        steps_verdict(&steps, &["unit-tests", "structural", "deopt"]),
    ));
    let text = read(dir, "c5c.txt")?;
    let q = parse_ratio(&text, "Q = t(C) / t(P) (C5c):").map_err(|e| format!("c5c.txt: {e}"))?;
    let speed = ratio_verdict(&q, 0.90, "speed-only miss (an H30 row after the time box)");
    let steps_c5c = steps_verdict(&steps, &["spfacts"]);
    let verdict = if steps_c5c == "go" { speed } else { steps_c5c };
    let iv = q
        .interval
        .map_or("-".into(), |(lo, hi)| format!("[{lo:.3}, {hi:.3}]"));
    out.push(sp(
        "C5c",
        format!(
            "crypto from a PROFILED file, output equal; Q = t(C) / t(P) {:.3} {iv}, {} rounds, IQR {:.1}%{}",
            q.median,
            q.rounds,
            q.iqr_percent,
            stays(&refused_names(&text))
        ),
        verdict,
    ));
    Ok(out.join("\n"))
}

/// The fixture run of `gate.sh verdict --fixtures` (VERDICT-DRY on fixtures): the inputs of a
/// gate run, written to `dir`: every step PASS, a clean tree, C4 crypto on the pass side, C4
/// navier-stokes marginal, C5c on the pass side.
pub fn write_fixtures(dir: &Path) -> Result<(), String> {
    let files = [
        (
            "meta.txt",
            "commit=0123456789abcdef0123456789abcdef01234567\nclean=yes\nhost=aarch64-apple-darwin, Apple M5 Max (fixture)\nload_before=1.10 1.20 1.30\nload_after=1.40 1.50 1.60\n",
        ),
        (
            "steps.txt",
            "unit-tests               PASS\nbuild-release            PASS\nstructural/release       PASS\nam3/compile              PASS\nam3/tier-n1              PASS\nam3/tier-n8              PASS\nam3/stress               PASS\ndeopt/each-guard         PASS\ndeopt/seeded-stress      PASS\nlinsolve/graph           PASS\nlinsolve/tier-n1         PASS\nlinsolve/tier-n12        PASS\nnavier/graph-lin_solve   PASS\nnavier/graph-project     PASS\nnavier/graph-advect      PASS\nnavier/set-n1            PASS\nnavier/set-n12           PASS\ndiff/kernels+corpus      PASS\nspfacts/write            PASS\nspfacts/p-n1             PASS\nspfacts/p-n8             PASS\nspfacts/p-ics-only       PASS\nspfacts/wrong            PASS\n",
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
}
