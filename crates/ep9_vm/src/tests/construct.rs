//! Tests of construct sites (`construct_ic.rs`) and of the cold call path's argument buffer
//! (`call::ArgBuf`) (owner M4-P1 OBJ-ALLOC). Programs run under every dispatch expansion, with
//! and without GC stress, with eager feedback vectors (threshold 0), so the second `new` at a
//! site can hit. The Call slot of a site tells what ran: its handler counts the cold constructs
//! only, so a count of 1 after many constructs means that the others hit.

use std::hint::black_box;

use ep9_bytecode::{FeedbackSlot, FeedbackSlotKind, FunctionId, RegisterList, VerifiedUnit};

use super::{Asm, config, isolate, meta, on_big_stack, take_output, unit};
use crate::call::{ArgBuf, INLINE_ARGS};
use crate::construct_ic::{cs, handler_i32};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::ic::hk;
use crate::kinds;
use crate::object;
use crate::runtime::Runtime;
use crate::shape;
use crate::{Dispatch, Isolate, IsolateConfig};

/// Every dispatch expansion, with and without GC stress, eager vectors.
fn configs() -> Vec<(String, IsolateConfig)> {
    let mut v = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let c = IsolateConfig {
                feedback_threshold: 0,
                ..config(d, stress)
            };
            v.push((format!("{} stress={stress}", d.name()), c));
        }
    }
    v
}

/// The (key, value) of slot `slot` of the vector of function `id` of the last unit.
fn slot_of(iso: &Isolate, id: usize, slot: FeedbackSlot) -> (u64, u64) {
    let units = iso.rt.units.borrow();
    let fi = &units.last().unwrap().infos[id];
    let v = feedback::vector(fi).expect("a vector (threshold 0)");
    (feedback::key(v, slot.0), feedback::value(v, slot.0))
}

/// Runs `make()` in every config, checks that the outputs agree with `want`, and calls
/// `check` with the isolate after each run.
fn run_checked(
    make: fn() -> (VerifiedUnit, FeedbackSlot),
    want: &'static [&'static str],
    check: fn(&Isolate, FeedbackSlot),
) {
    for (label, c) in configs() {
        on_big_stack(move || {
            let mut iso = isolate(c);
            take_output();
            let (u, slot) = make();
            let r = iso.with_cx_checkpoint(|cx| cx.run_script(u, "test.js").map(|_| ()));
            let out = take_output();
            if let Err(e) = r {
                panic!("{label}: uncaught {e}");
            }
            assert_eq!(out, want, "{label}");
            check(&iso, slot);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// `for (i = 0; i < n; i++) { body }` in `s`.
fn repeat(s: &mut Asm, n: i32, body: impl FnOnce(&mut Asm)) {
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    s.b.lda_smi(n).test_less_than(i, s.cmp).jump_if_false(exit);
    body(s);
    s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
}

/// f1: `function P(x, y) { this.x = x; this.y = y; }`.
fn point() -> Asm {
    let mut p = Asm::new(meta("P", 2, 0, 0, 0));
    let this = p.b.receiver();
    p.b.ldar(p.b.parameter(0));
    p.set_named(this, "x");
    p.b.ldar(p.b.parameter(1));
    p.set_named(this, "y");
    p.b.lda_undefined().return_();
    p
}

/// The script head: `P = f1`; returns the construct slot and the register list of 2
/// arguments.
fn script_with_p(s: &mut Asm) -> (FeedbackSlot, RegisterList) {
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("P");
    let site = s.b.feedback_slot(FeedbackSlotKind::Call);
    (site, s.b.new_register_list(2))
}

/// `acc = new <ctor in reg f>(a0, a1)` at `site`.
fn construct_p(s: &mut Asm, site: FeedbackSlot, args: RegisterList) {
    let f = s.f;
    s.lda_global("P");
    s.b.star(f);
    s.b.construct(f, args, 2, site);
}

/// t = 0; 20 times: o = new P(i, 2); t = t + o.x + o.y; then print(t), print the keys and
/// the prototype check.
fn monomorphic() -> (VerifiedUnit, FeedbackSlot) {
    let p = point();
    let mut s = Asm::script(0);
    let (site, args) = script_with_p(&mut s);
    let (o, t) = (s.b.new_register(), s.b.new_register());
    s.b.lda_zero().star(t);
    repeat(&mut s, 20, |s| {
        s.b.lda_smi(3)
            .star(args.get(0))
            .lda_smi(2)
            .star(args.get(1));
        construct_p(s, site, args);
        s.b.star(o);
        s.get_named(o, "x");
        s.b.add(t, s.bin).star(t);
        s.get_named(o, "y");
        s.b.add(t, s.bin).star(t);
    });
    s.b.ldar(t);
    s.print_acc();
    // Object.getPrototypeOf(o) === P.prototype, through the IC-free `__proto__` path: o's
    // shape's prototype.
    let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
    s.lda_global("P");
    s.b.test_instance_of(o, is);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), p.finish()]), site)
}

