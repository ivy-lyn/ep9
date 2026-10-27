//! Tests of the instanceof cache (`instance_ic.rs`, M4-P3 IC2): the slot states (mono, poly
//! 2 to 4, mega), the hit (a primitive, the chain walk, a new `prototype` value), the guards
//! (the rhs shape, the validity cell of its chain), the `SLOW` cases (bound, own or custom
//! `@@hasInstance`), the throws, and the weak key and shape.
//!
//! Each test runs under every dispatch expansion, with and without GC stress, with eager
//! feedback (`tests::ic::each`). The script defines `f0(a, b)` = `a instanceof b` (function 1;
//! its only slot is the InstanceOf slot), `mk()` that returns a new function, the empty
//! functions `C0` to `C5` and `Yes()` that returns true.

use std::hint::black_box;

use ep9_bytecode::{FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionId, VerifiedUnit};

use super::ic::{MASK, T, below_a_pad, collect_after_a_pad, each, int, key, object_prototype};
use super::{Asm, isolate, meta, unit};
use crate::bound;
use crate::feedback;
use crate::frame::{FALSE, TRUE, UNDEFINED};
use crate::ic::{self, State, hk};
use crate::instance_ic::io;
use crate::object::{self, attr, cell, rd, rd_i32};
use crate::property;
use crate::props;
use crate::runtime::{Atom, Root, val};
use crate::shape;
use crate::symbol::{self, WellKnownSymbol};
use crate::validity;
use crate::{IsolateConfig, Value};

/// The kind [`ic::describe_handler`] gives for an INSTANCEOF cell.
const INSTANCEOF: i32 = hk::INSTANCEOF | 0x100;
/// The empty functions.
const CLASSES: usize = 6;

/// `f0(a, b)`: `a instanceof b`.
fn instance_of_fn() -> FunctionCode {
    let mut b = FunctionBuilder::new(meta("f0", 2, 0, 0, 0));
    let (a, x) = (b.parameter(0), b.parameter(1));
    let s = b.feedback_slot(FeedbackSlotKind::InstanceOf);
    assert_eq!(s.index(), 0);
    b.ldar(x).test_instance_of(a, s).return_();
    b.finish().unwrap()
}

/// `mk()`: a new closure of function `inner` (an empty function).
fn mk_fn(inner: u32) -> FunctionCode {
    let mut b = FunctionBuilder::new(meta("mk", 0, 0, 0, 0));
    let cs = b.feedback_slot(FeedbackSlotKind::CreateClosure);
    let k = b.function_constant(FunctionId(inner));
    b.create_closure(k, cs, 0).return_();
    b.finish().unwrap()
}

fn empty_fn(name: &str, parent: u32) -> FunctionCode {
    let mut b = FunctionBuilder::new(meta(name, 0, parent, 0, 0));
    b.lda_undefined().return_();
    b.finish().unwrap()
}

fn yes_fn() -> FunctionCode {
    let mut b = FunctionBuilder::new(meta("Yes", 1, 0, 0, 0));
    b.lda_true().return_();
    b.finish().unwrap()
}

