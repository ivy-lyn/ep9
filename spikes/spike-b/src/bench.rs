//! `spike-b bench …`: the D21 round-trip bench of the Spike B protocol
//! (`docs/measurements/2026-10-22-spike-b-protocol.md`; owner P3-SPIKEB-BENCH, plan-v5 §6).
//! `bench.sh` runs it; it builds nothing and passes tau (from the SP-Dispatch `bench.json`)
//! as an argument, because the spike has no JSON parser (its Cargo files are frozen).
//!
//! ```text
//! spike-b bench --tau NS [--tau-samples a,b,..] [--dispatches D] [--tau-source FILE]
//!               [--rounds 15] [--marginal-rounds 31] [--n 100000000] [--commit TEXT]
//!                                     the rounds of protocol §5-§6, then the verdict
//! spike-b bench run <A|C|A0|J|JC> [--n N]
//!                                     one run in this process (one process per run, §5)
//! spike-b bench k                     k and the counted listing lines (§4)
//! spike-b bench ratio --ir-a X --ir-c Y --ir-loop Z --dispatches D --n N [--ir-ops O]
//!                                     the cachegrind instruction ratio of §8 (a report)
//! ```
//!
//! | Config | `drive` | `id` | Kernel | Entry from Rust |
//! |---|---|---|---|---|
//! | A | T0 | JIT (Baseline slot) | A | `Isolate::call` |
//! | C | T0 | not called (`f` = undefined) | C | `Isolate::call` |
//! | A0 | T0 | T0 | A | `Isolate::call` |
//! | J | JIT | T0 | A | `ep9_vm::call_from_rust` |
//! | JC | JIT | not called | C | `ep9_vm::call_from_rust` |
//!
//! `Isolate::call` on a function with a native slot throws not_implemented (call.rs), so the
//! configs that JIT `drive` enter it through `call_from_rust` (`harness` tests).
//!
//! The coordinator runs each config of a round in a child process (`spike-b bench run`): the
//! child makes the isolate, compiles and loads the kernel, installs the JIT code, calls
//! `drive(f, 10000)` untimed, then times one call `drive(f, n)` around the call only. A child
//! whose `drive` does not return n fails the run (exit 1), not the gate.

use std::process::Command;
use std::time::Instant;

use ep9_bytecode::{
    CompiledUnit, FunctionId, Instruction, Instructions, Opcode, OperandScale, UnitString,
    disassemble_function,
};
use ep9_vm::{DispatchKind, Value};

use crate::harness::{Config, Session, compile, exception_text};
use crate::structural::{LOOP_C_DEFS, LOOP_DEFS};

/// Loop iterations of a timed run (protocol §2).
pub const DEFAULT_N: u32 = 100_000_000;
/// The untimed warm-up call `drive(f, 10000)` (protocol §5).
pub const WARMUP_N: u32 = 10_000;
/// The gate: one round trip costs ≤ 18 loop-kernel dispatches (protocol header).
pub const GATE_LIMIT: f64 = 18.0;
/// Kept rounds of a run, and of the re-run of a marginal result (protocol §5).
pub const ROUNDS: usize = 15;
pub const MARGINAL_ROUNDS: usize = 31;
/// The IQR of t_A or t_C above this % of its median makes the run void (protocol §6).
pub const VOID_IQR_PCT: f64 = 5.0;

/// One configuration of a round (protocol §2 table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cfg {
    A,
    C,
    A0,
    J,
    JC,
}

impl Cfg {
    /// The configs of one round, in the order that round 0 starts with.
    pub const ALL: [Cfg; 5] = [Cfg::A, Cfg::C, Cfg::A0, Cfg::J, Cfg::JC];

    pub fn name(self) -> &'static str {
        match self {
            Cfg::A => "A",
            Cfg::C => "C",
            Cfg::A0 => "A0",
            Cfg::J => "J",
            Cfg::JC => "JC",
        }
    }

    pub fn parse(s: &str) -> Option<Cfg> {
        Cfg::ALL.into_iter().find(|c| c.name() == s)
    }

    /// Kernel A (with the call) or kernel C (without it).
    fn defs(self) -> &'static str {
        match self {
            Cfg::A | Cfg::A0 | Cfg::J => LOOP_DEFS,
            Cfg::C | Cfg::JC => LOOP_C_DEFS,
        }
    }

    /// The functions this config JIT-compiles and installs.
    fn jit(self) -> &'static [&'static str] {
        match self {
            Cfg::A => &["id"],
            Cfg::J | Cfg::JC => &["drive"],
            Cfg::C | Cfg::A0 => &[],
        }
    }

    /// True if `drive` runs native code (entered with `call_from_rust`).
    fn native_drive(self) -> bool {
        matches!(self, Cfg::J | Cfg::JC)
    }

    /// True if `drive` calls `f` (= `id`).
    fn calls_id(self) -> bool {
        matches!(self, Cfg::A | Cfg::A0 | Cfg::J)
    }
}

/// One timed run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub ns: u64,
    /// What `drive(f, n)` returned (it must be n).
    pub result: i64,
}

