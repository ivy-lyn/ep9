//! The class S timing harness of the protocol (§3) on the Mac (SPIKEC-2): a **report**, never
//! a reading. The gate runs and the Verdict rows are the orchestrator's, on the gate host, in
//! month 6 (plan v9 rule 3; protocol §3 "Host").
//!
//! ```text
//!  spike-c time <kernel> [--rounds R] [--n N] [--k K]
//!    round 0 (warm-up, dropped), rounds 1..=R (R >= 15):
//!      configs T0, C, H0, H1 in a counterbalanced order ([`order`]: each pair's two runs
//!      swap places from round to round), one new process each:
//!        spike-c once <config> <kernel> --n N --k K  ->  "secs <t>" + its output lines
//!    every run right, else the whole report is a correctness failure (exit 1):
//!      T0: a right driver run (Driver::matches)   C: T0's output, line for line
//!      H1: H0's output, line for line
//!    per round: R = t(T0) / t(C) (the gate unit), s = t(H0) / t(H1) (context), each from
//!      the two runs of that round only (a round whose pair has a failed run gives no ratio)
//!    median, the order-statistic interval of the median (n = 15: ranks 4 and 12), IQR
//!    (void if > 5% of the median), the implied share p = (1 - 1/R) / (1 - 1/s); no verdict
//!    words and no exit claim when any run was wrong
//! ```
//!
//! | Config | One process |
//! |---|---|
//! | T0 | the kernel file, Setup and one warm-up `run()` (the driver's n = 1 loop), the same compile as C (not installed), then per benchmark: Setup (and the hook), the timed `run()` x n, then the check line, TearDown and the name line |
//! | C | as T0, with the compiled first hot function installed (placement (b)) before the timed part |
//! | H0 | the kernel file, then the hot function alone in a fixed-input loop ([`HOT`]): a warm-up call, the same compile as H1 (not installed), the timed call with `k` rounds, then a digest of what it wrote |
//! | H1 | as H0, with the function compiled from the live facts of the H0 warm-up and installed |
//!
//! The compile runs in all four configs (its global, its fact export and its allocations), so
//! the VM state at the timer start differs only by the installed code.
//!
//! The timer is the process's own clock around the timed scripts only (protocol §3: the check
//! line is made after the timer stops). A guard failure in C or H1 deopts into T0 in the
//! same frame (month 5, H29), so the run stays right; the exits counted in the timed region
//! are part of what one process reports (`exits` line).

use std::process::Command;
use std::time::Instant;

use spike_b::harness::Config;

use crate::harness::Session;
use crate::kernels::{self, Kernel};
use crate::tier::{self, IntFacts};

/// One of the four configurations of protocol §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    T0,
    C,
    H0,
    H1,
}

impl Which {
    pub const ALL: [Which; 4] = [Which::T0, Which::C, Which::H0, Which::H1];

    pub fn name(self) -> &'static str {
        match self {
            Which::T0 => "t0",
            Which::C => "c",
            Which::H0 => "h0",
            Which::H1 => "h1",
        }
    }

    pub fn parse(s: &str) -> Option<Which> {
        Which::ALL.into_iter().find(|w| w.name() == s)
    }
}

