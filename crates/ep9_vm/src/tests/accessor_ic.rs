//! Tests of the accessor ICs (`ic.rs` IC_HANDLER kinds 9 GETTER, 10 SETTER) and the
//! dictionary-holder handler (11 PROTO_DICT_LOAD) (M6-P1 ACCIC; the H17 "Accessor ICs" part and
//! the H17 Dictionary v1 holder part; plan v11 V11-D17): own and inherited getters and setters,
//! a replaced pair, a holder that turns dictionary, a validity-cell pop, GC with a cached weak
//! holder, the cold hits through `super`, and a store that calls the setter every time after a
//! GETTER case is warm (MODES-CHECK-w24 MC-w24-8) under every preset this build runs.
//!
//! The harness is `tests::ic` (`T`, one site per function, every dispatch expansion, with and
//! without GC stress, eager feedback). Getters and setters are host functions that count their
//! calls ([`calls`]); [`ic::COLD_HITS`] counts the cold hits, so a test can tell a cached case
//! from the generic path.

use core::cell::Cell;
use std::hint::black_box;

use ep9_bytecode::{FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionFlags, FunctionId};

use super::ic::{
    MASK, SLOT, Site, T, below_a_pad, collect_after_a_pad, each, int, key, object_prototype,
    object_with, own,
};
use super::{Asm, meta, unit};
use crate::accessor;
use crate::frame::UNDEFINED;
use crate::ic::{self, KEY_STRING, State, hk, smi};
use crate::object::{self, attr};
use crate::props;
use crate::realm;
use crate::runtime::{Root, Runtime};
use crate::shape;
use crate::validity;
use crate::{Args, Cx, ExecOptions, Feedback, HostFn, Isolate, IsolateConfig, JsResult, Local};

/// The kinds [`ic::describe_handler`] gives for these IC_HANDLER cells.
const GETTER: i32 = hk::GETTER | 0x100;
const SETTER: i32 = hk::SETTER | 0x100;
const PROTO_DICT_LOAD: i32 = hk::PROTO_DICT_LOAD | 0x100;
const PROTO_LOAD: i32 = hk::PROTO_LOAD | 0x100;
const MISSING: i32 = hk::MISSING | 0x100;

// ---- host accessors that count their calls ------------------------------------------------------

thread_local! {
    /// Calls of [`get_this`], [`get_two`] and [`set_record`].
    static CALLS: [Cell<u32>; 3] = const { [const { Cell::new(0) }; 3] };
}

fn bump(i: usize) {
    CALLS.with(|c| c[i].set(c[i].get() + 1));
}

/// The calls of [`get_this`] (0), [`get_two`] (1) and [`set_record`] (2).
fn calls() -> [u32; 3] {
    CALLS.with(|c| [c[0].get(), c[1].get(), c[2].get()])
}

/// The cold hits so far ([`ic::cold_load`], [`ic::cold_setter`]).
fn cold() -> u32 {
    ic::COLD_HITS.with(Cell::get)
}

/// Clears the call and cold-hit counters.
fn reset() {
    CALLS.with(|c| c.iter().for_each(|x| x.set(0)));
    ic::COLD_HITS.with(|c| c.set(0));
}

/// A getter: `return this`.
fn get_this<'s>(_: &mut Cx<'s>, this: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> {
    bump(0);
    Ok(this)
}

/// A getter: `return 2`.
fn get_two<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> {
    bump(1);
    Ok(Local::int32(2))
}

/// A setter: the globals `seenThis` and `seenArg` get its receiver and its argument.
fn set_record<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    bump(2);
    let g = cx.global();
    cx.set(g, "seenThis", this, true)?;
    cx.set(g, "seenArg", args.get(0), true)?;
    Ok(Local::undefined())
}

fn host(rt: &Runtime, f: HostFn, name: &str) -> u64 {
    realm::new_host_function(rt, f, name, 0, false)
}

/// Defines (or redefines) the accessor `name` {get, set, enumerable, configurable} of `o`
/// with a new pair. GC point: `get` and `set` live in Rust locals.
fn define_accessor(rt: &Runtime, o: u64, name: &str, get: u64, set: u64) {
    let pair = accessor::new_pair(rt, get, set);
    let a = attr::ACCESSOR | attr::ENUMERABLE | attr::CONFIGURABLE;
    props::own_define(rt, o, key(rt, name), pair, a);
}