fn slot_kind(s: &Session, name: &str) -> Result<Option<DispatchKind>, String> {
    let f = s
        .iso
        .global(name)
        .ok_or_else(|| format!("no global {name}"))?;
    Ok(ep9_vm::native_target(&s.iso, f)
        .ok_or_else(|| format!("{name} has no bytecode"))?
        .kind)
}

/// Calls `drive(f, n)` once: `Isolate::call` for a T0 `drive`, `call_from_rust` for a JIT one.
/// Returns the result and the wall time of the call alone.
fn call_drive(s: &mut Session, cfg: Cfg, n: u32) -> Result<(i64, u64), String> {
    let drive = s.iso.global("drive").ok_or("no global drive")?;
    let f = if cfg.calls_id() {
        s.iso.global("id").ok_or("no global id")?
    } else {
        Value::UNDEFINED
    };
    let n = i32::try_from(n).map_err(|_| format!("n = {n} is not an int32"))?;
    // `drive` and `id` are global bindings, so both stay alive across the call (GC point).
    let args = [f, Value::int32(n)];
    let t = Instant::now();
    let r = if cfg.native_drive() {
        ep9_vm::call_from_rust(&mut s.iso, drive, Value::UNDEFINED, &args)
    } else {
        s.iso.call(drive, Value::UNDEFINED, &args)
    };
    let ns = u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let v = r.map_err(|e| format!("drive threw: {}", exception_text(&e)))?;
    let result = v
        .as_int32()
        .map(i64::from)
        .ok_or_else(|| format!("drive returned {}, not an int32", s.iso.display(v)))?;
    Ok((result, ns))
}

/// One run of `cfg` in this process (protocol §5): isolate, kernel, JIT install, the untimed
/// `drive(f, 10000)`, then one timed `drive(f, n)`. A wrong result of either call is an error.
pub fn run_config(cfg: Cfg, n: u32) -> Result<Run, String> {
    let mut s = Session::new(&Config::default())?;
    s.run(cfg.defs(), "kernel.js")
        .result
        .map_err(|e| format!("kernel: {e}"))?;
    if !cfg.jit().is_empty() {
        s.jit(cfg.jit())?;
    }
    // The slots must be what the config says (the JIT refuses nothing here: `structural ops`).
    let want_drive = if cfg.native_drive() {
        DispatchKind::Baseline
    } else {
        DispatchKind::Interp
    };
    let kind = slot_kind(&s, "drive")?;
    if kind != Some(want_drive) {
        return Err(format!(
            "{}: drive slot {kind:?}, want {want_drive:?}",
            cfg.name()
        ));
    }
    if cfg.calls_id() {
        let want = if cfg == Cfg::A {
            DispatchKind::Baseline
        } else {
            DispatchKind::Interp
        };
        let kind = slot_kind(&s, "id")?;
        if kind != Some(want) {
            return Err(format!("{}: id slot {kind:?}, want {want:?}", cfg.name()));
        }
    }
    let (warm, _) = call_drive(&mut s, cfg, WARMUP_N)?;
    if warm != i64::from(WARMUP_N) {
        return Err(format!("warm-up drive(f, {WARMUP_N}) returned {warm}"));
    }
    let (result, ns) = call_drive(&mut s, cfg, n)?;
    if result != i64::from(n) {
        return Err(format!(
            "{}: drive(f, {n}) returned {result}, not {n}: the run is invalid",
            cfg.name()
        ));
    }
    Ok(Run { ns, result })
}

// ---- k (protocol §4) ---------------------------------------------------------------------

/// The loop of `drive` in one kernel: the instructions from the `JumpLoop` target to the
/// `JumpLoop`, and their disassembly lines.
#[derive(Clone, Debug)]
pub struct LoopRange {
    pub start: u32,
    pub end: u32,
    pub count: usize,
    /// The lines of `disassemble_function` for the range, as printed.
    pub lines: Vec<String>,
}

fn function_named(unit: &CompiledUnit, name: &str) -> Result<FunctionId, String> {
    let want = UnitString::new(name);
    let ids: Vec<usize> = (0..unit.functions.len())
        .filter(|&i| unit.functions[i].name.map(|s| unit.string(s)) == Some(&want))
        .collect();
    match ids.as_slice() {
        [i] => Ok(FunctionId(*i as u32)),
        _ => Err(format!("{} functions named {name}", ids.len())),
    }
}

