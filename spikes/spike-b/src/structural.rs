//! The structural gate of Spike B (plan-v4 §5 P2-SPIKE-B task 4): `gate.sh structural` runs
//! `spike-b structural all` (checks (a)-(h)) and `spike-b structural ops`.
//!
//! | Check | What it shows | How |
//! |---|---|---|
//! | ops | the protocol kernels `drive` (A and C) and `id` JIT completely | the opcode list of every JITed function |
//! | (a) | JIT output equals T0 output | the loop, caller(f, n) and mixed-recursion kernels, under the default, `--feedback-threshold 0` and GC stress (every allocation, verifier, zapping) |
//! | (b) | T0→JIT, JIT→T0 and JIT→JIT run | a host function counts the JIT frames in a native stack walk; Error.stack names the JIT frame |
//! | (c) | an exception from a T0 callee crosses a JIT frame to a T0 catch, at the right line | Error.stack equals T0's, with `at g (defs.js:3:10)` |
//! | (d) | a native backtrace from a runtime function passes through JIT frames | the walk from a host function under JIT code reaches the frames that called the run |
//! | (e) | `TerminateHandle` stops a JIT loop | a thread terminates `spin(2e9)`, a loop with no call |
//! | (f) | deep T0↔JIT recursion is a catchable RangeError | the native bytes per level are reported |
//! | (g) | an object held only in a JIT callee-saved register survives a GC; a masked one is collected | a hand-written probe (`jit::hold_probe`) under `call_from_rust` |
//! | (h) | a panic in a runtime function called from JIT code aborts | a child process: SIGABRT (unix) or 0xC0000409 (Windows), in this build's panic mode |
//!
//! Every check runs on its own thread with a known stack size. A check returns its report
//! lines or the reason it failed.

use std::cell::RefCell;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ep9_platform::unwind::native_return_addresses;
use ep9_vm::{Args, Cx, JsResult, Local, Value};

use crate::harness::{Config, RunOutput, Session, is_jit_pc, run_program};
use crate::jit::hold_probe;

type Check = fn() -> Result<Vec<String>, String>;

/// The checks, in gate order.
pub const CHECKS: [(&str, &str, Check); 8] = [
    (
        "a",
        "JIT output equals T0 (loop, caller, mixed recursion; default, eager feedback, GC stress)",
        check_a,
    ),
    ("b", "the paths T0->JIT, JIT->T0 and JIT->JIT run", check_b),
    (
        "c",
        "a T0 exception crosses a JIT frame to a T0 catch; Error.stack names the JIT line",
        check_c,
    ),
    (
        "d",
        "a native backtrace from a runtime fn passes through JIT frames",
        check_d,
    ),
    ("e", "TerminateHandle stops a JIT loop", check_e),
    (
        "f",
        "deep T0<->JIT recursion is a catchable RangeError",
        check_f,
    ),
    (
        "g",
        "an object only in a JIT callee-saved register survives a GC; masked, it is collected",
        check_g,
    ),
    (
        "h",
        "a panic in a runtime fn called from JIT code aborts (child process)",
        check_h,
    ),
];

/// Native stack of a check thread.
const STACK: usize = if cfg!(debug_assertions) {
    64 << 20
} else {
    8 << 20
};

// ---- kernels -------------------------------------------------------------------------------

/// Protocol kernel A (docs/measurements/2026-10-22-spike-b-protocol.md §2) and two T0
/// callees that leave the int32 fast paths (a double and a string accumulator).
pub const LOOP_DEFS: &str = "\
function id(x) { return x; }
function drive(f, n) {
  let a = 0;
  for (let i = 0; i < n; i++) { a = f(a); a = a + 1; }
  return a;
}
function half(x) { return x + 0.5; }
function str(x) { return x + 'x'; }
";

/// Protocol kernel C: kernel A without the call statement.
pub const LOOP_C_DEFS: &str = "\
function drive(f, n) {
  let a = 0;
  for (let i = 0; i < n; i++) { a = a + 1; }
  return a;
}
";

fn loop_main(n: u32) -> String {
    let m = if n > 1000 { 3_000_000 } else { 300 };
    format!(
        "print(drive(id, {n}));\nprint(drive(id, 0));\nprint(drive(half, 10));\nprint(drive(str, 3));\n\
         print(drive(id, {m}));\n"
    )
}