/// The globals the setter wrote: (`seenThis`, `seenArg`).
fn seen(t: &T) -> (u64, u64) {
    let g = |n| t.iso.global_raw(n).expect("a setter ran").to_bits();
    (g("seenThis"), g("seenArg"))
}

fn shape_of(o: u64) -> u64 {
    shape::shape_of(o)
}

// ---- getters -----------------------------------------------------------------------------------

#[test]
fn an_own_getter_is_a_getter_case_that_reads_each_receivers_pair() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        reset();
        let p = object_prototype(t.rt());
        let (ga, gb) = (host(t.rt(), get_this, "ga"), host(t.rt(), get_two, "gb"));
        // Two objects of one shape with two pairs (`shape::add` reuses the transition).
        let a = object::new_object(t.rt(), p);
        define_accessor(t.rt(), a, "x", ga, UNDEFINED);
        let b = object::new_object(t.rt(), p);
        define_accessor(t.rt(), b, "x", gb, UNDEFINED);
        let s = shape_of(a);
        assert_eq!(shape_of(b), s);
        // The miss runs the generic load and records an own GETTER case.
        assert_eq!(t.ok(0, a, UNDEFINED), a);
        assert_eq!(cold(), 0);
        let (kind, chain, holder, loc) = t.describe(0, s).unwrap();
        assert_eq!((kind, chain, holder), (GETTER, int(0), UNDEFINED));
        assert_eq!(loc & smi::KIND_MASK, smi::FIELD_IN_OBJECT);
        // Cold hits: each receiver's own pair, with the receiver as `this`.
        for _ in 0..3 {
            assert_eq!(t.ok(0, a, UNDEFINED), a);
            assert_eq!(t.ok(0, b, UNDEFINED), int(2));
        }
        assert_eq!(cold(), 6);
        assert_eq!(calls(), [4, 3, 0]);
        assert_eq!(t.state(0), State::Monomorphic);
        // A pair with no getter gives undefined and calls nothing.
        let set = host(t.rt(), set_record, "s");
        let d = object::new_object(t.rt(), p);
        define_accessor(t.rt(), d, "x", UNDEFINED, set);
        assert_eq!(shape_of(d), s);
        assert_eq!(t.ok(0, d, UNDEFINED), UNDEFINED);
        assert_eq!(cold(), 7);
        assert_eq!(calls(), [4, 3, 0]);
        // A data `x` in another shape is a second case; the GETTER case stays.
        let e = object_with(t.rt(), p, &[("x", int(5))]);
        assert_eq!(t.ok(0, e, UNDEFINED), int(5));
        assert_eq!(t.ok(0, e, UNDEFINED), int(5));
        assert_eq!(t.state(0), State::Polymorphic(2));
        assert_eq!(t.kind(0, s), Some(GETTER));
        assert_eq!(t.ok(0, b, UNDEFINED), int(2));
        assert_eq!(cold(), 8);
        t.collect();
        assert_eq!(t.ok(0, a, UNDEFINED), a);
    });
}

#[test]
fn an_inherited_getter_keeps_the_holder_and_the_chains_cell() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_this, "ga");
        let h = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "x", ga, UNDEFINED);
        let o = object_with(t.rt(), h, &[("a", int(1))]);
        let o2 = object_with(t.rt(), h, &[("a", int(2))]);
        assert_eq!(shape_of(o), shape_of(o2));
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        let (kind, chain, holder, loc) = t.describe(0, shape_of(o)).unwrap();
        assert_eq!((kind, holder), (GETTER, h));
        assert!(validity::is_valid(chain));
        assert_eq!(loc & smi::KIND_MASK, smi::FIELD_IN_OBJECT);
        // The getter gets each receiver, not the holder.
        assert_eq!(t.ok(0, o2, UNDEFINED), o2);
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!((cold(), calls()[0]), (2, 3));
        // A GC keeps the case: the holder is alive (the receivers' chain), so +24 still names it.
        t.collect();
        assert_eq!(t.describe(0, shape_of(o)).unwrap().2, h);
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!((cold(), calls()[0]), (3, 4));
        // A value store elsewhere on the chain is no change: the case still hits.
        props::own_replace(t.rt(), h, key(t.rt(), "pad"), int(9));
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!(cold(), 4);
    });
}