/// The class S harness of a hot function: the fixed inputs (`setup`), and `__spikeCHot(k)`,
/// which calls the function `k` times and returns a number; `digest` is an expression printed
/// after the timed call.
pub struct HotHarness {
    pub function: &'static str,
    pub setup: &'static str,
    pub digest: &'static str,
    /// The `__spikeCHot` calls of the warm-up (the facts the compile reads).
    pub warm: u32,
    /// The default `k` of the timed call (about 0.15 s of T0 time on an M-series Mac).
    pub k: u32,
    /// For an inner closure no JS value reaches with its own inputs (WI-SC5: navier-stokes'
    /// `lin_solve` reads `width`, `height`, `rowSize`, `iterations` and calls `set_bnd`): the
    /// kernel file's lines `.0..=.1`, copied into the script `hot.js` on the same line
    /// numbers, inside a function whose first lines are `prelude` (the closure's variables)
    /// and which returns the hot function. The H configs compile and install that copy (same
    /// text, its own facts, by function id), not the kernel's closure.
    pub copy: Option<((u32, u32), &'static str)>,
}

/// The hot-function harnesses (the first hot function of each kernel that compiles today).
/// am3: a 37-digit `this` (28-bit digits, as crypto's own numbers) times a 28-bit `x`, added
/// into a 74-digit `w` at a moving offset; each call reads and writes the same cells, so every
/// call does the same work, and `w` stays 28-bit digits.
/// lin_solve (M5-P2 SPIKEC-6): the copy of `set_bnd` and `lin_solve` (navier-stokes.js:82-149)
/// at the kernel's size (Setup: 128 x 128, 20 iterations), called as `project` calls it
/// (`lin_solve(0, p, div, 1, 4)`, the iterative branch) on two arrays of doubles; every call
/// does the same work (20 sweeps of every cell).
pub const HOT: [HotHarness; 2] = [
    HotHarness {
        function: "am3",
        setup: "var __hx = nbi(), __hw = nbi();\n\
                for (var i = 0; i < 37; i++) __hx.array[i] = (i * 2654435 + 12345) & 0xfffffff;\n\
                for (var i = 0; i < 74; i++) __hw.array[i] = (i * 40503 + 7) & 0xfffffff;\n\
                function __spikeCHot(k) {\n\
                \x20 var c = 0;\n\
                \x20 for (var r = 0; r < k; r++) c = (c + __hx.am(0, 0x9e3779b, __hw, r % 37, 0, 37)) | 0;\n\
                \x20 return c;\n\
                }\n",
        digest: "(function () { var h = 0; for (var i = 0; i < 74; i++) h = (h * 31 + __hw.array[i]) | 0; return 'digest ' + h; })()",
        warm: 200,
        k: 100_000,
        copy: None,
    },
    HotHarness {
        function: "lin_solve",
        setup: "var __hn = 130 * 130, __hp = [], __hd = [];\n\
                for (var i = 0; i < __hn; i++) { __hp.push(0.5 + (i % 7) * 0.125); __hd.push(((i * 37) % 101) / 64 - 0.75); }\n\
                function __spikeCHot(k) {\n\
                \x20 for (var r = 0; r < k; r++) __hl(0, __hp, __hd, 1, 4);\n\
                \x20 return __hp[131];\n\
                }\n",
        digest: "(function () { var s = 0; for (var i = 0; i < __hn; i++) s += __hp[i] * ((i % 13) + 1); return 'digest ' + s; })()",
        warm: 3,
        k: 10,
        copy: Some((
            (82, 149),
            "var __hl = (function () {\nvar width = 128, height = 128, rowSize = width + 2, iterations = 20;\n",
        )),
    },
];

/// The default `k` of the H configs for `k`'s first hot function (100,000 without a harness).
pub fn default_k(k: &Kernel) -> u32 {
    hot_harness(k).map_or(100_000, |h| h.k)
}

/// The script `hot.js` of `h`: with a copy, the prelude, blank lines up to the first copied
/// line, the kernel's lines on their own line numbers, the end of the wrapper; then `setup`.
fn hot_script(h: &HotHarness, k: &Kernel) -> Result<String, String> {
    let Some(((first, last), prelude)) = h.copy else {
        return Ok(h.setup.to_string());
    };
    let text = std::fs::read_to_string(kernels::bench_dir().join(k.file))
        .map_err(|e| format!("{}: {e}", k.file))?;
    let lines: Vec<&str> = text.lines().collect();
    let body = lines
        .get(first as usize - 1..last as usize)
        .ok_or_else(|| format!("{}: no lines {first}-{last}", k.file))?;
    let used = prelude.matches('\n').count();
    let pad = (first as usize - 1)
        .checked_sub(used)
        .ok_or("the prelude is longer than the lines before the copy")?;
    Ok(format!(
        "{prelude}{}{}\nreturn {};\n}})();\n{}",
        "\n".repeat(pad),
        body.join("\n"),
        h.function,
        h.setup
    ))
}

/// What one process measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Once {
    /// Wall time of the timed scripts, seconds.
    pub secs: f64,
    /// Every printed line, in order (the check line and the name lines included).
    pub output: Vec<String>,
    /// For C and H1: the compiled code's size and its (fact, unbacked) guards.
    pub code: Option<(usize, usize, usize)>,
    /// For C and H1: the exits counted in the timed region (all exit records together).
    pub exits: Option<u64>,
}