/// `caller(f, n)`: a JIT loop that calls `f` and adds with `Add` (not `AddSmi`).
pub const CALLER_DEFS: &str = "\
function caller(f, n) {
  let s = 0;
  for (let i = 0; i < n; i++) { s = s + f(i); }
  return s;
}
function id(x) { return x; }
function sq(x) { return x * x; }
function big(x) { return x + 2147483000; }
function str(x) { return '<' + x + '>'; }
";

fn caller_main(n: u32) -> String {
    format!(
        "print(caller(id, {n}));\nprint(caller(sq, 100));\nprint(caller(big, 3));\nprint(caller(str, 3));\n\
         print(caller(caller, 0));\n"
    )
}

/// Mixed recursion: T0 functions recurse through JIT `jcall`; `twice(inc, x)` is JIT→JIT.
pub const MIXED_DEFS: &str = "\
function jcall(f, x) { return f(x); }
function twice(f, x) { return f(f(x)); }
function inc(x) { return ++x; }
function lt(a, b) { return a < b; }
function tdown(n) { if (n <= 0) return 0; return jcall(tdown, n - 1) + 1; }
function tfib(n) { if (n < 2) return n; return jcall(tfib, n - 1) + jcall(tfib, n - 2); }
function tthrow(n) { if (n <= 0) throw new Error('bottom ' + n); return jcall(tthrow, n - 1); }
function ttwice(n) { if (n <= 0) return 0; return twice(inc, jcall(ttwice, n - 1)); }
";

fn mixed_main(depth: u32, fib: u32) -> String {
    format!(
        "print(tdown({depth}));\nprint(tfib({fib}));\nprint(twice(inc, 40), twice(inc, 2147483646), twice(inc, '1'));\n\
         print(lt(1, 2), lt(2, 1), lt('b', 'a'), lt(1.5, 2), lt(1, NaN));\nprint(ttwice(50));\n\
         try {{ tthrow(30); }} catch (e) {{ print(e.message); }}\n"
    )
}

/// The JIT set of each kernel.
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
    panic!("boom from a host function under JIT code");
}

// ---- helpers -------------------------------------------------------------------------------

