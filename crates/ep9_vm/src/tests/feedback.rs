//! Feedback vector tests (H16, OM-12 to OM-14): when a function gets its lazy vector, the
//! vector's layout and first contents, and GC (the unit keeps the vector; a key is weak, a
//! value is strong). Each program runs under every dispatch expansion, with and without GC
//! stress (a collection before every allocation, then the heap verifier).

use std::hint::black_box;

use ep9_bytecode::{FeedbackSlotKind, FunctionId, RegisterList, VerifiedUnit};

use super::{Asm, config, isolate, meta, on_big_stack, profile::isolate as isolate_full, unit};
use crate::construct_ic::{cs, handler_i32};
use crate::feedback;
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::ic::hk;
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
    /// `new f()`: the T0 construct-site hit once the site has a `CONSTRUCT_SITE` handler
    /// (`construct_ic.rs`, M4-P1), else the cold path (`slow::call`).
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

/// The script: `f = function f() { loop }` (function 1, a global; the loop has `back_edges`
/// back edges), then one call of `f`.
fn program_loop_in_f(back_edges: i32) -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    let i = f.b.new_register();
    f.b.lda_zero().star(i);
    let head = f.b.bound_label();
    let exit = f.b.new_label();
    f.b.lda_smi(back_edges)
        .test_less_than(i, f.cmp)
        .jump_if_false(exit);
    f.b.ldar(i).inc(f.bin).star(i).jump_loop(head, 0, f.loop_);
    f.b.bind(exit);
    f.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let r = s.b.new_register();
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0).star(r);
    s.sta_global("f");
    s.b.call_undefined_receiver0(r, s.call);
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
/// slot of the function, pointing back at `fi`. These programs have no property IC site, so
/// only two writers touch the vector: T0 profile recording (M4-P1 PROFILE, `profile.rs`)
/// writes int32 counts into the Call, CreateClosure and JumpLoop slots, and a Call key that is
/// the hole or a FunctionInfo native word (not a cell); a `new f()` site (M4-P1 OBJ-ALLOC,
/// `construct_ic.rs`) keeps in its Call slot the constructor (a FUNCTION cell, weak) or the
/// hole, and an int32 count or a `CONSTRUCT_SITE` handler. Every other slot is still
/// (undefined, int32 0).
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
    let kinds_of = &unit.unit.functions[fi.function_id as usize].feedback_slots;
    for i in 0..n as u32 {
        let (key, value) = (feedback::key(v, i), feedback::value(v, i));
        let call = kinds_of[i as usize] == FeedbackSlotKind::Call;
        if !call {
            assert_eq!(key, UNDEFINED, "key of slot {i}");
        } else if kinds::is_cell_bits(key) {
            assert!(kinds::is_function(key), "key of Call slot {i}: {key:#x}");
        }
        if call && kinds::is_cell_bits(value) {
            assert_eq!(kinds::cell_kind(value), kinds::IC_HANDLER.get(), "slot {i}");
            assert_eq!(handler_i32(value, cs::KIND), hk::CONSTRUCT_SITE, "slot {i}");
        } else {
            assert!(
                value == int32_bits(int32_of(value)) && int32_of(value) >= 0,
                "value of slot {i}: {value:#x}"
            );
        }
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
                iso.with_cx_checkpoint(|cx| cx.run_script(make(), "test.js").map(|_| ()))
                    .unwrap();
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
    // The script gets its vector at entry (PERF3 step-2 item 3): no count.
    let [script, f] = run_states(t, || program(7, How::Fast, 0));
    assert_eq!((script, f), (made(0), none(7)));
    let [_, f] = run_states(t, || program(8, How::Fast, 0));
    assert_eq!(f, made(8));
    let [_, f] = run_states(t, || program(30, How::Fast, 0));
    assert_eq!(f, made(8), "no count once the vector exists");
    // `new f()`: the cold path (`slow::call`) or, once the site keeps a `CONSTRUCT_SITE`
    // handler, the construct-site hit (`construct_ic::fast`); both count the call.
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
    // 0 = eager: `f` gets a vector at its first call. The script (a unit's entry code) gets
    // its vector at entry under every threshold but u32::MAX, with no count (item 3).
    let [script, f] = run_states(0, || program(0, How::Fast, 0));
    assert_eq!((script, f), (made(0), none(0)), "f is never called");
    let [script, f] = run_states(0, || program(1, How::Fast, 0));
    assert_eq!((script, f), (made(0), made(1)));
    let [_, f] = run_states(0, || program(1, How::Construct, 0));
    assert_eq!(f, made(1));
    // 1: the first call; 3: the third.
    let [script, f] = run_states(1, || program(1, How::Fast, 0));
    assert_eq!((script, f), (made(0), made(1)));
    let [script, f] = run_states(3, || program(2, How::Mixed, 0));
    assert_eq!((script, f), (made(0), none(2)));
    let [_, f] = run_states(3, || program(3, How::Mixed, 0));
    assert_eq!(f, made(3));
    // u32::MAX = never: not at the entry, not at a loop's budget poll.
    let back_edges = 2 * BUDGET as i32;
    let [script, f] = run_states(u32::MAX, move || program(20, How::Mixed, back_edges));
    assert_eq!((script, f), (none(0), none(20)));
    let [_, f] = run_states(u32::MAX, move || program_loop_in_f(back_edges));
    assert_eq!(f, none(1));
}