/// The loop range of `drive` in `defs`. Fails if the range has a Wide or ExtraWide prefix (a
/// prefix takes T0's cold path, so it is not one dispatch), if `drive` has not exactly one
/// `JumpLoop`, or if the range has a branch other than the one loop exit (then not every
/// instruction runs once per iteration).
pub fn loop_range(defs: &str) -> Result<LoopRange, String> {
    let unit = compile(defs)?;
    let id = function_named(&unit, "drive")?;
    let f = unit.function(id);
    let insns: Vec<Instruction<'_>> = Instructions::new(&f.bytecode)
        .collect::<Result<_, _>>()
        .map_err(|(at, e)| format!("drive: decode error at {at}: {e}"))?;
    let loops: Vec<&Instruction<'_>> = insns
        .iter()
        .filter(|i| i.opcode == Opcode::JumpLoop)
        .collect();
    let [jl] = loops.as_slice() else {
        return Err(format!(
            "drive has {} JumpLoop instructions, want 1",
            loops.len()
        ));
    };
    let start = jl.jump_target().ok_or("JumpLoop without a target")?;
    let end = jl.offset;
    let range: Vec<&Instruction<'_>> = insns
        .iter()
        .filter(|i| (start..=end).contains(&i.offset))
        .collect();
    if range.first().map(|i| i.offset) != Some(start) {
        return Err(format!(
            "the JumpLoop target {start} is not an instruction start"
        ));
    }
    if let Some(p) = range.iter().find(|i| i.scale != OperandScale::Single) {
        return Err(format!(
            "a {:?} prefix at offset {} ({}) in the loop range {start}..={end}: the run fails (protocol §4)",
            p.scale, p.offset, p.opcode
        ));
    }
    let exits: Vec<&&Instruction<'_>> = range
        .iter()
        .filter(|i| i.offset != end && i.jump_target().is_some())
        .collect();
    let exits_ok = exits.len() == 1 && exits[0].jump_target().is_some_and(|t| t > end);
    if !exits_ok {
        return Err(format!(
            "the loop range {start}..={end} has branches {:?}; want one loop exit",
            exits
                .iter()
                .map(|i| (i.offset, i.opcode.to_string()))
                .collect::<Vec<_>>()
        ));
    }
    let lines = listing_lines(&disassemble_function(&unit, id), start, end);
    if lines.len() != range.len() {
        return Err(format!(
            "the listing has {} lines in {start}..={end}, the decoder {} instructions",
            lines.len(),
            range.len()
        ));
    }
    Ok(LoopRange {
        start,
        end,
        count: range.len(),
        lines,
    })
}

/// The instruction lines of a `disassemble_function` listing whose offset is in
/// `start..=end`. Instruction lines follow the `code (` header and start with the offset.
fn listing_lines(listing: &str, start: u32, end: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    for l in listing.lines() {
        let t = l.trim_start();
        if t.starts_with("code (") {
            in_code = true;
            continue;
        }
        if !in_code {
            continue;
        }
        let Some(off) = t
            .split_whitespace()
            .next()
            .and_then(|w| w.parse::<u32>().ok())
        else {
            if !out.is_empty() {
                break;
            }
            continue;
        };
        if (start..=end).contains(&off) {
            out.push(l.trim_end().to_string());
        }
    }
    out
}

/// k = (count in A) − (count in C) − 1, with both loop ranges.
pub fn k_value() -> Result<(i64, LoopRange, LoopRange), String> {
    let a = loop_range(LOOP_DEFS)?;
    let c = loop_range(LOOP_C_DEFS)?;
    let k = a.count as i64 - c.count as i64 - 1;
    Ok((k, a, c))
}

fn print_k(k: i64, a: &LoopRange, c: &LoopRange) {
    for (name, r) in [("A", a), ("C", c)] {
        println!(
            "k listing {name}: drive loop {}..={} ({} instructions; ep9_bytecode::disassemble_function, this build)",
            r.start, r.end, r.count
        );
        for l in &r.lines {
            println!("  {name} |{l}");
        }
    }
    println!(
        "k = {} - {} - 1 = {k} (protocol §4: extra T0 dispatches per iteration of A over C, the call not counted)",
        a.count, c.count
    );
}

// ---- statistics (protocol §6; the rule of sp-dispatch `stat`) ---------------------------

/// Rank k (1-based) of the order-statistic interval [x(k), x(n+1-k)] of the median: the
/// largest k with P(Bin(n, 1/2) <= k - 1) <= 0.025. n = 15 -> 4 (ranks 4 and 12); n = 31 -> 10
/// (ranks 10 and 22), as protocol §6. For n < 6 it returns 1 (min and max).
pub fn median_rank(n: usize) -> usize {
    // 0.5^n by repeated halving: exact, and clippy.toml disallows `f64::powi` (H36: the
    // compiler-rt helper is not correctly rounded).
    let mut pmf = (0..n).fold(1.0f64, |p, _| p * 0.5);
    let mut cdf = 0.0;
    let mut k = 1;
    for i in 0..n {
        cdf += pmf;
        if cdf > 0.025 {
            break;
        }
        k = i + 1;
        pmf *= (n - i) as f64 / (i + 1) as f64;
    }
    k.min(n.div_ceil(2))
}

/// Linear-interpolation quantile of sorted values.
fn quantile(s: &[f64], p: f64) -> f64 {
    let x = p * (s.len() - 1) as f64;
    let i = x.floor() as usize;
    match s.get(i + 1) {
        Some(&next) => s[i] + (next - s[i]) * (x - i as f64),
        None => s[i],
    }
}

/// Median, order-statistic interval and IQR (% of the median) of `v`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub median: f64,
    pub lo: f64,
    pub hi: f64,
    pub iqr_pct: f64,
}

pub fn summarize(v: &[f64]) -> Summary {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let k = median_rank(n);
    let median = quantile(&s, 0.5);
    Summary {
        n,
        median,
        lo: s[k - 1],
        hi: s[n - k],
        iqr_pct: (quantile(&s, 0.75) - quantile(&s, 0.25)) / median.abs() * 100.0,
    }
}

