//! The structural gate of placement (b) (P4-PLACEMENT-B task 9; `gate.sh b` runs
//! `spike-b structural-b all`): the checks of `gate.sh structural` (crate::structural) on
//! placement (b) code from [`crate::jit_b`]. Every session runs T0 with `Dispatch::Sibling`
//! ([`Config::threaded`]): (b) code runs only under a threaded expansion (plan-v6 D4).
//!
//! | Check | What it shows | How |
//! |---|---|---|
//! | ops | the protocol kernels `drive` and `id` (and the other kernels) compile completely | the opcode list of every (b) function; the code has no `ret` (it leaves by tail jumps) and no address (two isolates give the same bytes) |
//! | (a) | (b) output equals T0 output for the 11 opcodes | the loop, caller(f, n) and mixed-recursion kernels, under the default, `--feedback-threshold 0` and GC stress (every allocation, verifier, zapping) |
//! | (b) | T0 -> (b) calls of every call shape, hot and cold; (b) -> T0 and (b) -> (b) | the call shapes and the cold-arm calls (spread, bound, CallProperty with no receiver) equal T0; a host function counts the (b) frames in a native stack walk; Error.stack names the (b) frame |
//! | (c) | an exception crosses a (b) frame to a T0 catch, at the right line | a T0 throw under (b) `g`, and a throw from a (b) runtime call (`Add` -> valueOf); Error.stack equals T0's |
//! | (d) | a native backtrace from a runtime function passes through (b) frames | the walk from a host function under (b) code reaches the frames that called the run |
//! | (e) | the JumpLoop poll and `TerminateHandle` stop a (b) loop | a thread terminates `spin(2e9)`, a loop with no call |
//! | (f) | deep T0 <-> (b) recursion is a catchable RangeError | T0 `trec` -> (b) `jcall` -> call_any -> `trec` ...; the native bytes per level |
//! | (g) | an object held only in a (b) callee-saved register survives a GC; a masked one is collected | the (b) probe ([`crate::jit_b::hold_probe`]) under `call_from_rust` |
//! | (h) | a panic in a runtime function called from (b) code aborts | a child process: SIGABRT (unix) or 0xC0000409 (Windows) |
//!
//! The (b) code generator is built for AArch64 and x86-64 (M3-P0 P0-B-X64), so every check
//! runs on both. Only a target with no generator (`Arch::HOST` is `None`) prints SKIP with the
//! `Refusal` text; `gate.sh b` fails on a SKIP line on aarch64 and x86_64 (P0-D9).

use std::cell::RefCell;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ep9_platform::unwind::native_return_addresses;
use ep9_vm::{Args, Cx, DispatchKind, JsResult, Local, Value};

use crate::emit::{Arch, Refusal};
use crate::harness::{Config, InstallKind, RunOutput, Session, is_jit_pc};
use crate::jit_b::{BEmitter, hold_probe};
use crate::structural::{CALLER_DEFS, LOOP_C_DEFS, LOOP_DEFS, MIXED_DEFS, SPIN_DEFS};

type Check = fn() -> Result<Vec<String>, String>;

/// The checks, in gate order.
pub const CHECKS: [(&str, &str, Check); 8] = [
    (
        "a",
        "(b) output equals T0 (loop, caller, mixed recursion; default, eager feedback, GC stress)",
        check_a,
    ),
    (
        "b",
        "T0->(b) calls of every shape, hot and cold; (b)->T0 and (b)->(b)",
        check_b,
    ),
    (
        "c",
        "an exception crosses a (b) frame to a T0 catch; Error.stack names the (b) line",
        check_c,
    ),
    (
        "d",
        "a native backtrace from a runtime fn passes through (b) frames",
        check_d,
    ),
    (
        "e",
        "the JumpLoop poll and TerminateHandle stop a (b) loop",
        check_e,
    ),
    (
        "f",
        "deep T0<->(b) recursion is a catchable RangeError",
        check_f,
    ),
    (
        "g",
        "an object only in a (b) callee-saved register survives a GC; masked, it is collected",
        check_g,
    ),
    (
        "h",
        "a panic in a runtime fn called from (b) code aborts (child process)",
        check_h,
    ),
];