impl Once {
    /// The lines `spike-c once` prints, which [`Once::parse`] reads back.
    pub fn text(&self) -> String {
        let mut s = format!("secs {}\n", self.secs);
        if let Some((b, f, u)) = self.code {
            s.push_str(&format!("code {b} {f} {u}\n"));
        }
        if let Some(e) = self.exits {
            s.push_str(&format!("exits {e}\n"));
        }
        for l in &self.output {
            s.push_str(&format!("| {l}\n"));
        }
        s
    }

    pub fn parse(text: &str) -> Result<Once, String> {
        let mut secs = None;
        let mut code = None;
        let mut exits = None;
        let mut output = Vec::new();
        for l in text.lines() {
            if let Some(o) = l.strip_prefix("| ") {
                output.push(o.to_string());
            } else if l == "|" {
                output.push(String::new());
            } else if let Some(v) = l.strip_prefix("secs ") {
                secs = Some(v.parse::<f64>().map_err(|e| format!("secs {v}: {e}"))?);
            } else if let Some(v) = l.strip_prefix("code ") {
                let n: Vec<usize> = v.split(' ').filter_map(|x| x.parse().ok()).collect();
                let [b, f, u] = n[..] else {
                    return Err(format!("bad code line {l}"));
                };
                code = Some((b, f, u));
            } else if let Some(v) = l.strip_prefix("exits ") {
                exits = Some(v.parse::<u64>().map_err(|e| format!("exits {v}: {e}"))?);
            }
        }
        Ok(Once {
            secs: secs.ok_or("no secs line")?,
            output,
            code,
            exits,
        })
    }
}

/// The order of the four configs in `round`: the pairs (T0, C) and (H0, H1) each swap their
/// two runs every round, and the pairs swap places every two rounds, so over rounds 1..=15
/// each run of a pair comes first in 7 or 8 rounds (a position effect, such as the CPU state
/// left by the previous process, does not lean on one side of a ratio).
pub fn order(round: usize) -> [Which; 4] {
    use Which::*;
    match round % 4 {
        0 => [T0, C, H0, H1],
        1 => [C, T0, H1, H0],
        2 => [H0, H1, T0, C],
        _ => [H1, H0, C, T0],
    }
}

/// The hot harness of `k`'s first hot function, if it has one.
fn hot_harness(k: &Kernel) -> Option<&'static HotHarness> {
    let h = k.hot.first()?;
    HOT.iter().find(|x| x.function == h.name)
}

/// Runs `script` in `s` and adds its lines to `out`; an uncaught error is a wrong run.
fn step(s: &mut Session, script: &str, file: &str, out: &mut Vec<String>) -> Result<(), String> {
    let r = s.run(script, file);
    out.extend(r.output);
    r.result.map(|_| ()).map_err(|e| format!("{file}: {e}"))
}

