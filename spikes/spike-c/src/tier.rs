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
//! Both sessions use the threaded T0 dispatch (placement (b) code runs only under it). A run
//! that ends in the "deopt not built" trap names the guard ([`crate::emit_a64::trap_of`]).

use std::time::Instant;

use spike_b::harness::{Config, RunOutput};

use crate::harness::Session;
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
    /// The guard whose trap ended the compiled run, if one did.
    pub trap: Option<u32>,
    /// Output equal to T0, no uncaught error, a right driver run.
    pub equal: bool,
    /// Wall time of loop(n), seconds (one run each; a report, not a class S reading).
    pub t0_secs: f64,
    pub compiled_secs: f64,
    /// After loop(n), the compiled function called with a receiver of a shape no guard
    /// admits: the guard whose trap fired (proof that the compiled code is what runs), or
    /// `None` if the kernel has no probe.
    pub probe_trap: Option<u32>,
}

impl TierRun {
    /// One summary line.
    pub fn line(&self) -> String {
        let verdict = if self.equal {
            "output equal to T0".to_string()
        } else if let Some(g) = self.trap {
            format!("trap of guard g{g} (deopt not built)")
        } else {
            format!(
                "output differs: compiled {:?} {:?} vs T0 {:?} {:?}",
                self.compiled.result, self.compiled.output, self.t0.result, self.t0.output
            )
        };
        let probe = match self.probe_trap {
            Some(g) => format!("; probe: a new receiver shape traps at g{g}"),
            None => String::new(),
        };
        format!(
            "{} {} n = {}: {verdict}; T0 {:.3} s, compiled {:.3} s (one run, report only){probe}",
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
/// records if `int` asks for them). Returns the function value, the code and the report; the
/// caller installs the code.
pub fn compile_hot(
    s: &mut Session,
    hot: &kernels::Hot,
    file: &str,
    offset: u32,
    int: IntFacts,
) -> Result<(ep9_vm::Value, spike_b::emit::FunctionCode, CompileReport), String> {
    compile_hot_with(s, hot, file, offset, int, emit_a64::StubForm::Movz)
}

/// [`compile_hot`] with the exit stubs of `form`.
pub fn compile_hot_with(
    s: &mut Session,
    hot: &kernels::Hot,
    file: &str,
    offset: u32,
    int: IntFacts,
    form: emit_a64::StubForm,
) -> Result<(ep9_vm::Value, spike_b::emit::FunctionCode, CompileReport), String> {
    let reach = hot.reach.ok_or_else(|| {
        format!(
            "{}: an inner closure; install by function id is not built",
            hot.name
        )
    })?;
    // The function value first: `function_value` runs a one-line script (a global
    // assignment), which calls nothing, so the feedback the facts come from does not change.
    let f = s.function_value(reach)?;
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
    let input = s.input_of(f)?;
    let (code, report) = crate::compile_input_with(&input, &facts, form)
        .map_err(|r| format!("{}: {r}", hot.name))?;
    Ok((f, code, report))
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
    run_opts(k, n, config, int, emit_a64::StubForm::Movz)
}

/// [`run`] with the int facts of `int` and the exit stubs of `form`.
pub fn run_opts(
    k: &'static Kernel,
    n: u32,
    config: &Config,
    int: IntFacts,
    form: emit_a64::StubForm,
) -> Result<TierRun, String> {
    let hot = k.hot.first().ok_or("no hot function")?;
    if hot.reach.is_none() {
        return Err(format!(
            "{}: an inner closure; install by function id is not built",
            hot.name
        ));
    }
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
    let (f, code, report) = compile_hot_with(&mut s, hot, "defs.js", d1.offset, int, form)?;
    // SAFETY: `code` is the placement (b) code `emit_a64` made for `f` from `input_of(f)` just
    // above (no JS ran since), unchanged. emit_a64 keeps the (b) contract: the T0 handler
    // ABI, Cx and the heap read through `ep9_vm::layout` offsets (`vm_layout`, proved at the
    // start of this run), only RuntimeTable calls, x19-x28 saved and restored, and every
    // exit is the tail jump to `cx.table[*pc]`.
    unsafe { s.install_b(&[f], vec![code]) }?;
    if s.slot_kind(f) != Some(ep9_vm::DispatchKind::Opt) {
        return Err(format!(
            "{}: slot kind {:?} after install",
            hot.name,
            s.slot_kind(f)
        ));
    }
    let t = Instant::now();
    let compiled = s.run(&loop_n, "loop.js");
    let compiled_secs = t.elapsed().as_secs_f64();
    let trap = compiled
        .result
        .as_ref()
        .err()
        .and_then(|e| emit_a64::trap_of(e));
    let equal = compiled == t0 && t0.result.is_ok() && dn.matches(&compiled.output);
    // The probe: `this` of a shape the IC never saw. T0 would compute a number; the compiled
    // function's first shape guard traps.
    let probe_trap = match PROBES.iter().find(|(f, _)| *f == hot.name) {
        Some((_, js)) => {
            let p = s.run(js, "probe.js");
            match &p.result {
                Err(e) => emit_a64::trap_of(e),
                Ok(v) => return Err(format!("{}: the probe returned {v}, no trap", hot.name)),
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
        trap,
        equal,
        t0_secs,
        compiled_secs,
        probe_trap,
    })
}

/// Per hot function: a call whose receiver no shape guard admits (a literal object, a shape
/// the kernel never makes).
const PROBES: [(&str, &str); 1] = [(
    "am3",
    "var __spikeCProbe = { array: [1, 2], other: 3, am: am3 }; \
     __spikeCProbe.am(0, 5, { array: [0, 0] }, 0, 0, 1);",
)];

/// A script-level config C run: `defs` and `warm` run in T0 (the feedback), then the global
/// function `name` is compiled from the live facts and installed, then `main` runs. T0 runs
/// the same three scripts in a session of its own.
#[derive(Clone, Debug)]
pub struct ScriptRun {
    pub t0: RunOutput,
    pub compiled: RunOutput,
    pub report: CompileReport,
    /// The guard whose trap ended `main`, if one did.
    pub trap: Option<u32>,
}

impl ScriptRun {
    /// True if `main` printed the same and ended the same in both runs.
    pub fn equal(&self) -> bool {
        self.t0 == self.compiled
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
    run_script_opts(config, defs, warm, main, name, int, emit_a64::StubForm::Movz)
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
    let f = s.function_value(name)?;
    let (code, report) = {
        let script = s.script("defs.js").ok_or("no defs script")?;
        let ids = script.functions_named(name);
        let [id] = ids[..] else {
            return Err(format!("{name}: {} functions of that name", ids.len()));
        };
        let live = compile_facts(&s, script, id, int)?;
        let facts = facts::for_function(&live, id);
        let input = s.input_of(f)?;
        crate::compile_input_with(&input, &facts, form).map_err(|r| format!("{name}: {r}"))?
    };
    // SAFETY: as in `run`: emit_a64's placement (b) code for `f`, from `input_of(f)` with no
    // JS run since, unchanged.
    unsafe { s.install_b(&[f], vec![code]) }?;
    let compiled = s.run(main, "main.js");
    let trap = compiled
        .result
        .as_ref()
        .err()
        .and_then(|e| emit_a64::trap_of(e));
    Ok(ScriptRun {
        t0,
        compiled,
        report,
        trap,
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
        assert!(r.trap.is_none());
        r
    }

    fn traps(defs: &str, warm: &str, main: &str, name: &str) -> ScriptRun {
        let r = run_script(&Config::default(), defs, warm, main, name).expect("run");
        assert!(
            r.trap.is_some(),
            "{name}: no trap: compiled {:?} vs T0 {:?}",
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
        // `a + i` past 2^31 is a double in JS before `| 0`; the overflow guard traps instead
        // (T0 prints 704982704).
        let r = traps(SUM, warm, "print(sum(100000));", "sum");
        assert_eq!(r.t0.output, ["704982704"]);
        // A double that is an int32 passes the entry check; 2.5 does not (unbacked: the trap).
        equal(
            SUM,
            "sum(3);",
            "print(sum(4.0), sum(Math.floor(7.9)));",
            "sum",
        );
        traps(SUM, "sum(3);", "print(sum(2.5));", "sum");
        traps(SUM, "sum(3);", "print(sum('3'));", "sum");
    }

    /// With int records (the FACTS2 stand-in, and the live records since FACTS2) every int32
    /// input check of `sum` is a fact guard and only the overflow guards of `+` stay unbacked;
    /// the guard basis is data: the code is the same bytes as with no int fact, and a failing
    /// guard still ends in the trap (JIT mode: deopt, month 5).
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
            plain.equal() && plain.trap.is_none(),
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
            r.equal() && r.trap.is_none(),
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
        assert!(t.trap.is_some(), "{:?}", t.compiled);
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
    /// still reads as an int32 hint (the profile leaves it implicit), so its overflow guard is
    /// unbacked; the input checks keep the fact basis.
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
            r.trap.is_some(),
            "the overflow guard traps: {:?}",
            r.compiled
        );
        let g = &r.report.graph;
        for l in g.lines().filter(|l| l.contains(" guard g")) {
            if l.contains("Int32(Add)") {
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

    const ARR: &str = "function fill(a, n) { for (var i = 0; i < n; i++) { a[i] = (a[i] * 3 + i) & 1023; } return a[0]; }";

    #[test]
    fn arrays_equal_t0_and_trap_out_of_bounds() {
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
        // A read past the length or of a hole: the generic path is not built (trap).
        traps(ARR, warm, "print(fill([1, 2], 3));", "fill");
        traps(ARR, warm, "var h = [1, , 3]; print(fill(h, 3));", "fill");
        traps(ARR, warm, "print(fill({0: 1, length: 1}, 1));", "fill");
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

    /// Exit stubs (SPIKEC-3): one stub per guard (12 bytes while the id and the offset fit in
    /// 16 bits) and one shared exit; each failing guard still traps with its own id, so the
    /// stub passed the right one.
    #[test]
    fn each_guard_exits_through_its_own_stub() {
        if !aarch64() {
            return;
        }
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        let oob = traps(ARR, warm, "print(fill([1, 2], 3));", "fill");
        let g = &oob.report.graph;
        let loads = guard_ids(g, "LoadElement");
        let arrays = guard_ids(g, "CheckArray");
        assert!(!loads.is_empty() && !arrays.is_empty(), "{g}");
        assert!(
            loads.contains(&oob.trap.expect("trap")),
            "{:?} {g}",
            oob.trap
        );
        let hole = traps(ARR, warm, "var h = [1, , 3]; print(fill(h, 3));", "fill");
        assert!(
            loads.contains(&hole.trap.expect("trap")),
            "{:?} {g}",
            hole.trap
        );
        let obj = traps(ARR, warm, "print(fill({0: 1, length: 1}, 1));", "fill");
        assert!(
            arrays.contains(&obj.trap.expect("trap")),
            "{:?} {g}",
            obj.trap
        );
        let e = &oob.report.emit;
        assert_eq!(e.traps.len(), e.guards);
        assert_eq!(e.stub_bytes as usize, 12 * e.guards, "{}", e.exits_line());
        assert!(
            e.shared_exit_bytes > 0 && e.shared_exit_bytes <= 80,
            "{}",
            e.exits_line()
        );
        // The stubs sit one after another in guard order.
        for w in e.traps.windows(2) {
            assert_eq!(w[1].1 - w[0].1, 12, "{:?}", e.traps);
        }
    }

    /// The call form of the exit stubs (MC-w15-14): 4 bytes each (`bl` to the dispatch, the
    /// index from the return pc); every failing guard traps with the same id as with the
    /// 12-byte form, and a run with no failing guard gives T0's output.
    #[test]
    fn call_form_stubs_exit_through_the_same_guards() {
        if !aarch64() {
            return;
        }
        use emit_a64::StubForm;
        let warm = "var w = [1, 2, 3, 4]; for (var k = 0; k < 20; k++) fill(w, 4);";
        let run = |main: &str, form| {
            run_script_opts(&Config::default(), ARR, warm, main, "fill", IntFacts::Live, form)
                .expect("run")
        };
        let ok = run("var b = [5, 6, 7, 8, 9]; print(fill(b, 5), b.join());", StubForm::Call);
        assert!(ok.equal() && ok.trap.is_none(), "{:?} vs {:?}", ok.compiled, ok.t0);
        for main in [
            "print(fill([1, 2], 3));",
            "var h = [1, , 3]; print(fill(h, 3));",
            "print(fill({0: 1, length: 1}, 1));",
        ] {
            let movz = run(main, StubForm::Movz);
            let call = run(main, StubForm::Call);
            assert!(movz.trap.is_some(), "{main}");
            assert_eq!(call.trap, movz.trap, "{main}");
            assert_eq!(call.compiled.output, movz.compiled.output, "{main}");
        }
        let e = &ok.report.emit;
        assert_eq!(e.form, StubForm::Call);
        assert_eq!(e.stub_bytes as usize, 4 * e.guards, "{}", e.exits_line());
        assert_eq!(e.table_bytes as usize, 8 * e.guards, "{}", e.exits_line());
        for w in e.traps.windows(2) {
            assert_eq!(w[1].1 - w[0].1, 4, "{:?}", e.traps);
        }
        // The same records either way, except the stub pcs.
        let m = run("print(fill([1, 2], 2));", StubForm::Movz).report.emit.meta;
        assert_eq!(m.exits.len(), e.meta.exits.len());
        for (a, b) in m.exits.iter().zip(&e.meta.exits) {
            assert_eq!((a.guard, a.bytecode, &a.values), (b.guard, b.bytecode, &b.values));
        }
    }

    /// The per-exit metadata (SPIKEC-4): one record per guard, in stub order, at its stub's
    /// pc, with the guard's resume offset and one value per live register and the
    /// accumulator; the encoded table reads back equal.
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
        for (k, (x, &(id, pc))) in m.exits.iter().zip(&e.traps).enumerate() {
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
        assert_eq!(crate::deopt_meta::ExitTable::decode(&bytes).as_ref(), Ok(m));
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
            s.run(&format!("var pad = []; for (var i = 0; i < {pad}; i++) pad.push({{ i: i }});"), "pad.js")
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
            (key, code.code, report.emit.meta.encode())
        };
        let (k1, c1, m1) = compile(0);
        let (k2, c2, m2) = compile(5000);
        assert_ne!(k1, k2, "the two FunctionInfos are at different addresses");
        assert_eq!(c1, c2, "code bytes differ between isolates");
        assert_eq!(m1, m2, "metadata bytes differ between isolates");
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
        // An index of another form (a string) fails the int32 key check: the trap, never a
        // wrong value (T0 prints 8).
        let t = traps(AT, warm, "print(at([7, 8, 9], '1'));", "at");
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
        assert!(!g.contains("CheckArray") && !g.contains("LoadElement"), "{g}");
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
        assert!(g.contains("SetKeyedGeneric") && g.contains("GetKeyedGeneric"), "{g}");
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
        let defs = "function put(a, i, n) { for (var k = 0; k < n; k++) { a[i] = a; } return a[i]; }";
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
        // SAFETY: as in `run`: emit_a64's placement (b) code for `f`, from `input_of(f)` with
        // no JS run since, unchanged.
        unsafe { s.install_b(&[f], vec![code]) }.expect("install");
        s.run(main, "main.js").result.expect("main");
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
            assert_eq!(ok(other, IcState::Uninitialized), KeyedPath::Generic, "{other:?}");
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
        // A third shape: the guard fails (trap), never a wrong field.
        traps(NAMED, warm, "print(get({ w: 1, v: 9 }, 2));", "get");
        traps(NAMED, warm, "print(get(5, 2));", "get");
    }

    #[test]
    fn mul_overflow_and_minus_zero_trap() {
        if !aarch64() {
            return;
        }
        let defs =
            "function m(a, b) { var r = 0; for (var i = 0; i < 1; i++) { r = a * b; } return r; }";
        equal(defs, "m(3, 4);", "print(m(-3, 4), m(46340, 46340));", "m");
        traps(defs, "m(3, 4);", "print(m(65536, 65536));", "m");
        traps(defs, "m(3, 4);", "print(1 / m(0, -5));", "m");
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
        assert_eq!(r.probe_trap, Some(0), "{}", r.line());
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
        // One 12-byte exit stub per guard, one metadata record per exit.
        let e = &r.report.emit;
        assert_eq!(
            (e.guards, e.stub_bytes),
            (29, 29 * 12),
            "{}",
            e.exits_line()
        );
        assert_eq!(e.meta.exits.len(), 29, "{}", e.meta.line());
    }

    /// am3 with the call-form exit stubs (MC-w15-14): crypto still equals T0, the probe
    /// still traps at g0, and the stubs are 4 bytes each.
    #[test]
    fn am3_with_call_stubs_equals_t0() {
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
            emit_a64::StubForm::Call,
        )
        .expect("run");
        assert!(r.equal, "{}", r.line());
        assert_eq!(r.probe_trap, Some(0), "{}", r.line());
        let e = &r.report.emit;
        assert_eq!(
            (e.guards, e.stub_bytes, e.table_bytes),
            (29, 29 * 4, 29 * 8),
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
        assert_eq!(r.probe_trap, Some(0), "{}", r.line());
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
