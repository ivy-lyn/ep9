//! Tests of WeakRef and FinalizationRegistry (`weakref.rs`, `abi/weakref.rs`, WEAK_CELL kind
//! 80) (owner M4-P4 WEAKREF). ep9_vm tests cannot compile JS, so each test drives the `Cx`
//! methods; the JS-level tests are `ep9_builtins` `tests/weakref.rs`.
//!
//! Each test runs four ways: plain, GC stress (a collection before every allocation, then
//! the heap verifier), and both again with the generational GC (S1; under stress most
//! collections are minor). Cells are made below a pad of zeros and the stack is scrubbed
//! before each collection (the pattern of `tests/ordered_table.rs`), so only real references
//! keep a cell; weak handles watch the targets.
//!
//! | Test | Checks |
//! |---|---|
//! | `a_weak_ref_target_is_kept_for_the_job_then_dies` | AddToKeptObjects keeps the target through GCs until ClearKeptObjects; then the weak trace clears it and `deref` gives undefined; a live target stays |
//! | `deref_keeps_the_target_again` | `deref` adds the target to the kept list again; the list holds each value once |
//! | `a_strong_target_slot_keeps_the_target` | the negative control: a trace that visits TARGET as a strong slot keeps it, so the weak test can fail |
//! | `a_registry_calls_back_after_the_gc_not_during_it` | a dead target's held value goes to the callback at the cleanup hook, never during a GC; once only; a live target gets none |
//! | `unregister_removes_live_and_pending_cells` | unregister by token removes a live cell and a cell whose target died before the hook ran; true/false results |
//! | `tokens_and_registries_are_weak` | a dead token is cleared and its cell stays; a dead registry drops out of the chain and calls nothing |
//! | `a_throwing_callback_leaves_the_rest` | a throw stops the hook; the next call does the remaining cells |
//! | `a_minor_gc_clears_a_young_target` | with S1 on, a minor collection clears a young target and the hook calls back |
//! | `wrong_receivers_throw` | RequireInternalSlot TypeErrors, also for a hand-made object of the class |

use core::cell::{Cell, RefCell};
use std::hint::black_box;

use ep9_heap::{GlobalHandle, WeakHandle};

use super::{config, isolate, on_big_stack};
use crate::abi::ObjectClass;
use crate::frame::UNDEFINED;
use crate::runtime::{Runtime, val};
use crate::weakref::{self, NEGATIVE_CONTROL_STRONG_TARGET, registry};
use crate::{Args, Cx, Dispatch, Isolate, IsolateConfig, JsResult, Local, object};

/// Bytes of zeros between the test's frame and the frames that make the cells.
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> R {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    black_box(&mut pad);
    r
}

/// The four heap set-ups: (GC stress, generational).
const MODES: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

fn cfg(stress: bool, generational: bool) -> IsolateConfig {
    let mut c = config(Dispatch::DEFAULT, stress);
    c.heap.generational = generational;
    c
}