/// Native stack of a check thread (debug sibling calls grow it per instruction).
const STACK: usize = if cfg!(debug_assertions) {
    256 << 20
} else {
    8 << 20
};

/// Why the checks do not run on this target, if they do not: only when the build has no
/// emitter architecture at all ([`BEmitter`] builds both).
fn refusal() -> Option<Refusal> {
    match Arch::HOST {
        Some(a) => {
            debug_assert!(BEmitter::builds(a));
            None
        }
        None => Some(Refusal::Arch),
    }
}

// ---- kernels (the structural kernels, run with (b) code) ------------------------------------

fn loop_main(n: u32) -> String {
    let m = if n > 1000 { 3_000_000 } else { 300 };
    format!(
        "print(drive(id, {n}));\nprint(drive(id, 0));\nprint(drive(half, 10));\nprint(drive(str, 3));\n\
         print(drive(id, {m}));\n"
    )
}

fn caller_main(n: u32) -> String {
    format!(
        "print(caller(id, {n}));\nprint(caller(sq, 100));\nprint(caller(big, 3));\nprint(caller(str, 3));\n\
         print(caller(caller, 0));\n"
    )
}

fn mixed_main(depth: u32, fib: u32) -> String {
    format!(
        "print(tdown({depth}));\nprint(tfib({fib}));\nprint(twice(inc, 40), twice(inc, 2147483646), twice(inc, '1'));\n\
         print(lt(1, 2), lt(2, 1), lt('b', 'a'), lt(1.5, 2), lt(1, NaN));\nprint(ttwice(50));\n\
         try {{ tthrow(30); }} catch (e) {{ print(e.message); }}\n"
    )
}

const LOOP_JIT: &[&str] = &["id", "drive"];
const CALLER_JIT: &[&str] = &["caller", "id"];
const MIXED_JIT: &[&str] = &["jcall", "twice", "inc", "lt"];

// ---- host functions of the checks ----------------------------------------------------------

thread_local! {
    static MARKS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static WALK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// `jitdepth()`: the number of JIT frames in a native stack walk from here.
fn jitdepth<'s>(
    _cx: &mut Cx<'s>,
    _t: Local<'s>,
    _a: Args<'s>,
    _n: Local<'s>,
) -> JsResult<Local<'s>> {
    let n = native_return_addresses(4096)
        .into_iter()
        .filter(|&ip| is_jit_pc(ip))
        .count();
    Ok(Local::number(n as f64))
}

/// `mark()`: records the native stack pointer.
fn mark<'s>(_cx: &mut Cx<'s>, _t: Local<'s>, _a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    MARKS.with(|m| m.borrow_mut().push(ep9_platform::stack_pointer()));
    Ok(Local::undefined())
}

/// `bt()`: records a native stack walk from here.
fn bt<'s>(_cx: &mut Cx<'s>, _t: Local<'s>, _a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    let ips = native_return_addresses(4096);
    WALK.with(|w| *w.borrow_mut() = ips);
    Ok(Local::undefined())
}

/// `mk()`: a new object `{tag: 42}`.
fn mk<'s>(cx: &mut Cx<'s>, _t: Local<'s>, _a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    let o = cx.new_object();
    cx.set(o, "tag", Local::number(42.0), true)?;
    Ok(o.value())
}

/// Overwrites 256 KiB of stack below the caller (stale words of returned frames).
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    std::hint::black_box(&mut words);
}

/// `scrub()`: [`scrub_stack`].
fn scrub<'s>(_cx: &mut Cx<'s>, _t: Local<'s>, _a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    scrub_stack();
    Ok(Local::undefined())
}