/// The interval verdict of protocol §6: pass if the whole interval is ≤ 18, fail if the whole
/// interval is > 18, else marginal.
pub fn interval_verdict(rt: &Summary) -> &'static str {
    if rt.hi <= GATE_LIMIT {
        "pass"
    } else if rt.lo > GATE_LIMIT {
        "fail"
    } else {
        "marginal"
    }
}

/// The verdict of one set of rounds: void if the IQR of t_A or t_C is > 5% of its median,
/// else the interval verdict.
pub fn rounds_verdict(rt: &Summary, t_a: &Summary, t_c: &Summary) -> &'static str {
    if t_a.iqr_pct > VOID_IQR_PCT || t_c.iqr_pct > VOID_IQR_PCT {
        "void"
    } else {
        interval_verdict(rt)
    }
}

// ---- the rounds ----------------------------------------------------------------------------

/// The times of one round, in `Cfg::ALL` order.
type Round = [u64; 5];

fn idx(c: Cfg) -> usize {
    Cfg::ALL.iter().position(|&x| x == c).expect("in ALL")
}

/// Runs `cfg` in a child process of this binary and returns its time.
fn run_child(cfg: Cfg, n: u32) -> Result<u64, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let out = Command::new(&exe)
        .args(["bench", "run", cfg.name(), "--n", &n.to_string()])
        .output()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(format!(
            "run {} failed ({}): {}{}",
            cfg.name(),
            out.status,
            text.trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse_run(cfg, n, &text)
}

/// The time of a child's `spike-b-run` line; an error if there is none or if its `drive`
/// result is not n (the run is invalid, protocol §2).
fn parse_run(cfg: Cfg, n: u32, text: &str) -> Result<u64, String> {
    let line = text
        .lines()
        .find(|l| l.starts_with("spike-b-run "))
        .ok_or_else(|| format!("run {}: no result line in {text:?}", cfg.name()))?;
    let field = |k: &str| {
        line.split_whitespace()
            .find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(|| format!("run {}: no {k} in {line:?}", cfg.name()))
    };
    let (ns, result) = (field("ns")?, field("result")?);
    if result != i64::from(n) {
        return Err(format!(
            "run {}: drive returned {result}, not {n}: the run is invalid",
            cfg.name()
        ));
    }
    u64::try_from(ns).map_err(|_| format!("run {}: ns {ns}", cfg.name()))
}

/// Runs round 0 (warm-up, dropped) and `kept` more rounds. Round r starts at config `r mod 5`.
fn run_rounds(kept: usize, n: u32, label: &str) -> Result<Vec<Round>, String> {
    let m = Cfg::ALL.len();
    let mut rounds = Vec::with_capacity(kept);
    for r in 0..=kept {
        let mut t: Round = [0; 5];
        let mut order = Vec::with_capacity(m);
        for j in 0..m {
            let c = Cfg::ALL[(r + j) % m];
            t[idx(c)] = run_child(c, n)?;
            order.push(c.name());
        }
        let ms = |c: Cfg| t[idx(c)] as f64 / 1e6;
        eprintln!(
            "{label} round {r:2}{} [{}] A {:.1} ms, C {:.1} ms, A0 {:.1} ms, J {:.1} ms, JC {:.1} ms",
            if r == 0 { " (warm-up)" } else { "" },
            order.join(" "),
            ms(Cfg::A),
            ms(Cfg::C),
            ms(Cfg::A0),
            ms(Cfg::J),
            ms(Cfg::JC)
        );
        if r > 0 {
            rounds.push(t);
        }
    }
    Ok(rounds)
}

/// The per-round values of one set of rounds.
struct Derived {
    rt: Vec<f64>,
    a0: Vec<f64>,
    j_ns: Vec<f64>,
}

fn derive(rounds: &[Round], n: u32, tau: f64, k: i64) -> Derived {
    let per = |a: Cfg, b: Cfg, t: &Round| (t[idx(a)] as f64 - t[idx(b)] as f64) / f64::from(n);
    Derived {
        rt: rounds
            .iter()
            .map(|t| per(Cfg::A, Cfg::C, t) / tau - k as f64)
            .collect(),
        a0: rounds
            .iter()
            .map(|t| per(Cfg::A0, Cfg::C, t) / tau - k as f64)
            .collect(),
        j_ns: rounds.iter().map(|t| per(Cfg::J, Cfg::JC, t)).collect(),
    }
}

fn print_summary(what: &str, unit: &str, s: &Summary) {
    println!(
        "{what:<44} n={:<2} median {:>9.3} interval [{:.3}, {:.3}] IQR {:>5.1}% {unit}",
        s.n, s.median, s.lo, s.hi, s.iqr_pct
    );
}

/// Prints one set of rounds and returns (verdict, RT summary).
fn report_rounds(
    label: &str,
    rounds: &[Round],
    n: u32,
    tau: f64,
    k: i64,
) -> (&'static str, Summary) {
    let d = derive(rounds, n, tau, k);
    println!();
    println!(
        "{label}: {} kept rounds, n = {n}, tau = {tau:.6} ns, k = {k}",
        rounds.len()
    );
    println!(
        "{:>5} {:>13} {:>13} {:>13} {:>13} {:>13} {:>9} {:>9} {:>9} {:>9}",
        "round",
        "t_A ns",
        "t_C ns",
        "t_A0 ns",
        "t_J ns",
        "t_JC ns",
        "RT_r",
        "A0_r",
        "J_r ns",
        "J_r tau"
    );
    for (i, t) in rounds.iter().enumerate() {
        println!(
            "{:>5} {:>13} {:>13} {:>13} {:>13} {:>13} {:>9.3} {:>9.3} {:>9.3} {:>9.3}",
            i + 1,
            t[idx(Cfg::A)],
            t[idx(Cfg::C)],
            t[idx(Cfg::A0)],
            t[idx(Cfg::J)],
            t[idx(Cfg::JC)],
            d.rt[i],
            d.a0[i],
            d.j_ns[i],
            d.j_ns[i] / tau
        );
    }
    let col = |c: Cfg| -> Vec<f64> { rounds.iter().map(|t| t[idx(c)] as f64).collect() };
    let (t_a, t_c) = (summarize(&col(Cfg::A)), summarize(&col(Cfg::C)));
    for c in Cfg::ALL {
        let ms: Vec<f64> = col(c).iter().map(|t| t / 1e6).collect();
        print_summary(&format!("t_{}", c.name()), "ms", &summarize(&ms));
    }
    let rt = summarize(&d.rt);
    print_summary("RT (gate: T0->JIT->T0 round trip)", "dispatches", &rt);
    print_summary(
        "A0 report (T0->T0 call, same k)",
        "dispatches",
        &summarize(&d.a0),
    );
    let j = summarize(&d.j_ns);
    print_summary("J report (JIT->T0->JIT round trip)", "ns", &j);
    let j_tau: Vec<f64> = d.j_ns.iter().map(|x| x / tau).collect();
    print_summary("J report in tau units", "dispatches", &summarize(&j_tau));
    let v = rounds_verdict(&rt, &t_a, &t_c);
    println!(
        "{label} verdict: {v} (interval verdict {}; IQR t_A {:.1}%, t_C {:.1}%; void above {VOID_IQR_PCT}%)",
        interval_verdict(&rt),
        t_a.iqr_pct,
        t_c.iqr_pct
    );
    (v, rt)
}

/// The CPU model: `SPIKE_B_CPU` if set (a VM that hides the model), /proc/cpuinfo "model
/// name", else lscpu "Model name" (Linux), sysctl machdep.cpu.brand_string (macOS),
/// PROCESSOR_IDENTIFIER (Windows), else the arm64 "CPU implementer" and "CPU part" ids.
/// A value of "-" (lscpu in an arm64 VM) counts as missing.
pub fn cpu_model() -> String {
    if let Ok(m) = std::env::var("SPIKE_B_CPU")
        && !m.trim().is_empty()
    {
        return format!("{} (SPIKE_B_CPU)", m.trim());
    }
    let field = |text: &str, key: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            let v = v.trim();
            (k.trim() == key && !v.is_empty() && v != "-").then(|| v.to_string())
        })
    };
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    if let Some(m) = field(&cpuinfo, "model name") {
        return m;
    }
    let cmd = |prog: &str, args: &[&str]| {
        Command::new(prog)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    if let Some(m) = cmd("lscpu", &[]).and_then(|t| field(&t, "Model name")) {
        return m;
    }
    if let Some(m) = cmd("sysctl", &["-n", "machdep.cpu.brand_string"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return m;
    }
    if let Ok(m) = std::env::var("PROCESSOR_IDENTIFIER") {
        return m;
    }
    match (
        field(&cpuinfo, "CPU implementer"),
        field(&cpuinfo, "CPU part"),
    ) {
        (Some(i), Some(p)) => format!("unknown model (CPU implementer {i}, CPU part {p})"),
        _ => "unknown".into(),
    }
}

// ---- argument parsing ----------------------------------------------------------------------

struct Opts {
    tau: Option<f64>,
    tau_samples: Vec<f64>,
    dispatches: Option<f64>,
    tau_source: String,
    rounds: usize,
    marginal_rounds: usize,
    n: u32,
    commit: String,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        tau: None,
        tau_samples: Vec::new(),
        dispatches: None,
        tau_source: "(not given)".into(),
        rounds: ROUNDS,
        marginal_rounds: MARGINAL_ROUNDS,
        n: DEFAULT_N,
        commit: "(not given)".into(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value"));
        let num = |v: &String| v.parse::<f64>().map_err(|e| format!("{a} {v}: {e}"));
        match a.as_str() {
            "--tau" => o.tau = Some(num(val()?)?),
            "--tau-samples" => {
                o.tau_samples = val()?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        s.parse::<f64>()
                            .map_err(|e| format!("--tau-samples {s}: {e}"))
                    })
                    .collect::<Result<_, _>>()?
            }
            "--dispatches" => o.dispatches = Some(num(val()?)?),
            "--tau-source" => o.tau_source = val()?.clone(),
            "--rounds" => o.rounds = val()?.parse().map_err(|e| format!("--rounds: {e}"))?,
            "--marginal-rounds" => {
                o.marginal_rounds = val()?
                    .parse()
                    .map_err(|e| format!("--marginal-rounds: {e}"))?
            }
            "--n" => o.n = val()?.parse().map_err(|e| format!("--n: {e}"))?,
            "--commit" => o.commit = val()?.clone(),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if o.rounds == 0 {
        return Err("--rounds must be at least 1".into());
    }
    if i32::try_from(o.n).is_err() {
        return Err(format!("--n {} is not an int32", o.n));
    }
    Ok(o)
}

/// The labels of a run whose parameters are not the protocol's. bench.sh makes the run
/// provisional for each (decision M2-P3-SPIKEB-VERDICT): a debug build, fewer than 15 kept
/// rounds (§5), n other than 10^8 (§2), a marginal re-run with fewer than 31 kept rounds (§5).
pub fn parameter_labels(debug: bool, rounds: usize, marginal_rounds: usize, n: u32) -> Vec<String> {
    let mut labels = Vec::new();
    if debug {
        labels.push("debug build (the gate uses the release build)".into());
    }
    if rounds < ROUNDS {
        labels.push(format!("{rounds} kept rounds (< {ROUNDS})"));
    }
    if n != DEFAULT_N {
        labels.push(format!("n = {n} (protocol §2: {DEFAULT_N})"));
    }
    if marginal_rounds < MARGINAL_ROUNDS {
        labels.push(format!(
            "{marginal_rounds} kept rounds for a marginal re-run (< {MARGINAL_ROUNDS})"
        ));
    }
    labels
}

/// The label of a marginal result that cannot re-run (protocol §6 needs more rounds than the
/// first run), or None if it can: the verdict then stays "marginal" and the run is provisional.
pub fn no_rerun_label(rounds: usize, marginal_rounds: usize) -> Option<String> {
    (marginal_rounds <= rounds).then(|| {
        format!(
            "marginal result not re-run (--marginal-rounds {marginal_rounds} is not above --rounds {rounds}; protocol §6)"
        )
    })
}

/// The coordinator: header, k, the rounds, the reports, the verdict.
fn coordinate(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    let tau = o.tau.ok_or(
        "--tau <ns per loop-kernel dispatch> is required (bench.sh reads it from bench.json)",
    )?;
    if !(tau.is_finite() && tau > 0.0) {
        return Err(format!("--tau {tau} must be a positive number"));
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("== Spike B D21 bench (docs/measurements/2026-10-22-spike-b-protocol.md)");
    println!(
        "host: {}-{} ({profile} build)",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    println!("cpu: {}", cpu_model());
    println!("commit: {}", o.commit);
    println!("contracts: {}", crate::contracts_line());
    println!(
        "T0 dispatch: {:?} (spike_b::harness::DISPATCH)",
        crate::harness::DISPATCH
    );
    println!(
        "tau: {tau:.6} ns per loop-kernel dispatch (median_ns.sibling / dispatches of kernel loop)"
    );
    println!("tau source: {}", o.tau_source);
    if let (false, Some(d)) = (o.tau_samples.is_empty(), o.dispatches) {
        let per: Vec<f64> = o.tau_samples.iter().map(|x| x / d).collect();
        print_summary(
            "tau report (per-round sibling loop / dispatches)",
            "ns",
            &summarize(&per),
        );
    }
    let (k, a, c) = k_value()?;
    print_k(k, &a, &c);
    let mut labels = parameter_labels(cfg!(debug_assertions), o.rounds, o.marginal_rounds, o.n);
    let rounds = run_rounds(o.rounds, o.n, "run")?;
    let (mut verdict, mut rt) = report_rounds("run", &rounds, o.n, tau, k);
    if verdict == "marginal" {
        labels.push("marginal".into());
    }
    let no_rerun = no_rerun_label(o.rounds, o.marginal_rounds);
    if let Some(l) = no_rerun.filter(|_| verdict == "marginal") {
        // No re-run: the verdict stays "marginal", and bench.sh makes the run provisional.
        println!("{l}: the verdict stays marginal");
        labels.push(l);
    } else if verdict == "marginal" {
        // Protocol §6: run again with 31 kept rounds; the median decides, the label stays.
        let again = run_rounds(o.marginal_rounds, o.n, "marginal re-run")?;
        let (v2, rt2) = report_rounds("marginal re-run", &again, o.n, tau, k);
        rt = rt2;
        verdict = if v2 == "void" {
            "void"
        } else if rt2.median <= GATE_LIMIT {
            "pass"
        } else {
            "fail"
        };
        println!(
            "marginal re-run decides by its median {:.3}: {verdict}",
            rt2.median
        );
    }
    println!();
    println!(
        "SPIKE-B-RESULT {verdict} median={:.3} lo={:.3} hi={:.3} rounds={} k={k} tau={tau:.6} labels={}",
        rt.median,
        rt.lo,
        rt.hi,
        rt.n,
        if labels.is_empty() {
            "-".to_string()
        } else {
            labels.join("; ")
        }
    );
    Ok(true)
}

/// `spike-b bench run <cfg> [--n N]`.
fn run_one(args: &[String]) -> Result<bool, String> {
    let [name, rest @ ..] = args else {
        return Err("run <A|C|A0|J|JC> [--n N]".into());
    };
    let cfg = Cfg::parse(name).ok_or_else(|| format!("unknown config {name}"))?;
    let n = match rest {
        [] => DEFAULT_N,
        [flag, v] if flag == "--n" => v.parse().map_err(|e| format!("--n: {e}"))?,
        _ => return Err("run <A|C|A0|J|JC> [--n N]".into()),
    };
    let r = run_config(cfg, n)?;
    println!(
        "spike-b-run config={} n={n} ns={} result={}",
        cfg.name(),
        r.ns,
        r.result
    );
    Ok(true)
}

/// `spike-b bench ratio`: protocol §8, `((Ir_A - Ir_C) / n) / (Ir_loop / dispatches) - k`.
fn ratio(args: &[String]) -> Result<bool, String> {
    let mut v = std::collections::BTreeMap::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let key = a
            .strip_prefix("--")
            .filter(|k| ["ir-a", "ir-c", "ir-loop", "ir-ops", "dispatches", "n"].contains(k))
            .ok_or_else(|| format!("unknown option {a}"))?;
        let x = it.next().ok_or_else(|| format!("{a} needs a value"))?;
        v.insert(
            key,
            x.replace(',', "")
                .parse::<f64>()
                .map_err(|e| format!("{a} {x}: {e}"))?,
        );
    }
    let get = |k: &str| {
        v.get(k)
            .copied()
            .ok_or_else(|| format!("--{k} is required"))
    };
    let (ir_a, ir_c, ir_loop, d, n) = (
        get("ir-a")?,
        get("ir-c")?,
        get("ir-loop")?,
        get("dispatches")?,
        get("n")?,
    );
    let (k, a, c) = k_value()?;
    print_k(k, &a, &c);
    let per_dispatch = ir_loop / d;
    let r = ((ir_a - ir_c) / n) / per_dispatch - k as f64;
    println!(
        "cachegrind: Ir_A {ir_a:.0}, Ir_C {ir_c:.0}, n {n:.0}: (Ir_A - Ir_C) / n = {:.3} Ir per iteration",
        (ir_a - ir_c) / n
    );
    println!(
        "cachegrind: Ir_loop {ir_loop:.0} over {d:.0} dispatches = {per_dispatch:.4} Ir per dispatch"
    );
    if let Some(ops) = v.get("ir-ops") {
        println!(
            "cachegrind: process start-up inside Ir_loop (Ir of `sp-dispatch ops`): {ops:.0} = {:.4}% of Ir_loop",
            ops / ir_loop * 100.0
        );
    }
    println!(
        "CACHEGRIND-RATIO {r:.3} (dispatches; a report, never a gate; cpu: {})",
        cpu_model()
    );
    Ok(true)
}

/// Entry of `spike-b bench <args>` (`main.rs` dispatches here). Returns the exit status: 0 on
/// a complete run (whatever the verdict), 1 if a run is invalid, 2 on a usage error.
pub fn main(args: &[String]) -> i32 {
    let r = match args.first().map(String::as_str) {
        Some("run") => run_one(&args[1..]),
        Some("k") if args.len() == 1 => k_value().map(|(k, a, c)| {
            print_k(k, &a, &c);
            true
        }),
        Some("ratio") => ratio(&args[1..]),
        Some(a) if !a.starts_with("--") => {
            eprintln!(
                "usage: spike-b bench [--tau NS ...] | run <A|C|A0|J|JC> [--n N] | k | ratio ..."
            );
            return 2;
        }
        _ => coordinate(args),
    };
    match r {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            eprintln!("spike-b bench: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_rank_matches_the_protocol() {
        // Protocol §6: 15 rounds -> ranks 4 and 12; 31 rounds -> ranks 10 and 22.
        assert_eq!(median_rank(15), 4);
        assert_eq!(median_rank(31), 10);
        let v: Vec<f64> = (1..=15).map(f64::from).collect();
        let s = summarize(&v);
        assert_eq!((s.lo, s.median, s.hi), (4.0, 8.0, 12.0));
        let v: Vec<f64> = (1..=31).map(f64::from).collect();
        let s = summarize(&v);
        assert_eq!((s.lo, s.median, s.hi), (10.0, 16.0, 22.0));
    }

    #[test]
    fn verdicts_follow_protocol_section_6() {
        let at = |m: f64| -> Vec<f64> { (0..15).map(|i| m + f64::from(i) * 0.1).collect() };
        let quiet = summarize(&[100.0; 15]);
        assert_eq!(
            rounds_verdict(&summarize(&at(10.0)), &quiet, &quiet),
            "pass"
        );
        assert_eq!(
            rounds_verdict(&summarize(&at(18.5)), &quiet, &quiet),
            "fail"
        );
        // Interval [17.3, 18.1] straddles 18.
        assert_eq!(
            rounds_verdict(&summarize(&at(17.0)), &quiet, &quiet),
            "marginal"
        );
        let noisy: Vec<f64> = (0..15).map(|i| 100.0 + f64::from(i % 5) * 10.0).collect();
        let noisy = summarize(&noisy);
        assert!(noisy.iqr_pct > VOID_IQR_PCT);
        assert_eq!(
            rounds_verdict(&summarize(&at(10.0)), &noisy, &quiet),
            "void"
        );
        assert_eq!(
            rounds_verdict(&summarize(&at(10.0)), &quiet, &noisy),
            "void"
        );
    }

    /// k at this ABI: the loop ranges have no prefix and one exit, and the extra
    /// instructions of A are the call sequence (protocol §4 example at ABI 2: k = 5).
    #[test]
    fn k_counts_the_extra_call_sequence() {
        let (k, a, c) = k_value().expect("k");
        assert!(k > 0, "k = {k}");
        assert!(
            a.lines.iter().any(|l| l.contains("CallUndefinedReceiver1")),
            "{:?}",
            a.lines
        );
        assert!(!c.lines.iter().any(|l| l.contains("Call")), "{:?}", c.lines);
        assert!(a.lines.last().is_some_and(|l| l.contains("JumpLoop")));
        assert!(c.lines.last().is_some_and(|l| l.contains("JumpLoop")));
    }

    /// Every config returns n in this process (debug build, small n), with the slot kinds of
    /// the protocol table.
    #[test]
    fn every_config_returns_n() {
        for c in Cfg::ALL {
            let r = run_config(c, 1000).unwrap_or_else(|e| panic!("{}: {e}", c.name()));
            assert_eq!(r.result, 1000, "{}", c.name());
        }
    }

    #[test]
    fn a_prefix_in_the_loop_fails_the_run() {
        // 300 locals push the loop registers past r255: the loop range gets Wide prefixes.
        let mut defs = String::from("function drive(f, n) {\n");
        for i in 0..300 {
            defs.push_str(&format!("  let v{i} = {i};\n"));
        }
        defs.push_str(
            "  let a = v299;\n  for (let i = 0; i < n; i++) { a = a + 1; }\n  return a;\n}\n",
        );
        let e = loop_range(&defs).expect_err("prefix");
        assert!(e.contains("prefix"), "{e}");
    }

    #[test]
    fn a_wrong_drive_result_fails_the_run() {
        let ok = "spike-b-run config=A n=100 ns=5000 result=100\n";
        assert_eq!(parse_run(Cfg::A, 100, ok), Ok(5000));
        let bad = "spike-b-run config=A n=100 ns=5000 result=99\n";
        let e = parse_run(Cfg::A, 100, bad).expect_err("wrong result");
        assert!(e.contains("the run is invalid"), "{e}");
        assert!(parse_run(Cfg::C, 100, "no line").is_err());
    }

    /// Every protocol parameter that differs makes a label (bench.sh: provisional); the
    /// protocol's own parameters make none.
    #[test]
    fn parameters_other_than_the_protocols_are_labelled() {
        assert!(parameter_labels(false, ROUNDS, MARGINAL_ROUNDS, DEFAULT_N).is_empty());
        // More rounds than the protocol's are not fewer (protocol header: >= 15 rounds).
        assert!(parameter_labels(false, 20, 41, DEFAULT_N).is_empty());
        let one = |debug, rounds, marginal, n| {
            let l = parameter_labels(debug, rounds, marginal, n);
            assert_eq!(l.len(), 1, "{l:?}");
            l[0].clone()
        };
        assert!(one(true, ROUNDS, MARGINAL_ROUNDS, DEFAULT_N).contains("debug build"));
        assert!(one(false, 5, MARGINAL_ROUNDS, DEFAULT_N).contains("5 kept rounds (< 15)"));
        assert!(one(false, ROUNDS, MARGINAL_ROUNDS, 10_000_000).contains("n = 10000000"));
        assert!(one(false, ROUNDS, MARGINAL_ROUNDS, DEFAULT_N + 1).contains("protocol §2"));
        assert!(one(false, ROUNDS, 16, DEFAULT_N).contains("16 kept rounds for a marginal re-run"));
        assert_eq!(parameter_labels(true, 1, 1, 1000).len(), 4);
    }

    /// A marginal result re-runs only with more rounds than the first run; otherwise it gets a
    /// label and stays marginal (bench.sh: provisional, not "unknown bench verdict").
    #[test]
    fn a_marginal_result_that_cannot_re_run_is_labelled() {
        assert_eq!(no_rerun_label(ROUNDS, MARGINAL_ROUNDS), None);
        assert_eq!(no_rerun_label(1, 2), None);
        for (rounds, marginal) in [(15, 15), (15, 3), (40, 31)] {
            let l = no_rerun_label(rounds, marginal).expect("no re-run");
            assert!(l.starts_with("marginal result not re-run"), "{l}");
            assert!(l.contains(&format!("--marginal-rounds {marginal}")), "{l}");
        }
    }

    #[test]
    fn listing_lines_take_the_code_range_only() {
        let listing = "function f1 ...\n  code (9 bytes):\n      0    LdaZero ; 0c\n      2 S> Star r0 ; 18 f9\n      5    JumpLoop @2 ; 7d\n  constants:\n      0 \"x\"\n";
        let l = listing_lines(listing, 2, 5);
        assert_eq!(l.len(), 2, "{l:?}");
        assert!(l[0].contains("Star r0") && l[1].contains("JumpLoop"));
    }
}
