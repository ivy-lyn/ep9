//! Feedback vector tests (H16, OM-12 to OM-14): when a function gets its lazy vector, the
//! vector's layout and first contents, and GC (the unit keeps the vector; a key is weak, a
//! value is strong). Each program runs under every dispatch expansion, with and without GC
//! stress (a collection before every allocation, then the heap verifier).

use std::hint::black_box;

use ep9_bytecode::{FunctionId, RegisterList, VerifiedUnit};

use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::feedback;
use crate::frame::{UNDEFINED, int32_bits};
use crate::interp::BUDGET;
use crate::kinds;
use crate::object;
use crate::runtime::Runtime;
use crate::unit::FunctionInfo;
use crate::{Dispatch, Isolate, IsolateConfig, Value};

/// How the script calls `f`.
#[derive(Clone, Copy, Debug)]
enum How {
    /// `f()`: the T0 call fast path.
    Fast,
    /// `new f()`: always the cold path (`slow::call`).
    Construct,
    /// `f()` and `new f()` in turn, starting with `f()`.
    Mixed,
}

/// The script: `f = function f() {}` (function 1, a global), then `calls` calls of `f`, then
/// a loop with `back_edges` back edges (none for 0).
fn program(calls: u32, how: How, back_edges: i32) -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    f.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let r = s.b.new_register();
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0).star(r);
    s.sta_global("f");
    for i in 0..calls {
        let construct = match how {
            How::Fast => false,
            How::Construct => true,
            How::Mixed => i % 2 == 1,
        };
        if construct {
            s.b.ldar(r).construct(r, RegisterList::EMPTY, 0, s.call);
        } else {
            s.b.call_undefined_receiver0(r, s.call);
        }
    }
    if back_edges > 0 {
        let i = s.b.new_register();
        s.b.lda_zero().star(i);
        let head = s.b.bound_label();
        let exit = s.b.new_label();
        s.b.lda_smi(back_edges)
            .test_less_than(i, s.cmp)
            .jump_if_false(exit);
        s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
        s.b.bind(exit);
    }
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), f.finish()])
}

/// The feedback state of one function after a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct State {
    vector: bool,
    invocations: u32,
}

/// The FunctionInfo of function `id` of the last unit `iso` loaded.
fn with_info<T>(iso: &Isolate, id: usize, f: impl FnOnce(&Runtime, &FunctionInfo) -> T) -> T {
    let units = iso.rt.units.borrow();
    let u = units.last().expect("a loaded unit");
    f(&iso.rt, &u.infos[id])
}

/// Checks the vector of `fi` against OM-12 (if it has one) and returns the state.
fn state(fi: &FunctionInfo) -> State {
    if let Some(v) = feedback::vector(fi) {
        check_vector(fi, v);
    }
    State {
        vector: feedback::vector(fi).is_some(),
        invocations: fi.invocations.get(),
    }
}

/// The vector `v` of `fi`: a FEEDBACK cell in `fi`'s unit slot, with one slot per feedback
/// slot of the function, pointing back at `fi`. Every slot is still (undefined, int32 0).
fn check_vector(fi: &FunctionInfo, v: u64) {
    assert!(feedback::is_vector(v), "not a FEEDBACK cell: {v:#x}");
    let unit = fi.loaded_unit().unwrap();
    let slot = feedback::unit_slot(unit, fi.function_id).unwrap();
    assert!(core::ptr::eq(fi.feedback.get(), slot));
    assert_eq!(slot.bits(), v);
    assert!(core::ptr::eq(feedback::info_of(v), fi));
    let n = unit.unit.functions[fi.function_id as usize]
        .feedback_slots
        .len();
    assert_eq!(feedback::slot_count(v) as usize, n);
    for i in 0..n as u32 {
        assert_eq!(feedback::key(v, i), UNDEFINED, "key of slot {i}");
        assert_eq!(feedback::value(v, i), int32_bits(0), "value of slot {i}");
    }
}

