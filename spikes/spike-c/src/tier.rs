//! Config C of the protocol (§3), as a correctness run (SPIKEC-1; timing is SPIKEC-2 on): a
//! kernel's driver runs twice, once in T0 and once with its first hot function compiled from
//! the live facts of a warm-up run, and the two outputs must be equal line for line.
//!
//! ```text
//!  driver(k, 1) and driver(k, n) share the text up to "// ---- end of <file> ----":
//!    defs = stub harness + Math.random + <kernel>.js ; loop(n) = Setup, run() x n, check, TearDown
//!  T0 session:        defs ; loop(1) ; loop(n)                       -> output A
//!  compiled session:  defs ; loop(1) (feedback) ; facts::live ; compile the hot function ;
//!                     install_b (Opt slot) ; loop(n)                  -> output B
//!  right = A == B, both runs end with no uncaught error, and B is a right run of the driver
//!          (Driver::matches: the check line and "<benchmark> <n>" lines)
//! ```
//!
//! Both sessions use the threaded T0 dispatch (placement (b) code runs only under it). The
//! compiled code is installed with its exit table (H29): a failing guard deopts into T0 in
//! the same frame, and the VM counts the exits per record ([`fired`] names their guards).

use std::time::Instant;

use spike_b::harness::{Config, RunOutput};

use crate::harness::{Session, Target};
use crate::kernels::{self, Kernel};
use crate::{CompileReport, emit_a64, facts};

/// One config C run against T0.
#[derive(Clone, Debug)]
pub struct TierRun {
    pub kernel: &'static str,
    pub function: &'static str,
    pub n: u32,
    pub t0: RunOutput,
    pub compiled: RunOutput,
    pub report: CompileReport,
    /// The exits counted in loop(n), per exit record (record order, see [`fired`]).
    pub exits: Vec<u64>,
    /// Output equal to T0, no uncaught error, a right driver run.
    pub equal: bool,
    /// Wall time of loop(n), seconds (one run each; a report, not a class S reading).
    pub t0_secs: f64,
    pub compiled_secs: f64,
    /// After loop(n), the compiled function called with a receiver of a shape no guard
    /// admits, in both sessions: the first guard whose exit counter rose (proof that the
    /// compiled code is what runs and deopts), or `None` if the kernel has no probe. The
    /// probe's result equals T0's (else the run is an error).
    pub probe_exit: Option<u32>,
}

/// The guards of the exit records whose counter is not 0 (`exits` in record order).
pub fn fired(report: &CompileReport, exits: &[u64]) -> Vec<u32> {
    report
        .emit
        .meta
        .exits
        .iter()
        .zip(exits)
        .filter(|(_, n)| **n > 0)
        .map(|(r, _)| r.guard)
        .collect()
}

impl TierRun {
    /// One summary line.
    pub fn line(&self) -> String {
        let verdict = if self.equal {
            "output equal to T0".to_string()
        } else {
            format!(
                "output differs: compiled {:?} {:?} vs T0 {:?} {:?}",
                self.compiled.result, self.compiled.output, self.t0.result, self.t0.output
            )
        };
        let probe = match self.probe_exit {
            Some(g) => format!("; probe: a new receiver shape deopts at g{g}, result as T0"),
            None => String::new(),
        };
        let total: u64 = self.exits.iter().sum();
        let guards = fired(&self.report, &self.exits)
            .iter()
            .map(|g| format!("g{g}"))
            .collect::<Vec<_>>()
            .join(" ");
        let exits = if total == 0 {
            "; exits 0".to_string()
        } else {
            format!("; exits {total} (guards {guards})")
        };
        format!(
            "{} {} n = {}: {verdict}{exits}; T0 {:.3} s, compiled {:.3} s (one run, report \
             only){probe}",
            self.kernel, self.function, self.n, self.t0_secs, self.compiled_secs
        )
    }
}

/// The driver text split at the end of the kernel file: (defs, loop).
pub fn split(d: &kernels::Driver, file: &str) -> Result<(String, String), String> {
    let marker = format!("// ---- end of {file} ----\n");
    let at = d
        .text
        .find(&marker)
        .ok_or_else(|| format!("{file}: no end marker in the driver"))?;
    Ok((d.text[..at].to_string(), d.text[at..].to_string()))
}

/// Runs config C of `k` (its first hot function compiled) and T0, with `n` timed runs.
pub fn run(k: &'static Kernel, n: u32, config: &Config) -> Result<TierRun, String> {
    run_with(k, n, config, IntFacts::Live)
}

/// Where the int facts of a compile come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntFacts {
    /// The live snapshot as `export_facts` writes it (since FACTS2, M4-P2, with the
    /// BinaryOp and CompareOp records).
    Live,
    /// The live snapshot plus the test stand-in for FACTS2's int records
    /// ([`facts::with_implicit_int32`]).
    ImplicitInt32,
    /// The live snapshot without the function's BinaryOp and CompareOp records
    /// ([`facts::without_int`]): the tests' "no int fact" baseline.
    WithoutInt,
}

/// Compiles the hot function `hot` of the script `file` of `s` (the kernel file's line L is
/// line L + `offset` there) from the live facts of what `s` ran so far (with the stand-in int
/// records if `int` asks for them). Returns the install target (the function value if a JS
/// expression reaches it, else the function by its id: an inner closure, WI-SC5), the code
/// and the report; the caller installs the code.
pub fn compile_hot(
    s: &mut Session,
    hot: &kernels::Hot,
    file: &str,
    offset: u32,
    int: IntFacts,
) -> Result<(Target, spike_b::emit::FunctionCode, CompileReport), String> {
    compile_hot_with(s, hot, file, offset, int, emit_a64::StubForm::Call)
}

/// [`compile_hot`] with the exit stubs of `form`.
pub fn compile_hot_with(
    s: &mut Session,
    hot: &kernels::Hot,
    file: &str,
    offset: u32,
    int: IntFacts,
    form: emit_a64::StubForm,
) -> Result<(Target, spike_b::emit::FunctionCode, CompileReport), String> {
    let opts = emit_a64::EmitOptions {
        form,
        ..Default::default()
    };
    compile_hot_opts(s, hot, file, offset, int, &opts)
}

/// [`compile_hot`] with the emitter options `opts` (the test builds too).
pub fn compile_hot_opts(
    s: &mut Session,
    hot: &kernels::Hot,
    file: &str,
    offset: u32,
    int: IntFacts,
    opts: &emit_a64::EmitOptions<'_>,
) -> Result<(Target, spike_b::emit::FunctionCode, CompileReport), String> {
    // The function value first: `function_value` runs a one-line script (a global
    // assignment), which calls nothing, so the feedback the facts come from does not change.
    let value = match hot.reach {
        Some(reach) => Some(s.function_value(reach)?),
        None => None,
    };
    let script = s.script(file).ok_or_else(|| format!("no script {file}"))?;
    let ids = script.functions_on_line(hot.lines.0 + offset);
    let [id] = ids[..] else {
        return Err(format!(
            "{}: {} functions on line {}",
            hot.name,
            ids.len(),
            hot.lines.0
        ));
    };
    let live = compile_facts(s, script, id, int)?;
    let facts = facts::for_function(&live, id);
    let target = match value {
        Some(f) => Target::Value(f),
        None => Target::Id {
            file: file.to_string(),
            id,
        },
    };
    let input = s.input_of_target(&target)?;
    let (code, report) = crate::compile_input_opts(&input, &facts, opts)
        .map_err(|r| format!("{}: {r}", hot.name))?;
    Ok((target, code, report))
}

/// The facts of function `id` of `script` for a compile: the live snapshot, with the stand-in
/// int records if `int` asks for them.
fn compile_facts(
    s: &Session,
    script: &crate::harness::Script,
    id: u32,
    int: IntFacts,
) -> Result<ep9_facts::FactSnapshot, String> {
    let live = facts::live(s, script)?;
    match int {
        IntFacts::Live => Ok(live),
        IntFacts::ImplicitInt32 => facts::with_implicit_int32(&live, script.unit.unit(), id),
        IntFacts::WithoutInt => Ok(facts::without_int(&live, id)),
    }
}

/// [`run`] with the int facts of `int`.
pub fn run_with(
    k: &'static Kernel,
    n: u32,
    config: &Config,
    int: IntFacts,
) -> Result<TierRun, String> {
    run_opts(k, n, config, int, emit_a64::StubForm::Call)
}

/// [`run`] with the int facts of `int` and the exit stubs of `form`.
pub fn run_opts(
    k: &'static Kernel,
    n: u32,
    config: &Config,
    int: IntFacts,
    form: emit_a64::StubForm,
) -> Result<TierRun, String> {
    let opts = emit_a64::EmitOptions {
        form,
        ..Default::default()
    };
    run_full(k, n, config, int, &opts)
}

/// [`run_opts`] with the emitter options `opts`. With a force table (WI-SC4) the compiled
/// function's guards count down their words and exit when one ends, and the run also
/// collects garbage and runs the heap verifier after loop(n) (an error if it finds any).
pub fn run_full(
    k: &'static Kernel,
    n: u32,
    config: &Config,
    int: IntFacts,
    opts: &emit_a64::EmitOptions<'_>,
) -> Result<TierRun, String> {
    let hot = k.hot.first().ok_or("no hot function")?;
    let d1 = kernels::driver(k, 1)?;
    let dn = kernels::driver(k, n)?;
    let (defs, loop1) = split(&d1, k.file)?;
    let (defs_n, loop_n) = split(&dn, k.file)?;
    // Only the first line (a comment that names n) may differ.
    let body = |t: &str| t.split_once('\n').map(|x| x.1.to_string());
    if body(&defs) != body(&defs_n) || d1.offset != dn.offset {
        return Err("the drivers differ before the loop".into());
    }
    let config = config.threaded();
    // The layout export, proved on a session of their own (so both runs see the same scripts).
    crate::vm_layout::check(&mut Session::new(&config)?).map_err(|e| format!("vm layout: {e}"))?;

    let mut t0s = Session::new(&config)?;
    t0s.run(&defs, "defs.js")
        .result
        .map_err(|e| format!("T0 defs: {e}"))?;
    let w = t0s.run(&loop1, "warm.js");
    w.result.map_err(|e| format!("T0 warm-up: {e}"))?;
    let t = Instant::now();
    let t0 = t0s.run(&loop_n, "loop.js");
    let t0_secs = t.elapsed().as_secs_f64();

    let mut s = Session::new(&config)?;
    s.run(&defs, "defs.js")
        .result
        .map_err(|e| format!("defs: {e}"))?;
    let w2 = s.run(&loop1, "warm.js");
    w2.result.map_err(|e| format!("warm-up: {e}"))?;
    if w2.output != w.output {
        return Err("the two warm-up runs differ (both are T0)".into());
    }
    let (f, code, report) = compile_hot_opts(&mut s, hot, "defs.js", d1.offset, int, opts)?;
    let meta = report.emit.meta.encode();
    // SAFETY: `code` is the placement (b) code `emit_a64` made for `f` from `input_of(f)` just
    // above (no JS ran since), unchanged, and `meta` its exit table. emit_a64 keeps the (b)
    // contract: the T0 handler ABI, Cx and the heap read through `ep9_vm::layout` offsets
    // (`vm_layout`, proved at the start of this run), only RuntimeTable calls (the deopt
    // entry among them), x19-x28 saved and restored, and every exit is the tail jump to
    // `cx.table[*pc]`.
    unsafe { s.install_targets(std::slice::from_ref(&f), vec![code], vec![Some(meta)]) }?;
    if s.slot_kind_of(&f) != Some(ep9_vm::DispatchKind::Opt) {
        return Err(format!(
            "{}: slot kind {:?} after install",
            hot.name,
            s.slot_kind_of(&f)
        ));
    }
    let t = Instant::now();
    let compiled = s.run(&loop_n, "loop.js");
    let compiled_secs = t.elapsed().as_secs_f64();
    let counts = |s: &Session| {
        s.exit_counts_of(&f)
            .ok_or("no exit counters after the install")
    };
    let exits = counts(&s)?;
    if opts.force.is_some() {
        s.verify()?;
    }
    let equal = compiled == t0 && t0.result.is_ok() && dn.matches(&compiled.output);
    // The probe: `this` of a shape the IC never saw. The compiled function's first shape
    // guard deopts, and T0 finishes the call: the result must be T0's own.
    let probe_exit = match PROBES.iter().find(|(f, _)| *f == hot.name) {
        Some((_, js)) => {
            let want = t0s.run(js, "probe.js");
            let got = s.run(js, "probe.js");
            if got != want || want.result.is_err() {
                return Err(format!("{}: the probe gave {got:?}, T0 {want:?}", hot.name));
            }
            // An output that prints nothing compares nothing (M5-P1 review).
            if want.output.is_empty() {
                return Err(format!("{}: the probe printed nothing", hot.name));
            }
            let after = counts(&s)?;
            let rose: Vec<u64> = after.iter().zip(&exits).map(|(a, b)| a - b).collect();
            match fired(&report, &rose).first() {
                Some(&g) => Some(g),
                None => return Err(format!("{}: the probe did not deopt", hot.name)),
            }
        }
        None => None,
    };
    Ok(TierRun {
        kernel: k.name,
        function: hot.name,
        n,
        t0,
        compiled,
        report,
        exits,
        equal,
        t0_secs,
        compiled_secs,
        probe_exit,
    })
}