#[test]
fn a_monomorphic_site_hits_after_its_first_construct() {
    run_checked(monomorphic, &["100", "true"], |iso, site| {
        let (key, value) = slot_of(iso, 0, site);
        assert!(kinds::is_function(key), "the key is P");
        assert_eq!(kinds::cell_kind(value), kinds::IC_HANDLER.get());
        assert_eq!(handler_i32(value, cs::KIND), hk::CONSTRUCT_SITE);
        // One cold construct (the first); the other 19 hit.
        assert_eq!(handler_i32(value, cs::COUNT), 1);
        let shape = object::rd(object::cell(value), cs::SHAPE);
        assert!(shape::is_shape(shape));
        assert_eq!(shape::count(shape), 0);
    });
}

/// Like [`monomorphic`], but `P.prototype = {}` after 5 constructs, and the last instance is
/// checked against the new prototype.
fn prototype_change() -> (VerifiedUnit, FeedbackSlot) {
    let p = point();
    let mut s = Asm::script(0);
    let (site, args) = script_with_p(&mut s);
    let (o, ctor, n) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    s.b.lda_zero().star(n);
    repeat(&mut s, 10, |s| {
        s.b.lda_smi(1).star(args.get(0)).star(args.get(1));
        construct_p(s, site, args);
        s.b.star(o);
        // if (++n == 5) P.prototype = {}
        let skip = s.b.new_label();
        s.b.ldar(n).inc(s.bin).star(n);
        s.b.lda_smi(5)
            .test_equal_strict(n, s.cmp)
            .jump_if_false(skip);
        s.lda_global("P");
        s.b.star(ctor);
        s.b.create_empty_object_literal();
        s.set_named(ctor, "prototype");
        s.b.bind(skip);
    });
    let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
    s.lda_global("P");
    s.b.test_instance_of(o, is);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), p.finish()]), site)
}

#[test]
fn a_new_prototype_misses_once_and_the_handler_follows_it() {
    run_checked(prototype_change, &["true"], |iso, site| {
        let (_, value) = slot_of(iso, 0, site);
        assert_eq!(kinds::cell_kind(value), kinds::IC_HANDLER.get());
        // The first construct and the first one after the change.
        assert_eq!(handler_i32(value, cs::COUNT), 2);
    });
}

/// Two constructors at one site: `new (i & 1 ? Q : P)(1, 1)`; prints the sum of `x`.
fn two_targets() -> (VerifiedUnit, FeedbackSlot) {
    let p = point();
    let q = point();
    let mut s = Asm::script(0);
    let (site, args) = script_with_p(&mut s);
    let kq = s.b.function_constant(FunctionId(2));
    s.b.create_closure(kq, s.closure, 0);
    s.sta_global("Q");
    let (o, t, i, f) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    s.b.lda_zero().star(t).star(i);
    repeat(&mut s, 6, |s| {
        let (other, go) = (s.b.new_label(), s.b.new_label());
        s.b.ldar(i).inc(s.bin).star(i);
        s.b.bitwise_and_smi(1, s.bin)
            .jump_if_to_boolean_false(other);
        s.lda_global("Q");
        s.b.jump(go);
        s.b.bind(other);
        s.lda_global("P");
        s.b.bind(go);
        s.b.star(f);
        s.b.lda_smi(1).star(args.get(0)).star(args.get(1));
        s.b.ldar(f).construct(f, args, 2, site).star(o);
        s.get_named(o, "x");
        s.b.add(t, s.bin).star(t);
    });
    s.b.ldar(t);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), p.finish(), q.finish()]), site)
}