/// Function ids (pre-order, as the verifier's function tree wants): 0 script, 1 f0, 2 mk,
/// 3 mk's inner function, 4.. C0-C5, then Yes.
fn program() -> VerifiedUnit {
    let first_class = 4;
    let yes = first_class + CLASSES as u32;
    let mut s = Asm::script(0);
    let mut globals: Vec<(u32, String)> = vec![(1, "f0".into()), (2, "mk".into())];
    globals.extend((0..CLASSES).map(|i| (first_class + i as u32, format!("C{i}"))));
    globals.push((yes, "Yes".into()));
    for (id, name) in &globals {
        let k = s.b.function_constant(FunctionId(*id));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    let mut fns = vec![s.finish(), instance_of_fn(), mk_fn(3), empty_fn("inner", 2)];
    fns.extend((0..CLASSES).map(|i| empty_fn(&format!("C{i}"), 0)));
    fns.push(yes_fn());
    unit("", fns)
}

fn new_t(c: IsolateConfig) -> T {
    let mut iso = isolate(c);
    iso.with_cx_checkpoint(|cx| cx.run_script(program(), "instance_ic.js").map(|_| ()))
        .unwrap();
    T { iso }
}

fn global(t: &T, name: &str) -> u64 {
    t.iso.global_raw(name).unwrap().to_bits()
}

/// `C<i>`.
fn class(t: &T, i: usize) -> u64 {
    global(t, &format!("C{i}"))
}

/// `f.prototype` (made on first use: a constructor's `prototype` is lazy).
fn prototype_of(t: &mut T, f: u64) -> u64 {
    let k = t.rt().atom(Atom::Prototype);
    property::get(&mut t.iso.cx, f, k).unwrap()
}

/// A new object whose `[[Prototype]]` is `f.prototype`.
fn instance(t: &mut T, f: u64) -> u64 {
    let p = prototype_of(t, f);
    object::new_object(t.rt(), p)
}

/// `a instanceof b` through `f0`.
fn test(t: &mut T, a: u64, b: u64) -> bool {
    match t.ok(0, a, b) {
        TRUE => true,
        FALSE => false,
        v => panic!("instanceof gave {v:#x}"),
    }
}

/// The handler the slot keeps for `f`, as (kind, +16, +24, +32).
fn describe(t: &T, f: u64) -> Option<(i32, u64, u64, i32)> {
    t.handler(0, f).map(ic::describe_handler)
}

fn has_instance(t: &T) -> u64 {
    symbol::well_known(t.rt(), WellKnownSymbol::HasInstance)
}

// ---- states and hits -------------------------------------------------------------------------

#[test]
fn a_hit_answers_from_the_handler_and_reads_prototype_each_time() {
    each(|c| {
        let mut t = new_t(c);
        let c0 = class(&t, 0);
        let o = instance(&mut t, c0);
        assert!(test(&mut t, o, c0));
        assert_eq!(t.state(0), State::Monomorphic);
        let (k, vc, s, field) = describe(&t, c0).expect("a case for C0");
        assert_eq!(k, INSTANCEOF);
        assert!(validity::is_valid(vc));
        assert_eq!(s, shape::shape_of(c0));
        let f = shape::lookup(t.rt(), s, t.rt().atom(Atom::Prototype)).unwrap();
        assert_eq!(field, i32::from(f.details.field()));
        // Hits: an instance two levels down, an object of another chain, primitives.
        let deeper = object::new_object(t.rt(), o);
        assert!(test(&mut t, deeper, c0));
        let other = object::new_plain_object(t.rt());
        assert!(!test(&mut t, other, c0));
        assert!(!test(&mut t, int(1), c0));
        assert!(!test(&mut t, UNDEFINED, c0));
        // A null-prototype object: the walk ends at null.
        let bare = object::new_object(t.rt(), crate::frame::NULL);
        assert!(!test(&mut t, bare, c0));
        // `C0.prototype = p2`: a value store (no shape change), so the case still hits and
        // reads the new value.
        let p2 = object::new_plain_object(t.rt());
        let pk = t.rt().atom(Atom::Prototype);
        assert!(property::set(&mut t.iso.cx, c0, pk, p2, true).is_ok());
        assert_eq!(shape::shape_of(c0), s);
        let o2 = object::new_object(t.rt(), p2);
        assert!(test(&mut t, o2, c0));
        assert!(!test(&mut t, o, c0));
        assert_eq!(describe(&t, c0).map(|d| d.0), Some(INSTANCEOF));
        assert_eq!(t.state(0), State::Monomorphic);
    });
}

#[test]
fn five_functions_go_poly_2_3_4_then_mega_and_stay_right() {
    each(|c| {
        let mut t = new_t(c);
        // Arrays on the stack, not a `Vec`: the conservative scan must see them.
        let fs: [u64; 5] = core::array::from_fn(|i| class(&t, i));
        let objs: [u64; 5] = core::array::from_fn(|i| instance(&mut t, fs[i]));
        let want = [
            State::Monomorphic,
            State::Polymorphic(2),
            State::Polymorphic(3),
            State::Polymorphic(4),
            State::Megamorphic,
        ];
        for i in 0..5 {
            assert!(test(&mut t, objs[i], fs[i]));
            assert_eq!(t.state(0), want[i], "function {i}");
            for j in 0..=i {
                for (k, &f) in fs.iter().enumerate().take(i + 1) {
                    assert_eq!(test(&mut t, objs[j], f), j == k, "C{j} instance, C{k}");
                }
                if i < 4 {
                    assert_eq!(describe(&t, fs[j]).map(|d| d.0), Some(INSTANCEOF));
                }
            }
        }
        // Megamorphic is sticky; the generic path answers.
        let c5 = class(&t, 5);
        let o5 = instance(&mut t, c5);
        assert!(test(&mut t, o5, c5));
        assert!(!test(&mut t, objs[0], c5));
        assert_eq!(t.state(0), State::Megamorphic);
        t.collect();
        assert!(test(&mut t, objs[2], fs[2]));
    });
}

// ---- guards ------------------------------------------------------------------------------------

#[test]
fn a_new_has_instance_on_the_chain_misses_through_the_validity_cell() {
    each(|c| {
        let mut t = new_t(c);
        let c1 = class(&t, 1);
        let o = object::new_plain_object(t.rt());
        // C1's [[Prototype]] = q, an object whose [[Prototype]] is Function.prototype. C1 is a
        // prototype first, so it keeps a fast (unique) shape through the change (OM-10).
        let child = object::new_object(t.rt(), c1);
        let fp = t.rt().root(Root::FunctionPrototype);
        let q = object::new_object(t.rt(), fp);
        assert!(property::set_prototype_of(t.rt(), c1, q));
        assert!(!shape::is_dictionary(shape::shape_of(c1)));
        assert!(!test(&mut t, child, c1));
        assert!(!test(&mut t, o, c1));
        let (k, vc, ..) = describe(&t, c1).unwrap();
        assert_eq!(k, INSTANCEOF);
        assert!(validity::is_valid(vc));
        // q[@@hasInstance] = Yes: q's cell goes invalid; the generic path calls Yes.
        let yes = global(&t, "Yes");
        let hi = has_instance(&t);
        props::own_define(t.rt(), q, hi, yes, attr::DEFAULT);
        assert!(!validity::is_valid(vc));
        assert!(test(&mut t, o, c1));
        // The chain now has a writable `@@hasInstance`: SLOW, and the generic path every time.
        assert_eq!(t.handler(0, c1), Some(ic::SLOW));
        assert!(test(&mut t, int(3), c1));
        assert_eq!(t.state(0), State::Monomorphic);
    });
}

#[test]
fn an_own_has_instance_changes_the_shape_and_is_slow() {
    each(|c| {
        let mut t = new_t(c);
        let c2 = class(&t, 2);
        let o = instance(&mut t, c2);
        assert!(test(&mut t, o, c2));
        let s = shape::shape_of(c2);
        assert_eq!(describe(&t, c2).map(|d| d.2), Some(s));
        // A static `[Symbol.hasInstance]` (non-writable, as a class field would not be: any
        // own one is SLOW). The shape changes, so the old handler misses.
        let yes = global(&t, "Yes");
        let hi = has_instance(&t);
        props::own_define(t.rt(), c2, hi, yes, 0);
        assert_ne!(shape::shape_of(c2), s);
        let plain = object::new_plain_object(t.rt());
        assert!(test(&mut t, plain, c2));
        assert_eq!(t.handler(0, c2), Some(ic::SLOW));
        assert!(test(&mut t, int(0), c2));
    });
}

#[test]
fn bound_and_non_function_rhs_are_slow_and_throws_record_nothing() {
    each(|c| {
        let mut t = new_t(c);
        let c3 = class(&t, 3);
        let o = instance(&mut t, c3);
        let b = bound::new_bound_function(&mut t.iso.cx, c3, UNDEFINED, &[]).unwrap();
        // The slot's keys are weak, and the state below counts `b`'s case: a handle keeps `b`
        // after its last use. Without it a release build under GC stress freed `b`, and the
        // record of `h` took its dead case (Monomorphic; M4-P4 P4-ORCH-9).
        let kept_b = t.keep(b);
        assert!(test(&mut t, o, b));
        assert_eq!(t.handler(0, b), Some(ic::SLOW));
        assert!(test(&mut t, o, b));
        // A plain object with a callable `@@hasInstance`.
        let h = object::new_plain_object(t.rt());
        let yes = global(&t, "Yes");
        let hi = has_instance(&t);
        props::own_define(t.rt(), h, hi, yes, attr::DEFAULT);
        let kept_h = t.keep(h);
        assert!(test(&mut t, int(5), h));
        assert_eq!(t.handler(0, h), Some(ic::SLOW));
        assert_eq!(t.state(0), State::Polymorphic(2));
        // Throws: a primitive rhs, a non-callable object; nothing new is recorded.
        t.throws(0, o, int(1), "TypeError");
        let plain = object::new_plain_object(t.rt());
        t.throws(0, o, plain, "TypeError");
        assert_eq!(t.state(0), State::Polymorphic(2));
        t.release(kept_b);
        t.release(kept_h);
    });
}

#[test]
fn a_non_object_prototype_misses_and_throws_and_an_arrow_records_nothing() {
    each(|c| {
        let mut t = new_t(c);
        let c4 = class(&t, 4);
        let o = instance(&mut t, c4);
        assert!(test(&mut t, o, c4));
        assert_eq!(describe(&t, c4).map(|d| d.0), Some(INSTANCEOF));
        let pk = t.rt().atom(Atom::Prototype);
        assert!(property::set(&mut t.iso.cx, c4, pk, int(7), true).is_ok());
        // A primitive lhs is false before `prototype` is read (OrdinaryHasInstance step 3).
        assert!(!test(&mut t, int(1), c4));
        t.throws(0, o, c4, "TypeError");
        // The case is still current: nothing new to write.
        assert_eq!(describe(&t, c4).map(|d| d.0), Some(INSTANCEOF));
        // A function whose lazy `prototype` is not made yet, with a primitive lhs: false, and
        // no case (its `prototype` is not in the shape).
        let c5 = class(&t, 5);
        assert!(!test(&mut t, int(1), c5));
        assert_eq!(t.handler(0, c5), None);
    });
}

#[test]
fn the_site_is_read_only_at_a_test_instance_of() {
    each(|c| {
        let mut t = new_t(c);
        // Function.prototype[@@hasInstance].call(C0, o) from Rust: a host call that reaches
        // `property::ordinary_has_instance`, not a TestInstanceOf: no slot changes.
        // f0 runs once (a TypeError, which records nothing), so it has its vector.
        t.throws(0, int(1), int(1), "TypeError");
        let c0 = class(&t, 0);
        let o = instance(&mut t, c0);
        assert!(property::ordinary_has_instance(&mut t.iso.cx, c0, o).unwrap());
        let b = bound::new_bound_function(&mut t.iso.cx, c0, UNDEFINED, &[]).unwrap();
        assert!(property::ordinary_has_instance(&mut t.iso.cx, b, o).unwrap());
        assert_eq!(t.state(0), State::Uninitialized);
        let v = t.vector(0);
        assert_eq!(feedback::key(v, 0), UNDEFINED);
    });
}

// ---- weak references ---------------------------------------------------------------------------

/// Calls `mk()` for a new function `g`, then `o instanceof g` for an instance `o`, and checks
/// the slot is monomorphic on `g`. Returns the masked addresses of `g`, its shape and `o`.
#[inline(never)]
fn made_a_lone_function(t: &mut T) -> [usize; 3] {
    let mk = global(t, "mk");
    let g = t
        .iso
        .call_raw(val(mk), Value::UNDEFINED, &[])
        .unwrap()
        .to_bits();
    let o = instance(t, g);
    assert!(test(t, o, g));
    assert_eq!(t.state(0), State::Monomorphic);
    assert_eq!(feedback::key(t.vector(0), 0), g);
    let s = shape::shape_of(g);
    [
        black_box(g as usize) ^ MASK,
        black_box(s as usize) ^ MASK,
        black_box(o as usize) ^ MASK,
    ]
}

#[test]
fn a_dead_function_and_its_shape_read_undefined() {
    each(|c| {
        if c.heap.gc_stress != 0 {
            // GC stress collects at every allocation of `made_a_lone_function`; the check
            // needs one collection after it.
            return;
        }
        let mut t = new_t(c);
        let (dead, pad_low) = below_a_pad(|| made_a_lone_function(&mut t));
        collect_after_a_pad(&mut t, pad_low, &dead);
        let v = t.vector(0);
        assert_eq!(feedback::key(v, 0), UNDEFINED);
        assert_eq!(t.state(0), State::Uninitialized);
        // The handler (the strong value half) stays; its shape is weak.
        let h = feedback::value(v, 0);
        assert_eq!(rd_i32(cell(h), io::KIND), hk::INSTANCEOF);
        assert_eq!(rd(cell(h), io::SHAPE), UNDEFINED);
        // The site still answers, and records a new case.
        let c0 = class(&t, 0);
        let o = instance(&mut t, c0);
        assert!(test(&mut t, o, c0));
        assert_eq!(t.state(0), State::Monomorphic);
    });
}

#[test]
fn the_object_prototype_function_is_cached_too() {
    each(|c| {
        // `Object` (a host constructor): its `prototype` is a non-writable data property.
        let mut t = new_t(c);
        let object_ctor = global(&t, "Object");
        let o = object::new_plain_object(t.rt());
        assert!(test(&mut t, o, object_ctor));
        assert_eq!(describe(&t, object_ctor).map(|d| d.0), Some(INSTANCEOF));
        let bare = object::new_object(t.rt(), crate::frame::NULL);
        assert!(!test(&mut t, bare, object_ctor));
        let p = object_prototype(t.rt());
        assert_eq!(prototype_of(&mut t, object_ctor), p);
        let k = key(t.rt(), "x");
        assert!(!test(&mut t, k, object_ctor));
    });
}