#[test]
fn a_loop_that_runs_once_gets_a_vector_at_its_first_budget_poll() {
    // A fresh isolate starts with at most BUDGET units, and each back edge takes one, so the
    // first poll comes at or before back edge BUDGET. `f` is called once (1 < 8).
    let t = IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD;
    let [script, f] = run_states(t, || program_loop_in_f(BUDGET as i32));
    assert_eq!((script, f), (made(0), made(1)));
    // A short loop has no poll.
    let [_, f] = run_states(t, || program_loop_in_f(100));
    assert_eq!(f, none(1));
}

#[test]
fn a_script_gets_its_vector_at_entry() {
    // PERF3 step-2 item 3 (M4-P2 FACTS2): a script's sites record from its first instruction,
    // with no loop and no call. Before, a script (entered once) got a vector only at a loop
    // poll, so its global loads and calls missed with no vector (ic-stats "no-vector").
    let t = IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD;
    let [script, _] = run_states(t, || program(0, How::Fast, 0));
    assert_eq!(script, made(0));
    // Its CreateClosure slot saw the one closure (`profile::record_closure`).
    on_big_stack(|| {
        let mut iso = isolate_full(IsolateConfig::default());
        iso.with_cx_checkpoint(|cx| {
            cx.run_script(program(1, How::Fast, 0), "test.js")
                .map(|_| ())
        })
        .unwrap();
        with_info(&iso, 0, |_, fi| {
            let v = feedback::vector(fi).expect("the script has a vector");
            let unit = fi.loaded_unit().unwrap();
            let kinds_of = &unit.unit.functions[0].feedback_slots;
            let closure = kinds_of
                .iter()
                .position(|&k| k == FeedbackSlotKind::CreateClosure)
                .unwrap();
            assert_eq!(feedback::value(v, closure as u32), int32_bits(1));
        });
    });
}