/// Runs `make()` with `threshold` under every dispatch expansion, with and without GC
/// stress. Returns the states of the script (function 0) and `f` (function 1), which must
/// agree across the runs.
fn run_states(
    threshold: u32,
    make: impl Fn() -> VerifiedUnit + Copy + Send + 'static,
) -> [State; 2] {
    let mut first: Option<(String, [State; 2])> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let states = on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: threshold,
                    ..config(d, stress)
                });
                iso.run_script(make(), "test.js").unwrap();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                [0, 1].map(|id| with_info(&iso, id, |_, fi| state(fi)))
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, states)),
                Some((l0, s0)) => assert_eq!(&states, s0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// No vector yet, after `invocations` counted calls.
fn none(invocations: u32) -> State {
    State {
        vector: false,
        invocations,
    }
}

/// A vector, made on call number `invocations` (or at a poll, after that many calls).
fn made(invocations: u32) -> State {
    State {
        vector: true,
        invocations,
    }
}

#[test]
fn no_vector_after_7_calls_and_a_vector_on_the_8th() {
    assert_eq!(IsolateConfig::default().feedback_threshold, 8);
    let t = IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD;
    // The T0 fast path counts and goes cold on the 8th call; `slow::call` makes the vector.
    let [script, f] = run_states(t, || program(7, How::Fast, 0));
    assert_eq!((script, f), (none(1), none(7)));
    let [_, f] = run_states(t, || program(8, How::Fast, 0));
    assert_eq!(f, made(8));
    let [_, f] = run_states(t, || program(30, How::Fast, 0));
    assert_eq!(f, made(8), "no count once the vector exists");
    // `new f()` is always cold: `slow::call` counts and makes it.
    let [_, f] = run_states(t, || program(7, How::Construct, 0));
    assert_eq!(f, none(7));
    let [_, f] = run_states(t, || program(8, How::Construct, 0));
    assert_eq!(f, made(8));
    // Both paths count into one number.
    let [_, f] = run_states(t, || program(7, How::Mixed, 0));
    assert_eq!(f, none(7));
    let [_, f] = run_states(t, || program(8, How::Mixed, 0));
    assert_eq!(f, made(8));
}

#[test]
fn thresholds_0_1_3_and_max() {
    // 0 = eager: the script (an entry from Rust) and `f` get a vector at their first call.
    let [script, f] = run_states(0, || program(0, How::Fast, 0));
    assert_eq!((script, f), (made(1), none(0)), "f is never called");
    let [script, f] = run_states(0, || program(1, How::Fast, 0));
    assert_eq!((script, f), (made(1), made(1)));
    let [_, f] = run_states(0, || program(1, How::Construct, 0));
    assert_eq!(f, made(1));
    // 1: the first call; 3: the third.
    let [script, f] = run_states(1, || program(1, How::Fast, 0));
    assert_eq!((script, f), (made(1), made(1)));
    let [script, f] = run_states(3, || program(2, How::Mixed, 0));
    assert_eq!((script, f), (none(1), none(2)));
    let [_, f] = run_states(3, || program(3, How::Mixed, 0));
    assert_eq!(f, made(3));
    // u32::MAX = never, not even at a loop's budget poll.
    let back_edges = 2 * BUDGET as i32;
    let [script, f] = run_states(u32::MAX, move || program(20, How::Mixed, back_edges));
    assert_eq!((script, f), (none(1), none(20)));
}

#[test]
fn a_loop_that_runs_once_gets_a_vector_at_its_first_budget_poll() {
    // A fresh isolate starts with at most BUDGET units, and each back edge takes one, so the
    // first poll comes at or before back edge BUDGET. The script is entered once (1 < 8).
    let t = IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD;
    let [script, f] = run_states(t, || program(0, How::Fast, BUDGET as i32));
    assert_eq!((script, f), (made(1), none(0)));
    // A short loop has no poll.
    let [script, _] = run_states(t, || program(0, How::Fast, 100));
    assert_eq!(script, none(1));
}

#[test]
fn entries_from_rust_count_as_calls() {
    on_big_stack(|| {
        let mut iso = isolate(IsolateConfig::default());
        iso.run_script(program(0, How::Fast, 0), "test.js").unwrap();
        let f = iso.global("f").unwrap();
        for i in 1..=8 {
            iso.call(f, Value::UNDEFINED, &[]).unwrap();
            let want = if i < 8 { none(i) } else { made(8) };
            assert_eq!(
                with_info(&iso, 1, |_, fi| state(fi)),
                want,
                "after call {i}"
            );
        }
    });
}

// ---- dead cells: the stack must not keep them ----------------------------------------------
//
// The GC test below expects the key cell to die, and the value cell to live only through the
// vector. So no copy of either address may sit where the conservative scan of the collection
// reads: the callee-saved registers and `[sp, stack high)` at the spill trampoline (ep9_heap
// css.rs). The cells are made, and read, only below a 64 KiB pad (W4-P3FIX-DEADSHAPE-TEST, as
// in tests/ic.rs):
//
//   test frame        gets only masked addresses, and calls nothing that reads the cells
//                     before the collection
//   ---------------   <- the scan stops here (checked: `collect_after_a_pad`)
//   below_a_pad       DEEP bytes of zeros
//   ---------------   <- `pad_low`
//   made_*            frames that make and read the cells (inline(never))
//
// A frame may spill a caller-saved register that it never set, such as the unused payload of a
// returned enum; on x86-64 Linux (debug) that kept a dead shape in tests/ic.rs. Hence every
// read of the cells happens below the pad.

const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;

/// Bytes of zeros between the test's frame and the frames that make the cells: far more than a
/// collection's own frames take (`Isolate::collect_garbage` down to the spill trampoline).
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller, so stale words of returned frames cannot
/// keep a dead cell alive through the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's. `made` must
/// only call an `#[inline(never)]` function that makes the cells and returns their masked
/// addresses. Returns that result and `pad_low`, the lowest byte of the pad: every frame of
/// `made`'s callee lies below it.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> (R, usize) {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    (r, black_box(&mut pad).as_ptr().addr())
}