/// One configuration in this process (`spike-c once`): `n` timed `run()` calls per benchmark
/// for T0 and C, `k` calls of the hot function for H0 and H1.
pub fn once(which: Which, k: &'static Kernel, n: u32, hot_k: u32) -> Result<Once, String> {
    let config = Config::default().threaded();
    let d1 = kernels::driver(k, 1)?;
    let (defs, loop1) = tier::split(&d1, k.file)?;
    let mut s = Session::new(&config)?;
    let mut out = Vec::new();
    step(&mut s, &defs, "defs.js", &mut out)?;
    let hot = k.hot.first().ok_or("no hot function")?;
    let mut code = None;
    // The compiled function in C and H1, and the exits it counted in the timed region.
    let mut compiled = None;
    let mut timed_exits = 0u64;
    let mut secs = 0.0;
    // The exits counted so far for `f` (all records), when `f` runs compiled code.
    let counted = |s: &Session, f: &crate::harness::Target| {
        s.exit_counts_of(f).map(|c| c.iter().sum::<u64>())
    };
    match which {
        Which::T0 | Which::C => {
            // The warm-up: the driver's n = 1 loop (its lines are not part of the output).
            let mut warm = Vec::new();
            step(&mut s, &loop1, "warm.js", &mut warm)?;
            // The compile runs in T0 too (not installed), so both start the timer from the
            // same VM state but the installed code.
            let (f, c, report) =
                tier::compile_hot(&mut s, hot, "defs.js", d1.offset, IntFacts::Live)?;
            if which == Which::C {
                code = Some((
                    report.emit.code_bytes,
                    report.fact_guards,
                    report.unbacked_guards,
                ));
                let meta = report.emit.meta.encode();
                // SAFETY: `c` is emit_a64's placement (b) code for `f`, from `input_of(f)`
                // in `compile_hot` with no JS run since, unchanged (as `tier::run`); `meta`
                // is its exit table.
                unsafe { s.install_targets(std::slice::from_ref(&f), vec![c], vec![Some(meta)]) }?;
                compiled = Some(f);
            }
            let names = kernels::benchmark_names(
                &std::fs::read_to_string(kernels::bench_dir().join(k.file))
                    .map_err(|e| format!("{}: {e}", k.file))?,
            );
            step(
                &mut s,
                "var __spikeCB = __ep9Benchmarks;",
                "bench.js",
                &mut out,
            )?;
            for j in 0..names.len() {
                let hook = if k.hook.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", k.hook)
                };
                step(
                    &mut s,
                    &format!("__spikeCB[{j}].Setup();{hook}"),
                    "setup.js",
                    &mut out,
                )?;
                let before = compiled.as_ref().and_then(|f| counted(&s, f));
                let t = Instant::now();
                step(
                    &mut s,
                    &format!("for (var i = 0; i < {n}; i++) __spikeCB[{j}].run();"),
                    "timed.js",
                    &mut out,
                )?;
                secs += t.elapsed().as_secs_f64();
                if let (Some(a), Some(b)) = (before, compiled.as_ref().and_then(|f| counted(&s, f)))
                {
                    timed_exits += b - a;
                }
                let check = if k.check.is_empty() {
                    String::new()
                } else {
                    format!("print({});\n", k.check)
                };
                step(
                    &mut s,
                    &format!(
                        "{check}__spikeCB[{j}].TearDown(); print(__spikeCB[{j}].name + \" {n}\");"
                    ),
                    "check.js",
                    &mut out,
                )?;
            }
        }
        Which::H0 | Which::H1 => {
            let h = hot_harness(k).ok_or_else(|| format!("{}: no hot harness", hot.name))?;
            step(&mut s, &hot_script(h, k)?, "hot.js", &mut out)?;
            step(
                &mut s,
                &format!("__spikeCHot({});", h.warm),
                "hot-warm.js",
                &mut out,
            )?;
            // As for T0 and C: the compile runs in H0 too, not installed. A copied function
            // is compiled from hot.js (its lines are the kernel's line numbers there).
            let (file, offset) = match h.copy {
                Some(_) => ("hot.js", 0),
                None => ("defs.js", d1.offset),
            };
            let (f, c, report) = tier::compile_hot(&mut s, hot, file, offset, IntFacts::Live)?;
            if which == Which::H1 {
                code = Some((
                    report.emit.code_bytes,
                    report.fact_guards,
                    report.unbacked_guards,
                ));
                let meta = report.emit.meta.encode();
                // SAFETY: as for config C above.
                unsafe { s.install_targets(std::slice::from_ref(&f), vec![c], vec![Some(meta)]) }?;
                compiled = Some(f);
            }
            let before = compiled.as_ref().and_then(|f| counted(&s, f));
            let t = Instant::now();
            step(
                &mut s,
                &format!("var __spikeCR = __spikeCHot({hot_k});"),
                "hot-timed.js",
                &mut out,
            )?;
            secs = t.elapsed().as_secs_f64();
            if let (Some(a), Some(b)) = (before, compiled.as_ref().and_then(|f| counted(&s, f))) {
                timed_exits += b - a;
            }
            step(
                &mut s,
                &format!("print('result ' + __spikeCR, {});", h.digest),
                "hot-check.js",
                &mut out,
            )?;
        }
    }
    Ok(Once {
        secs,
        output: out,
        code,
        exits: compiled.map(|_| timed_exits),
    })
}