#[test]
fn entries_from_rust_count_as_calls() {
    on_big_stack(|| {
        let mut iso = isolate(IsolateConfig::default());
        iso.with_cx_checkpoint(|cx| {
            cx.run_script(program(0, How::Fast, 0), "test.js")
                .map(|_| ())
        })
        .unwrap();
        let f = iso.global_raw("f").unwrap();
        for i in 1..=8 {
            iso.call_raw(f, Value::UNDEFINED, &[]).unwrap();
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
/// Inlined (rule 35), so the scrub runs in the test's own frame and its zeros lie under the
/// frame of [`collect_and_check`] too: an out-of-line helper's own frame is not under its
/// own scrub (`tests/ordered_table.rs` `collect`: CI run 40733504045). The collection and
/// the checks stay out of line, so the unmasked addresses they compute never reach the
/// test's frame.
#[inline(always)]
fn collect_after_a_pad(iso: &mut Isolate, pad_low: usize, cells: &[usize]) {
    scrub_stack();
    collect_and_check(iso, pad_low, cells);
}

/// The collection and the checks of [`collect_after_a_pad`], out of line.
#[inline(never)]
fn collect_and_check(iso: &mut Isolate, pad_low: usize, cells: &[usize]) {
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
    iso.with_cx_checkpoint(|cx| {
        cx.run_script(program(1, How::Fast, 0), "test.js")
            .map(|_| ())
    })
    .unwrap();
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
        iso.with_cx_checkpoint(|cx| {
            cx.run_script(program(4, How::Mixed, 0), "test2.js")
                .map(|_| ())
        })
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

/// The script: `f = function f() {}` (function 1) and `g = function g(a) { loop }` (function
/// 2; each iteration calls `a` `calls` times, then a back edge; `back_edges` back edges),
/// then one call `g(f)`.
fn program_calls_per_back_edge(calls: u32, back_edges: i32) -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    f.b.lda_undefined().return_();
    let mut g = Asm::new(meta("g", 1, 0, 0, 0));
    let a = g.b.parameter(0);
    let i = g.b.new_register();
    g.b.lda_zero().star(i);
    let head = g.b.bound_label();
    let exit = g.b.new_label();
    g.b.lda_smi(back_edges)
        .test_less_than(i, g.cmp)
        .jump_if_false(exit);
    for _ in 0..calls {
        g.b.call_undefined_receiver0(a, g.call);
    }
    g.b.ldar(i).inc(g.bin).star(i).jump_loop(head, 0, g.loop_);
    g.b.bind(exit);
    g.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let (rf, rg) = (s.b.new_register(), s.b.new_register());
    let kf = s.b.function_constant(FunctionId(1));
    s.b.create_closure(kf, s.closure, 0).star(rf);
    let kg = s.b.function_constant(FunctionId(2));
    s.b.create_closure(kg, s.closure, 0).star(rg);
    let args = s.b.new_register_list(1);
    s.b.ldar(rf).star(args.get(0));
    s.b.call_undefined_receiver1(rg, args.get(0), s.call);
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), f.finish(), g.finish()])
}

/// The state of function `id` after a run of `make()` with `threshold` in the match loop
/// (the budget does not depend on the dispatch or on GC stress, and a debug build of the
/// other expansions has no tail calls, so a long loop of calls would use up the native
/// stack).
fn run_state_of(
    threshold: u32,
    id: usize,
    make: impl Fn() -> VerifiedUnit + Copy + Send + 'static,
) -> State {
    on_big_stack(move || {
        let mut iso = isolate(IsolateConfig {
            feedback_threshold: threshold,
            ..config(Dispatch::Match, false)
        });
        iso.with_cx_checkpoint(|cx| cx.run_script(make(), "test.js").map(|_| ()))
            .unwrap();
        with_info(&iso, id, |_, fi| state(fi))
    })
}

#[test]
fn a_loop_whose_polls_land_on_calls_gets_its_vector() {
    // PERF3 step-2 item 4 (the budget phase lock): with k calls and 1 back edge per
    // iteration, k + 1 divides BUDGET for k = 1, 3, 4, so every poll lands on the same unit
    // of the body; for some k that is a call, and `g` (called once, 1 < 8) got no vector,
    // since `feedback::ensure` ran only at the JumpLoop poll. The call poll in `slow::call`
    // now runs it too.
    let t = IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD;
    for calls in 1..=5 {
        let back_edges = BUDGET as i32;
        let g = run_state_of(t, 2, move || program_calls_per_back_edge(calls, back_edges));
        assert!(g.vector, "{calls} calls per back edge: {g:?}");
    }
}

// ---- the feedback mode, written once (M6-P0 SCHEMA6; MC-w21-8, MC-w22-13) ----

/// The isolate's options are written once, before its first realm (`exec::install`): a
/// second write panics and leaves the first mode, which `Cx::table`, the cold hooks
/// (`profile::records`) and the export (`facts_export::keep`) all read.
#[test]
fn the_feedback_mode_is_written_once() {
    use crate::exec::{self, ExecOptions, Feedback};
    for (first, second) in [
        (Feedback::FULL, Feedback::ICS_ONLY),
        (Feedback::ICS_ONLY, Feedback::FULL),
    ] {
        let iso = Isolate::new_with(
            IsolateConfig::default(),
            ExecOptions::jitless().feedback(first),
        )
        .unwrap();
        assert_eq!(exec::feedback_mode(&iso.rt), first);
        assert_eq!(exec::records(&iso.rt), first == Feedback::FULL);
        let again = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            exec::install(&iso.rt, ExecOptions::jitless().feedback(second));
        }));
        let msg = again.expect_err("a second write of the options panics");
        let msg = msg
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| msg.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        assert!(msg.contains("written once"), "{msg}");
        assert_eq!(exec::feedback_mode(&iso.rt), first, "the first mode stays");
        // `Cx::table` was picked from the same mode (the two tables differ).
        let words = |t: &[crate::interp::Handler; 256]| -> Vec<usize> {
            t.iter().map(|h| *h as usize).collect()
        };
        let d = IsolateConfig::default().dispatch;
        assert_ne!(words(d.table(first)), words(d.table(second)));
        assert_eq!(words(&iso.cx.table), words(d.table(first)));
    }
}