#[test]
fn a_string_receiver_calls_an_inherited_getter_of_string_prototype() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("icLen2")]);
        reset();
        let sp = t.rt().root(Root::StringPrototype);
        let ga = host(t.rt(), get_this, "ga");
        define_accessor(t.rt(), sp, "icLen2", ga, UNDEFINED);
        let s = crate::string::new_string(t.rt(), "abc");
        assert_eq!(t.ok(0, s, UNDEFINED), s);
        assert_eq!(t.kind(0, KEY_STRING), Some(GETTER));
        assert_eq!(t.ok(0, s, UNDEFINED), s);
        assert_eq!((cold(), calls()[0]), (1, 2));
        // A symbol has ShapeId 0 too: it takes the generic path (Symbol.prototype has no
        // `icLen2`).
        let sym = crate::symbol::new_symbol(t.rt(), UNDEFINED);
        assert_eq!(t.ok(0, sym, UNDEFINED), UNDEFINED);
        assert_eq!(cold(), 1);
    });
}

// ---- setters -----------------------------------------------------------------------------------

#[test]
fn own_and_inherited_setters_are_setter_cases_that_call_the_setter() {
    each(|c| {
        let mut t = T::new(c, &[Site::Set("x", false), Site::Set("y", true)]);
        reset();
        let op = object_prototype(t.rt());
        let set = host(t.rt(), set_record, "set");
        // Own: `x` on the receiver.
        let a = object::new_object(t.rt(), op);
        define_accessor(t.rt(), a, "x", UNDEFINED, set);
        t.ok(0, a, int(7));
        assert_eq!(seen(&t), (a, int(7)));
        let (kind, chain, holder, _) = t.describe(0, shape_of(a)).unwrap();
        assert_eq!((kind, chain, holder), (SETTER, int(0), UNDEFINED));
        t.ok(0, a, int(8));
        assert_eq!(seen(&t), (a, int(8)));
        assert_eq!((cold(), calls()[2]), (1, 2));
        // The store calls the setter and adds nothing: the receiver keeps its shape.
        assert_eq!(props::own_count(a), 1);
        // Inherited (strict site): `y` on the holder.
        let h = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "y", UNDEFINED, set);
        let o = object_with(t.rt(), h, &[("b", int(1))]);
        t.ok(1, o, int(10));
        assert_eq!(seen(&t), (o, int(10)));
        let (kind, chain, holder, _) = t.describe(1, shape_of(o)).unwrap();
        assert_eq!((kind, holder), (SETTER, h));
        assert!(validity::is_valid(chain));
        t.ok(1, o, int(11));
        assert_eq!(seen(&t), (o, int(11)));
        assert_eq!((cold(), calls()[2]), (2, 4));
        assert_eq!(own(t.rt(), o, "y"), None);
    });
}

#[test]
fn a_setter_case_with_no_setter_takes_the_generic_store() {
    each(|c| {
        let mut t = T::new(c, &[Site::Set("x", false), Site::Set("x", true)]);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_two, "ga");
        let o = object::new_object(t.rt(), op);
        define_accessor(t.rt(), o, "x", ga, UNDEFINED);
        // Sloppy: the store fails quietly, every time; strict: a TypeError, every time.
        // A store that throws records nothing, so only the sloppy site has a case.
        for _ in 0..3 {
            t.ok(0, o, int(1));
            t.throws(1, o, int(1), "TypeError");
        }
        assert_eq!(t.kind(0, shape_of(o)), Some(SETTER));
        assert_eq!(t.kind(1, shape_of(o)), None);
        // The cold body knew the case: it ran the generic store and made no new handler.
        let h = t.handler(0, shape_of(o));
        t.ok(0, o, int(1));
        assert_eq!(t.handler(0, shape_of(o)), h);
        assert_eq!(cold(), 3);
        assert_eq!(calls(), [0, 0, 0]);
        assert_eq!(own(t.rt(), o, "x").map(accessor::is_pair), Some(true));
    });
}

// ---- MC-w24-8: a store to an accessor calls the setter every time ------------------------------