/// Scrubs the stack and collects, after the cells were made by [`below_a_pad`] (which gave
/// `pad_low`). Then checks that the collection can show something about them: its
/// conservative scan stopped above the pad, and kept none of `cells` (masked addresses).
fn collect_after_a_pad(iso: &mut Isolate, pad_low: usize, cells: &[usize]) {
    scrub_stack();
    iso.collect_garbage();
    let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
    let scan_low = high - 8 * iso.heap().stats().last_stack_words;
    assert!(
        scan_low > pad_low,
        "the conservative scan read the frames below the pad ({scan_low:#x} <= {pad_low:#x})"
    );
    assert_not_kept(&iso.rt, cells);
}

/// Panics if the last conservative scan kept one of `cells` (masked addresses): the check
/// that follows would not be about the vector then.
fn assert_not_kept(rt: &Runtime, cells: &[usize]) {
    let css = rt.heap.last_conservative_roots();
    for &m in cells {
        let addr = m ^ MASK;
        assert!(
            !css.iter().any(|c| c.addr() == addr),
            "the conservative scan kept {addr:#x}"
        );
    }
}

/// Fills slot 0 of `v` with (a new string, a new object) and slot 1 with (the global object,
/// 5), in a frame of its own. Run it [`below_a_pad`]. Returns the masked addresses of the
/// string and the object, so no stack word points at them.
#[inline(never)]
fn made_filled_slots(rt: &Runtime, v: u64) -> [usize; 2] {
    let key = object::new_string(rt, "a key only the vector holds");
    let value = object::new_object(rt, rt.global());
    feedback::set_slot(rt, v, 0, key, value);
    feedback::set_slot(rt, v, 1, rt.global(), int32_bits(5));
    [
        black_box(key as usize) ^ MASK,
        black_box(value as usize) ^ MASK,
    ]
}

/// An isolate under GC stress with eager feedback, after a run that called `f` once.
fn eager_isolate() -> Isolate {
    let mut c = config(Dispatch::DEFAULT, true);
    c.heap.record_conservative_roots = true;
    c.feedback_threshold = 0;
    let mut iso = isolate(c);
    iso.run_script(program(1, How::Fast, 0), "test.js").unwrap();
    iso
}

fn vector_of_f(iso: &Isolate) -> u64 {
    with_info(iso, 1, |_, fi| feedback::vector(fi)).expect("f has a vector")
}

#[test]
fn the_vector_survives_gc_while_its_unit_lives() {
    on_big_stack(|| {
        let mut iso = eager_isolate();
        let v = vector_of_f(&iso);
        let collections = iso.heap().stats().collections;
        for _ in 0..3 {
            iso.collect_garbage();
        }
        iso.verify_heap().unwrap();
        // More code runs (under stress, each allocation collects), and f is called again.
        iso.run_script(program(4, How::Mixed, 0), "test2.js")
            .unwrap();
        assert!(iso.heap().stats().collections > collections + 3);
        // The first unit's f still has the same vector, unchanged.
        let units = iso.rt.units.borrow();
        let fi = &units[units.len() - 2].infos[1];
        assert_eq!(feedback::vector(fi), Some(v));
        check_vector(fi, v);
        // The second unit's f has its own vector.
        let fi2 = &units[units.len() - 1].infos[1];
        let v2 = feedback::vector(fi2).unwrap();
        assert_ne!(v2, v);
        check_vector(fi2, v2);
    });
}

#[test]
fn a_key_to_a_dead_cell_is_cleared_and_a_value_is_kept() {
    on_big_stack(|| {
        let mut iso = eager_isolate();
        let v = vector_of_f(&iso);
        assert!(feedback::slot_count(v) >= 2);
        let ([key, value], pad_low) = below_a_pad(|| made_filled_slots(&iso.rt, v));
        collect_after_a_pad(&mut iso, pad_low, &[key, value]);
        iso.verify_heap().unwrap();
        // The key was the only reference to its string: weak, cleared to undefined.
        assert_eq!(feedback::key(v, 0), UNDEFINED);
        assert!(iso.heap().stats().last_weak_cleared >= 1);
        // The value is strong: the object lives, at the same address.
        let kept = feedback::value(v, 0);
        assert_eq!(kept as usize, value ^ MASK);
        assert_eq!(kinds::cell_kind(kept), kinds::OBJECT.get());
        // A key to a live cell stays.
        assert_eq!(feedback::key(v, 1), iso.rt.global());
        assert_eq!(feedback::value(v, 1), int32_bits(5));
    });
}