/// Runs `f` on an isolate with the host function `cb` (it logs its argument), for each of
/// [`MODES`], on a big stack; then collects and verifies the heap.
fn each_mode(f: fn(&mut Isolate, &str)) {
    for (stress, generational) in MODES {
        on_big_stack(move || {
            let mut iso = isolate(cfg(stress, generational));
            iso.register_host_function("cb", 1, cb);
            iso.register_host_function("thrower", 1, thrower);
            LOG.with(|l| l.borrow_mut().clear());
            let label = format!("stress={stress} generational={generational}");
            f(&mut iso, &label);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

thread_local! {
    /// What `cb` was called with (int32 held values; -1 for anything else).
    static LOG: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Calls of `thrower` before it throws.
    static THROW_AFTER: Cell<u32> = const { Cell::new(0) };
}

fn cb<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let v = args.get(0).as_int32().unwrap_or(-1);
    LOG.with(|l| l.borrow_mut().push(v));
    Ok(Local::undefined())
}

fn thrower<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let v = args.get(0).as_int32().unwrap_or(-1);
    LOG.with(|l| l.borrow_mut().push(v));
    if THROW_AFTER.with(|t| t.replace(t.get().saturating_sub(1))) == 0 {
        return Err(cx.throw_type_error("thrower"));
    }
    Ok(Local::undefined())
}

fn take_log() -> Vec<i32> {
    LOG.with(|l| core::mem::take(&mut *l.borrow_mut()))
}

/// True if the cell of `h` is alive. Read below a pad.
#[inline(never)]
fn alive(rt: &Runtime, h: WeakHandle) -> bool {
    below_a_pad(|| black_box(rt.heap.weak(h)).is_some())
}

/// A full collection after a stack scrub (rule 35: the scrub runs in the test's own frame,
/// before and after). Inlined, so the zeros of the first scrub also lie under the frame of
/// [`gc_now`]: an out-of-line helper's own frame is not under its own scrub, and a slot that
/// it never writes (on Win64 the home space it gives its callees) keeps what the last call at
/// that depth left there (`tests/ordered_table.rs` `collect`: CI run 40733504045).
#[inline(always)]
fn collect(iso: &Isolate) {
    scrub_stack();
    gc_now(iso, false);
    scrub_stack();
}

/// A minor collection (S1 on) after a stack scrub, as [`collect`].
#[inline(always)]
fn collect_young(iso: &Isolate) {
    scrub_stack();
    gc_now(iso, true);
    scrub_stack();
}

/// A collection (minor if `minor`), out of line, so that its frames lie on the zeros of the
/// caller's scrub.
#[inline(never)]
fn gc_now(iso: &Isolate, minor: bool) {
    if minor {
        iso.rt.heap.collect_minor();
    } else {
        iso.rt.heap.collect();
    }
}

/// Runs `f` with a builtin-ABI context of `iso`, below a pad.
#[inline(never)]
fn with_cx<R>(iso: &mut Isolate, f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R) -> R {
    below_a_pad(|| {
        let iso: &mut Isolate = iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    })
}

/// The global function `name`.
fn global_fn<'s>(cx: &mut Cx<'s>, name: &str) -> Local<'s> {
    cx.global_binding(name).unwrap().expect("a host function")
}

/// A WeakRef (rooted by a global handle) to a fresh object that nothing else holds, and a
/// weak handle on that object.
fn weak_ref_to_fresh(iso: &mut Isolate) -> (GlobalHandle, WeakHandle) {
    with_cx(iso, |cx, rt| {
        let t = cx.new_object();
        let w = cx.new_weak_ref(Local::null(), t);
        (
            rt.heap.new_global(val(w.bits())),
            rt.heap.new_weak(val(t.bits())),
        )
    })
}

/// A global handle on the live cell of `h`, made below a pad (so no stale stack word in the
/// caller's frame keeps the cell after the handle goes).
#[inline(never)]
fn hold(iso: &Isolate, h: WeakHandle) -> GlobalHandle {
    below_a_pad(|| iso.rt.heap.new_global(iso.rt.heap.weak(h).expect("alive")))
}

/// A weak handle on the cell of the global handle `g`, made below a pad.
#[inline(never)]
fn watch(iso: &Isolate, g: GlobalHandle) -> WeakHandle {
    below_a_pad(|| iso.rt.heap.new_weak(iso.rt.heap.global(g)))
}

/// `deref` of the WeakRef in `w`: true if it gives undefined.
fn deref(iso: &mut Isolate, w: GlobalHandle) -> bool {
    with_cx(iso, |cx, rt| {
        let wr: Local<'_> = Local::from_bits(rt.heap.global(w).to_bits());
        cx.weak_ref_deref(wr).unwrap().is_undefined()
    })
}

fn clear_kept(iso: &mut Isolate) {
    with_cx(iso, |cx, _| cx.clear_kept_objects());
}

fn cleanup(iso: &mut Isolate) -> Result<(), String> {
    with_cx(iso, |cx, rt| {
        cx.cleanup_finalization_registries().map_err(|_| {
            let e: Local<'_> = Local::from_bits(rt.take_pending());
            cx.to_rust_string(e).unwrap()
        })
    })
}