/// Per hot function: a call whose receiver no shape guard admits (a literal object, a shape
/// the kernel never makes). It prints what the call computes, so the compare with T0 checks
/// the deopt's values: am3's carry and the destination digits (2 digits, with 28-bit inputs
/// so the carry is not 0).
const PROBES: [(&str, &str); 1] = [(
    "am3",
    "var __spikeCProbe = { array: [268435455, 123456789], other: 3, am: am3 }; \
     var __spikeCW = { array: [268435455, 7] }; \
     var __spikeCC = __spikeCProbe.am(0, 268435455, __spikeCW, 0, 3, 2); \
     print(__spikeCC + \" \" + __spikeCW.array.join(\",\"));",
)];

/// The length of the force tables of the deopt gate (guard ids are below the node count).
pub const FORCE_LEN: usize = 4096;

/// One forced run of [`force_each_guard`].
#[derive(Clone, Debug)]
pub struct ForcedRun {
    pub guard: u32,
    /// The guard's word reached 0: its `at`-th pass took the exit.
    pub forced: bool,
    /// The exits the VM counted for the records of this guard, and for all other records.
    pub counted: u64,
    pub others: u64,
    /// Output equal to T0 (and the verifier found nothing).
    pub equal: bool,
}

impl ForcedRun {
    /// Right: equal to T0, the guard was forced, the VM counted that one exit and no other.
    pub fn right(&self) -> bool {
        self.equal && self.forced && self.counted == 1 && self.others == 0
    }
}

/// `gate.sh deopt` (WI-SC4): for each guard of `k`'s first hot function, a config C run of
/// loop(n) with only that guard forced, at its `at`-th pass: the output must equal T0, the
/// heap verifier must find nothing, and the VM's exit counters must hold exactly that exit.
pub fn force_each_guard(
    k: &'static Kernel,
    n: u32,
    config: &Config,
    at: u32,
) -> Result<Vec<ForcedRun>, String> {
    let plain = run(k, n, config)?;
    let mut guards: Vec<u32> = plain
        .report
        .emit
        .meta
        .exits
        .iter()
        .map(|e| e.guard)
        .collect();
    guards.sort_unstable();
    guards.dedup();
    let mut out = Vec::new();
    for g in guards {
        let t = emit_a64::ForceTable::new(FORCE_LEN);
        t.arm(g, at);
        let opts = emit_a64::EmitOptions {
            force: Some(&t),
            ..Default::default()
        };
        let r = run_full(k, n, config, IntFacts::Live, &opts)?;
        let (mut counted, mut others) = (0, 0);
        for (rec, c) in r.report.emit.meta.exits.iter().zip(&r.exits) {
            if rec.guard == g {
                counted += c;
            } else {
                others += c;
            }
        }
        out.push(ForcedRun {
            guard: g,
            forced: t.get(g) == 0,
            counted,
            others,
            equal: r.equal,
        });
    }
    Ok(out)
}

/// A script-level config C run: `defs` and `warm` run in T0 (the feedback), then the global
/// function `name` is compiled from the live facts and installed, then `main` runs. T0 runs
/// the same three scripts in a session of its own.
#[derive(Clone, Debug)]
pub struct ScriptRun {
    pub t0: RunOutput,
    pub compiled: RunOutput,
    pub report: CompileReport,
    /// The exits counted in `main`, per exit record (record order).
    pub exits: Vec<u64>,
}

impl ScriptRun {
    /// True if `main` printed the same and ended the same in both runs.
    pub fn equal(&self) -> bool {
        self.t0 == self.compiled
    }

    /// The guards whose exit ran in `main` (record order).
    pub fn fired(&self) -> Vec<u32> {
        fired(&self.report, &self.exits)
    }
}

/// Runs `defs`, `warm` and `main` in T0 and in config C with `name` compiled (see
/// [`ScriptRun`]). Errors if `name` does not compile or T0's `defs`/`warm` fail.
pub fn run_script(
    config: &Config,
    defs: &str,
    warm: &str,
    main: &str,
    name: &str,
) -> Result<ScriptRun, String> {
    run_script_with(config, defs, warm, main, name, IntFacts::Live)
}

/// [`run_script`] with the int facts of `int`.
pub fn run_script_with(
    config: &Config,
    defs: &str,
    warm: &str,
    main: &str,
    name: &str,
    int: IntFacts,
) -> Result<ScriptRun, String> {
    run_script_opts(
        config,
        defs,
        warm,
        main,
        name,
        int,
        emit_a64::StubForm::Call,
    )
}

/// [`run_script`] with the int facts of `int` and the exit stubs of `form`.
pub fn run_script_opts(
    config: &Config,
    defs: &str,
    warm: &str,
    main: &str,
    name: &str,
    int: IntFacts,
    form: emit_a64::StubForm,
) -> Result<ScriptRun, String> {
    let opts = emit_a64::EmitOptions {
        form,
        ..Default::default()
    };
    run_script_full(config, defs, warm, main, name, int, &opts)
}