fn session(config: &Config, defs: &str) -> Result<Session, String> {
    let mut s = Session::new(config)?;
    for (name, f) in [
        ("jitdepth", jitdepth as ep9_vm::HostFn),
        ("mark", mark),
        ("bt", bt),
        ("mk", mk),
        ("scrub", scrub),
        ("boom", boom),
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

/// The opcode list of each JITed function of `defs` (`names`).
fn ops_lines(defs: &str, names: &[&str]) -> Result<Vec<String>, String> {
    let s = session(&Config::default(), defs)?;
    names
        .iter()
        .map(|n| {
            let c = s.compile_function(n)?;
            Ok(format!(
                "{n}: {} ops, {} bytes: {}",
                c.ops.len(),
                c.code.len(),
                c.ops_line()
            ))
        })
        .collect()
}

// ---- the checks ----------------------------------------------------------------------------

/// `structural ops`: every kernel function JITs completely; prints the opcode lists.
pub fn check_ops() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (k, defs, names) in [
        ("protocol A", LOOP_DEFS, &["id", "drive"][..]),
        ("protocol C", LOOP_C_DEFS, &["drive"][..]),
        ("caller", CALLER_DEFS, CALLER_JIT),
        ("mixed", MIXED_DEFS, MIXED_JIT),
        ("terminate (e)", SPIN_DEFS, &["spin"][..]),
    ] {
        for l in ops_lines(defs, names)? {
            out.push(format!("{k}: {l}"));
        }
    }
    let mut a = ops_lines(LOOP_DEFS, &["drive"])?;
    if !a[0].contains("AddSmi") {
        return Err(format!("protocol A drive has no AddSmi: {}", a.remove(0)));
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
            let t0 = run_program(&config, defs, &[], &main)?;
            let jit_run = run_program(&config, defs, jit, &main)?;
            expect_ok(&t0, kname)?;
            if t0 != jit_run {
                return Err(format!(
                    "{kname} under {cname}: JIT differs from T0\nT0:\n{}JIT:\n{}",
                    t0.text(),
                    jit_run.text()
                ));
            }
            out.push(format!(
                "{kname:6} {cname:22} JIT [{}] = T0: {}",
                jit.join(", "),
                t0.output.join(" | ")
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
";

fn check_b() -> Result<Vec<String>, String> {
    let main = "print(jitdepth());\nprint(g(t, 0));\nprint(g(h, t));\nprint(g(t2, 0));\nprint(g(where, 0));\n";
    let mut s = session(&Config::default(), PATHS_DEFS)?;
    s.jit(&["g", "h"])?;
    for name in ["g", "h"] {
        let f = ep9_vm::NativeCx::global_value(&s.iso, name).ok_or("no g")?;
        let k = ep9_vm::native_target(&s.iso, f).and_then(|t| t.kind);
        if k != Some(ep9_vm::DispatchKind::Baseline) {
            return Err(format!("{name} slot kind {k:?}"));
        }
    }
    let r = s.run(main, "main.js");
    expect_ok(&r, "paths")?;
    let want = ["0", "1", "2", "2"];
    if r.output[..4] != want {
        return Err(format!(
            "JIT frame counts {:?}, want {want:?}",
            &r.output[..4]
        ));
    }
    let stack = &r.output[4];
    if !(stack.contains("at where (defs.js:5:") && stack.contains("at g (defs.js:1:")) {
        return Err(format!("Error.stack misses the JIT frame:\n{stack}"));
    }
    Ok(vec![
        "T0 only: 0 JIT frames".into(),
        "T0->JIT g->T0 t: 1 JIT frame (JIT->T0)".into(),
        "T0->JIT g->JIT h->T0 t: 2 JIT frames (JIT->JIT)".into(),
        "T0->JIT g->T0 t2->JIT g->T0 t: 2 JIT frames".into(),
        format!("Error.stack under JIT g: {}", stack.replace('\n', " | ")),
    ])
}

const EXC_DEFS: &str = "\
function thrower(x) { throw new Error('boom ' + x); }
function g(f, x) {
  return f(x);
}
";

fn check_c() -> Result<Vec<String>, String> {
    let main = "try { g(thrower, 1); } catch (e) { print(e.message); print(e.stack); }\nprint('after');\n\
                try { g(g, thrower); } catch (e) { print(e.stack); }\n";
    let t0 = run_program(&Config::default(), EXC_DEFS, &[], main)?;
    let jit = run_program(&Config::default(), EXC_DEFS, &["g"], main)?;
    expect_ok(&jit, "exceptions")?;
    if t0 != jit {
        return Err(format!(
            "JIT differs from T0\nT0:\n{}JIT:\n{}",
            t0.text(),
            jit.text()
        ));
    }
    // `return f(x);` is line 3, column 10 of defs.js.
    let line = "    at g (defs.js:3:10)";
    if !jit.output[1].lines().any(|l| l == line) || jit.output[2] != "after" {
        return Err(format!("no `{line}` in:\n{}", jit.output[1]));
    }
    Ok(vec![
        format!("caught: {}", jit.output[0]),
        format!(
            "Error.stack (equals T0): {}",
            jit.output[1].replace('\n', " | ")
        ),
        format!("two JIT frames: {}", jit.output[3].replace('\n', " | ")),
    ])
}

/// The native return addresses of this function's callers (taken in a frame of its own).
#[inline(never)]
fn outer_walk() -> Vec<usize> {
    native_return_addresses(4096)
}

#[inline(never)]
fn check_d() -> Result<Vec<String>, String> {
    let mut s = session(&Config::default(), PATHS_DEFS)?;
    s.jit(&["g", "h"])?;
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
        .ok_or_else(|| format!("no JIT frame in the walk ({} frames)", inner.len()))?;
    if jit.len() != 2 {
        return Err(format!(
            "{} JIT frames in the walk, want 2 (g and h)",
            jit.len()
        ));
    }
    if common < 2 || inner.len() - common <= last_jit {
        return Err(format!(
            "the walk stops at the JIT frames: {} frames, JIT at {jit:?}, {common} frames shared with the caller",
            inner.len()
        ));
    }
    Ok(vec![format!(
        "walk from bt(): {} frames, JIT frames at {jit:?}, then {} more; the last {common} equal the frames that called the run",
        inner.len(),
        inner.len() - last_jit - 1
    )])
}

/// A loop with no call: only the JumpLoop poll can stop it.
pub const SPIN_DEFS: &str = "\
function spin(n) {
  let a = 0;
  for (let i = 0; i < n; i++) { a = a + 1; }
  return a;
}
";

fn check_e() -> Result<Vec<String>, String> {
    let mut s = session(&Config::default(), SPIN_DEFS)?;
    s.jit(&["spin"])?;
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
        format!("spin(2e9) in JIT code terminated {took:?} after the start (request at {delay:?})"),
        "the isolate runs JIT code again after the termination: spin(5) = 5".into(),
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
    for (label, jit) in [("T0<->JIT", &["jcall"][..]), ("T0 only", &[][..])] {
        MARKS.with(|m| m.borrow_mut().clear());
        let mut s = session(&Config::default(), REC_DEFS)?;
        if !jit.is_empty() {
            s.jit(jit)?;
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
            if jit.is_empty() { "T0" } else { "JIT" },
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

/// The masked word of check (g): the cage is below 2^47, so `address ^ MASK` has high bits
/// set and is never a heap address (and its Value bits are a double).
const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Runs the (g) probe with `mask`; returns `(result bits, conservative roots, session)`.
#[inline(never)]
fn run_hold(mask: u64) -> Result<(u64, Vec<usize>, Session), String> {
    let config = Config {
        record_roots: true,
        zap: true,
        ..Config::default()
    };
    let mut s = session(&config, HOLD_DEFS)?;
    let input = s.function_input("hold")?;
    let arch = crate::emit::Arch::HOST.ok_or("no host arch")?;
    let code = hold_probe(arch, &input, mask).map_err(|e| e.to_string())?;
    s.install(&["hold"], vec![code])?;
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
    // Positive: the object lives only in x19 / rbx across gc().
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
        "held in the callee-saved register: {addr:#x} survived gc() (found by the conservative scan), tag = 42"
    ));
    // Every run's session stays alive to the end of the check, so the 6 cages are reserved
    // at once and no two runs share an address. The scan reads the whole thread stack, this
    // frame too, and this frame may still hold an earlier run's address (`addr`, `bits`).
    // When a dropped cage's base came back for a later round (2047 slots of 64 GiB, about 1
    // run in 100 on linux arm64), that stale word named the new masked object and failed the
    // round. In a live cage of its own, it is outside the masked run's heap.
    let mut seen = vec![addr];
    let mut keep = vec![s];
    // Negative control: the same code with the register XORed: nothing else holds the object.
    for round in 0..5 {
        let (masked, roots, s) = run_hold(MASK)?;
        let addr = (masked ^ MASK) as usize;
        if seen.contains(&addr) {
            return Err(format!(
                "round {round}: the masked object {addr:#x} has an earlier run's address"
            ));
        }
        seen.push(addr);
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
        keep.push(s);
    }
    out.push("masked (XOR 0x5a5a...): the object was collected by gc(), 5 of 5 rounds".into());
    Ok(out)
}

/// The environment variable that marks the child of check (h).
const H_CHILD: &str = "SPIKE_B_H_CHILD";

/// The child of check (h): JIT `g` calls the host function `boom`, which panics inside
/// `call_any`; the panic must abort at the table function's non-unwinding ABI boundary
/// (or at once with `panic = "abort"`).
fn h_child() -> ! {
    let r = (|| -> Result<(), String> {
        let mut s = session(&Config::default(), PATHS_DEFS)?;
        s.jit(&["g"])?;
        let _ = s.run("g(boom, 1);", "main.js");
        Ok(())
    })();
    eprintln!("spike-b: the panic did not abort: {r:?}");
    std::process::exit(3)
}

fn check_h() -> Result<Vec<String>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(&exe)
        .args(["structural", "h-child"])
        .env(H_CHILD, "1")
        .output()
        .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.contains("boom from a host function under JIT code") {
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
        "{} build: the child ended with {:?} after the panic in boom() under JIT g",
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

/// `spike-b structural <all | ops | a..h | h-child>`.
pub fn main(args: &[String]) -> ExitCode {
    let which = args.first().map(String::as_str).unwrap_or("all");
    if which == "h-child" {
        if std::env::var_os(H_CHILD).is_none() {
            eprintln!("spike-b: h-child runs only under check (h)");
            return ExitCode::from(2);
        }
        h_child();
    }
    let mut fail = 0;
    let mut run = |name: &str, title: &str, check: Check| {
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
    };
    match which {
        "all" => {
            run(
                "ops",
                "the kernels JIT completely; their opcode lists",
                check_ops,
            );
            for (name, title, check) in CHECKS {
                run(name, title, check);
            }
        }
        "ops" => run(
            "ops",
            "the kernels JIT completely; their opcode lists",
            check_ops,
        ),
        c => match CHECKS.iter().find(|(n, _, _)| *n == c) {
            Some(&(name, title, check)) => run(name, title, check),
            None => {
                eprintln!("spike-b structural: unknown check {c:?} (all, ops, a..h)");
                return ExitCode::from(2);
            }
        },
    }
    println!(
        "structural: {} on {} ({}, {}, T0 dispatch {:?})",
        if fail == 0 { "PASS" } else { "FAIL" },
        std::env::consts::ARCH,
        std::env::consts::OS,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        crate::harness::DISPATCH
    );
    if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