#[test]
fn a_weak_ref_target_is_kept_for_the_job_then_dies() {
    each_mode(|iso, label| {
        let (w, t) = weak_ref_to_fresh(iso);
        // A live target: a global handle keeps it.
        let (w2, t2) = weak_ref_to_fresh(iso);
        let keep2 = hold(iso, t2);
        for _ in 0..3 {
            collect(iso);
            assert!(alive(&iso.rt, t), "kept to the end of the job ({label})");
        }
        assert!(!deref(iso, w), "deref gives the kept target ({label})");
        clear_kept(iso);
        collect(iso);
        assert!(!alive(&iso.rt, t), "dead after ClearKeptObjects ({label})");
        assert!(deref(iso, w), "deref gives undefined ({label})");
        assert!(alive(&iso.rt, t2), "a live target stays ({label})");
        assert!(!deref(iso, w2), "deref of a live target ({label})");
        iso.rt.heap.drop_global(keep2);
        clear_kept(iso);
        collect(iso);
        assert!(deref(iso, w2), "it dies once nothing holds it ({label})");
        iso.rt.heap.drop_global(w);
        iso.rt.heap.drop_global(w2);
    });
}

#[test]
fn deref_keeps_the_target_again() {
    each_mode(|iso, label| {
        let (w, t) = weak_ref_to_fresh(iso);
        let keep = hold(iso, t);
        clear_kept(iso);
        assert_eq!(iso.rt.kept_objects.len(), 0);
        for _ in 0..100 {
            assert!(!deref(iso, w));
        }
        assert_eq!(
            iso.rt.kept_objects.len(),
            1,
            "one entry per value ({label})"
        );
        iso.rt.heap.drop_global(keep);
        collect(iso);
        assert!(alive(&iso.rt, t), "deref kept it ({label})");
        clear_kept(iso);
        collect(iso);
        assert!(
            !alive(&iso.rt, t),
            "and ClearKeptObjects let it go ({label})"
        );
        iso.rt.heap.drop_global(w);
    });
}

/// The negative control of the weak trace: with TARGET traced as a strong slot, the target
/// of the first test survives ClearKeptObjects.
#[test]
fn a_strong_target_slot_keeps_the_target() {
    on_big_stack(|| {
        let mut iso = isolate(cfg(false, false));
        NEGATIVE_CONTROL_STRONG_TARGET.with(|c| c.set(true));
        let (w, t) = weak_ref_to_fresh(&mut iso);
        clear_kept(&mut iso);
        collect(&iso);
        let kept = alive(&iso.rt, t);
        NEGATIVE_CONTROL_STRONG_TARGET.with(|c| c.set(false));
        assert!(kept, "a strong TARGET slot keeps the target");
        collect(&iso);
        assert!(!alive(&iso.rt, t), "the weak trace lets it go");
        iso.rt.heap.drop_global(w);
    });
}

/// A FinalizationRegistry with the callback `name` (rooted by a global handle).
fn new_registry(iso: &mut Isolate, name: &str) -> GlobalHandle {
    with_cx(iso, |cx, rt| {
        let f = global_fn(cx, name);
        let r = cx.new_finalization_registry(Local::null(), f);
        rt.heap.new_global(val(r.bits()))
    })
}

/// Registers a fresh object (that nothing else holds) in `r` with the held value `held` and
/// the token in `token` (None: no token). Returns a weak handle on the target.
fn register_fresh(
    iso: &mut Isolate,
    r: GlobalHandle,
    held: i32,
    token: Option<GlobalHandle>,
) -> WeakHandle {
    with_cx(iso, |cx, rt| {
        let reg: Local<'_> = Local::from_bits(rt.heap.global(r).to_bits());
        let t = cx.new_object();
        let tok: Local<'_> = match token {
            Some(h) => Local::from_bits(rt.heap.global(h).to_bits()),
            None => Local::undefined(),
        };
        cx.finalization_registry_register(reg, t, Local::int32(held), tok)
            .unwrap();
        rt.heap.new_weak(val(t.bits()))
    })
}

fn unregister(iso: &mut Isolate, r: GlobalHandle, token: GlobalHandle) -> bool {
    with_cx(iso, |cx, rt| {
        let reg: Local<'_> = Local::from_bits(rt.heap.global(r).to_bits());
        let tok: Local<'_> = Local::from_bits(rt.heap.global(token).to_bits());
        cx.finalization_registry_unregister(reg, tok).unwrap()
    })
}