/// The function `f<i>` of [`preset_program`]: `return a.x` (`store` false) or `a.x = b`
/// (sloppy).
fn site_function(i: usize, store: bool) -> FunctionCode {
    let mut m = meta(&format!("f{i}"), 2, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS;
    let mut b = FunctionBuilder::new(m);
    let (a, x) = (b.parameter(0), b.parameter(1));
    let kind = if store {
        FeedbackSlotKind::StoreNamed
    } else {
        FeedbackSlotKind::LoadNamed
    };
    let s = b.feedback_slot(kind);
    assert_eq!(s.index(), SLOT as usize);
    let k = b.string_constant("x");
    if store {
        b.ldar(x).set_named_property(a, k, s);
    } else {
        b.get_named_property(a, k, s);
    }
    b.return_();
    b.finish().unwrap()
}

/// The script of `tests::ic` for the sites `f0`: `return a.x`, `f1`: `a.x = b`, run in an
/// isolate with the options `exec`.
fn preset_t(c: IsolateConfig, exec: ExecOptions) -> T {
    let mut s = Asm::script(0);
    for i in 0..2 {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(&format!("f{i}"));
    }
    s.b.lda_undefined().return_();
    let fns = vec![s.finish(), site_function(0, false), site_function(1, true)];
    let mut iso = Isolate::new_with(c, exec).unwrap();
    iso.with_cx_checkpoint(|cx| cx.run_script(unit("", fns), "accessor_ic.js").map(|_| ()))
        .unwrap();
    T { iso }
}

/// MODES-CHECK-w24 MC-w24-8 (V8 64b57c27d1 "Do not use cached accessor properties for
/// stores"): after a load site warmed a GETTER case of `x`, every store to `x` calls the
/// setter, with the receiver and the stored value, under the JIT preset (when the build has
/// it) and the jitless preset with either feedback mode. The two sites keep their own kinds.
#[test]
fn a_store_calls_the_setter_every_time_after_a_getter_case_is_warm() {
    let mut presets = vec![
        ExecOptions::jitless(),
        ExecOptions::jitless().feedback(Feedback::FULL),
    ];
    if cfg!(feature = "jit") {
        presets.push(ExecOptions::jit());
    }
    for exec in presets {
        for &d in crate::Dispatch::ALL {
            for stress in [false, true] {
                super::on_big_stack(move || {
                    let mut c = super::config(d, stress);
                    c.feedback_threshold = 0;
                    c.heap.verify_after_gc = true;
                    store_after_warm_getter(preset_t(c, exec));
                });
            }
        }
    }
}

fn store_after_warm_getter(mut t: T) {
    reset();
    let op = object_prototype(t.rt());
    let (ga, set) = (host(t.rt(), get_two, "ga"), host(t.rt(), set_record, "set"));
    // An own pair on `a`, an inherited one for `o` (holder `h`).
    let a = object::new_object(t.rt(), op);
    define_accessor(t.rt(), a, "x", ga, set);
    let h = object_with(t.rt(), op, &[("pad", int(0))]);
    define_accessor(t.rt(), h, "x", ga, set);
    let o = object_with(t.rt(), h, &[("b", int(0))]);
    // Warm the load site: GETTER cases for both shapes.
    for _ in 0..4 {
        assert_eq!(t.ok(0, a, UNDEFINED), int(2));
        assert_eq!(t.ok(0, o, UNDEFINED), int(2));
    }
    assert_eq!(t.kind(0, shape_of(a)), Some(GETTER));
    assert_eq!(t.kind(0, shape_of(o)), Some(GETTER));
    let gets = calls()[1];
    assert_eq!(gets, 8);
    // Stores, with loads between them: the setter runs at every store and sees its value.
    for i in 0..6 {
        for &r in &[a, o] {
            t.ok(1, r, int(100 + i));
            assert_eq!(seen(&t), (r, int(100 + i)));
            assert_eq!(t.ok(0, r, UNDEFINED), int(2));
        }
    }
    assert_eq!(calls(), [0, gets + 12, 12]);
    assert_eq!(t.kind(1, shape_of(a)), Some(SETTER));
    assert_eq!(t.kind(1, shape_of(o)), Some(SETTER));
    assert_eq!(t.kind(0, shape_of(a)), Some(GETTER));
    // No store wrote a value into the receiver or the holder.
    assert_eq!(own(t.rt(), a, "x").map(accessor::is_pair), Some(true));
    assert_eq!(own(t.rt(), h, "x").map(accessor::is_pair), Some(true));
    assert_eq!(own(t.rt(), o, "x"), None);
    t.collect();
}

// ---- changes: a replaced pair, a holder that turns dictionary, a validity-cell pop ---------------

#[test]
fn a_replaced_pair_is_read_at_the_next_hit() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Get("y")]);
        reset();
        let op = object_prototype(t.rt());
        let (ga, gb) = (host(t.rt(), get_this, "ga"), host(t.rt(), get_two, "gb"));
        // Own: a redefine keeps the shape and stores a new pair in the field.
        let a = object::new_object(t.rt(), op);
        define_accessor(t.rt(), a, "x", ga, UNDEFINED);
        assert_eq!(t.ok(0, a, UNDEFINED), a);
        let s = shape_of(a);
        let h0 = t.handler(0, s);
        define_accessor(t.rt(), a, "x", gb, UNDEFINED);
        assert_eq!(shape_of(a), s);
        assert_eq!(t.ok(0, a, UNDEFINED), int(2));
        assert_eq!(
            t.handler(0, s),
            h0,
            "the same case: the pair is read from the receiver"
        );
        assert_eq!((cold(), calls()), (1, [1, 1, 0]));
        // Inherited: a new pair on a prototype is a change (the epoch rule): the cell pops,
        // the miss calls the new getter and records a new case.
        let h = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "y", ga, UNDEFINED);
        let o = object_with(t.rt(), h, &[("b", int(0))]);
        assert_eq!(t.ok(1, o, UNDEFINED), o);
        let old = t.describe(1, shape_of(o)).unwrap().1;
        define_accessor(t.rt(), h, "y", gb, UNDEFINED);
        assert!(!validity::is_valid(old));
        assert_eq!(t.ok(1, o, UNDEFINED), int(2));
        let (kind, cell, holder, _) = t.describe(1, shape_of(o)).unwrap();
        assert_eq!((kind, holder), (GETTER, h));
        assert!(validity::is_valid(cell) && cell != old);
        assert_eq!(t.ok(1, o, UNDEFINED), int(2));
        assert_eq!((cold(), calls()), (2, [2, 3, 0]));
    });
}