/// `boom()`: panics (check (h)).
fn boom<'s>(_cx: &mut Cx<'s>, _t: Local<'s>, _a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    panic!("boom from a host function under (b) code");
}

/// `bind(f, this, ...args)`: `f.bind(this, ...args)` (the cold arm of a bound callee).
fn bind<'s>(cx: &mut Cx<'s>, _t: Local<'s>, a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    let target = a
        .get(0)
        .as_function()
        .ok_or_else(|| cx.throw_type_error("bind: not a function"))?;
    cx.new_bound_function(target, a.get(1), a.rest(2))
        .map(|f| f.value())
}

// ---- helpers -------------------------------------------------------------------------------

fn session(config: &Config, defs: &str) -> Result<Session, String> {
    let mut s = Session::new(&config.threaded())?;
    for (name, f) in [
        ("jitdepth", jitdepth as ep9_vm::HostFn),
        ("mark", mark),
        ("bt", bt),
        ("mk", mk),
        ("scrub", scrub),
        ("boom", boom),
        ("bind", bind),
    ] {
        s.iso.register_host_function(name, 0, f);
    }
    let d = s.run(defs, "defs.js");
    d.result.map_err(|e| format!("defs: {e}"))?;
    Ok(s)
}

fn expect_ok(r: &RunOutput, what: &str) -> Result<(), String> {
    match &r.result {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("{what}: uncaught {e}\noutput: {:?}", r.output)),
    }
}

/// Runs `defs`, installs `jit` as (b) code (none: all T0), then runs `main`; checks the heap.
fn run_program(config: &Config, defs: &str, jit: &[&str], main: &str) -> Result<RunOutput, String> {
    let mut s = session(config, defs)?;
    if !jit.is_empty() {
        s.jit_b(jit)?;
        for name in jit {
            let k = slot_kind(&s, name)?;
            if k != Some(DispatchKind::Opt) {
                return Err(format!("{name}: slot kind {k:?}, want Opt"));
            }
        }
    }
    let r = s.run(main, "main.js");
    s.iso.collect_garbage();
    s.iso
        .verify_heap()
        .map_err(|e| format!("heap verifier after the run: {:?}", e.errors))?;
    Ok(r)
}

fn slot_kind(s: &Session, name: &str) -> Result<Option<DispatchKind>, String> {
    let f = ep9_vm::NativeCx::global_value(&s.iso, name).ok_or(format!("no global {name}"))?;
    Ok(ep9_vm::native_target(&s.iso, f)
        .ok_or(format!("{name} has no bytecode"))?
        .kind)
}

/// Runs `main` with T0 only and with `jit` as (b) code; both must equal.
fn parity(
    config: &Config,
    defs: &str,
    jit: &[&str],
    main: &str,
    what: &str,
) -> Result<RunOutput, String> {
    let t0 = run_program(config, defs, &[], main)?;
    let b = run_program(config, defs, jit, main)?;
    expect_ok(&t0, what)?;
    if t0 != b {
        return Err(format!(
            "{what}: (b) differs from T0\nT0:\n{}(b):\n{}",
            t0.text(),
            b.text()
        ));
    }
    Ok(b)
}

// ---- the checks ----------------------------------------------------------------------------

/// `structural-b ops`: every kernel function compiles completely with the (b) emitter.
pub fn check_ops() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (k, defs, names) in [
        ("protocol A", LOOP_DEFS, &["id", "drive"][..]),
        ("protocol C", LOOP_C_DEFS, &["drive"][..]),
        ("caller", CALLER_DEFS, CALLER_JIT),
        ("mixed", MIXED_DEFS, MIXED_JIT),
        ("terminate (e)", SPIN_DEFS, &["spin"][..]),
    ] {
        let s = session(&Config::default(), defs)?;
        for n in names {
            let c = s.compile_function_b(n)?;
            out.push(format!(
                "{k}: {n}: {} ops, {} bytes: {}",
                c.ops.len(),
                c.code.len(),
                c.ops_line()
            ));
        }
    }
    Ok(out)
}