/// `exec::install` is the one writer of `Runtime::exec` (a source scan of this crate), and no
/// module but `exec.rs` reads the mode from the field: every reader goes through
/// `exec::feedback_mode` or `exec::records`.
#[test]
fn only_exec_rs_touches_the_feedback_mode_field() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    // The needles are built here, so this file does not match itself.
    let (set, get) = (
        ["exec", ".set("].concat(),
        ["exec", ".get().feedback_mode()"].concat(),
    );
    let (mut writes, mut mode_reads) = (Vec::new(), Vec::new());
    for f in &files {
        let rel = f
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(f).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains(set.as_str()) {
                writes.push(format!("{rel}:{}", n + 1));
            }
            if code.contains(get.as_str()) {
                mode_reads.push(format!("{rel}:{}", n + 1));
            }
        }
    }
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].starts_with("exec.rs:"), "{writes:?}");
    assert!(
        mode_reads
            .iter()
            .all(|r| r.starts_with("exec.rs:") || r.starts_with("tests/exec.rs:")),
        "{mode_reads:?}"
    );
}

// ---- CompareOp classes (M7-P2 RECCOST) -----------------------------------------------------

/// The old `profile::compare_operand` (a test chain from the int32 tag to the cell kind), kept
/// word for word as the oracle of the RECCOST form: the records must stay byte-equal (H16).
fn old_compare_operand(v: u64) -> u16 {
    use crate::frame::{FALSE, NULL, TRUE};
    use ep9_facts::CompareHint;
    let is_i32 = v & ep9_value::encoding::NUMBER_TAG == ep9_value::encoding::NUMBER_TAG;
    let is_number = v & ep9_value::encoding::NUMBER_TAG != 0;
    let h = if is_i32 {
        CompareHint::INT32
    } else if is_number {
        CompareHint::NUMBER
    } else if v == TRUE || v == FALSE {
        CompareHint::BOOLEAN
    } else if v == NULL || v == UNDEFINED {
        CompareHint::NULLISH
    } else {
        match kinds::cell_kind(v) {
            1..=7 if crate::string::is_atom(v) => CompareHint::ATOM,
            1..=7 => CompareHint::STRING,
            8 => CompareHint::SYMBOL,
            9 => CompareHint::BIGINT,
            16..=63 => CompareHint::RECEIVER,
            _ => CompareHint::OTHER,
        }
    };
    h.bits()
}

/// The old `profile::compare_bits` (with the old `compare_pair`), the oracle.
fn old_compare_bits(l: u64, r: u64) -> u32 {
    use ep9_facts::CompareHint;
    const INT_NUM: u16 = CompareHint::INT32.bits() | CompareHint::NUMBER.bits();
    const STRINGS: u16 = CompareHint::ATOM.bits() | CompareHint::STRING.bits();
    const OBJ_NULL: u16 = CompareHint::RECEIVER.bits() | CompareHint::NULLISH.bits();
    let tag = ep9_value::encoding::NUMBER_TAG;
    if l & tag == tag && r & tag == tag {
        return 0;
    }
    let (a, b) = (old_compare_operand(l), old_compare_operand(r));
    let h = if a == b {
        a
    } else {
        match a | b {
            INT_NUM => CompareHint::NUMBER.bits(),
            STRINGS => CompareHint::STRING.bits(),
            OBJ_NULL => OBJ_NULL,
            _ => CompareHint::OTHER.bits(),
        }
    };
    u32::from(h)
}