/// [`run_script`] with the int facts of `int` and the emitter options `opts` (the test builds
/// too: forced deopts, int32 junk).
pub fn run_script_full(
    config: &Config,
    defs: &str,
    warm: &str,
    main: &str,
    name: &str,
    int: IntFacts,
    opts: &emit_a64::EmitOptions<'_>,
) -> Result<ScriptRun, String> {
    let config = config.threaded();
    let mut t0s = Session::new(&config)?;
    t0s.run(defs, "defs.js")
        .result
        .map_err(|e| format!("T0 defs: {e}"))?;
    t0s.run(warm, "warm.js")
        .result
        .map_err(|e| format!("T0 warm: {e}"))?;
    let t0 = t0s.run(main, "main.js");
    let mut s = Session::new(&config)?;
    s.run(defs, "defs.js")
        .result
        .map_err(|e| format!("defs: {e}"))?;
    s.run(warm, "warm.js")
        .result
        .map_err(|e| format!("warm: {e}"))?;
    // A global function by its value; any other function of `defs` (an inner closure) by its
    // function id (WI-SC5).
    let f = match ep9_vm::NativeCx::global_value(s.iso(), name) {
        Some(_) => Some(s.function_value(name)?),
        None => None,
    };
    let (f, code, report) = {
        let script = s.script("defs.js").ok_or("no defs script")?;
        let ids = script.functions_named(name);
        let [id] = ids[..] else {
            return Err(format!("{name}: {} functions of that name", ids.len()));
        };
        let live = compile_facts(&s, script, id, int)?;
        let facts = facts::for_function(&live, id);
        let f = match f {
            Some(f) => Target::Value(f),
            None => Target::Id {
                file: "defs.js".into(),
                id,
            },
        };
        let input = s.input_of_target(&f)?;
        let (code, report) =
            crate::compile_input_opts(&input, &facts, opts).map_err(|r| format!("{name}: {r}"))?;
        (f, code, report)
    };
    let meta = report.emit.meta.encode();
    // SAFETY: as in `run`: emit_a64's placement (b) code for `f`, from `input_of_target(f)`
    // with no JS run since, unchanged, and its exit table.
    unsafe { s.install_targets(std::slice::from_ref(&f), vec![code], vec![Some(meta)]) }?;
    let compiled = s.run(main, "main.js");
    let exits = s
        .exit_counts_of(&f)
        .ok_or("no exit counters after the install")?;
    Ok(ScriptRun {
        t0,
        compiled,
        report,
        exits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A function compiled with a Mac-only emitter: skip elsewhere.
    fn aarch64() -> bool {
        if spike_b::emit::Arch::HOST == Some(spike_b::emit::Arch::Aarch64) {
            return true;
        }
        eprintln!("skip: the emitter is aarch64 only");
        false
    }

    fn equal(defs: &str, warm: &str, main: &str, name: &str) -> ScriptRun {
        let r = run_script(&Config::default(), defs, warm, main, name).expect("run");
        assert!(
            r.equal(),
            "{name}: compiled {:?} vs T0 {:?}\n{}",
            r.compiled,
            r.t0,
            r.report.graph
        );
        assert!(r.fired().is_empty(), "{name}: exits {:?}", r.exits);
        r
    }

    /// `main` in config C equals T0 and at least one guard deopted on the way (the deopt
    /// entry rebuilt the frame, and T0 finished the work).
    fn deopts(defs: &str, warm: &str, main: &str, name: &str) -> ScriptRun {
        let r = run_script(&Config::default(), defs, warm, main, name).expect("run");
        assert!(
            r.equal(),
            "{name}: compiled {:?} vs T0 {:?}\n{}",
            r.compiled,
            r.t0,
            r.report.graph
        );
        assert!(
            !r.fired().is_empty(),
            "{name}: no exit: compiled {:?} vs T0 {:?}",
            r.compiled,
            r.t0
        );
        r
    }

    const SUM: &str =
        "function sum(n) { var a = 0; for (var i = 0; i < n; i++) { a = (a + i) | 0; } return a; }";

    #[test]
    fn int_loop_equals_t0() {
        if !aarch64() {
            return;
        }
        let warm = "for (var k = 0; k < 20; k++) sum(10);";
        let r = equal(
            SUM,
            warm,
            "print(sum(0), sum(1), sum(1000), sum(65536), sum(-5));",
            "sum",
        );
        assert_eq!(r.compiled.output, ["0 0 499500 2147450880 0"]);
        // Since FACTS2 (M4-P2) the live facts hold the BinaryOp and CompareOp records, so
        // the input check of `i < n` is a fact guard; the overflow guards of `a + i` and
        // `i++` stay unbacked (a `+` that overflows stays implicit int32 in the profile).
        assert_eq!(
            (r.report.fact_guards, r.report.unbacked_guards),
            (1, 2),
            "{}",
            r.report.graph
        );
        // `a + i` past 2^31 is a double in JS before `| 0`; the overflow guard deopts and T0
        // finishes the loop (704982704).
        let r = deopts(SUM, warm, "print(sum(100000));", "sum");
        assert_eq!(r.t0.output, ["704982704"]);
        // A double that is an int32 passes the entry check; 2.5 does not (unbacked: a deopt).
        equal(
            SUM,
            "sum(3);",
            "print(sum(4.0), sum(Math.floor(7.9)));",
            "sum",
        );
        deopts(SUM, "sum(3);", "print(sum(2.5));", "sum");
        deopts(SUM, "sum(3);", "print(sum('3'));", "sum");
    }

    /// With int records (the FACTS2 stand-in, and the live records since FACTS2) every int32
    /// input check of `sum` is a fact guard and only the overflow guards of `+` stay unbacked;
    /// the guard basis is data: the code is the same bytes as with no int fact, and a failing
    /// guard deopts into T0 (month 5, H29).
    #[test]
    fn int_facts_back_the_int32_guards() {
        if !aarch64() {
            return;
        }
        let warm = "for (var k = 0; k < 20; k++) sum(10);";
        let main = "print(sum(0), sum(1), sum(1000), sum(65536), sum(-5));";
        // The baseline with no int fact (since FACTS2 the live facts hold them).
        let plain = run_script_with(
            &Config::default(),
            SUM,
            warm,
            main,
            "sum",
            IntFacts::WithoutInt,
        )
        .expect("run");
        assert!(
            plain.equal() && plain.fired().is_empty(),
            "{:?} vs {:?}",
            plain.compiled,
            plain.t0
        );
        assert_eq!(plain.report.fact_guards, 0, "{}", plain.report.graph);
        // The overflow guards of `+` (`a + i`, `i++`): never backed (see `crate::build`).
        let overflow = |g: &str| {
            g.lines()
                .filter(|l| l.contains("Int32(Add)") && l.contains(" guard g"))
                .count()
        };
        assert_eq!(overflow(&plain.report.graph), 2, "{}", plain.report.graph);
        let backed = (plain.report.unbacked_guards - 2, 2);
        // The live records (FACTS2) back the same guards as the stand-in.
        let live = equal(SUM, warm, main, "sum");
        assert_eq!(
            (live.report.fact_guards, live.report.unbacked_guards),
            backed,
            "{}",
            live.report.graph
        );
        let r = run_script_with(
            &Config::default(),
            SUM,
            warm,
            main,
            "sum",
            IntFacts::ImplicitInt32,
        )
        .expect("run");
        assert!(
            r.equal() && r.fired().is_empty(),
            "{:?} vs {:?}",
            r.compiled,
            r.t0
        );
        assert_eq!(
            (r.report.fact_guards, r.report.unbacked_guards),
            backed,
            "{}",
            r.report.graph
        );
        assert_eq!(r.report.emit.code_bytes, plain.report.emit.code_bytes);
        let t = run_script_with(
            &Config::default(),
            SUM,
            warm,
            "print(sum(100000));",
            "sum",
            IntFacts::ImplicitInt32,
        )
        .expect("run");
        assert!(t.equal() && !t.fired().is_empty(), "{:?}", t.compiled);
    }

    /// Only a hint inside {int32} (the empty set included, MODES-D10) backs a guard: a hint
    /// with an overflow, number or other bit, a record of another kind and no record leave it
    /// unbacked ([`crate::build::int_basis_of`]).
    #[test]
    fn int_basis_reads_only_int32_hints() {
        use crate::build::int_basis_of;
        use crate::ir::GuardBasis;
        use ep9_facts::{BinaryHint, CompareHint, FactSource, SlotFacts};
        let recs = [
            SlotFacts::BinaryOp(BinaryHint::INT32),
            SlotFacts::BinaryOp(BinaryHint::EMPTY),
            SlotFacts::BinaryOp(BinaryHint::INT32.union(BinaryHint::INT32_OVERFLOW)),
            SlotFacts::BinaryOp(BinaryHint::NUMBER),
            SlotFacts::CompareOp(CompareHint::INT32),
            SlotFacts::CompareOp(CompareHint::INT32.union(CompareHint::NUMBER)),
            SlotFacts::TypeOf(ep9_facts::TypeSet::EMPTY),
        ];
        let f = facts::FunctionFacts {
            source: FactSource::Profiled,
            function: 1,
            records: recs
                .iter()
                .enumerate()
                .map(|(i, r)| (i as u32, r))
                .collect(),
        };
        let fact = GuardBasis::Fact(FactSource::Profiled);
        let got: Vec<GuardBasis> = (0..8).map(|s| int_basis_of(&f, s)).collect();
        let u = GuardBasis::Unbacked;
        assert_eq!(got, [fact, fact, u, u, fact, u, u, u]);
    }

    /// Review F2 (M4-P2): an int32 `+` that overflows in the very warm-up the facts come from
    /// gets no fact for its overflow guard. Since M5-P1 PROFILE5 (MODES-D16) T0 records that
    /// overflow as `INT32_OVERFLOW`, so the site's hint is not inside {int32}: its input check
    /// is unbacked too, as at a `*` site that overflowed. The other sites keep the fact basis.
    #[test]
    fn an_add_overflow_guard_is_never_fact_backed() {
        if !aarch64() {
            return;
        }
        let defs = "function wrap(a, n) { var s = 0; for (var i = 0; i < n; i++) { s = (s + a) | 0; } return s; }";
        let r = run_script(
            &Config::default(),
            defs,
            "for (var k = 0; k < 20; k++) wrap(1073741824, 10);",
            "print(wrap(1073741824, 10));",
            "wrap",
        )
        .expect("run");
        assert_eq!(r.t0.output, ["-2147483648"]);
        assert!(
            r.equal() && !r.fired().is_empty(),
            "the overflow guard deopts: {:?}",
            r.compiled
        );
        let g = &r.report.graph;
        for l in g.lines().filter(|l| l.contains(" guard g")) {
            // @15 is the `s + a` site; every `+` overflow guard is unbacked.
            if l.contains("Int32(Add)") || l.ends_with("-> @15") {
                assert!(l.contains("Unbacked"), "{l}\n{g}");
            } else {
                assert!(l.contains("Fact(Runtime)"), "{l}\n{g}");
            }
        }
        assert!(g.contains("Int32(Add)"));
    }

    /// Review F3 (M4-P2): a loop phi's entry check is backed only by a check of the phi in the
    /// loop header. Here the only check of `i` runs in a branch on later iterations (`i` is
    /// then `k - 1`); the entry value 1.5 never reached a profiled int site, so the entry
    /// check is unbacked (before the fix it was a fact guard that failed on every call).
    #[test]
    fn a_phi_entry_check_is_backed_only_from_the_header() {
        if !aarch64() {
            return;
        }
        let defs = "function g(x, n) { var i = x; var s = 0; for (var k = 0; k < n; k = k + 1) { if (k > 0) { s = i + 1; } i = k; } return i + s; }";
        let r = run_script(
            &Config::default(),
            defs,
            "for (var j = 0; j < 20; j++) g(1.5, 5);",
            "print(g(1.5, 5));",
            "g",
        )
        .expect("run");
        assert_eq!(r.t0.output, ["8"]);
        let g = &r.report.graph;
        let entry: Vec<&str> = g
            .lines()
            .filter(|l| l.contains("CheckInt32(n1)") && l.contains(" guard g"))
            .collect();
        assert_eq!(entry.len(), 1, "{g}");
        assert!(entry[0].contains("Unbacked"), "{g}");
        // The header check of `k < n` still backs the entry check of `k` (a constant 0, so
        // no check) and the check of `n`.
        assert!(
            g.lines()
                .any(|l| l.contains("CheckInt32(n2)") && l.contains("Fact(Runtime)")),
            "{g}"
        );
    }

    /// R-SPIKE-P4-1 (M5-P1), the reviewer's probe `lp`: the loop is entered only when
    /// `c > 0`, so its preheader may branch past it. The entry check of the phi `s` (whose
    /// entry value `a` is a string or a double in the last calls) may not sit there: with
    /// `c = 0` it would fail on a path that never enters the loop, and a deopt would resume T0
    /// inside it. The phi stays Tagged; every call equals T0.
    #[test]
    fn lp_a_preheader_that_branches_past_the_loop_has_no_entry_check() {
        if !aarch64() {
            return;
        }
        let lp = "function lp(a, n, c) { var s = a; var i = 0; if (c > 0) { while (i < n) { s = (s + i) | 0; i = (i + 1) | 0; } } return s; }";
        let r = equal(
            lp,
            "for (var q = 0; q < 20; q++) lp(q, 4, 1);",
            "print(lp(8, 4, 0)); print(lp('str', 4, 0)); print(lp(1.5, 4, 0)); print(lp(2, 5, 1));",
            "lp",
        );
        assert_eq!(r.t0.output, ["8", "str", "1.5", "12"]);
        let g = &r.report.graph;
        assert!(
            !g.lines()
                .any(|l| l.contains("CheckInt32(n1)") && l.contains(" guard g")),
            "an entry check of `s`:\n{g}"
        );
    }

    const ARR: &str = "function fill(a, n) { for (var i = 0; i < n; i++) { a[i] = (a[i] * 3 + i) & 1023; } return a[0]; }";

    #[test]
    fn arrays_equal_t0_and_deopt_out_of_bounds() {
        if !aarch64() {
            return;
        }
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        equal(
            ARR,
            warm,
            "var b = [5, 6, 7, 8, 9]; print(fill(b, 5), b.join());",
            "fill",
        );
        // A read past the length or of a hole: the generic path is not built (a deopt; T0
        // reads `undefined`).
        deopts(ARR, warm, "print(fill([1, 2], 3));", "fill");
        deopts(ARR, warm, "var h = [1, , 3]; print(fill(h, 3));", "fill");
        deopts(ARR, warm, "print(fill({0: 1, length: 1}, 1));", "fill");
    }

    /// The ids of the guards on the graph lines that contain `op`.
    fn guard_ids(graph: &str, op: &str) -> Vec<u32> {
        graph
            .lines()
            .filter(|l| l.contains(op))
            .filter_map(|l| l.split(" guard g").nth(1))
            .filter_map(|t| t.split(' ').next()?.parse().ok())
            .collect()
    }

    /// Exit stubs (SPIKEC-3): one stub per guard (12 bytes in the movz form while the stub
    /// offset and the id fit in 16 bits) and one shared exit; each failing guard counts on
    /// its own exit record, so the stub passed the right one (SPIKEC-5: the deopt entry).
    #[test]
    fn each_guard_exits_through_its_own_stub() {
        if !aarch64() {
            return;
        }
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        let movz = |main: &str| {
            let r = run_script_opts(
                &Config::default(),
                ARR,
                warm,
                main,
                "fill",
                IntFacts::Live,
                emit_a64::StubForm::Movz,
            )
            .expect("run");
            assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
            r
        };
        let oob = movz("print(fill([1, 2], 3));");
        let g = &oob.report.graph;
        let loads = guard_ids(g, "LoadElement");
        let arrays = guard_ids(g, "CheckArray");
        assert!(!loads.is_empty() && !arrays.is_empty(), "{g}");
        assert!(
            matches!(oob.fired()[..], [x] if loads.contains(&x)),
            "{:?} {g}",
            oob.fired()
        );
        let hole = movz("var h = [1, , 3]; print(fill(h, 3));");
        assert!(
            matches!(hole.fired()[..], [x] if loads.contains(&x)),
            "{:?} {g}",
            hole.fired()
        );
        let obj = movz("print(fill({0: 1, length: 1}, 1));");
        assert!(
            matches!(obj.fired()[..], [x] if arrays.contains(&x)),
            "{:?} {g}",
            obj.fired()
        );
        let e = &oob.report.emit;
        assert_eq!(e.exits.len(), e.guards);
        assert_eq!(e.stub_bytes as usize, 12 * e.guards, "{}", e.exits_line());
        assert!(
            e.shared_exit_bytes > 0 && e.shared_exit_bytes <= SHARED_EXIT_MAX,
            "{}",
            e.exits_line()
        );
        // The stubs sit one after another in guard order.
        for w in e.exits.windows(2) {
            assert_eq!(w[1].1 - w[0].1, 12, "{:?}", e.exits);
        }
    }

    /// The most bytes the shared exit may take (it saves x0-x30 for the deopt entry).
    const SHARED_EXIT_MAX: u32 = 160;

    /// The call form of the exit stubs (MC-w15-14, the default since SPIKEC-5): 4 bytes each
    /// (`bl` to the shared exit, the stub found from the return pc); every failing guard
    /// deopts through the same exit record as with the 12-byte movz form, and every run
    /// gives T0's output.
    #[test]
    fn call_form_stubs_exit_through_the_same_guards() {
        if !aarch64() {
            return;
        }
        use emit_a64::StubForm;
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        let run = |main: &str, form| {
            run_script_opts(
                &Config::default(),
                ARR,
                warm,
                main,
                "fill",
                IntFacts::Live,
                form,
            )
            .expect("run")
        };
        let ok = run(
            "var b = [5, 6, 7, 8, 9]; print(fill(b, 5), b.join());",
            StubForm::Call,
        );
        assert!(
            ok.equal() && ok.fired().is_empty(),
            "{:?} vs {:?}",
            ok.compiled,
            ok.t0
        );
        for main in [
            "print(fill([1, 2], 3));",
            "var h = [1, , 3]; print(fill(h, 3));",
            "print(fill({0: 1, length: 1}, 1));",
        ] {
            let movz = run(main, StubForm::Movz);
            let call = run(main, StubForm::Call);
            assert!(movz.equal() && call.equal(), "{main}");
            assert!(!movz.fired().is_empty(), "{main}");
            assert_eq!(call.fired(), movz.fired(), "{main}");
            assert_eq!(call.exits, movz.exits, "{main}");
        }
        let e = &ok.report.emit;
        assert_eq!(e.form, StubForm::Call);
        assert_eq!(e.stub_bytes as usize, 4 * e.guards, "{}", e.exits_line());
        assert!(
            e.shared_exit_bytes > 0 && e.shared_exit_bytes <= SHARED_EXIT_MAX,
            "{}",
            e.exits_line()
        );
        for w in e.exits.windows(2) {
            assert_eq!(w[1].1 - w[0].1, 4, "{:?}", e.exits);
        }
        // The same records either way, except the stub pcs.
        let m = run("print(fill([1, 2], 2));", StubForm::Movz)
            .report
            .emit
            .meta;
        assert_eq!(m.exits.len(), e.meta.exits.len());
        for (a, b) in m.exits.iter().zip(&e.meta.exits) {
            assert_eq!(
                (a.guard, a.bytecode, &a.values),
                (b.guard, b.bytecode, &b.values)
            );
        }
    }

    /// The per-exit metadata (SPIKEC-4): one record per guard, in stub order, at its stub's
    /// pc, with the guard's resume offset and one value per live register and the
    /// accumulator; the VM's reader takes the encoded table.
    #[test]
    fn metadata_matches_the_frame_states() {
        if !aarch64() {
            return;
        }
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        let r = equal(ARR, warm, "print(fill([3, 4], 2));", "fill");
        let e = &r.report.emit;
        let m = &e.meta;
        assert_eq!(m.exits.len(), e.guards);
        for (k, (x, &(id, pc))) in m.exits.iter().zip(&e.exits).enumerate() {
            assert_eq!((x.guard, x.pc, x.counter as usize), (id, pc, k));
            assert!((x.pc as usize) < e.code_bytes);
        }
        // Each record against its guard's frame state in the graph text: "guard g<id> ... ->
        // @<offset>".
        for x in &m.exits {
            let line = r
                .report
                .graph
                .lines()
                .find(|l| l.contains(&format!(" guard g{} ", x.guard)))
                .expect("guard line");
            assert!(line.ends_with(&format!("-> @{}", x.bytecode)), "{line}");
        }
        assert!(m.exits.iter().any(|x| !x.values.is_empty()), "{m:?}");
        let bytes = m.encode();
        assert_eq!(
            ep9_vm::NativeCx::check_deopt_meta(&bytes),
            Ok(m.exits.len())
        );
    }

    /// Position independence (C5b, the part that needs no runtime): the same function compiled
    /// from the same facts in two isolates (other FunctionInfo and heap addresses) gives
    /// byte-equal code and byte-equal exit metadata, so neither holds an address.
    #[test]
    fn metadata_is_the_same_in_two_isolates() {
        if !aarch64() {
            return;
        }
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        let compile = |pad: u32| {
            let mut s = Session::new(&Config::default().threaded()).expect("session");
            // Different allocations before the function, so its heap and FunctionInfo
            // addresses differ between the two.
            s.run(
                &format!("var pad = []; for (var i = 0; i < {pad}; i++) pad.push({{ i: i }});"),
                "pad.js",
            )
            .result
            .expect("pad");
            s.run(ARR, "defs.js").result.expect("defs");
            s.run(warm, "warm.js").result.expect("warm");
            let f = s.function_value("fill").expect("fill");
            let script = s.script("defs.js").expect("defs");
            let id = script.functions_named("fill")[0];
            let live = facts::live(&s, script).expect("facts");
            let input = s.input_of(f).expect("input");
            let key = input.key;
            let (code, report) =
                crate::compile_input(&input, &facts::for_function(&live, id)).expect("compile");
            (key, code.code, report.emit.meta.encode(), s)
        };
        // Both sessions stay alive, so the two FunctionInfos cannot share an address.
        let (k1, c1, m1, _s1) = compile(0);
        let (k2, c2, m2, _s2) = compile(5000);
        assert_ne!(k1, k2, "the two FunctionInfos are at different addresses");
        assert_eq!(c1, c2, "code bytes differ between isolates");
        assert_eq!(m1, m2, "metadata bytes differ between isolates");
    }

    /// The double path (SPIKEC-4): a `+` or `*` whose record saw a double takes CheckNumber
    /// and Float64 ops; the result boxes as `Value::number` does (an int32 Value for an
    /// integer, -0 and NaN kept apart), equal to T0; a string operand fails the number check
    /// (a deopt, never a wrong value). Most arrays of this test end in a `null` that `n` never
    /// reaches (an ELEMENTS kind: Values); since WI-SC7 arrays of doubles alone (a DOUBLE kind:
    /// raw f64 in an ELEMENTS_DOUBLE cell) run compiled too, with no exit.
    const DOT: &str = "function dot(a, b, n) { var s = 0; for (var i = 0; i < n; i++) { s = s + a[i] * b[i]; } return s; }";

    #[test]
    fn doubles_equal_t0() {
        if !aarch64() {
            return;
        }
        let warm = "for (var q = 0; q < 20; q++) dot([0.5, 1.5, 2, null], [2, 0.25, 3, null], 3);";
        let r = equal(
            DOT,
            warm,
            "print(dot([0.5, 2, null], [4, 0.25, null], 2), dot([1, 2, 3, null], [4, 5, 6, null], 3), dot([0.1, 0.2, null], [1, 1, null], 2));\n\
             print(dot([NaN, null], [1, null], 1), 1 / dot([-0, null], [1, null], 1), dot([1e300, null], [1e300, null], 1), dot([-1e300, null], [1e300, null], 1));\n\
             print(dot([3, null], [1, null], 0), dot([2147483647, 1, null], [1, 1, null], 2), dot([-2147483648, null], [1, null], 1), dot([0.5, 0.5, null], [1, 1, null], 2) === 1);",
            "dot",
        );
        assert_eq!(
            r.compiled.output,
            [
                "2.5 32 0.30000000000000004",
                "NaN Infinity Infinity -Infinity",
                "0 2147483648 -2147483648 true"
            ]
        );
        let g = &r.report.graph;
        assert!(
            g.contains("Float64(Mul)") && g.contains("Float64(Add)"),
            "{g}"
        );
        for l in g.lines().filter(|l| l.contains("CheckNumber")) {
            assert!(l.contains("Fact(Runtime)"), "{l}\n{g}");
        }
        deopts(DOT, warm, "print(dot(['x', null], [1, null], 1));", "dot");
        // Arrays of doubles alone (ELEMENTS_DOUBLE cells): raw f64 loads, no exit; a NaN, a -0
        // and an int32 array mixed in.
        let r = equal(
            DOT,
            warm,
            "print(dot([0.5, 1.5], [2, 0.25], 2), dot([0.5, NaN], [2, 1], 2), 1 / dot([-0.5, 1.5], [0, -0], 2), dot([1.5, 2.5], [2, 4], 2));",
            "dot",
        );
        assert_eq!(r.compiled.output, ["1.375 NaN Infinity 13"]);
        // GC stress: the boxed doubles and the arrays around the compiled loop.
        let r = run_script(
            &Config::stress(),
            DOT,
            warm,
            "var xs = []; for (var k = 0; k < 300; k++) xs.push(k / 7); xs.push(null); print(dot(xs, xs, 300));",
            "dot",
        )
        .expect("run");
        assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
    }

    /// WI-SC7: `scale` stores a Float64 into each element and `put` an Int32. On DOUBLE
    /// arrays the stores write raw f64 (NaN as the pure NaN, -0 kept); on an INT32 array a
    /// double that is no int32 takes the generic store out of line, which makes the INT32 to
    /// DOUBLE transition as T0 does, and the loop goes on in the compiled code; an integral
    /// double stays an int32 Value. No exit in any of it; equal to T0, with GC stress too
    /// (the transition allocates the ELEMENTS_DOUBLE cell).
    const SCALE: &str = "function scale(a, n, f) { for (var i = 0; i < n; i++) { a[i] = a[i] * f; } return a; }\n\
        function put(a, n, v) { for (var i = 0; i < n; i++) { a[i] = (v + i) | 0; } return a; }";

    #[test]
    fn double_stores_and_the_kind_transition_equal_t0() {
        if !aarch64() {
            return;
        }
        let warm = "for (var q = 0; q < 20; q++) { scale([0.5, 1.5, 2.5], 3, 1.5); put([0.5, 1.5], 2, q); }";
        let main = "var d = scale([0.5, 1.5, 2.5, 4], 4, 0.5); print(d.join(), 1 / scale([0.5], 1, -0)[0]);\n\
            print(scale([0.5, 1.5], 2, NaN).join(), scale([1, 2, 3], 3, 2).join());\n\
            var t = [1, 2, 3, 4]; scale(t, 4, 0.25); t[0] = 'x'; print(t.join(), put([1.5, 2.5, 3.5], 3, 7).join());\n\
            var h = new Array(3); h[0] = 1; h[1] = 2; h[2] = 3; print(scale(h, 3, 1.5).join(), scale([1, 2], 2, -1).join());";
        let want = [
            "0.25,0.75,1.25,2 -Infinity",
            "NaN,NaN 2,4,6",
            "x,0.5,0.75,1 7,8,9",
            "1.5,3,4.5 -1,-2",
        ];
        for config in [Config::default(), Config::stress()] {
            let defs = SCALE;
            for name in ["scale", "put"] {
                let r = run_script(&config, defs, warm, main, name).expect("run");
                assert!(r.equal(), "{name}: {:?} vs {:?}", r.compiled, r.t0);
                assert!(r.fired().is_empty(), "{name}: exits {:?}", r.exits);
                assert_eq!(r.compiled.output, want, "{name}");
            }
        }
        let r = run_script(&Config::default(), SCALE, warm, main, "scale").expect("run");
        assert!(
            r.report.graph.contains("StoreElement") && !r.report.graph.contains("SetKeyedGeneric"),
            "{}",
            r.report.graph
        );
    }

    /// WI-SC7 (MODES-CHECK-w19 MC-w19-5 (4), outside O3): every element op, a raw f64 load
    /// included, reads its array through a kind guard (`CheckArray`), so no pass can place the
    /// load before the guard; and a Float64 value at an exit is in a general register or a
    /// native stack slot (MC-w19-8: the H29 form has no FP register location, and the emitter
    /// keeps doubles as their bits in x registers between ops), so the deopt core boxes it.
    #[test]
    fn element_ops_read_their_kind_guard_and_doubles_at_exits_are_in_x_registers() {
        if !aarch64() {
            return;
        }
        let warm = "for (var q = 0; q < 20; q++) scale([0.5, 1.5, 2.5], 3, 1.5);";
        let r = run_script(&Config::default(), SCALE, warm, "print(1);", "scale").expect("run");
        let g = &r.report.graph;
        let mut ops = 0;
        for l in g
            .lines()
            .filter(|l| l.contains("LoadElement") || l.contains("StoreElement"))
        {
            // `nK = Op(nA, ...)`: the first input is the array.
            let arr = l.split('(').nth(1).and_then(|t| t.split([',', ')']).next());
            let arr = arr.expect("an input").trim();
            let def = g
                .lines()
                .find(|d| d.trim_start().starts_with(&format!("{arr} ")))
                .unwrap_or_else(|| panic!("no def of {arr}\n{g}"));
            assert!(def.contains("CheckArray"), "{l} reads {def}\n{g}");
            ops += 1;
        }
        assert!(ops >= 2, "{g}");
        let meta = &r.report.emit.meta;
        let doubles: Vec<_> = meta
            .exits
            .iter()
            .flat_map(|e| &e.values)
            .filter(|v| v.repr == crate::deopt_meta::ValueRepr::Float64)
            .collect();
        assert!(!doubles.is_empty(), "{}", meta.line());
        for v in doubles {
            assert!(
                matches!(
                    v.loc,
                    crate::deopt_meta::ValueLoc::Reg(_) | crate::deopt_meta::ValueLoc::Stack(_)
                ),
                "{v:?}"
            );
        }
    }

    /// w17 O10 and MODES-CHECK-w19 outside O2 (a guard proven to fail): a guard that fails on
    /// every call exits on every call, as a speculation exit (the only class the eager exits
    /// have, `deopt_meta`), and its counter equals the number of calls: the code stays
    /// installed (SCP:128), so each call enters it again. The H32 site budget and the pin it
    /// leads to are not built in the spike (A4); this is the input they will read.
    #[test]
    fn a_guard_that_fails_on_every_call_counts_every_call() {
        if !aarch64() {
            return;
        }
        let warm = "for (var q = 0; q < 20; q++) dot([1, 2, 3], [4, 5, 6], 3);";
        let calls = 7;
        let main = format!(
            "var o = {{ 0: 1, 1: 2, length: 2 }}; var s = 0; for (var k = 0; k < {calls}; k++) s += dot(o, o, 2); print(s);"
        );
        let r = run_script(&Config::default(), DOT, warm, &main, "dot").expect("run");
        assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
        assert_eq!(r.compiled.output, ["35"]);
        let fired: Vec<(usize, u64)> = r
            .exits
            .iter()
            .copied()
            .enumerate()
            .filter(|&(_, c)| c > 0)
            .collect();
        let [(k, count)] = fired[..] else {
            panic!("one exit expected: {:?}", r.exits);
        };
        assert_eq!(count, calls, "{:?}", r.exits);
        let guard = r.report.emit.exits[k].0;
        assert!(
            r.report
                .graph
                .lines()
                .any(|l| l.contains("CheckArray") && l.contains(&format!(" guard g{guard} "))),
            "g{guard}\n{}",
            r.report.graph
        );
    }

    /// WI-SC4 (C2): each guard of `dot` and of `scale` forced alone at its second pass: the
    /// output equals T0, the forced word ends at 0 and the VM counts that one exit and no
    /// other; then every guard armed from a seed, with GC stress: equal to T0, and each
    /// guard's counter at least its forced exit.
    #[test]
    fn forcing_each_guard_deopts_to_t0() {
        if !aarch64() {
            return;
        }
        use crate::emit_a64::{EmitOptions, ForceTable};
        let warm = "for (var q = 0; q < 20; q++) { dot([1, 2, 3], [4, 5, 6], 3); dot([0.5, 1.5], [2, 0.25], 2); scale([0.5, 1.5, 2.5], 3, 1.5); }";
        let main = "print(dot([1, 2, 3], [4, 5, 6], 3), dot([0.5, 1.5], [2, 0.25], 2), scale([1, 2, 3], 3, 0.5).join());";
        let defs = format!("{DOT}\n{SCALE}");
        for name in ["dot", "scale"] {
            // No natural exit in `main`: each exit below is the forced one.
            let plain = run_script(&Config::default(), &defs, warm, main, name).expect("run");
            assert!(
                plain.equal() && plain.fired().is_empty(),
                "{name}: {:?}",
                plain.exits
            );
            let mut guards: Vec<u32> = plain
                .report
                .emit
                .meta
                .exits
                .iter()
                .map(|e| e.guard)
                .collect();
            guards.sort_unstable();
            guards.dedup();
            assert!(guards.len() >= 4, "{name}: {guards:?}");
            let mut forced = 0;
            for &g in &guards {
                let t = ForceTable::new(FORCE_LEN);
                t.arm(g, 2);
                let opts = EmitOptions {
                    force: Some(&t),
                    ..Default::default()
                };
                let r = run_script_full(
                    &Config::default(),
                    &defs,
                    warm,
                    main,
                    name,
                    IntFacts::Live,
                    &opts,
                )
                .expect("run");
                assert!(r.equal(), "{name} g{g}: {:?} vs {:?}", r.compiled, r.t0);
                let fired = r.fired();
                if t.get(g) == 0 {
                    forced += 1;
                    assert_eq!(fired, [g], "{name} g{g}: {:?}", r.exits);
                    assert_eq!(r.exits.iter().sum::<u64>(), 1, "{name} g{g}");
                } else {
                    assert!(fired.is_empty(), "{name} g{g}: {:?}", r.exits);
                }
            }
            // A guard on a path `main` takes fewer than twice (an error path) stays unforced.
            assert!(
                forced * 2 > guards.len(),
                "{name}: {forced} of {}",
                guards.len()
            );
            let t = ForceTable::seeded(5, FORCE_LEN);
            let opts = EmitOptions {
                force: Some(&t),
                ..Default::default()
            };
            let r = run_script_full(
                &Config::stress(),
                &defs,
                warm,
                main,
                name,
                IntFacts::Live,
                &opts,
            )
            .expect("run");
            assert!(r.equal(), "{name} seeded: {:?} vs {:?}", r.compiled, r.t0);
            for &g in &guards {
                let c: u64 = r
                    .report
                    .emit
                    .meta
                    .exits
                    .iter()
                    .zip(&r.exits)
                    .filter(|(e, _)| e.guard == g)
                    .map(|(_, c)| c)
                    .sum();
                assert!(c >= u64::from(t.get(g) == 0), "{name} g{g}: {c}");
            }
        }
    }

    /// MODES-CHECK-w19 outside O1 (V8 b60744d16a: arm64 code that read "an int32 is
    /// zero-extended"): in the int32 junk build every Int32 and Bool result gets 0xDEADBEEF in
    /// its upper 32 bits, in its register or spill slot. Int loops, overflow, -0 and
    /// out-of-bounds deopts, element ops, a phi swap and int32 values at exits (the deopt core
    /// boxes the low half) give T0's output and the same exits as the plain build. The junk
    /// build is larger (the movk pairs are there).
    #[test]
    fn junk_in_the_upper_half_of_int32_values_changes_nothing() {
        if !aarch64() {
            return;
        }
        use crate::emit_a64::EmitOptions;
        let fib = "function fib(n) { var a = 0, b = 1; for (var i = 0; i < n; i++) { var t = a; a = b; b = (t + b) & 65535; } return a; }";
        let m =
            "function m(a, b) { var r = 0; for (var i = 0; i < 1; i++) { r = a * b; } return r; }";
        let cases: [(&str, &str, &str, &str); 6] = [
            (
                SUM,
                "sum",
                "for (var k = 0; k < 20; k++) sum(10);",
                "print(sum(0), sum(1000), sum(-5), sum(100000), sum(2.5));",
            ),
            (fib, "fib", "fib(5);", "print(fib(10), fib(1000));"),
            (
                m,
                "m",
                "m(3, 4);",
                "print(m(-3, 4), m(46340, 46340), m(65536, 65536), 1 / m(0, -5));",
            ),
            (
                DOT,
                "dot",
                "for (var q = 0; q < 20; q++) dot([1, 2, 3], [4, 5, 6], 3);",
                "print(dot([1, 2, 3], [4, 5, 6], 3), dot([1, 2], [3, 4], 5), dot([0.5, 1.5], [2, 0.25], 2));",
            ),
            (
                SCALE,
                "put",
                "for (var q = 0; q < 20; q++) put([1, 2, 3], 3, q);",
                "print(put([1, 2, 3], 3, -7).join(), put([1.5, 2.5], 2, 2147483647).join(), put([1], 3, 1).join());",
            ),
            (
                SCALE,
                "scale",
                "for (var q = 0; q < 20; q++) scale([1, 2, 3], 3, 2);",
                "print(scale([1, 2, 3], 3, 2).join(), scale([1, 2], 2, 0.5).join());",
            ),
        ];
        for (defs, name, warm, main) in cases {
            let plain = run_script(&Config::default(), defs, warm, main, name).expect("run");
            let opts = EmitOptions {
                int32_junk: true,
                ..Default::default()
            };
            let junk = run_script_full(
                &Config::default(),
                defs,
                warm,
                main,
                name,
                IntFacts::Live,
                &opts,
            )
            .expect("run");
            assert!(junk.equal(), "{name}: {:?} vs {:?}", junk.compiled, junk.t0);
            assert_eq!(junk.compiled, plain.compiled, "{name}");
            assert_eq!(junk.exits, plain.exits, "{name}");
            assert!(
                junk.report.emit.code_bytes > plain.report.emit.code_bytes,
                "{name}: no junk was written"
            );
        }
    }

    /// w16 O6: a termination that a T0 frame sees after a deopt unwinds through T0 callers
    /// with no `catch` or `finally` of theirs running (`error::unwind` skips handlers while
    /// terminating). `terminate()` asks before the call; the compiled `dot` takes its kind
    /// guard's exit at once (the receiver is not an Array: no compiled poll ran yet), T0
    /// finishes the loop and stops at its next poll. Equal to T0: no `catch`, `finally` or
    /// `after` line, the run ends in the termination, and the next script runs.
    #[test]
    fn a_termination_after_a_deopt_runs_no_catch_or_finally() {
        if !aarch64() {
            return;
        }
        let defs = format!(
            "{DOT}\nfunction caller(a, n) {{ try {{ return dot(a, a, n); }} catch (e) {{ print('catch'); }} finally {{ print('finally'); }} }}"
        );
        let warm = "for (var q = 0; q < 20; q++) dot([1, 2, 3], [4, 5, 6], 3);";
        let main = "print('before'); terminate(); var o = { 0: 1, 1: 2, length: 2 }; caller(o, 1000000); print('after');";
        let r = run_script(&Config::default(), &defs, warm, main, "dot").expect("run");
        assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
        assert_eq!(r.compiled.output, ["before"]);
        assert!(r.compiled.result.is_err(), "{:?}", r.compiled);
        assert_eq!(r.exits.iter().sum::<u64>(), 1, "{:?}", r.exits);
        let g = r.fired()[0];
        assert!(
            r.report
                .graph
                .lines()
                .any(|l| l.contains("CheckArray") && l.contains(&format!(" guard g{g} "))),
            "g{g}\n{}",
            r.report.graph
        );
    }

    /// C5b (SCP §6.2): the deopt metadata holds offsets only, so the same code and metadata
    /// finalized again at another address deopts the same: `dot` installed from module A,
    /// run (int, double and deopting calls), uninstalled, the same bytes and metadata
    /// installed from module B at another start, run again: the same output and the same
    /// exit counters, both equal to T0.
    #[test]
    fn a_moved_copy_of_code_and_metadata_deopts_the_same() {
        if !aarch64() {
            return;
        }
        let warm = "for (var q = 0; q < 20; q++) dot([1, 2, 3], [4, 5, 6], 3);";
        let main = "print(dot([1, 2, 3], [4, 5, 6], 3), dot([0.5, 1.5], [2, 0.25], 2), dot([1, 2], [3, 4], 5), dot(['x', 1], [1, 1], 2), dot({ 0: 1, length: 1 }, [2], 1));";
        let config = Config::default().threaded();
        let mut t0s = Session::new(&config).expect("session");
        t0s.run(DOT, "defs.js").result.expect("defs");
        t0s.run(warm, "warm.js").result.expect("warm");
        let t0 = t0s.run(main, "main.js");
        let mut s = Session::new(&config).expect("session");
        s.run(DOT, "defs.js").result.expect("defs");
        s.run(warm, "warm.js").result.expect("warm");
        let f = s.function_value("dot").expect("dot");
        let script = s.script("defs.js").expect("defs");
        let id = script.functions_named("dot")[0];
        let live = facts::live(&s, script).expect("facts");
        let input = s.input_of(f).expect("input");
        let (code, report) =
            crate::compile_input(&input, &facts::for_function(&live, id)).expect("compile");
        let meta = report.emit.meta.encode();
        let mut runs = Vec::new();
        for _ in 0..2 {
            // SAFETY: emit_a64's placement (b) code for `f` from `input_of(f)` with no JS
            // run since but `main`, which changes no feedback the code reads (the guards read
            // nothing from the feedback vector), and its own metadata, unchanged.
            let placed = unsafe { s.install_b(&[f], vec![code.clone()], vec![Some(meta.clone())]) }
                .expect("install")[0]
                .code
                .start;
            let out = s.run(main, "main.js");
            let exits = s.exit_counts(f).expect("counters");
            s.uninstall(f).expect("uninstall");
            runs.push((placed, out, exits));
        }
        assert_ne!(runs[0].0, runs[1].0, "the copy is at the same place");
        assert_eq!(runs[0].1, t0);
        assert_eq!(runs[1].1, runs[0].1);
        assert_eq!(runs[1].2, runs[0].2);
        assert!(runs[0].2.iter().sum::<u64>() >= 2, "{:?}", runs[0].2);
    }

    /// `/` always takes the double path; `===` compares int32s behind CheckInt32 (SPIKEC-4).
    #[test]
    fn division_and_strict_equality_equal_t0() {
        if !aarch64() {
            return;
        }
        let defs = "function ratio(a, b, c) { if (c === 1) { return a / b; } return b / a; }";
        let r = equal(
            defs,
            "for (var q = 0; q < 20; q++) { ratio(1, 4, 1); ratio(3, 6, 0); }",
            "print(ratio(1, 4, 1), ratio(6, 3, 1), ratio(1, 0, 1), ratio(-1, 0, 1), 1 / ratio(0, -5, 1), ratio(0, 0, 1), ratio(2, 1, 0), ratio(1.5, 3, 1));",
            "ratio",
        );
        assert_eq!(
            r.compiled.output,
            ["0.25 2 Infinity -Infinity -Infinity NaN 0.5 0.5"]
        );
        assert!(
            r.report.graph.contains("Float64(Div)"),
            "{}",
            r.report.graph
        );
        assert!(r.report.graph.contains("Compare(Eq)"), "{}", r.report.graph);
        // A string `c` fails the int32 check of `===` (T0 says false).
        deopts(
            defs,
            "for (var q = 0; q < 20; q++) { ratio(1, 4, 1); ratio(3, 6, 0); }",
            "print(ratio(1, 4, '1'));",
            "ratio",
        );
    }

    /// Context slot loads and calls (SPIKEC-4 built them; WI-SC5 emits them): `inner`, an
    /// inner closure installed by its function id, reads `k` from its context, for a closure
    /// made before the install and one made after (both run the code: one dispatch slot);
    /// `callit` calls through `RuntimeTable::call_any` with its operands in their VM frame
    /// slots: a compiled callee, a T0 closure, a host function, a throw that a T0 caller
    /// catches, and a callee that stores to the context `inner` reads. Equal to T0 with no
    /// exit, with GC stress too.
    #[test]
    fn context_loads_and_calls_equal_t0() {
        if !aarch64() {
            return;
        }
        let defs = "function outer(k) { function inner(x) { return (x + k) | 0; } return inner; }\n\
            function callit(f, x) { return f(x); }\n\
            var box = (function () { var v = 1; return { get: function (x) { return (x + v) | 0; }, set: function (x) { v = x; return 0; } }; })();\n\
            var inner5 = outer(5); for (var q = 0; q < 20; q++) { inner5(q); callit(inner5, q); box.get(q); }";
        let main = "var later = outer(-3); print(inner5(1), later(2), callit(inner5, 7), callit(later, 7));\n\
            print(callit(function (y) { return y * 2; }, 4), callit(Math.abs, -9), callit(box.set, 40), box.get(2));\n\
            try { callit(function () { throw 'boom'; }, 1); } catch (e) { print('caught ' + e); }";
        let want = ["6 -1 12 4", "8 9 0 42", "caught boom"];
        for config in [Config::default(), Config::stress()] {
            for (name, op) in [
                ("inner", "LoadContextSlot"),
                ("callit", "Call"),
                ("get", "LoadContextSlot"),
            ] {
                let r = run_script(&config, defs, "", main, name).expect("run");
                assert!(r.equal(), "{name}: {:?} vs {:?}", r.compiled, r.t0);
                assert_eq!(r.compiled.output, want, "{name}");
                assert!(r.fired().is_empty(), "{name}: exits {:?}", r.exits);
                assert!(r.report.graph.contains(op), "{name}\n{}", r.report.graph);
            }
        }
    }

    /// Codex M5-P2 r1 spikec6 1: a `LoadContextSlot` past the length of the frame's context
    /// (a verified unit that BytecodeGen never makes: the verifier bounds a slot by the unit's
    /// largest scope, H02-Q2; here `mk2`'s 2 slots, while `g`'s context has 1) exits to T0, which
    /// runs the instruction again and throws its internal error, as a T0 run does. Before the
    /// guard the compiled code read the word past the context: a wrong value, and under GC
    /// stress an abort in the deopt of a later guard.
    #[test]
    fn a_context_slot_past_the_context_exits_to_t0() {
        if !aarch64() {
            return;
        }
        /// `g`'s first context load (the `x > 100` branch, which the warm-up never takes)
        /// reads slot 1.
        fn slot_1(file: &str, mut u: ep9_bytecode::CompiledUnit) -> ep9_bytecode::CompiledUnit {
            if file != "defs.js" {
                return u;
            }
            let want = ep9_bytecode::UnitString::new("g");
            let g = (0..u.functions.len())
                .find(|&f| u.functions[f].name.is_some_and(|n| *u.string(n) == want))
                .expect("g");
            let code = &mut u.functions[g].bytecode;
            let at = ep9_bytecode::Instructions::new(code)
                .map(|i| i.expect("decodes"))
                .find(|i| {
                    i.opcode == ep9_bytecode::Opcode::LdaImmutableCurrentContextSlot
                        || i.opcode == ep9_bytecode::Opcode::LdaCurrentContextSlot
                })
                .map(|i| (i.offset + 1) as usize)
                .expect("a context load");
            assert_eq!(code[at], 0, "slot 0 (a one-byte operand)");
            code[at] = 1;
            u
        }
        let defs = "function mk() { var a = 7; return function g(x) { if (x > 100) { return a + 1; } return a; }; }\n\
            function mk2() { var p = 1, q = 2; return function h() { return p + q; }; }\n\
            var g1 = mk(); var h1 = mk2();";
        let warm = "for (var q = 0; q < 20; q++) { g1(q); h1(); }";
        let main = "try { print(g1(200)); } catch (e) { print('threw ' + e); } print(g1(3));";
        crate::harness::PATCH.with(|p| *p.borrow_mut() = Some(slot_1));
        for config in [Config::default(), Config::stress()] {
            let r = run_script(&config, defs, warm, main, "g").expect("run");
            assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
            assert_eq!(
                r.compiled.output,
                [
                    "threw Error: ep9 internal error: context slot out of range",
                    "7"
                ]
            );
            assert_eq!(r.exits.iter().sum::<u64>(), 1, "exits {:?}", r.exits);
            assert!(
                r.report.graph.contains("LoadContextSlot(1)"),
                "{}",
                r.report.graph
            );
        }
        crate::harness::PATCH.with(|p| *p.borrow_mut() = None);
    }

    /// Call operands and parameters (WI-SC5): the frontend copies each call operand into a
    /// fresh local register, so `call_any` reads them from local slots the code writes, and a
    /// parameter's slot (where its incoming value lives, read by later nodes) is never
    /// written: `b` keeps the incoming `a` after `a` changes and is passed. The emitter
    /// refuses a parameter operand that holds a new value (the frontend makes none today).
    #[test]
    fn a_call_never_writes_over_an_incoming_parameter() {
        if !aarch64() {
            return;
        }
        let defs = "function id(x) { return x; }\n\
            function pass(a, g) { return g(a); }\n\
            function swap(a, g) { var b = a; a = (a + 1) | 0; var c = g(a); return (b + c) | 0; }";
        let warm = "for (var q = 0; q < 20; q++) { pass(q, id); swap(q, id); }";
        let main = "print(pass(7, id), swap(7, id), pass('s', id));";
        for name in ["pass", "swap"] {
            let r = run_script(&Config::default(), defs, warm, main, name).expect(name);
            assert!(
                r.equal() && r.fired().is_empty(),
                "{:?} vs {:?}",
                r.compiled,
                r.t0
            );
            assert_eq!(r.compiled.output, ["7 15 s"]);
            let call = r
                .report
                .graph
                .lines()
                .find(|l| l.contains("Call {"))
                .expect("a call");
            assert!(!call.contains("Register(-"), "{call}");
        }
    }

    /// Kernel 2 (SPIKEC-4): `lin_solve` (an inner closure of navier-stokes, found by its
    /// function id) builds into a checked graph from the live facts of an n = 1 run, its
    /// registers allocate and verify, and the emitter takes it (WI-SC5).
    #[test]
    fn lin_solve_graph_builds() {
        if !kernels::bench_present() {
            eprintln!("skip: no v8-v7 checkout");
            return;
        }
        let k = kernels::kernel("navier-stokes").expect("navier-stokes");
        let h = &k.hot[0];
        assert_eq!(h.name, "lin_solve");
        let d = kernels::driver(k, 1).expect("driver");
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(&d.text, "driver.js").result.expect("T0");
        let script = s.script("driver.js").expect("script");
        let ids = script.functions_on_line(h.lines.0 + d.offset);
        let [id] = ids[..] else { panic!("{ids:?}") };
        let live = facts::live(&s, script).expect("facts");
        let input = script.unit_input(id, h.name).expect("input");
        let g = crate::graph_report(&input, &facts::for_function(&live, id)).expect("graph");
        let lines = g.lines().join("\n");
        for op in [
            "Float64",
            "CheckNumber",
            "LoadContextSlot",
            "Call",
            "LoadElement",
        ] {
            assert!(g.ops.contains_key(op), "{op}: {lines}");
        }
        assert!(g.fact_guards > 0, "{lines}");
        // The inner loop's double sum: every CheckNumber is backed by its number record.
        for l in g.graph.lines().filter(|l| l.contains("CheckNumber")) {
            assert!(l.contains("Fact(Runtime)"), "{l}\n{}", g.graph);
        }
        // Since WI-SC5 the emitter takes every op of it.
        if aarch64() {
            assert!(g.emit.is_ok(), "{:?}", g.emit);
        }
    }

    /// Kernel 2 (WI-SC5): navier-stokes with `lin_solve` compiled and installed by its
    /// function id (no JS value reaches it) prints T0's check line (every cell of the three
    /// fields digested), with no exit at all: the double arrays pass the kind guard and the
    /// raw f64 element ops (WI-SC7), so no CheckArray exit measures the deopt path.
    #[test]
    fn lin_solve_by_function_id_equals_t0() {
        if !aarch64() || !kernels::bench_present() {
            return;
        }
        let k = kernels::kernel("navier-stokes").expect("navier-stokes");
        let r = run(k, 1, &Config::default()).expect("run");
        assert!(r.equal, "{}", r.line());
        assert_eq!(r.exits.iter().sum::<u64>(), 0, "{}", r.line());
        assert!(
            r.compiled.output[0].starts_with("checksum 328.09"),
            "{:?}",
            r.compiled
        );
        assert!(
            r.report.graph.contains("StoreElement"),
            "{}",
            r.report.graph
        );
    }

    /// Keyed facts (SPIKEC-3): an index-only LoadKeyed record backs the key's int32 check;
    /// the element guards stay unbacked (FACT_SCHEMA 1 has no receiver, bounds or hole field);
    /// a site that saw a key that is not an index takes the generic keyed entry (SPIKEC-4;
    /// SPIKEC-3 refused it).
    const AT: &str = "function at(a, k) { return a[k] | 0; }";

    #[test]
    fn keyed_facts_back_only_the_key_check() {
        if !aarch64() {
            return;
        }
        let warm = "var w = [1, 2, 3, 4]; for (var q = 0; q < 20; q++) at(w, q & 3);";
        let r = equal(AT, warm, "print(at([7, 8, 9], 2), at([5], 0));", "at");
        assert_eq!(r.compiled.output, ["9 5"]);
        let g = &r.report.graph;
        // The key `k` is parameter 1 (n2).
        let key: Vec<&str> = g
            .lines()
            .filter(|l| l.contains("CheckInt32(n2)") && l.contains(" guard g"))
            .collect();
        assert_eq!(key.len(), 1, "{g}");
        assert!(key[0].contains("Fact(Runtime)"), "{g}");
        for l in g
            .lines()
            .filter(|l| l.contains("CheckArray") || l.contains("LoadElement"))
        {
            assert!(l.contains("Unbacked"), "{l}\n{g}");
        }
        let sites = &r.report.build.keyed;
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].keys, Some(ep9_facts::KeySet::INDEX));
        assert!(!sites[0].store);
        // An index of another form (a string) fails the int32 key check: a deopt, never a
        // wrong value (T0 prints 8).
        let t = deopts(AT, warm, "print(at([7, 8, 9], '1'));", "at");
        assert_eq!(t.t0.output, ["8"]);
        // A name key in the warm-up: the generic path (SPIKEC-4), no keyed guard.
        let r = equal(
            AT,
            "var w = [1, 2]; for (var q = 0; q < 20; q++) at(w, q & 1); at(w, 'length');",
            "print(at(w, 0), at(w, 'length'), at([4, 5, 6], '2'), at({ x: 9 }, 'x'));",
            "at",
        );
        assert_eq!(r.compiled.output, ["1 2 6 9"]);
        let g = &r.report.graph;
        assert!(g.contains("GetKeyedGeneric"), "{g}");
        assert!(
            !g.contains("CheckArray") && !g.contains("LoadElement"),
            "{g}"
        );
        let sites = &r.report.build.keyed;
        assert!(sites.len() == 1 && sites[0].generic, "{sites:?}");
    }

    /// The generic keyed path (SPIKEC-4, RuntimeTable 8 and 9): name, symbol and other keys,
    /// getters and setters (JS runs inside the call), stores of strings and objects (the
    /// entry's store barrier), a throw from inside the call that the caller catches, all
    /// equal to T0; and values in caller-saved registers live across the call.
    /// `s` lives in a register across each call; `t[0]` shows it after the loop.
    const GEN: &str = "function gen(o, k, v, n, t) {\n\
        var s = 0;\n\
        for (var i = 0; i < n; i++) { o[k] = v; s = (s + i) | 0; }\n\
        t[0] = s;\n\
        return o[k];\n\
    }";

    #[test]
    fn generic_keyed_path_equals_t0() {
        if !aarch64() {
            return;
        }
        let warm = "var t = [0]; var w = {}; var sy = Symbol('s'); for (var q = 0; q < 20; q++) { gen(w, 'a', q, 3, t); gen(w, sy, 'x', 2, t); gen([1, 2], q & 1, 'y', 1, t); }";
        let main = "var log = [];\n\
            var acc = { get z() { log.push('get'); return 'Z'; }, set z(v) { log.push('set ' + v); } };\n\
            print(gen({}, 'p', 'str', 4, t), t[0], typeof gen({}, sy, { u: 1 }, 2, t), gen([0, 0, 0], 2, 'el', 3, t), t[0]);\n\
            print(gen(acc, 'z', 7, 2, t), t[0], log.join());\n\
            print(gen({}, 1.5, 3, 2, t), gen({}, true, 4, 1, t), gen({}, null, 5, 1, t), gen({}, 'x', 0.5, 300, t), t[0]);\n\
            try { gen(null, 'q', 1, 1, t); } catch (e) { print('caught', e instanceof TypeError); }\n\
            gen(Object.freeze({ f: 1 }), 'f', 2, 1, t); print('sloppy ok');";
        let r = equal(GEN, warm, main, "gen");
        assert_eq!(
            r.compiled.output,
            [
                "str 6 object el 3",
                "Z 1 set 7,set 7,get",
                "3 4 5 0.5 44850",
                "caught true",
                "sloppy ok"
            ]
        );
        let g = &r.report.graph;
        assert!(
            g.contains("SetKeyedGeneric") && g.contains("GetKeyedGeneric"),
            "{g}"
        );
        // `o[k] = v` and `o[k]` saw names and symbols. `t[0] = s` saw only indexes but stores
        // the loop phi `s`, which is still Tagged when the builder translates the store (the
        // phis are unboxed after the walk), so it takes the generic store too.
        let sites = &r.report.build.keyed;
        assert_eq!(sites.len(), 3, "{sites:?}");
        assert!(sites.iter().all(|k| k.generic), "{sites:?}");
        assert_eq!(sites[1].keys, Some(ep9_facts::KeySet::INDEX), "{sites:?}");
        // Under GC stress (a collection before every allocation): the setter, the getter and
        // the string concatenation allocate inside and after the call.
        let r = run_script(&Config::stress(), GEN, warm, main, "gen").expect("run");
        assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
    }

    /// A store of a value that is not an int32 takes the generic store even on an index-only
    /// site (the entry does the barrier); an int32 value keeps the element path.
    #[test]
    fn a_store_of_a_non_int32_value_is_generic() {
        if !aarch64() {
            return;
        }
        let defs =
            "function put(a, i, n) { for (var k = 0; k < n; k++) { a[i] = a; } return a[i]; }";
        let r = equal(
            defs,
            "var w = [0, 0]; for (var q = 0; q < 20; q++) put(w, q & 1, 2);",
            "var b = [1, 2, 3]; print(put(b, 2, 3) === b, b.length, b[2] === b); gc(); print(b[2].length);",
            "put",
        );
        assert_eq!(r.compiled.output, ["true 3 true", "3"]);
        let store: Vec<_> = r.report.build.keyed.iter().filter(|k| k.store).collect();
        assert!(store.len() == 1 && store[0].generic, "{store:?}");
        assert_eq!(store[0].keys, Some(ep9_facts::KeySet::INDEX));
    }

    /// Native code records no feedback (MC-w15-6): after the compiled `gen` ran with new key
    /// kinds through the generic entries, its live facts are what they were before; the same
    /// script in T0 changes them (the control: the test can see a change).
    #[test]
    fn the_generic_path_records_no_feedback() {
        if !aarch64() {
            return;
        }
        let config = Config::default().threaded();
        let warm = "var t = [0]; var w = {}; for (var q = 0; q < 20; q++) gen(w, 'a', q, 2, t);";
        let main = "var sy = Symbol('n'); gen({}, sy, 1, 2, t); gen({}, 3, 1, 2, t); gen({}, {}, 1, 1, t);";
        let facts_of = |s: &Session| {
            let script = s.script("defs.js").expect("defs");
            let id = script.functions_named("gen")[0];
            let live = facts::live(s, script).expect("facts");
            let recs: Vec<(u32, ep9_facts::SlotFacts)> = facts::for_function(&live, id)
                .records
                .into_iter()
                .map(|(k, f)| (k, f.clone()))
                .collect();
            recs
        };
        let mut t0 = Session::new(&config).expect("session");
        t0.run(GEN, "defs.js").result.expect("defs");
        t0.run(warm, "warm.js").result.expect("warm");
        let before_t0 = facts_of(&t0);
        t0.run(main, "main.js").result.expect("main");
        assert_ne!(facts_of(&t0), before_t0, "T0 records the new keys");

        let mut s = Session::new(&config).expect("session");
        s.run(GEN, "defs.js").result.expect("defs");
        s.run(warm, "warm.js").result.expect("warm");
        let before = facts_of(&s);
        assert_eq!(before, before_t0);
        let f = s.function_value("gen").expect("gen");
        let (code, report) = {
            let script = s.script("defs.js").expect("defs");
            let id = script.functions_named("gen")[0];
            let live = facts::live(&s, script).expect("facts");
            let input = s.input_of(f).expect("input");
            crate::compile_input(&input, &facts::for_function(&live, id)).expect("compile")
        };
        assert!(report.build.keyed.iter().all(|k| k.generic));
        let meta = report.emit.meta.encode();
        // SAFETY: as in `run`: emit_a64's placement (b) code for `f`, from `input_of(f)` with
        // no JS run since, unchanged, and its exit table.
        unsafe { s.install_b(&[f], vec![code], vec![Some(meta)]) }.expect("install");
        s.run(main, "main.js").result.expect("main");
        // No guard failed (a deopt would let T0 record): the compiled code ran it all.
        let exits = s.exit_counts(f).expect("counters");
        assert!(exits.iter().all(|&n| n == 0), "{exits:?}");
        assert_eq!(facts_of(&s), before, "the compiled gen recorded feedback");
    }

    /// A store site that never ran has a record with no keys: its key check stays unbacked
    /// (a never-run store backs nothing); the load's constant key needs no check.
    #[test]
    fn a_never_run_store_backs_no_key_check() {
        if !aarch64() {
            return;
        }
        let defs = "function put(a, k, f) { if (f > 0) { a[k] = 1; } return a[0] | 0; }";
        let r = equal(
            defs,
            "var w = [1, 2]; for (var q = 0; q < 20; q++) put(w, 1, 0);",
            "var b = [3, 4]; print(put(b, 1, 1), b.join());",
            "put",
        );
        assert_eq!(r.compiled.output, ["3 3,1"]);
        let g = &r.report.graph;
        let key: Vec<&str> = g
            .lines()
            .filter(|l| l.contains("CheckInt32(n2)") && l.contains(" guard g"))
            .collect();
        assert_eq!(key.len(), 1, "{g}");
        assert!(key[0].contains("Unbacked"), "{g}");
        let store: Vec<_> = r.report.build.keyed.iter().filter(|k| k.store).collect();
        assert_eq!(store.len(), 1, "{:?}", r.report.build.keyed);
        assert_eq!(store[0].keys, Some(ep9_facts::KeySet::EMPTY));
        assert_eq!(store[0].key_basis, crate::ir::GuardBasis::Unbacked);
    }

    /// [`crate::build::keyed_path_of`]: only keys = {index} backs the key check, whatever the
    /// IC state; no keys leaves it unbacked; a name, symbol or other key takes the generic
    /// path (SPIKEC-4).
    #[test]
    fn key_basis_reads_index_only_records() {
        use crate::build::{KeyedPath, keyed_path_of};
        use crate::ir::GuardBasis;
        use ep9_facts::{FactSource, IcState, KeySet, KeyedFacts, PropertyFacts};
        let rec = |keys: KeySet, state: IcState| KeyedFacts {
            property: PropertyFacts {
                state,
                ..PropertyFacts::default()
            },
            keys,
        };
        let src = FactSource::Profiled;
        let ok = |k: KeySet, s: IcState| keyed_path_of(&rec(k, s), src);
        let fact = KeyedPath::Element(GuardBasis::Fact(src));
        assert_eq!(ok(KeySet::INDEX, IcState::Uninitialized), fact);
        assert_eq!(ok(KeySet::INDEX, IcState::Monomorphic), fact);
        assert_eq!(
            ok(KeySet::EMPTY, IcState::Uninitialized),
            KeyedPath::Element(GuardBasis::Unbacked)
        );
        for other in [
            KeySet::INDEX.union(KeySet::NAME),
            KeySet::SYMBOL,
            KeySet::OTHER,
            KeySet::NAME,
        ] {
            assert_eq!(
                ok(other, IcState::Uninitialized),
                KeyedPath::Generic,
                "{other:?}"
            );
        }
        assert_eq!(ok(KeySet::INDEX, IcState::Megamorphic), fact);
    }

    const NAMED: &str = "function P(x) { this.v = x; this.w = x * 2; }\n\
        function Q(x) { this.v = x; this.z = 1; }\n\
        function get(o, n) { var s = 0; for (var i = 0; i < n; i++) { s = (s + o.v) | 0; } return s; }";

    #[test]
    fn named_loads_guard_on_the_ic_shapes() {
        if !aarch64() {
            return;
        }
        // Two shapes with `v` at field 0: both pass the guard.
        let warm = "for (var k = 0; k < 20; k++) { get(new P(k), 2); get(new Q(k), 2); }";
        let r = equal(
            NAMED,
            warm,
            "print(get(new P(5), 10), get(new Q(7), 3));",
            "get",
        );
        // The shape guard of `o.v` plus, since FACTS2 (M4-P2), the 2 int32 input checks; the
        // 2 overflow guards of `+` stay unbacked.
        assert_eq!(
            (r.report.fact_guards, r.report.unbacked_guards),
            (3, 2),
            "{}",
            r.report.graph
        );
        // A third shape: the guard fails (a deopt), never a wrong field.
        deopts(NAMED, warm, "print(get({ w: 1, v: 9 }, 2));", "get");
        deopts(NAMED, warm, "print(get(5, 2));", "get");
    }

    #[test]
    fn mul_overflow_and_minus_zero_deopt() {
        if !aarch64() {
            return;
        }
        let defs =
            "function m(a, b) { var r = 0; for (var i = 0; i < 1; i++) { r = a * b; } return r; }";
        equal(defs, "m(3, 4);", "print(m(-3, 4), m(46340, 46340));", "m");
        deopts(defs, "m(3, 4);", "print(m(65536, 65536));", "m");
        deopts(defs, "m(3, 4);", "print(1 / m(0, -5));", "m");
    }

    /// A swap in a loop makes a phi move cycle (a <- b, b <- a + b).
    #[test]
    fn phi_cycles_and_many_values() {
        if !aarch64() {
            return;
        }
        let fib = "function fib(n) { var a = 0, b = 1; for (var i = 0; i < n; i++) { var t = a; a = b; b = (t + b) & 65535; } return a; }";
        let r = equal(
            fib,
            "fib(5);",
            "print(fib(0), fib(1), fib(10), fib(1000));",
            "fib",
        );
        assert!(r.report.graph.contains("Phi"), "{}", r.report.graph);
        // 20 values live around the loop: more than the 15 registers, so some spill.
        let mut body = String::from("function many(n) { ");
        for i in 0..20 {
            body.push_str(&format!("var v{i} = {i}; "));
        }
        body.push_str("for (var i = 0; i < n; i++) { ");
        for i in 0..20 {
            body.push_str(&format!("v{i} = (v{i} + v{}) & 4095; ", (i + 1) % 20));
        }
        body.push_str("} return ");
        let all: Vec<String> = (0..20).map(|i| format!("v{i}")).collect();
        body.push_str(&all.join(" ^ "));
        body.push_str("; }");
        let r = equal(
            &body,
            "many(2);",
            "print(many(0), many(7), many(300));",
            "many",
        );
        assert!(r.report.spilled > 0, "{}", r.report.line());
    }

    /// Nested loops: a phi of an outer loop header stays live (keeps its register) through
    /// the inner loop's back edge. The first case is the Codex r1 reproduction: before the
    /// fix the compiled `f(1, 4)` returned 8, T0 4.
    #[test]
    fn nested_loops_keep_outer_phis() {
        if !aarch64() {
            return;
        }
        // An outer phi (`v`) read in the inner loop.
        let f = "function f(n, m) { var s = 0, v = 1; for (var i = 0; i < n; i++) { for (var j = 0; j < m; j++) s = (s + v) & 65535; v = (i + 2) | 0; } return s; }";
        let r = equal(
            f,
            "f(2, 2);",
            "print(f(1, 4), f(2, 4), f(3, 5), f(0, 3), f(4, 0), f(30, 40));",
            "f",
        );
        assert_eq!(r.t0.output, ["4 12 30 0 0 18600"]);
        // Three deep: phis of the outer (`u`) and middle (`w`) headers read only in the
        // innermost loop (their next values do not read them).
        let g = "function g(a, b, c) { var s = 0, u = 1, w = 3; for (var i = 0; i < a; i++) { for (var j = 0; j < b; j++) { for (var k = 0; k < c; k++) s = (s + u + w + k) & 65535; w = (j * 7 + 3) & 255; } u = (i + 2) | 0; } return s; }";
        equal(
            g,
            "g(2, 2, 2);",
            "print(g(1, 1, 4), g(2, 3, 4), g(3, 2, 5), g(5, 6, 7), g(0, 1, 1), g(2, 0, 3));",
            "g",
        );
        // Inner phis (`j`, `s`) and outer phis (`i`, `t`) read after the inner loop, and an
        // outer-loop value (`p`, not a phi) read in and after it.
        let h = "function h(n, m) { var s = 0, t = 0; for (var i = 0; i < n; i++) { var p = (i * 3) | 0; for (var j = 0; j < m; j++) s = (s + j + p) & 65535; t = (t + p + i + j + s) & 65535; } return s ^ t; }";
        equal(
            h,
            "h(2, 2);",
            "print(h(1, 4), h(3, 1), h(4, 5), h(0, 2), h(5, 0), h(50, 60));",
            "h",
        );
        // A swap-style phi cycle in the inner loop that reads the outer phi `c` (read nowhere
        // else).
        let sw = "function sw(n, m) { var a = 1, b = 2, c = 1; for (var i = 0; i < n; i++) { for (var j = 0; j < m; j++) { var t = a; a = b; b = (t + b + c) & 65535; } c = (i * 5 + 1) | 0; } return (a ^ (b << 3)) | 0; }";
        equal(
            sw,
            "sw(2, 2);",
            "print(sw(1, 4), sw(2, 3), sw(3, 5), sw(0, 2), sw(6, 7), sw(20, 30));",
            "sw",
        );
    }

    /// SPIKEC-1's acceptance as a unit test: crypto with am3 compiled equals T0 (n = 1), and
    /// the probe shows the compiled am3 runs. Skips without the v8-v7 checkout.
    #[test]
    fn am3_in_crypto_equals_t0() {
        if !aarch64() {
            return;
        }
        if !kernels::bench_present() {
            eprintln!("skip: no v8-v7 checkout");
            return;
        }
        let k = kernels::kernel("crypto").expect("crypto");
        let r = run(k, 1, &Config::default()).expect("run");
        assert!(r.equal, "{}", r.line());
        assert_eq!(r.probe_exit, Some(0), "{}", r.line());
        // Since FACTS2 (M4-P2) the live facts back the int32 input checks too, as with the
        // stand-in (`am3_with_int_facts_equals_t0`): 11 fact guards; unbacked are the 6 keyed
        // guards, the 9 overflow guards of `+` and `-`, and the entry checks of the loop
        // phis `i`, `j` and `c`, whose only checks are in the loop body, not the header.
        assert_eq!(
            (r.report.fact_guards, r.report.unbacked_guards),
            (11, 18),
            "{}",
            r.report.graph
        );
        // Keyed facts (SPIKEC-3): the 4 keyed sites (3 loads, 1 store) have index-only
        // records, so each key check would be a fact guard; but every key is a loop phi
        // (`i`, `j`) that is already int32, so no key check is left in the graph, and the
        // element guards stay unbacked (no receiver, bounds or hole field in schema 1).
        let k = &r.report.build.keyed;
        assert_eq!(k.len(), 4, "{k:?}");
        assert_eq!(k.iter().filter(|s| s.store).count(), 1, "{k:?}");
        assert!(
            k.iter().all(|s| s.keys == Some(ep9_facts::KeySet::INDEX)
                && s.key_basis == crate::ir::GuardBasis::Fact(ep9_facts::FactSource::Runtime)),
            "{k:?}"
        );
        // One 4-byte exit stub per guard (the call form), one metadata record per exit.
        let e = &r.report.emit;
        assert_eq!((e.guards, e.stub_bytes), (29, 29 * 4), "{}", e.exits_line());
        assert_eq!(e.meta.exits.len(), 29, "{}", e.meta.line());
    }

    /// am3 with the movz-form exit stubs (MC-w15-14, a report since the call form is the
    /// default): crypto still equals T0, the probe still deopts at g0, and the stubs are 12
    /// bytes each.
    #[test]
    fn am3_with_movz_stubs_equals_t0() {
        if !aarch64() {
            return;
        }
        if !kernels::bench_present() {
            eprintln!("skip: no v8-v7 checkout");
            return;
        }
        let k = kernels::kernel("crypto").expect("crypto");
        let r = run_opts(
            k,
            1,
            &Config::default(),
            IntFacts::Live,
            emit_a64::StubForm::Movz,
        )
        .expect("run");
        assert!(r.equal, "{}", r.line());
        assert_eq!(r.probe_exit, Some(0), "{}", r.line());
        let e = &r.report.emit;
        assert_eq!(
            (e.guards, e.stub_bytes),
            (29, 29 * 12),
            "{}",
            e.exits_line()
        );
    }

    /// am3 with the FACTS2 stand-in int records: the int32 input checks are fact guards; the
    /// keyed element guards (no schema-1 field backs them), the overflow guards of `+` and `-` and the entry
    /// checks of the phis that the header does not check stay unbacked, and the output still
    /// equals T0.
    #[test]
    fn am3_with_int_facts_equals_t0() {
        if !aarch64() {
            return;
        }
        if !kernels::bench_present() {
            eprintln!("skip: no v8-v7 checkout");
            return;
        }
        let k = kernels::kernel("crypto").expect("crypto");
        let r = run_with(k, 1, &Config::default(), IntFacts::ImplicitInt32).expect("run");
        assert!(r.equal, "{}", r.line());
        assert_eq!(r.probe_exit, Some(0), "{}", r.line());
        let unbacked: Vec<&str> = r
            .report
            .graph
            .lines()
            .filter(|l| l.contains("Unbacked"))
            .collect();
        let count = |what: &[&str]| {
            unbacked
                .iter()
                .filter(|l| what.iter().any(|w| l.contains(w)))
                .count()
        };
        let keyed = count(&["CheckArray", "LoadElement", "StoreElement"]);
        let overflow = count(&["Int32(Add)", "Int32(Sub)"]);
        let entry = count(&["CheckInt32"]);
        assert_eq!(r.report.unbacked_guards, unbacked.len());
        assert_eq!(
            (keyed, overflow, entry, unbacked.len()),
            (6, 9, 3, 18),
            "{}",
            r.report.graph
        );
    }

    /// GC stress (a collection before every allocation, the verifier after each): the
    /// compiled loop polls and the warm-up and main scripts allocate around it.
    #[test]
    fn gc_stress() {
        if !aarch64() {
            return;
        }
        let defs = "function walk(a, n) { var s = 0; for (var i = 0; i < n; i++) { s = (s + a[i]) & 65535; } return s; }";
        let warm = "var xs = []; for (var k = 0; k < 50; k++) xs.push(k); walk(xs, 50);";
        let main = "var ys = []; for (var k = 0; k < 2000; k++) ys.push(k * 3); print(walk(ys, 2000)); gc(); print(walk(xs, 50));";
        let r = run_script(&Config::stress(), defs, warm, main, "walk").expect("run");
        assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
    }
}