fn check_a() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (cname, config) in Config::check_a() {
        let stress = config.gc_stress != 0;
        let kernels = [
            (
                "loop",
                LOOP_DEFS,
                LOOP_JIT,
                loop_main(if stress { 500 } else { 100_000 }),
            ),
            (
                "caller",
                CALLER_DEFS,
                CALLER_JIT,
                caller_main(if stress { 300 } else { 100_000 }),
            ),
            (
                "mixed",
                MIXED_DEFS,
                MIXED_JIT,
                mixed_main(if stress { 60 } else { 400 }, if stress { 10 } else { 18 }),
            ),
        ];
        for (kname, defs, jit, main) in kernels {
            let r = parity(&config, defs, jit, &main, kname)?;
            out.push(format!(
                "{kname:6} {cname:22} (b) [{}] = T0: {}",
                jit.join(", "),
                r.output.join(" | ")
            ));
        }
    }
    Ok(out)
}

const PATHS_DEFS: &str = "\
function g(f, x) { return f(x); }
function h(f, x) { return f(x); }
function t(x) { return jitdepth(); }
function t2(x) { return g(t, x); }
function where(x) { return new Error('w').stack; }
function id(x) { return x; }
var o = { id: id };
function wide(x) { return wide2(x); }
";

/// The call shapes from T0 (one line of `print` each), then the cold-arm calls.
const SHAPES_MAIN: &str = "\
print(id(1), id(2, 3), id(), id(4, 5, 6), id(7, 8, 9, 10));
print(o.id(11), o.id(), o.id(12, 13), o.id(14, 15, 16), o.id(17, 18, 19, 20));
print(id(...[21]), id(...[]), o.id(...[22, 23]));
var bid = bind(id, null, 24);
print(bid(), bid(25));
var s = 0; for (var i = 0; i < 25000; i++) { s = s + id(i); } print(s);
print(wide(26));
";

/// [`PATHS_DEFS`] and `wide2(x)`: 300 locals, so its call of `id` has a Wide prefix.
fn paths_defs() -> String {
    let mut d = String::from(PATHS_DEFS);
    d.push_str("function wide2(x) {\n");
    for i in 0..300 {
        d.push_str(&format!("  var l{i} = x;\n"));
    }
    d.push_str("  return id(l299);\n}\n");
    d
}