/// The median of `v` (not empty).
pub fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let m = s.len() / 2;
    if s.len() % 2 == 1 {
        s[m]
    } else {
        (s[m - 1] + s[m]) / 2.0
    }
}

/// The 1-based ranks (lo, hi) of the order-statistic interval of the median of `n` values at
/// 95% or more: the largest `lo` with P(Bin(n, 1/2) < lo) <= 2.5%, and `hi = n + 1 - lo`
/// (n = 15: ranks 4 and 12, protocol §3). `None` below 6 values (no such interval).
pub fn median_ranks(n: usize) -> Option<(usize, usize)> {
    // cdf = P(X <= j) for X ~ Bin(n, 1/2); p = P(X = j) = C(n, j) / 2^n, from 2^-n (n halvings,
    // exact) by the running ratio. Rank lo = j + 1 is good while P(X < lo) = P(X <= j) <= 2.5%.
    let mut p = (0..n).fold(1.0f64, |a, _| a * 0.5);
    let mut cdf = 0.0f64;
    let mut lo = None;
    for j in 0..n {
        cdf += p;
        if cdf > 0.025 {
            break;
        }
        lo = Some(j + 1);
        p = p * (n - j) as f64 / (j + 1) as f64;
    }
    lo.map(|lo| (lo, n + 1 - lo))
}

/// Q1 and Q3 of `v` (linear between the order statistics, "type 7").
pub fn quartiles(v: &[f64]) -> (f64, f64) {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let q = |p: f64| {
        let h = (s.len() - 1) as f64 * p;
        let (i, f) = (h.floor() as usize, h - h.floor());
        if i + 1 < s.len() {
            s[i] + f * (s[i + 1] - s[i])
        } else {
            s[i]
        }
    };
    (q(0.25), q(0.75))
}

/// The class S summary of paired ratios.
#[derive(Clone, Debug)]
pub struct Summary {
    pub median: f64,
    /// The order-statistic interval of the median, if there are enough rounds.
    pub interval: Option<(f64, f64)>,
    /// IQR / median.
    pub iqr_share: f64,
}

impl Summary {
    pub fn of(ratios: &[f64]) -> Summary {
        let median = median(ratios);
        let mut s = ratios.to_vec();
        s.sort_by(f64::total_cmp);
        let interval = median_ranks(s.len()).map(|(lo, hi)| (s[lo - 1], s[hi - 1]));
        let (q1, q3) = quartiles(ratios);
        Summary {
            median,
            interval,
            iqr_share: (q3 - q1) / median,
        }
    }

    /// The verdict words of protocol §3 against `bar`, as a report (never a reading).
    pub fn side(&self, bar: f64) -> &'static str {
        if self.iqr_share > 0.05 {
            return "void (IQR > 5% of the median)";
        }
        match self.interval {
            None => "too few rounds",
            Some((lo, _)) if lo >= bar => "interval on the pass side",
            Some((_, hi)) if hi < bar => "interval on the fail side",
            Some(_) => "marginal",
        }
    }

    /// [`Summary::line`] with no verdict words (a report with a wrong run).
    pub fn line_without_verdict(&self) -> String {
        let iv = self
            .interval
            .map_or("-".to_string(), |(a, b)| format!("[{a:.3}, {b:.3}]"));
        format!(
            "median {:.3}, interval {iv}, IQR {:.1}% of the median; no verdict (a run was wrong)",
            self.median,
            100.0 * self.iqr_share
        )
    }

    pub fn line(&self, bar: f64) -> String {
        let iv = self
            .interval
            .map_or("-".to_string(), |(a, b)| format!("[{a:.3}, {b:.3}]"));
        format!(
            "median {:.3}, interval {iv}, IQR {:.1}% of the median; vs {bar}: {}",
            self.median,
            100.0 * self.iqr_share,
            self.side(bar)
        )
    }
}