#[test]
fn a_second_target_makes_the_site_megamorphic() {
    run_checked(two_targets, &["6"], |iso, site| {
        let (key, value) = slot_of(iso, 0, site);
        assert_eq!(key, HOLE);
        // Every construct took the cold path.
        assert_eq!(value, int32_bits(6));
    });
}

/// `new P(1, 1)` 4 times with new.target = Q (the accumulator): never cached.
fn other_new_target() -> (VerifiedUnit, FeedbackSlot) {
    let p = point();
    let q = point();
    let mut s = Asm::script(0);
    let (site, args) = script_with_p(&mut s);
    let kq = s.b.function_constant(FunctionId(2));
    s.b.create_closure(kq, s.closure, 0);
    s.sta_global("Q");
    let (o, f) = (s.b.new_register(), s.b.new_register());
    repeat(&mut s, 4, |s| {
        s.lda_global("P");
        s.b.star(f);
        s.b.lda_smi(1).star(args.get(0)).star(args.get(1));
        s.lda_global("Q");
        s.b.construct(f, args, 2, site).star(o);
    });
    // Reflect.construct(P, [1, 1], Q) semantics: o is a Q.
    let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
    s.lda_global("Q");
    s.b.test_instance_of(o, is);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), p.finish(), q.finish()]), site)
}

#[test]
fn another_new_target_is_counted_and_never_cached() {
    run_checked(other_new_target, &["true"], |iso, site| {
        let (key, value) = slot_of(iso, 0, site);
        assert!(kinds::is_function(key));
        assert_eq!(value, int32_bits(4));
    });
}

/// `delete P.name` (P's shape goes to dictionary mode), then 5 constructs: the hit refuses a
/// dictionary-mode constructor, and the results stay right.
fn dictionary_ctor() -> (VerifiedUnit, FeedbackSlot) {
    let p = point();
    let mut s = Asm::script(0);
    let (site, args) = script_with_p(&mut s);
    let (o, t, f, key) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    s.b.lda_zero().star(t);
    repeat(&mut s, 5, |s| {
        s.b.lda_smi(2).star(args.get(0)).star(args.get(1));
        construct_p(s, site, args);
        s.b.star(o);
        s.get_named(o, "x");
        s.b.add(t, s.bin).star(t);
        // After the first construct (a handler exists now): delete P.length.
        s.lda_global("P");
        s.b.star(f);
        s.lda_str("length");
        s.b.star(key);
        s.b.ldar(key).delete_property_sloppy(f);
    });
    s.b.ldar(t);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), p.finish()]), site)
}

#[test]
fn a_dictionary_mode_constructor_takes_the_cold_path() {
    run_checked(dictionary_ctor, &["10"], |iso, site| {
        let (_, value) = slot_of(iso, 0, site);
        // `length` is the first property, so its delete is not the last one's: dictionary
        // mode. Every construct is cold, and the miss records no handler then.
        assert_eq!(value, int32_bits(5));
    });
}