fn check_b() -> Result<Vec<String>, String> {
    let defs = paths_defs();
    let config = Config::default();
    // Every shape: (b) = T0.
    let r = parity(
        &config,
        &defs,
        &["g", "h", "id"],
        SHAPES_MAIN,
        "call shapes",
    )?;
    let mut out = vec![format!("call shapes (b) = T0: {}", r.output.join(" | "))];
    // The Wide call is really prefixed.
    {
        let s = session(&config, &defs)?;
        let f = ep9_vm::NativeCx::global_value(&s.iso, "wide2").ok_or("no wide2")?;
        let t = ep9_vm::native_target(&s.iso, f).ok_or("wide2: no bytecode")?;
        let wide = ep9_bytecode::Instructions::new(t.bytecode)
            .filter_map(Result::ok)
            .any(|i| {
                i.opcode == ep9_bytecode::Opcode::CallUndefinedReceiver1
                    && i.scale != ep9_bytecode::OperandScale::Single
            });
        if !wide {
            return Err("wide2's call of id has no Wide prefix".into());
        }
    }
    let main = "print(jitdepth());\nprint(g(t, 0));\nprint(g(h, t));\nprint(g(t2, 0));\nprint(g(where, 0));\n";
    let mut s = session(&config, &defs)?;
    s.jit_b(&["g", "h"])?;
    for name in ["g", "h"] {
        let k = slot_kind(&s, name)?;
        if k != Some(DispatchKind::Opt) {
            return Err(format!("{name} slot kind {k:?}"));
        }
    }
    let r = s.run(main, "main.js");
    expect_ok(&r, "paths")?;
    let want = ["0", "1", "2", "2"];
    if r.output[..4] != want {
        return Err(format!(
            "(b) frame counts {:?}, want {want:?}",
            &r.output[..4]
        ));
    }
    let stack = &r.output[4];
    if !(stack.contains("at where (defs.js:5:") && stack.contains("at g (defs.js:1:")) {
        return Err(format!("Error.stack misses the (b) frame:\n{stack}"));
    }
    out.extend([
        "hot: CallUndefinedReceiver0-2 and CallUndefinedReceiver, CallProperty0-2 and CallProperty, a Wide call (scaled step); cold arm: o.id() with no receiver list, spread, bound, the budget polls of a 25000-call loop".to_string(),
        "T0 only: 0 (b) frames".into(),
        "T0->(b) g->T0 t: 1 (b) frame in the native walk ((b)->T0 through call_any)".into(),
        "T0->(b) g->(b) h->T0 t: 2 (b) frames ((b)->(b))".into(),
        "T0->(b) g->T0 t2->(b) g->T0 t: 2 (b) frames".into(),
        format!("Error.stack under (b) g: {}", stack.replace('\n', " | ")),
    ]);
    Ok(out)
}

const EXC_DEFS: &str = "\
function thrower(x) { throw new Error('boom ' + x); }
function g(f, x) {
  return f(x);
}
function addv(a, b) {
  return a + b;
}
var bad = { valueOf() { throw new Error('valueOf ' + 1); } };
";

fn check_c() -> Result<Vec<String>, String> {
    let main = "try { g(thrower, 1); } catch (e) { print(e.message); print(e.stack); }\nprint('after');\n\
                try { g(g, thrower); } catch (e) { print(e.stack); }\n\
                try { addv(1, bad); } catch (e) { print(e.stack); }\nprint(addv(2, 3));\n";
    let r = parity(
        &Config::default(),
        EXC_DEFS,
        &["g", "addv"],
        main,
        "exceptions",
    )?;
    // `return f(x);` is line 3, column 10; `a + b` line 6, column 10 of defs.js.
    let line = "    at g (defs.js:3:10)";
    if !r.output[1].lines().any(|l| l == line) || r.output[2] != "after" {
        return Err(format!("no `{line}` in:\n{}", r.output[1]));
    }
    if !r.output[4]
        .lines()
        .any(|l| l.starts_with("    at addv (defs.js:6:"))
    {
        return Err(format!("no `at addv (defs.js:6:..)` in:\n{}", r.output[4]));
    }
    Ok(vec![
        format!("caught: {}", r.output[0]),
        format!(
            "T0 throw under (b) g, Error.stack (equals T0): {}",
            r.output[1].replace('\n', " | ")
        ),
        format!("two (b) frames: {}", r.output[3].replace('\n', " | ")),
        format!(
            "throw from a (b) runtime call (Add -> valueOf): {}",
            r.output[4].replace('\n', " | ")
        ),
    ])
}

/// The native return addresses of this function's callers (taken in a frame of its own).
#[inline(never)]
fn outer_walk() -> Vec<usize> {
    native_return_addresses(4096)
}