#[test]
fn a_holder_that_turns_dictionary_pops_the_cell() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Get("z")]);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_this, "ga");
        // An accessor holder: GETTER, then SLOW once the holder is a dictionary (a
        // dictionary-mode holder's accessor has no case in v1).
        let h = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "x", ga, UNDEFINED);
        let o = object_with(t.rt(), h, &[("b", int(0))]);
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        let cell = t.describe(0, shape_of(o)).unwrap().1;
        props::to_dictionary(t.rt(), h);
        assert!(!validity::is_valid(cell));
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!(t.kind(0, shape_of(o)), Some(smi::SLOW));
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!((cold(), calls()[0]), (0, 3));
        // A data holder: PROTO_LOAD, then PROTO_DICT_LOAD once it is a dictionary.
        let d = object_with(t.rt(), op, &[("z", int(5)), ("pad", int(0))]);
        let q = object_with(t.rt(), d, &[("b", int(0))]);
        assert_eq!(t.ok(1, q, UNDEFINED), int(5));
        assert_eq!(t.kind(1, shape_of(q)), Some(PROTO_LOAD));
        props::to_dictionary(t.rt(), d);
        assert!(shape::is_dictionary(shape_of(d)));
        assert_eq!(t.ok(1, q, UNDEFINED), int(5));
        let (kind, chain, holder, loc) = t.describe(1, shape_of(q)).unwrap();
        assert_eq!((kind, holder, loc), (PROTO_DICT_LOAD, d, 0));
        assert!(validity::is_valid(chain));
        assert_eq!(t.ok(1, q, UNDEFINED), int(5));
        assert_eq!(cold(), 1);
    });
}