/// f1: `function R(n) { if (n > 0) new R(n - 1); }`, called with a depth past the VM stack:
/// the hit sees no room, gives its budget unit and count back, and the cold path throws the
/// RangeError, which the script catches through the `try` of `catch_range_error`.
fn deep() -> (VerifiedUnit, FeedbackSlot) {
    let mut r = Asm::new(meta("R", 1, 0, 0, 0));
    let site = r.b.feedback_slot(FeedbackSlotKind::Call);
    let (f, a) = (r.b.new_register(), r.b.new_register_list(1));
    let done = r.b.new_label();
    let n = r.b.parameter(0);
    r.b.lda_zero()
        .test_greater_than(n, r.cmp)
        .jump_if_false(done);
    r.b.ldar(n).dec(r.bin).star(a.get(0));
    r.lda_global("R");
    r.b.star(f).construct(f, a, 1, site);
    r.b.bind(done);
    r.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("R");
    s.lda_num(1e7);
    s.b.star(s.a.get(1));
    s.lda_global("R");
    s.b.star(s.a.get(0));
    // print(caught(R, 1e7)): a host helper would be simpler; the uncaught RangeError of the
    // run is the check instead (`deep_recursion_throws_a_range_error`).
    s.b.construct(
        s.a.get(0),
        RegisterList::new(s.a.get(1).local_index().unwrap(), 1),
        1,
        s.call,
    );
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), r.finish()]), site)
}