#[inline(never)]
fn check_d() -> Result<Vec<String>, String> {
    let mut s = session(&Config::default(), &paths_defs())?;
    s.jit_b(&["g", "h"])?;
    let outer = outer_walk();
    let r = s.run("g(h, bt);\n", "main.js");
    expect_ok(&r, "backtrace")?;
    let inner = WALK.with(|w| std::mem::take(&mut *w.borrow_mut()));
    let jit: Vec<usize> = (0..inner.len())
        .filter(|&i| s.is_jit_pc(inner[i]))
        .collect();
    let common = inner
        .iter()
        .rev()
        .zip(outer.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let last_jit = *jit
        .last()
        .ok_or_else(|| format!("no (b) frame in the walk ({} frames)", inner.len()))?;
    if jit.len() != 2 {
        return Err(format!(
            "{} (b) frames in the walk, want 2 (g and h)",
            jit.len()
        ));
    }
    if common < 2 || inner.len() - common <= last_jit {
        return Err(format!(
            "the walk stops at the (b) frames: {} frames, (b) at {jit:?}, {common} frames shared with the caller",
            inner.len()
        ));
    }
    Ok(vec![format!(
        "walk from bt(): {} frames, (b) frames at {jit:?}, then {} more; the last {common} equal the frames that called the run",
        inner.len(),
        inner.len() - last_jit - 1
    )])
}

fn check_e() -> Result<Vec<String>, String> {
    let mut s = session(&Config::default(), SPIN_DEFS)?;
    s.jit_b(&["spin"])?;
    let warm = s.run("print(spin(1000));", "warm.js");
    expect_ok(&warm, "warm-up")?;
    let h = s.iso.terminate_handle();
    let delay = Duration::from_millis(200);
    let t = std::thread::spawn(move || {
        std::thread::sleep(delay);
        h.terminate();
    });
    let start = Instant::now();
    let r = s.run("spin(2000000000);", "spin.js");
    let took = start.elapsed();
    t.join().map_err(|_| "terminator thread panicked")?;
    match &r.result {
        Err(e) if e.contains("terminated") => {}
        other => {
            return Err(format!(
                "spin(2e9) was not terminated: {other:?} after {took:?}"
            ));
        }
    }
    let again = s.run("print(spin(5));", "again.js");
    expect_ok(&again, "after the termination")?;
    if again.output != ["5"] {
        return Err(format!("after the termination: {:?}", again.output));
    }
    Ok(vec![
        format!("spin(2e9) in (b) code terminated {took:?} after the start (request at {delay:?})"),
        "the isolate runs (b) code again after the termination: spin(5) = 5".into(),
    ])
}

const REC_DEFS: &str = "\
var depth = 0;
function jcall(f, x) { return f(x); }
function trec(n) { if (n === 10 || n === 110) mark(); depth = n; return jcall(trec, n + 1); }
";

fn check_f() -> Result<Vec<String>, String> {
    let main = "try { trec(0); } catch (e) { print(e instanceof RangeError); print(e.message); }\n\
                print(depth);\nprint('after');\n";
    let mut out = Vec::new();
    for (label, jit) in [("T0<->(b)", &["jcall"][..]), ("T0 only", &[][..])] {
        MARKS.with(|m| m.borrow_mut().clear());
        let mut s = session(&Config::default(), REC_DEFS)?;
        if !jit.is_empty() {
            s.jit_b(jit)?;
        }
        let r = s.run(main, "main.js");
        expect_ok(&r, label)?;
        if r.output.len() != 4 || r.output[0] != "true" || r.output[3] != "after" {
            return Err(format!("{label}: no catchable RangeError: {:?}", r.output));
        }
        let marks = MARKS.with(|m| m.borrow().clone());
        let per_level = match marks.as_slice() {
            [a, b] => (a - b) as f64 / 100.0,
            _ => return Err(format!("{label}: marks {marks:?}, output {:?}", r.output)),
        };
        let depth: u64 = r.output[2].parse().map_err(|_| "depth")?;
        out.push(format!(
            "{label}: caught RangeError ({}) at depth {depth}; native stack {per_level:.0} bytes per level \
             (one T0 trec + one {} jcall); used about {} KiB of the {} KiB thread stack",
            r.output[1],
            if jit.is_empty() { "T0" } else { "(b)" },
            (per_level * depth as f64 / 1024.0) as u64,
            STACK >> 10
        ));
    }
    Ok(out)
}

const HOLD_DEFS: &str = "\
function hold(mk, scrub, gc) { let o = mk(); scrub(); gc(); return o; }
function tagof(o) { return o.tag; }
";

/// The masked word of check (g) (as the structural check).
const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Runs the (b) (g) probe with `mask`; returns `(result bits, conservative roots, session)`.
#[inline(never)]
fn run_hold(mask: u64) -> Result<(u64, Vec<usize>, Session), String> {
    let config = Config {
        record_roots: true,
        zap: true,
        ..Config::default()
    };
    let mut s = session(&config, HOLD_DEFS)?;
    let input = s.function_input("hold")?;
    let arch = Arch::HOST.ok_or("no host arch")?;
    let code = hold_probe(arch, &input, mask).map_err(|e| e.to_string())?;
    s.install_as(InstallKind::B, &["hold"], vec![code])?;
    let g =
        |s: &Session, n: &str| ep9_vm::NativeCx::global_value(&s.iso, n).ok_or(format!("no {n}"));
    let (hold, mk, scrub, gc) = (g(&s, "hold")?, g(&s, "mk")?, g(&s, "scrub")?, g(&s, "gc")?);
    let r = ep9_vm::call_from_rust(&mut s.iso, hold, Value::UNDEFINED, &[mk, scrub, gc])
        .map_err(|e| format!("hold: {}", e.message))?;
    let roots = s
        .iso
        .heap()
        .last_conservative_roots()
        .iter()
        .map(|c| c.addr())
        .collect();
    Ok((r.to_bits(), roots, s))
}

fn check_g() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let (bits, roots, mut s) = run_hold(0)?;
    let addr = bits as usize;
    if !s.iso.heap().is_cell(addr) {
        return Err(format!("the held object {addr:#x} was collected"));
    }
    if !roots.contains(&addr) {
        return Err(format!(
            "the conservative scan did not find the held object {addr:#x}"
        ));
    }
    // SAFETY: `bits` is the object `mk` made, alive (checked above).
    let obj = unsafe { Value::from_bits_unchecked(bits) };
    let tagof = ep9_vm::NativeCx::global_value(&s.iso, "tagof").ok_or("no tagof")?;
    // `tagof` runs in T0: `call_from_rust` takes the T0 path of `call::call_with` for it.
    let tag = ep9_vm::call_from_rust(&mut s.iso, tagof, Value::UNDEFINED, &[obj])
        .map_err(|e| e.message)?;
    if tag.as_int32() != Some(42) {
        return Err(format!("the held object reads tag = {tag:?}"));
    }

    s.iso.verify_heap().map_err(|e| format!("{:?}", e.errors))?;
    out.push(format!(
        "held in the (b) callee-saved register: {addr:#x} survived gc() (found by the conservative scan), tag = 42"
    ));
    drop(s);
    for round in 0..5 {
        let (masked, roots, s) = run_hold(MASK)?;
        let addr = (masked ^ MASK) as usize;
        if roots.contains(&addr) {
            return Err(format!(
                "round {round}: the conservative scan found the masked object {addr:#x}"
            ));
        }
        if s.iso.heap().is_cell(addr) {
            return Err(format!(
                "round {round}: the masked object {addr:#x} survived gc()"
            ));
        }
    }
    out.push("masked (XOR 0x5a5a...): the object was collected by gc(), 5 of 5 rounds".into());
    Ok(out)
}