/// The host line: CPU model and load, where the OS tells them.
pub fn host_line() -> String {
    let run = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let cpu = run("sysctl", &["-n", "machdep.cpu.brand_string"])
        .or_else(|| {
            std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|s| s.trim().to_string())
            })
        })
        .unwrap_or_else(|| "unknown CPU".into());
    let load = run("sysctl", &["-n", "vm.loadavg"])
        .or_else(|| std::fs::read_to_string("/proc/loadavg").ok())
        .unwrap_or_else(|| "unknown".into());
    format!(
        "{} on {}, load {}",
        std::env::consts::ARCH,
        cpu,
        load.trim()
    )
}

/// The whole class S report of `k` (`spike-c time`): spawns `exe once ...` per config and
/// round. Returns the report text and whether every run was right.
pub fn time(
    exe: &std::path::Path,
    k: &'static Kernel,
    rounds: usize,
    n: u32,
    hot_k: u32,
) -> Result<(String, bool), String> {
    let dn = kernels::driver(k, n)?;
    let mut report = format!(
        "Spike C class S timing, {} (report only, never a reading: protocol §3)\nhost: {}\n\
         rounds: {rounds} + 1 warm-up (dropped); n = {n} run() per benchmark; hot calls k = {hot_k}\n",
        k.name,
        host_line()
    );
    // Per round (1..=R), the time of each config's run, if it exited 0 and printed lines.
    let mut t: Vec<[Option<f64>; 4]> = Vec::new();
    let mut ref_t0: Option<Vec<String>> = None;
    let mut ref_h0: Option<Vec<String>> = None;
    let mut wrong = Vec::new();
    let mut code = None;
    // The most exits one C or H1 run counted in its timed region.
    let mut most_exits: Option<u64> = None;
    for round in 0..=rounds {
        let mut times = [None; 4];
        for which in order(round) {
            let out = Command::new(exe)
                .args([
                    "once",
                    which.name(),
                    k.name,
                    "--n",
                    &n.to_string(),
                    "--k",
                    &hot_k.to_string(),
                ])
                .output()
                .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
            let text = String::from_utf8_lossy(&out.stdout);
            let o = if out.status.success() {
                Once::parse(&text)?
            } else {
                wrong.push(format!(
                    "round {round} {}: exit {:?}: {}",
                    which.name(),
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
                continue;
            };
            let right = match which {
                Which::T0 => {
                    let ok =
                        dn.matches(&o.output) && ref_t0.as_ref().is_none_or(|r| *r == o.output);
                    ref_t0.get_or_insert(o.output.clone());
                    ok
                }
                Which::C => ref_t0.as_ref().is_none_or(|r| *r == o.output) && dn.matches(&o.output),
                Which::H0 => {
                    let ok = ref_h0.as_ref().is_none_or(|r| *r == o.output);
                    ref_h0.get_or_insert(o.output.clone());
                    ok
                }
                Which::H1 => ref_h0.as_ref().is_none_or(|r| *r == o.output),
            };
            if !right {
                wrong.push(format!(
                    "round {round} {}: output {:?}",
                    which.name(),
                    o.output
                ));
            }
            if which == Which::C {
                code = code.or(o.code);
            }
            if let Some(e) = o.exits {
                most_exits = Some(most_exits.map_or(e, |m| m.max(e)));
            }
            times[i_of(which)] = Some(o.secs);
        }
        if round > 0 {
            t.push(times);
        }
    }
    let ok = wrong.is_empty();
    for w in &Which::ALL {
        let v: Vec<f64> = t.iter().filter_map(|r| r[i_of(*w)]).collect();
        if !v.is_empty() {
            report.push_str(&format!(
                "t({}) median {:.4} s over {} runs\n",
                w.name(),
                median(&v),
                v.len()
            ));
        }
    }
    let r = paired_ratios(&t, Which::T0, Which::C);
    let s = paired_ratios(&t, Which::H0, Which::H1);
    // A wrong run voids the report: the summaries are printed, the verdict words are not.
    let words = |x: &Summary| {
        if ok {
            x.line(3.0)
        } else {
            x.line_without_verdict()
        }
    };
    if !r.is_empty() {
        let sr = Summary::of(&r);
        report.push_str(&format!("R = t(T0) / t(C) (gate unit): {}\n", words(&sr)));
        if !s.is_empty() {
            let ss = Summary::of(&s);
            report.push_str(&format!("s = t(H0) / t(H1) (context): {}\n", words(&ss)));
            if ss.median > 1.0 && sr.median > 1.0 {
                let p = (1.0 - 1.0 / sr.median) / (1.0 - 1.0 / ss.median);
                report.push_str(&format!(
                    "implied T0 share of the compiled set p = (1 - 1/R) / (1 - 1/s) = {:.1}% \
                     (node --jitless share {:.1}%, protocol §2.1)\n",
                    100.0 * p,
                    k.hot[0].jitless_share
                ));
            }
        }
    }
    if let Some((b, f, u)) = code {
        report.push_str(&format!(
            "compiled {}: {b} bytes, guards {f} fact + {u} unbacked",
            k.hot[0].name
        ));
        if let Some(e) = most_exits {
            report.push_str(&format!(
                "; exits in the timed region: at most {e} per C or H1 run (a guard failure \
                 deopts into T0)"
            ));
        }
        report.push('\n');
    }
    if ok {
        report.push_str("correctness: every run right (T0 a right driver run, C = T0, H1 = H0)\n");
    } else {
        report.push_str(&format!("correctness FAIL: {} wrong runs\n", wrong.len()));
        for w in &wrong {
            report.push_str(&format!("  {w}\n"));
        }
    }
    Ok((report, ok))
}

/// The ratios t(a) / t(b) of the rounds in which both runs gave a time, each from its own
/// round (a missing run drops that round's ratio; it never pairs runs of two rounds).
pub fn paired_ratios(t: &[[Option<f64>; 4]], a: Which, b: Which) -> Vec<f64> {
    t.iter()
        .filter_map(|r| Some(r[i_of(a)]? / r[i_of(b)]?))
        .collect()
}

fn i_of(w: Which) -> usize {
    match w {
        Which::T0 => 0,
        Which::C => 1,
        Which::H0 => 2,
        Which::H1 => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_of_the_median_interval() {
        assert_eq!(median_ranks(15), Some((4, 12)));
        assert_eq!(median_ranks(30), Some((10, 21)));
        assert_eq!(median_ranks(5), None);
        assert_eq!(median_ranks(6), Some((1, 6)));
    }

    #[test]
    fn median_quartiles_and_side() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(quartiles(&[1.0, 2.0, 3.0, 4.0, 5.0]), (2.0, 4.0));
        let pass: Vec<f64> = (0..15).map(|i| 3.5 + 0.001 * i as f64).collect();
        assert_eq!(Summary::of(&pass).side(3.0), "interval on the pass side");
        let fail: Vec<f64> = (0..15).map(|i| 2.0 + 0.001 * i as f64).collect();
        assert_eq!(Summary::of(&fail).side(3.0), "interval on the fail side");
        let wide: Vec<f64> = (0..15).map(|i| 2.0 + 0.2 * i as f64).collect();
        assert!(Summary::of(&wide).side(3.0).starts_with("void"));
        let edge: Vec<f64> = (0..15).map(|i| 2.993 + 0.001 * i as f64).collect();
        assert_eq!(Summary::of(&edge).side(3.0), "marginal");
    }

    /// Review F5 (M4-P2): over rounds 1..=15 each run of a pair comes first in 7 or 8 rounds,
    /// and every round runs each config once.
    #[test]
    fn the_order_is_counterbalanced() {
        let first = |a: Which, b: Which| {
            (1..=15)
                .filter(|&r| {
                    let o = order(r);
                    let pos = |w| o.iter().position(|&x| x == w).expect("each config");
                    pos(a) < pos(b)
                })
                .count()
        };
        for (a, b) in [(Which::T0, Which::C), (Which::H0, Which::H1)] {
            let n = first(a, b);
            assert!(
                (7..=8).contains(&n),
                "{a:?} before {b:?} in {n} of 15 rounds"
            );
        }
        for r in 0..8 {
            let mut o = order(r).to_vec();
            o.sort_by_key(|&w| i_of(w));
            assert_eq!(o, Which::ALL);
        }
    }

    /// Review F7 and Codex r1 (M4-P2): a failed run drops its round's ratio and never shifts
    /// the pairing of later rounds.
    #[test]
    fn a_failed_run_drops_only_its_round() {
        let t = [
            [Some(4.0), Some(1.0), Some(6.0), Some(1.0)],
            [Some(8.0), None, Some(6.0), Some(2.0)],
            [Some(9.0), Some(3.0), None, Some(1.0)],
        ];
        assert_eq!(paired_ratios(&t, Which::T0, Which::C), [4.0, 3.0]);
        assert_eq!(paired_ratios(&t, Which::H0, Which::H1), [6.0, 3.0]);
        let s = Summary::of(&[4.0; 15]);
        assert!(s.line(3.0).contains("pass side"));
        let quiet = s.line_without_verdict();
        assert!(
            quiet.contains("no verdict") && !quiet.contains("pass side"),
            "{quiet}"
        );
    }

    #[test]
    fn once_text_round_trip() {
        let o = Once {
            secs: 0.25,
            output: vec!["Encrypt 8".into(), String::new(), "x".into()],
            code: Some((3292, 2, 27)),
            exits: Some(4),
        };
        assert_eq!(Once::parse(&o.text()).expect("parse"), o);
    }

    /// lin_solve's hot harness (WI-SC5): the copy in hot.js is compiled by its function id and
    /// installed; H1 prints H0's lines with no exit.
    #[test]
    fn the_lin_solve_harness_compiles_its_copy() {
        if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64)
            || !kernels::bench_present()
        {
            eprintln!("skip: aarch64 and the v8-v7 checkout only");
            return;
        }
        let k = kernels::kernel("navier-stokes").expect("navier-stokes");
        let h0 = once(Which::H0, k, 1, 1).expect("h0");
        let h1 = once(Which::H1, k, 1, 1).expect("h1");
        assert_eq!(h1.output, h0.output);
        assert!(h0.output[0].starts_with("result "), "{h0:?}");
        assert_eq!(h1.exits, Some(0));
        assert!(h1.code.is_some() && h0.code.is_none());
    }

    /// The four configurations in this process, n = 1: C prints T0's lines, H1 H0's, and the
    /// compiled code is the one `tier` makes. Skips off aarch64 and without the checkout.
    #[test]
    fn four_configs_agree_on_crypto() {
        if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64)
            || !kernels::bench_present()
        {
            eprintln!("skip: aarch64 and the v8-v7 checkout only");
            return;
        }
        let k = kernels::kernel("crypto").expect("crypto");
        let t0 = once(Which::T0, k, 1, 50).expect("t0");
        let c = once(Which::C, k, 1, 50).expect("c");
        assert!(
            kernels::driver(k, 1).expect("d").matches(&t0.output),
            "{t0:?}"
        );
        assert_eq!(c.output, t0.output);
        assert!(c.code.is_some() && t0.code.is_none());
        assert!(c.exits.is_some() && t0.exits.is_none());
        let h0 = once(Which::H0, k, 1, 50).expect("h0");
        let h1 = once(Which::H1, k, 1, 50).expect("h1");
        assert_eq!(h1.output, h0.output);
        assert_eq!(h0.output.len(), 1, "{h0:?}");
        assert!(h0.output[0].starts_with("result "), "{h0:?}");
    }
}