#[test]
fn a_dictionary_holder_case_reads_the_entry_at_each_hit() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("z")]);
        reset();
        let op = object_prototype(t.rt());
        // A holder with more properties than a fast shape keeps in place: a dictionary.
        let d = object_with(t.rt(), op, &[("z", int(1)), ("w", int(0))]);
        props::to_dictionary(t.rt(), d);
        let mid = object_with(t.rt(), d, &[("m", int(0))]);
        let q = object_with(t.rt(), mid, &[("b", int(0))]);
        assert_eq!(t.ok(0, q, UNDEFINED), int(1));
        assert_eq!(t.kind(0, shape_of(q)), Some(PROTO_DICT_LOAD));
        // A value store in the holder is no change: the hit reads the new value.
        props::own_replace(t.rt(), d, key(t.rt(), "z"), int(2));
        assert_eq!(t.ok(0, q, UNDEFINED), int(2));
        assert_eq!(cold(), 1);
        // A delete in the dictionary pops the cell: the load reads undefined (a MISSING case).
        props::own_delete(t.rt(), d, key(t.rt(), "z"));
        assert_eq!(t.ok(0, q, UNDEFINED), UNDEFINED);
        assert_eq!(t.kind(0, shape_of(q)), Some(MISSING));
        // Back, as an accessor of the dictionary: no case (SLOW), the getter runs.
        let ga = host(t.rt(), get_this, "ga");
        define_accessor(t.rt(), d, "z", ga, UNDEFINED);
        assert_eq!(t.ok(0, q, UNDEFINED), q);
        assert_eq!(t.kind(0, shape_of(q)), Some(smi::SLOW));
        assert_eq!(t.ok(0, q, UNDEFINED), q);
        assert_eq!((cold(), calls()[0]), (1, 2));
        // A receiver in dictionary mode stays SLOW (H16 K12).
        let r = object_with(t.rt(), d, &[("e", int(0)), ("f", int(0))]);
        props::to_dictionary(t.rt(), r);
        define_accessor(t.rt(), r, "z", ga, UNDEFINED);
        assert_eq!(t.ok(0, r, UNDEFINED), r);
        assert_eq!(t.kind(0, shape_of(r)), Some(smi::SLOW));
    });
}

#[test]
fn a_change_on_the_chain_pops_the_cell_of_an_inherited_getter() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_this, "ga");
        let h = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "x", ga, UNDEFINED);
        let mid = object_with(t.rt(), h, &[("m", int(0))]);
        let o = object_with(t.rt(), mid, &[("b", int(0))]);
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        let cell = t.describe(0, shape_of(o)).unwrap().1;
        assert_eq!((cold(), calls()[0]), (1, 2));
        // A data `x` on the prototype between: the cell pops, the load reads the data.
        props::own_define(t.rt(), mid, key(t.rt(), "x"), int(9), attr::DEFAULT);
        assert!(!validity::is_valid(cell));
        assert_eq!(t.ok(0, o, UNDEFINED), int(9));
        assert_eq!(t.kind(0, shape_of(o)), Some(PROTO_LOAD));
        assert_eq!(t.ok(0, o, UNDEFINED), int(9));
        assert_eq!((cold(), calls()[0]), (1, 2));
        // The data `x` deleted: the cell pops again and the getter answers again.
        assert!(props::own_delete(t.rt(), mid, key(t.rt(), "x")).is_some());
        assert_eq!(t.ok(0, o, UNDEFINED), o);
        assert_eq!(t.kind(0, shape_of(o)), Some(GETTER));
        assert_eq!(calls()[0], 3);
    });
}

// ---- super ---------------------------------------------------------------------------------------

#[test]
fn a_super_load_calls_the_getter_with_the_receiver() {
    each(|c| {
        let mut t = T::new(c, &[Site::Super("x")]);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_this, "ga");
        // `super.x` from the home object `home` (its prototype `start` has the accessor).
        let start = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), start, "x", ga, UNDEFINED);
        let home = object::new_object(t.rt(), start);
        let receiver = object_with(t.rt(), op, &[("r", int(0))]);
        for _ in 0..3 {
            assert_eq!(t.ok(0, home, receiver), receiver);
        }
        let (kind, chain, ..) = t.describe(0, shape_of(start)).unwrap();
        assert_eq!((kind, chain), (GETTER, int(0)));
        assert_eq!((cold(), calls()[0]), (2, 3));
        // The op list (`ic_ops::Op::CallGetter` docs; M6-P1 Claude review R4): the guard and
        // the pair are on the lookup start (`Val::Receiver` at 0x22), and the getter got the
        // site's `this` (`receiver` above), not `start`.
        use crate::ic_ops::{Holder, Loc, Op, Operation, Ref, Val, cases, ops_of};
        let cs = cases(t.vector(0), 0, Operation::LoadNamed);
        let ops = ops_of(&cs[0], Operation::LoadNamed);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    Op::GuardShape(Val::Receiver, Ref::CaseKey),
                    Op::CallGetter(Holder::Receiver, Loc(_))
                ]
            ),
            "{ops:?}"
        );
    });
}

// ---- GC: the holder is weak ----------------------------------------------------------------------