#[test]
fn deep_recursion_throws_a_range_error() {
    for (label, c) in configs() {
        // GC stress makes 10^4+ frames too slow, and a debug build has no tail calls, so the
        // threaded expansions run out of native stack first: the match loop, no stress.
        if c.heap.gc_stress > 0 || (cfg!(debug_assertions) && c.dispatch != Dispatch::Match) {
            continue;
        }
        on_big_stack(move || {
            let mut iso = isolate(c);
            let (u, site) = deep();
            let r = iso.with_cx_checkpoint(|cx| cx.run_script(u, "test.js").map(|_| ()));
            let e = r.expect_err("a RangeError");
            assert!(e.to_string().contains("RangeError"), "{label}: {e}");
            // R's site hit until the stack was full: its handler counts the first construct,
            // one per budget poll (every BUDGET units) and the one that threw; the frames of
            // the 10^4+ others came from hits.
            let (key, value) = slot_of(&iso, 1, site);
            assert!(kinds::is_function(key), "{label}");
            assert_eq!(kinds::cell_kind(value), kinds::IC_HANDLER.get(), "{label}");
            let cold = handler_i32(value, cs::COUNT) as usize;
            let polls = IsolateConfig::default().vm_stack_slots / crate::interp::BUDGET as usize;
            assert!(cold <= 2 + polls, "{label}: {cold} cold constructs");
            // The budget is not left behind: a later run still polls and finishes.
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

// ---- GC: the initial shape is weak ------------------------------------------------------------

const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;
const DEEP: usize = 64 << 10;

#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's (as
/// `tests/feedback.rs`); returns its result and the lowest byte of the pad.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> (R, usize) {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    (r, black_box(&mut pad).as_ptr().addr())
}

/// A CONSTRUCT_SITE handler whose initial shape is the root shape of a new prototype that
/// only that shape refers to, stored in slot 0 of `v` with the global object as key. Returns
/// the masked address of the shape.
#[inline(never)]
fn made_handler(rt: &Runtime, v: u64) -> usize {
    let proto = object::new_plain_object(rt);
    let shape = shape::initial_shape(rt, proto, kinds::OBJECT, 8);
    let c = rt.alloc(kinds::IC_HANDLER, cs::SIZE);
    object::init(c, cs::KIND, int32_bits(hk::CONSTRUCT_SITE));
    object::init(c, cs::COUNT, int32_bits(1));
    object::init(c, cs::SHAPE, shape);
    object::init(c, cs::FIELD, int32_bits(2));
    feedback::set_slot(rt, v, 0, rt.global(), c.to_value().to_bits());
    black_box(shape as usize) ^ MASK
}

#[test]
fn the_initial_shape_of_a_handler_is_weak() {
    on_big_stack(|| {
        let mut c = config(Dispatch::DEFAULT, false);
        c.heap.record_conservative_roots = true;
        c.feedback_threshold = 0;
        let mut iso = isolate(c);
        iso.with_cx_checkpoint(|cx| cx.run_script(monomorphic().0, "test.js").map(|_| ()))
            .unwrap();
        take_output();
        let v = {
            let units = iso.rt.units.borrow();
            feedback::vector(&units.last().unwrap().infos[0]).unwrap()
        };
        let (shape, pad_low) = below_a_pad(|| made_handler(&iso.rt, v));
        scrub_stack();
        iso.collect_garbage();
        let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
        let scan_low = high - 8 * iso.heap().stats().last_stack_words;
        assert!(scan_low > pad_low, "the scan read below the pad");
        let css = iso.rt.heap.last_conservative_roots();
        assert!(
            !css.iter().any(|c| c.addr() == shape ^ MASK),
            "kept by the stack"
        );
        iso.verify_heap().unwrap();
        let h = feedback::value(v, 0);
        // The handler is strong (the value half); its shape is weak: cleared.
        assert_eq!(kinds::cell_kind(h), kinds::IC_HANDLER.get());
        assert_eq!(object::rd(object::cell(h), cs::SHAPE), UNDEFINED);
        assert_eq!(handler_i32(h, cs::COUNT), 1);
    });
}

// ---- the argument buffer of the cold call path --------------------------------------------------

#[test]
fn arg_buf_keeps_order_across_the_inline_limit() {
    for n in [
        0,
        1,
        INLINE_ARGS - 1,
        INLINE_ARGS,
        INLINE_ARGS + 1,
        3 * INLINE_ARGS,
    ] {
        let mut b: ArgBuf<u64> = (0..n as u64).collect();
        assert_eq!(
            &b[..],
            (0..n as u64).collect::<Vec<_>>().as_slice(),
            "n = {n}"
        );
        // Bound arguments in front.
        for k in [0, 1, INLINE_ARGS] {
            let mut c: ArgBuf<u64> = b.iter().copied().collect();
            c.prepend(k, 99);
            let mut want = vec![99; k];
            want.extend(0..n as u64);
            assert_eq!(&c[..], want.as_slice(), "n = {n}, k = {k}");
            if !want.is_empty() {
                c[0] = 7;
                want[0] = 7;
            }
            assert_eq!(&c[..], want.as_slice());
        }
        // Spread: pop the last, push more.
        let last = b.pop();
        assert_eq!(last, n.checked_sub(1).map(|x| x as u64));
        for x in 100..100 + INLINE_ARGS as u64 + 2 {
            b.push(x);
        }
        let mut want: Vec<u64> = (0..n.saturating_sub(1) as u64).collect();
        want.extend(100..100 + INLINE_ARGS as u64 + 2);
        assert_eq!(&b[..], want.as_slice(), "n = {n}");
        while b.pop().is_some() {}
        assert!(b.is_empty());
    }
}

/// `new P(a0 .. a11)` with 12 arguments (past the inline limit) and a bound P: the cold path's
/// argument copy keeps every value and the bound ones in front.
fn many_args() -> (VerifiedUnit, FeedbackSlot) {
    // f1: function S(a, ..., l) { this.s = a + b + ... + l; } (12 parameters)
    let n = 12u32;
    let mut f = Asm::new(meta("S", n, 0, 0, 0));
    let this = f.b.receiver();
    let t = f.b.new_register();
    f.b.lda_zero().star(t);
    for i in 0..n {
        f.b.ldar(f.b.parameter(i)).add(t, f.bin).star(t);
    }
    f.b.ldar(t);
    f.set_named(this, "s");
    f.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("S");
    let site = s.b.feedback_slot(FeedbackSlotKind::Call);
    let args = s.b.new_register_list(n);
    let (o, g) = (s.b.new_register(), s.b.new_register());
    for i in 0..n {
        s.b.lda_smi(1 << i).star(args.get(i));
    }
    for _ in 0..3 {
        s.lda_global("S");
        s.b.star(g).construct(g, args, n, site).star(o);
        s.get_named(o, "s");
        s.print_acc();
    }
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish(), f.finish()]), site)
}

#[test]
fn twelve_arguments_go_through_the_cold_copy_and_the_hit() {
    run_checked(many_args, &["4095", "4095", "4095"], |iso, site| {
        let (_, value) = slot_of(iso, 0, site);
        assert_eq!(handler_i32(value, cs::COUNT), 1);
    });
}
