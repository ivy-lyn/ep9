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
    let hot = k.hot.first().ok_or("no hot function")?;
    let reach = hot.reach.ok_or_else(|| {
        format!(
            "{}: an inner closure; install by function id is not built",
            hot.name
        )
    })?;
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
    // The layout copies, on a session of their own (so both runs see the same scripts).
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
    // The function value first: `function_value` runs a one-line script (a global
    // assignment), which calls nothing, so the feedback the facts come from does not change.
    let f = s.function_value(reach)?;
    let (code, report) = {
        let script = s.script("defs.js").ok_or("no defs script")?;
        let ids = script.functions_on_line(hot.lines.0 + d1.offset);
        let [id] = ids[..] else {
            return Err(format!(
                "{}: {} functions on line {}",
                hot.name,
                ids.len(),
                hot.lines.0
            ));
        };
        let live = facts::live(&s, script)?;
        let facts = facts::for_function(&live, id);
        let input = s.input_of(f)?;
        crate::compile_input(&input, &facts).map_err(|r| format!("{}: {r}", hot.name))?
    };
    // SAFETY: `code` is the placement (b) code `emit_a64` made for `f` from `input_of(f)` just
    // above (no JS ran since), unchanged. emit_a64 keeps the (b) contract: the T0 handler
    // ABI, Cx read through `ep9_vm::layout` offsets (and the `vm_layout` copies, checked at
    // the start of this run), only RuntimeTable calls, x19-x28 saved and restored, and every
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
        let live = facts::live(&s, script)?;
        let facts = facts::for_function(&live, id);
        let input = s.input_of(f)?;
        crate::compile_input(&input, &facts).map_err(|r| format!("{name}: {r}"))?
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
        assert_eq!(r.report.fact_guards, 0);
        // `a + i` past 2^31 is a double in JS before `| 0`; the unbacked overflow guard
        // traps instead (T0 prints 704982704).
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
        assert_eq!(r.report.fact_guards, 1, "{}", r.report.graph);
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
        assert_eq!(r.report.fact_guards, 2, "{}", r.report.graph);
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