/// A fresh object rooted by a global handle (a token).
fn new_token(iso: &mut Isolate) -> GlobalHandle {
    with_cx(iso, |cx, rt| {
        let o = cx.new_object();
        rt.heap.new_global(val(o.bits()))
    })
}

#[test]
fn a_registry_calls_back_after_the_gc_not_during_it() {
    each_mode(|iso, label| {
        let r = new_registry(iso, "cb");
        let t1 = register_fresh(iso, r, 1, None);
        let t2 = register_fresh(iso, r, 2, None);
        let keep3 = new_token(iso);
        with_cx(iso, |cx, rt| {
            let reg: Local<'_> = Local::from_bits(rt.heap.global(r).to_bits());
            let t: Local<'_> = Local::from_bits(rt.heap.global(keep3).to_bits());
            cx.finalization_registry_register(reg, t, Local::int32(3), Local::undefined())
                .unwrap();
        });
        collect(iso);
        assert!(!alive(&iso.rt, t1) && !alive(&iso.rt, t2), "{label}");
        assert_eq!(
            take_log(),
            Vec::<i32>::new(),
            "no callback during a GC ({label})"
        );
        assert_eq!(cleanup(iso), Ok(()));
        let mut got = take_log();
        got.sort_unstable();
        assert_eq!(got, vec![1, 2], "each held value once ({label})");
        collect(iso);
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(take_log(), Vec::<i32>::new(), "not twice ({label})");
        // The live target (3) dies once its handle goes.
        iso.rt.heap.drop_global(keep3);
        collect(iso);
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(take_log(), vec![3], "{label}");
        iso.rt.heap.drop_global(r);
    });
}

#[test]
fn unregister_removes_live_and_pending_cells() {
    each_mode(|iso, label| {
        let r = new_registry(iso, "cb");
        let tok = new_token(iso);
        let other = new_token(iso);
        assert!(!unregister(iso, r, tok), "empty registry ({label})");
        let dead = register_fresh(iso, r, 1, Some(tok));
        let keep = new_token(iso);
        with_cx(iso, |cx, rt| {
            let reg: Local<'_> = Local::from_bits(rt.heap.global(r).to_bits());
            let t: Local<'_> = Local::from_bits(rt.heap.global(keep).to_bits());
            let k: Local<'_> = Local::from_bits(rt.heap.global(tok).to_bits());
            cx.finalization_registry_register(reg, t, Local::int32(2), k)
                .unwrap();
        });
        let _ = register_fresh(iso, r, 3, Some(other));
        collect(iso);
        assert!(!alive(&iso.rt, dead), "{label}");
        // The cell of target 1 is empty and its callback has not run; unregister drops it
        // with the live cell of target 2 (same token).
        assert!(unregister(iso, r, tok), "{label}");
        assert!(!unregister(iso, r, tok), "already gone ({label})");
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(
            take_log(),
            vec![3],
            "only the cell with another token ({label})"
        );
        iso.rt.heap.drop_global(keep);
        collect(iso);
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(
            take_log(),
            Vec::<i32>::new(),
            "target 2 was unregistered ({label})"
        );
        for h in [r, tok, other] {
            iso.rt.heap.drop_global(h);
        }
    });
}

#[test]
fn tokens_and_registries_are_weak() {
    each_mode(|iso, label| {
        // A token that nothing else holds dies; its cell stays and calls back.
        let r = new_registry(iso, "cb");
        let keep = new_token(iso);
        let tok = new_token(iso);
        let tok_weak = watch(iso, tok);
        with_cx(iso, |cx, rt| {
            let reg: Local<'_> = Local::from_bits(rt.heap.global(r).to_bits());
            let t: Local<'_> = Local::from_bits(rt.heap.global(keep).to_bits());
            let k: Local<'_> = Local::from_bits(rt.heap.global(tok).to_bits());
            cx.finalization_registry_register(reg, t, Local::int32(7), k)
                .unwrap();
        });
        iso.rt.heap.drop_global(tok);
        collect(iso);
        assert!(!alive(&iso.rt, tok_weak), "the token is weak ({label})");
        let cells = object::slot_get(iso.rt.heap.global(r).to_bits(), registry::CELLS);
        assert_eq!(
            weakref::token(cells),
            UNDEFINED,
            "its slot is cleared ({label})"
        );
        iso.rt.heap.drop_global(keep);
        collect(iso);
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(take_log(), vec![7], "{label}");
        // A registry that dies with its targets calls nothing and leaves the chain.
        let r2 = new_registry(iso, "cb");
        let r2_weak = watch(iso, r2);
        let _ = register_fresh(iso, r2, 8, None);
        iso.rt.heap.drop_global(r2);
        collect(iso);
        assert!(!alive(&iso.rt, r2_weak), "{label}");
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(
            take_log(),
            Vec::<i32>::new(),
            "a dead registry calls nothing ({label})"
        );
        // The chain now holds only `r`.
        let first = iso.rt.kept_objects.registries();
        assert_eq!(weakref::target(first), iso.rt.heap.global(r).to_bits());
        assert_eq!(weakref::next(first), UNDEFINED, "{label}");
        iso.rt.heap.drop_global(r);
    });
}