/// Builds `o → h → Object.prototype` with an accessor `x` on `h` and loads `o.x` through `f0`
/// twice (a miss, then a cold hit). Nothing but the IC holds `o`, `h` or `o`'s shape after the
/// call. Run it [`below_a_pad`]. Returns the masked addresses of `o` and `h`.
#[inline(never)]
fn made_an_inherited_getter_case(t: &mut T) -> [usize; 2] {
    let op = object_prototype(t.rt());
    let ga = host(t.rt(), get_two, "ga");
    let h = object_with(t.rt(), op, &[("pad", int(0))]);
    define_accessor(t.rt(), h, "x", ga, UNDEFINED);
    let o = object_with(t.rt(), h, &[("b", int(0))]);
    assert_eq!(t.ok(0, o, UNDEFINED), int(2));
    assert_eq!(t.ok(0, o, UNDEFINED), int(2));
    let (kind, _, holder, _) = t.describe(0, shape_of(o)).unwrap();
    assert_eq!((kind, holder), (GETTER, h));
    [o, h].map(|x| black_box(x as usize) ^ MASK)
}

#[test]
fn a_cached_holder_is_weak() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        reset();
        let (dead, pad_low) = below_a_pad(|| made_an_inherited_getter_case(&mut t));
        collect_after_a_pad(&mut t, pad_low, &dead);
        assert_eq!((cold(), calls()[1]), (1, 2));
        // The case key (the receiver's shape) died; the handler (the strong value half) lives
        // on, and its holder reads undefined: the IC kept the holder alive through nothing.
        assert_eq!(t.state(0), State::Uninitialized);
        let handler = crate::feedback::value(t.vector(0), SLOT);
        let (kind, _, holder, _) = ic::describe_handler(handler);
        assert_eq!((kind, holder), (GETTER, UNDEFINED));
        // The site works on.
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_this, "ga");
        let h2 = object_with(t.rt(), op, &[("pad2", int(0))]);
        define_accessor(t.rt(), h2, "x", ga, UNDEFINED);
        let o2 = object_with(t.rt(), h2, &[("b", int(0))]);
        assert_eq!(t.ok(0, o2, UNDEFINED), o2);
        assert_eq!(t.ok(0, o2, UNDEFINED), o2);
        assert_eq!(cold(), 2);
        t.collect();
    });
}

// ---- no stub cache entries (v1) ----------------------------------------------------------------

#[test]
fn megamorphic_accessor_sites_put_nothing_in_the_stub_cache() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_two, "ga");
        let pads = ["a", "b", "c", "d", "e", "f"];
        let objs: Vec<u64> = pads
            .iter()
            .map(|&p| {
                let o = object_with(t.rt(), op, &[(p, int(0))]);
                define_accessor(t.rt(), o, "x", ga, UNDEFINED);
                o
            })
            .collect();
        let kept = t.keep_all(&objs);
        for &o in &objs {
            assert_eq!(t.ok(0, o, UNDEFINED), int(2));
        }
        assert_eq!(t.state(0), State::Megamorphic);
        // The sixth shape's miss computed a GETTER case for the megamorphic entry; it is not
        // in the stub cache, so every load runs the getter through the generic path.
        let last = objs[5];
        let id = shape_of(last) as u32;
        let x = key(t.rt(), "x");
        assert_eq!(
            crate::stub_cache::probe(t.rt(), crate::stub_cache::Op::Load, id, x),
            None
        );
        assert_eq!(t.ok(0, last, UNDEFINED), int(2));
        assert_eq!(cold(), 0);
        assert_eq!(calls()[1], 7);
        t.release_all(kept);
    });
}

