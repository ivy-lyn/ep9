//! The M6-P1 JIT review tests of the SPIKEC-9 ops (the reviewer's experiments, the gaps of
//! the corpus): unfused double compares, ToInt32 of a double at the +/-2^31 edges, global
//! loads (a getter, a TDZ read, a `let` that shadows a global property). Each function runs
//! in several builds, and each output must equal T0:
//!
//! ```text
//!  unforced | int32 junk (bits 32-63 of int32 registers) | forced seeds 1..8 | GC stress
//! ```
//!
//! The F1 pattern (a kind guard in an `if` arm) is
//! `tier::tests::a_kind_guard_in_an_if_arm_stays_in_the_loop`.

use spike_b::harness::Config;

use super::{FORCE_LEN, IntFacts, ScriptRun, run_script, run_script_full};
use crate::emit_a64::{EmitOptions, ForceTable};

/// A function compiled with a Mac-only emitter: skip elsewhere.
fn aarch64() -> bool {
    if spike_b::emit::Arch::HOST == Some(spike_b::emit::Arch::Aarch64) {
        return true;
    }
    eprintln!("skip: the emitter is aarch64 only");
    false
}

/// Runs `name` compiled in each build of the module doc; returns the unforced run and the
/// list of differences from T0 (and of errors).
fn check(tag: &str, defs: &str, warm: &str, main: &str, name: &str) -> (ScriptRun, Vec<String>) {
    let mut bad = Vec::new();
    let mut add = |what: String, r: Result<ScriptRun, String>| match r {
        Ok(r) if !r.equal() => bad.push(format!(
            "[{tag}] {what} differs: T0 {:?} {:?} vs C {:?} {:?}",
            r.t0.result, r.t0.output, r.compiled.result, r.compiled.output
        )),
        Ok(_) => {}
        Err(e) => bad.push(format!("[{tag}] {what}: error {e}")),
    };
    let junk = EmitOptions {
        int32_junk: true,
        ..Default::default()
    };
    let d = Config::default();
    add(
        "junk".into(),
        run_script_full(&d, defs, warm, main, name, IntFacts::Live, &junk),
    );
    for seed in 1..=8u64 {
        let t = ForceTable::seeded(seed, FORCE_LEN);
        let opts = EmitOptions {
            force: Some(&t),
            ..Default::default()
        };
        add(
            format!("forced seed {seed}"),
            run_script_full(&d, defs, warm, main, name, IntFacts::Live, &opts),
        );
    }
    add(
        "stress".into(),
        run_script(&Config::stress(), defs, warm, main, name),
    );
    let r = run_script(&d, defs, warm, main, name).unwrap_or_else(|e| panic!("[{tag}] {e}"));
    if !r.equal() {
        bad.push(format!(
            "[{tag}] unforced differs: T0 {:?} {:?} vs C {:?} {:?}",
            r.t0.result, r.t0.output, r.compiled.result, r.compiled.output
        ));
    }
    (r, bad)
}

/// Fails with every difference of `bad`; the unforced graph of each run in `runs` must
/// contain `op` (the test covers that op).
fn report(bad: Vec<String>, runs: &[ScriptRun], op: &str) {
    for b in &bad {
        eprintln!("MISMATCH {b}");
    }
    assert!(bad.is_empty(), "{} mismatches: {bad:?}", bad.len());
    for r in runs {
        assert!(r.report.graph.contains(op), "no {op}\n{}", r.report.graph);
    }
}

const CMP: &str = "\
function lt(x, y) { return x * 1 < y * 1; }\n\
function le(x, y) { return x * 1 <= y * 1; }\n\
function gt(x, y) { return x * 1 > y * 1; }\n\
function ge(x, y) { return x * 1 >= y * 1; }\n\
function eq(x, y) { return x * 1 === y * 1; }\n";

/// A double compare whose Bool is a value (not fused into a branch): all 81 pairs of
/// {0.5, -0.5, 0, -0, NaN, +/-Infinity, 1.5, 3} for <, <=, >, >=, ===.
#[test]
fn unfused_double_compares_equal_t0() {
    if !aarch64() {
        return;
    }
    let main = "\
var vals = [0.5, -0.5, 0, -0, NaN, Infinity, -Infinity, 1.5, 3];\n\
var out = [];\n\
for (var p = 0; p < vals.length; p++) for (var q = 0; q < vals.length; q++)\n\
  out.push(F(vals[p], vals[q]) ? 1 : 0);\n\
print(out.join(''));\n";
    let (mut bad, mut runs) = (Vec::new(), Vec::new());
    for f in ["lt", "le", "gt", "ge", "eq"] {
        let warm =
            format!("for (var w = 0; w < 300; w++) {{ {f}(w * 0.5, 7.25); {f}(1.5, w * 0.25); }}");
        let (r, b) = check(f, CMP, &warm, &main.replace('F', f), f);
        bad.extend(b);
        runs.push(r);
    }
    report(bad, &runs, "CompareFloat64");
}