#[test]
fn a_throwing_callback_leaves_the_rest() {
    each_mode(|iso, label| {
        let r = new_registry(iso, "thrower");
        for i in 0..3 {
            let _ = register_fresh(iso, r, i, None);
        }
        collect(iso);
        THROW_AFTER.with(|t| t.set(0));
        let e = cleanup(iso).unwrap_err();
        assert!(e.contains("thrower"), "{e} ({label})");
        assert_eq!(take_log().len(), 1, "it stopped at the throw ({label})");
        THROW_AFTER.with(|t| t.set(10));
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(take_log().len(), 2, "the next call did the rest ({label})");
        assert_eq!(cleanup(iso), Ok(()));
        assert_eq!(take_log().len(), 0, "{label}");
        iso.rt.heap.drop_global(r);
    });
}

/// With S1 on, a target that is young when its cell is made dies in a minor collection;
/// the hook calls back with its held value.
#[test]
fn a_minor_gc_clears_a_young_target() {
    on_big_stack(|| {
        let mut iso = isolate(cfg(false, true));
        iso.register_host_function("cb", 1, cb);
        take_log();
        let r = new_registry(&mut iso, "cb");
        // A full collection first: the registry is old from here on.
        collect(&iso);
        let t = register_fresh(&mut iso, r, 5, None);
        let (w, wt) = weak_ref_to_fresh(&mut iso);
        clear_kept(&mut iso);
        let before = iso.rt.heap.stats().minor_collections;
        collect_young(&iso);
        assert_eq!(
            iso.rt.heap.stats().minor_collections,
            before + 1,
            "a minor GC ran"
        );
        assert!(!alive(&iso.rt, t), "the young target died in the minor GC");
        assert!(!alive(&iso.rt, wt), "the WeakRef target too");
        assert!(deref(&mut iso, w));
        assert_eq!(cleanup(&mut iso), Ok(()));
        assert_eq!(take_log(), vec![5]);
        iso.rt.heap.drop_global(r);
        iso.rt.heap.drop_global(w);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

#[test]
fn wrong_receivers_throw() {
    each_mode(|iso, label| {
        with_cx(iso, |cx, rt| {
            let o = cx.new_object();
            let e = cx.weak_ref_deref(o).map(|_| ());
            assert!(e.is_err(), "{label}");
            rt.take_pending();
            // An object of the class with other slots (made with `new_object_with_slots`) is
            // not a WeakRef.
            let fake = cx.new_object_with_slots(Local::null(), ObjectClass::WeakRef, 1);
            assert!(cx.weak_ref_deref(fake).is_err(), "{label}");
            rt.take_pending();
            let fake =
                cx.new_object_with_slots(Local::null(), ObjectClass::FinalizationRegistry, 3);
            assert!(
                cx.finalization_registry_unregister(fake, o).is_err(),
                "{label}"
            );
            rt.take_pending();
            let f = global_fn(cx, "cb");
            let reg = cx.new_finalization_registry(Local::null(), f);
            assert!(cx.weak_ref_deref(reg).is_err(), "{label}");
            rt.take_pending();
            let w = cx.new_weak_ref(Local::null(), o);
            assert!(
                cx.finalization_registry_register(w, o, Local::int32(1), Local::undefined())
                    .is_err(),
                "{label}"
            );
            rt.take_pending();
        });
    });
}