/// The environment variable that marks the child of check (h).
const H_CHILD: &str = "SPIKE_B_BH_CHILD";

/// The child of check (h): (b) `g` calls the host function `boom`, which panics inside
/// `call_any`; the panic must abort at the table function's non-unwinding ABI boundary (or at
/// once with `panic = "abort"`).
fn h_child() -> ! {
    let r = (|| -> Result<(), String> {
        let mut s = session(&Config::default(), &paths_defs())?;
        s.jit_b(&["g"])?;
        let _ = s.run("g(boom, 1);", "main.js");
        Ok(())
    })();
    eprintln!("spike-b: the panic did not abort: {r:?}");
    std::process::exit(3)
}

fn check_h() -> Result<Vec<String>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(&exe)
        .args(["structural-b", "h-child"])
        .env(H_CHILD, "1")
        .output()
        .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.contains("boom from a host function under (b) code") {
        return Err(format!("the child's stderr has no panic message: {stderr}"));
    }
    let unwind = !cfg!(panic = "abort");
    if unwind && !stderr.contains("panic in a function that cannot unwind") {
        return Err(format!(
            "unwind build: no non-unwinding boundary abort: {stderr}"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if out.status.signal() != Some(6) {
            return Err(format!(
                "the child was not killed by SIGABRT: {:?}\n{stderr}",
                out.status
            ));
        }
    }
    #[cfg(windows)]
    if out.status.code() != Some(0xC000_0409_u32 as i32) {
        return Err(format!(
            "the child did not end with 0xC0000409: {:?}\n{stderr}",
            out.status
        ));
    }
    Ok(vec![format!(
        "{} build: the child ended with {:?} after the panic in boom() under (b) g",
        if unwind {
            "panic=unwind (abort at the extern boundary)"
        } else {
            "panic=abort"
        },
        out.status
    )])
}

// ---- driver --------------------------------------------------------------------------------

/// Runs `check` on a thread with [`STACK`] bytes of native stack.
fn on_thread(check: Check) -> Result<Vec<String>, String> {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(check)
        .map_err(|e| e.to_string())?
        .join()
        .unwrap_or_else(|p| {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            Err(format!("panicked: {msg}"))
        })
}

/// `spike-b structural-b <all | ops | a..h | h-child>`.
pub fn main(args: &[String]) -> ExitCode {
    let which = args.first().map(String::as_str).unwrap_or("all");
    if which == "h-child" {
        if std::env::var_os(H_CHILD).is_none() {
            eprintln!("spike-b: h-child runs only under check (h)");
            return ExitCode::from(2);
        }
        h_child();
    }
    let selected: Vec<(&str, &str, Check)> = match which {
        "all" => std::iter::once((
            "ops",
            "the kernels compile completely with the (b) emitter; their opcode lists",
            check_ops as Check,
        ))
        .chain(CHECKS)
        .collect(),
        "ops" => vec![(
            "ops",
            "the kernels compile completely with the (b) emitter; their opcode lists",
            check_ops as Check,
        )],
        c => match CHECKS.iter().find(|(n, _, _)| *n == c) {
            Some(&row) => vec![row],
            None => {
                eprintln!("spike-b structural-b: unknown check {c:?} (all, ops, a..h)");
                return ExitCode::from(2);
            }
        },
    };
    let mut fail = 0;
    let skip = refusal();
    for (name, title, check) in selected {
        if let Some(r) = &skip {
            println!(
                "check ({name}) SKIP: {title}: {r} ({})",
                std::env::consts::ARCH
            );
            continue;
        }
        let start = Instant::now();
        match on_thread(check) {
            Ok(lines) => {
                println!("check ({name}) PASS: {title} [{:.1?}]", start.elapsed());
                for l in lines {
                    println!("    {l}");
                }
            }
            Err(e) => {
                fail += 1;
                println!("check ({name}) FAIL: {title}");
                for l in e.lines() {
                    println!("    {l}");
                }
            }
        }
    }
    println!(
        "structural-b: {} on {} ({}, {}, T0 dispatch Sibling)",
        match (fail, &skip) {
            (0, Some(_)) => "SKIP (placement (b) not built for this target)",
            (0, None) => "PASS",
            _ => "FAIL",
        },
        std::env::consts::ARCH,
        std::env::consts::OS,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