const TRUNC: &str = "\
function t1(x, y) { return (x * y) | 0; }\n\
function tsum(a, n, y) { var s = 0; for (var i = 0; i < n; i++) { s = s ^ ((a[i] * y) | 0); } return s; }\n";

/// `x | 0` of a double (TruncateFloat64; `x * y`, not `x * 1`: MulSmi has no double form):
/// -0, +/-0.5, the +/-2^31 edges (2147483647.9,
/// -2147483648.9, 2147483648, -2147483649), +/-2^32 values, NaN, +/-Infinity, 1e-300.
#[test]
fn to_int32_of_a_double_at_the_edges_equals_t0() {
    if !aarch64() {
        return;
    }
    let warm =
        "for (var w = 0; w < 300; w++) { t1(w * 0.75, 1); tsum([0.5, 1.5, w * 0.25], 3, 1); }";
    let main = "\
var vals = [0.5, -0.5, -0, 0, 1.9, -1.9, 2147483647.9, -2147483648.9, 2147483648, -2147483649,\n\
  4294967296.5, 4294967295, -4294967297, 1e10, -1e10, NaN, Infinity, -Infinity, 3.000000001, 1e-300];\n\
var out = [];\n\
for (var p = 0; p < vals.length; p++) out.push(t1(vals[p], 1), 1 / t1(vals[p], 1));\n\
print(out.join(' '));\n\
print(tsum(vals, vals.length, 1), tsum([0.5, -0.5, 7.9], 3, 1));\n";
    let (r1, mut bad) = check("t1", TRUNC, warm, main, "t1");
    let (r2, b) = check("tsum", TRUNC, warm, main, "tsum");
    bad.extend(b);
    report(bad, &[r1, r2], "TruncateFloat64");
}

const GLOB: &str = "\
var G = 1.5;\n\
var cnt = 0;\n\
function gl(n) { var s = 0; for (var i = 0; i < n; i++) s = s + G; return s; }\n\
function gz() { return Z; }\n\
function gh() { return H; }\n";

/// Global loads (RuntimeTable entry 11): a value that changes and becomes a string, a deleted
/// global, a getter on the global object (its run count too), a TDZ read of a script `let`,
/// and a `let` that shadows a global property.
#[test]
fn global_loads_getter_tdz_and_let_shadow_equal_t0() {
    if !aarch64() {
        return;
    }
    let warm =
        "globalThis.Z = 4; globalThis.H = 9; for (var w = 0; w < 300; w++) { gl(3); gz(); gh(); }";
    let main_gl = "print(gl(4)); G = 2.25; print(gl(4)); G = 'x'; try { print(gl(2)); } catch (e) { print('E', e.name); }\n\
        delete globalThis.G; try { print(gl(1)); } catch (e) { print('E', e.name, e.message); }\n";
    let main_gh = "Object.defineProperty(globalThis, 'H', { get: function () { cnt++; return cnt * 10; }, configurable: true });\n\
        print(gh(), gh(), cnt); delete globalThis.H; try { gh(); } catch (e) { print('E', e.name, e.message); }\n";
    let main_gz = "print(gz()); let Z = 7; print(gz()); Z = 8; print(gz());\n";
    let main_tdz = "try { print(gz()); } catch (e) { print('E', e.name, e.message); } let Z = 7; print(gz());\n";
    let mut bad = Vec::new();
    let mut runs = Vec::new();
    for (tag, main, name) in [
        ("gl", main_gl, "gl"),
        ("gh", main_gh, "gh"),
        ("gz", main_gz, "gz"),
        ("gz-tdz", main_tdz, "gz"),
    ] {
        let (r, b) = check(tag, GLOB, warm, main, name);
        bad.extend(b);
        runs.push(r);
    }
    report(bad, &runs, "LoadGlobal");
}

/// A global getter that calls a function with many locals, between two global loads of a
/// compiled function whose parameters stay live across them.
#[test]
fn a_global_getter_keeps_the_callers_frame() {
    if !aarch64() {
        return;
    }
    let defs = "function busy(p, q) { var a1 = p + 1, a2 = p + 2, a3 = p + 3, a4 = p + 4, a5 = q + 5, a6 = q + 6, a7 = 7, a8 = 8; return a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8; }\n\
        var GH2v = 1; Object.defineProperty(globalThis, 'GH2', { get: function () { return busy(100, 200) + GH2v; }, configurable: true });\n\
        function gp(a, b) { var x = a * 2; var t = GH2; var u = GH2; return a + b + t + x + u; }\n";
    let warm = "for (var w = 0; w < 300; w++) { gp(w, 1); }";
    let main = "print(gp(3, 4), gp(5, 6)); GH2v = 0.5; print(gp(1, 2));\n";
    let (r, bad) = check("gp", defs, warm, main, "gp");
    report(bad, &[r], "LoadGlobal");
}