/// Checks `profile::compare_bits` against the oracle on every pair of `values`; returns the
/// number of pairs.
fn check_compare_pairs(values: &[u64]) -> usize {
    let mut n = 0;
    for &l in values {
        for &r in values {
            assert_eq!(
                crate::profile::compare_bits(l, r),
                old_compare_bits(l, r),
                "compare_bits({l:#x}, {r:#x})"
            );
            n += 1;
        }
    }
    n
}

/// RECCOST (M7-P2) changed how `profile::compare_bits` finds the class of an operand (the tag
/// as `v >> 49`, the cell kind by a table, the immediates by `| 1` and `| 8`), not the classes:
/// every pair of a value set gives the bits of the old test chain. The set: numbers at the
/// edges of the int32 and double tags; every immediate below 256 with `OTHER_TAG` (true, false,
/// null, undefined, the hole and the bit patterns no Value has); fake cells of all 256 kind
/// bytes with and without the atom flag (only the header is read); and real cells of an
/// isolate (strings, an atom, a symbol, a BigInt, objects, an array, a function).
#[test]
fn compare_classes_equal_the_old_test_chain() {
    use ep9_value::{ShapeId, Word0};
    let mut values: Vec<u64> = Vec::new();
    for i in [0, 1, -1, 7, i32::MAX, i32::MIN] {
        values.push(int32_bits(i));
    }
    for d in [
        0.5,
        -0.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
        2147483648.0,
    ] {
        values.push(Value::number(d).to_bits());
    }
    values.extend((0u64..256).filter(|v| v & ep9_value::encoding::OTHER_TAG != 0));
    // Fake cells: one 8-byte header each, 8-aligned, below 2^48, so `OTHER_TAG` is clear.
    let fakes: Vec<Word0> = (0..=255u8)
        .flat_map(|k| {
            [0, kinds::FLAG_ATOM, 0xFFFE, 0xFFFF].map(|f| Word0::new(ShapeId::new(0), 0, k, f))
        })
        .collect();
    for w in &fakes {
        let bits = (w as *const Word0).expose_provenance() as u64;
        assert!(kinds::is_cell_bits(bits) && bits != 0);
        values.push(bits);
    }
    let fake_pairs = check_compare_pairs(&values);
    assert_eq!(fake_pairs, values.len() * values.len());

    let real = on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        let rt = iso.cx.rt();
        let cells = [
            object::new_string(rt, "a text that is not interned"),
            object::new_string(rt, "\u{3b1}\u{3b2}"),
            crate::atoms::intern_str(rt, "length"),
            crate::atoms::intern_str(rt, "x"),
            crate::symbol::new_symbol(rt, UNDEFINED),
            crate::bigint::alloc(rt, &ep9_bigint::from_i64(-5)),
            object::new_plain_object(rt),
            object::new_object(rt, crate::frame::NULL),
        ];
        let mut values: Vec<u64> = cells.to_vec();
        values.extend([int32_bits(3), Value::number(0.25).to_bits(), UNDEFINED]);
        values.extend([crate::frame::NULL, crate::frame::TRUE, crate::frame::FALSE]);
        values.push(crate::frame::HOLE);
        let n = check_compare_pairs(&values);
        let classes: Vec<u16> = cells.iter().map(|&c| old_compare_operand(c)).collect();
        black_box(&mut iso);
        (n, classes)
    });
    use ep9_facts::CompareHint as C;
    assert_eq!(
        real.1,
        [
            C::STRING.bits(),
            C::STRING.bits(),
            C::ATOM.bits(),
            C::ATOM.bits(),
            C::SYMBOL.bits(),
            C::BIGINT.bits(),
            C::RECEIVER.bits(),
            C::RECEIVER.bits(),
        ],
        "the real cells cover each cell class"
    );
    assert!(real.0 > 0);
}