/// A megamorphic site keeps no accessor or dictionary-holder case, so its miss path makes no
/// IC_HANDLER cell either: after the warm-up, more inherited-getter loads, inherited-setter
/// stores and dictionary-holder loads allocate nothing (M6-P1 Claude review R1, Codex
/// cx-accic-1: each one made a 40-byte cell that was garbage at once).
#[test]
fn megamorphic_accessor_and_dictionary_holder_misses_allocate_nothing() {
    each(|c| {
        let sites = [Site::Get("x"), Site::Set("x", false), Site::Get("z")];
        let mut t = T::new(c, &sites);
        reset();
        let op = object_prototype(t.rt());
        let ga = host(t.rt(), get_two, "ga");
        let set = host(t.rt(), set_record, "s");
        let h = object_with(t.rt(), op, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "x", ga, set);
        let d = object_with(t.rt(), op, &[("z", int(1)), ("w", int(0))]);
        props::to_dictionary(t.rt(), d);
        // Six shapes on each holder; each object is rooted when it is made (GC stress).
        let (mut on_h, mut on_d, mut kept) = (vec![], vec![], vec![]);
        for p in ["a", "b", "c", "d", "e", "f"] {
            for (proto, to) in [(h, &mut on_h), (d, &mut on_d)] {
                let o = object_with(t.rt(), proto, &[(p, int(0))]);
                kept.push(t.keep(o));
                to.push(o);
            }
        }
        let round = |t: &mut T| {
            for (&o, &q) in on_h.iter().zip(&on_d) {
                assert_eq!(t.ok(0, o, UNDEFINED), int(2));
                t.ok(1, o, int(3));
                assert_eq!(t.ok(2, q, UNDEFINED), int(1));
            }
        };
        round(&mut t);
        assert_eq!(
            [t.state(0), t.state(1), t.state(2)],
            [State::Megamorphic; 3]
        );
        let before = t.rt().heap.stats().allocated_bytes;
        for _ in 0..4 {
            round(&mut t);
        }
        assert_eq!(t.rt().heap.stats().allocated_bytes - before, 0);
        assert_eq!(calls()[1..], [30, 30]);
        t.release_all(kept);
    });
}

// ---- the op lists of these cases ----------------------------------------------------------------

/// `ic_ops::ops_of` reads the cases ACCIC records (CBC need 7; H16 GETTER, SETTER row): an own
/// GETTER is GuardShape, CallGetter on the receiver's pair; an inherited one adds
/// GuardProtoValidity and reads the holder's pair; a SETTER case of a 0x24 store is GuardShape,
/// CallSetter on the receiver's pair.
#[test]
fn ops_of_reads_the_getter_and_setter_cases() {
    use crate::ic_ops::{Holder, Loc, Op, Operation, Proto, Ref, Val, cases, ops_of};
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Get("x"), Site::Set("x", false)]);
        let p = object_prototype(t.rt());
        let ga = host(t.rt(), get_this, "ga");
        // Site 0: an own getter.
        let a = object::new_object(t.rt(), p);
        define_accessor(t.rt(), a, "x", ga, UNDEFINED);
        t.ok(0, a, UNDEFINED);
        assert_eq!(t.kind(0, shape_of(a)), Some(GETTER));
        // Site 1: a getter on the prototype holder `h`.
        let h = object_with(t.rt(), p, &[("pad", int(0))]);
        define_accessor(t.rt(), h, "x", ga, UNDEFINED);
        let o = object_with(t.rt(), h, &[("a", int(1))]);
        t.ok(1, o, UNDEFINED);
        assert_eq!(t.kind(1, shape_of(o)), Some(GETTER));
        // Site 2: an own setter.
        let set = host(t.rt(), set_record, "s");
        let d = object::new_object(t.rt(), p);
        define_accessor(t.rt(), d, "x", UNDEFINED, set);
        t.ok(2, d, int(3));
        assert_eq!(t.kind(2, shape_of(d)), Some(SETTER));
        let lists = |t: &T, i: usize, op| {
            let cs = cases(t.vector(i), 0, op);
            cs.iter()
                .map(|k| ops_of(k, op).as_slice().to_vec())
                .collect::<Vec<_>>()
        };
        let shape = Op::GuardShape(Val::Receiver, Ref::CaseKey);
        let chain = Op::GuardProtoValidity(Proto::OfShape(Ref::CaseKey));
        let own = lists(&t, 0, Operation::LoadNamed);
        assert!(
            matches!(own[0].as_slice(), [s, Op::CallGetter(Holder::Receiver, Loc(_))] if *s == shape),
            "{own:?}"
        );
        let inh = lists(&t, 1, Operation::LoadNamed);
        assert!(
            matches!(
                inh[0].as_slice(),
                [s, c, Op::CallGetter(Holder::Ref(Ref::HandlerRef), Loc(_))]
                    if *s == shape && *c == chain
            ),
            "{inh:?}"
        );
        let st = lists(&t, 2, Operation::StoreNamed);
        assert!(
            matches!(st[0].as_slice(), [s, Op::CallSetter(Holder::Receiver, Loc(_))] if *s == shape),
            "{st:?}"
        );
    });
}
