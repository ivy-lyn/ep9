//! The builtin ABI v1 (H20): sizes, `Args`, calls and construct, the throw protocol,
//! `ScratchList` and `ArgsBuilder` under GC stress, and ObjectOps v0.

use std::cell::RefCell;

use ep9_bytecode::{
    FunctionFlags, FunctionId, FunctionKind, RegisterList, RuntimeId, VerifiedUnit,
};
use ep9_value::Throw;

use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::object::{attr, own_define};
use crate::runtime::Runtime;
use crate::{
    Args, Cx, Dispatch, HostFn, IntrinsicId, Isolate, JsArray, JsResult, Local, PrimitiveType,
    PropertyDescriptor, ToPrimitiveHint,
};

/// Runs `f` with a builtin-ABI context of a fresh isolate. `stress`: a collection before
/// every allocation, then the heap verifier.
fn with_cx<R>(
    stress: bool,
    setup: fn(&mut Isolate),
    f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R,
) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    setup(&mut iso);
    let out = {
        let iso = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

fn no_setup(_: &mut Isolate) {}

/// Defines the global `name` as a host function (`constructor`: it has [[Construct]]).
fn define(iso: &mut Isolate, name: &str, f: HostFn, constructor: bool) {
    let rt = &*iso.rt;
    let func = crate::realm::new_host_function(rt, f, name, 0, constructor);
    let key = crate::atoms::intern_str(rt, name);
    own_define(rt, rt.global(), key, func, attr::HIDDEN);
}

/// The global `name`.
fn global<'s>(cx: &mut Cx<'s>, name: &str) -> Local<'s> {
    let g = cx.global();
    cx.get(g, name).unwrap()
}

/// Takes the pending exception: "Name: message".
fn take_error(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    )
}

/// The texts of the elements of the array `a`.
fn texts<'s>(cx: &mut Cx<'s>, a: Local<'s, JsArray>) -> Vec<String> {
    let n = crate::object::array_len(a.bits());
    (0..n)
        .map(|i| {
            let v = cx.get(a, i.to_string().as_str()).unwrap();
            cx.to_rust_string(v).unwrap()
        })
        .collect()
}

thread_local! {
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn record(line: String) {
    SEEN.with(|s| s.borrow_mut().push(line));
}

fn take_seen() -> Vec<String> {
    SEEN.with(|s| core::mem::take(&mut *s.borrow_mut()))
}

#[test]
fn sizes_are_one_word() {
    // H20: one word each; JsResult<Local> uses the NonZeroU64 niche of Value (Err = 0).
    assert_eq!(size_of::<Local<'static>>(), 8);
    assert_eq!(size_of::<Args<'static>>(), 8);
    assert_eq!(size_of::<JsResult<Local<'static>>>(), 8);
    assert_eq!(size_of::<Cx<'static>>(), 8);
    assert_eq!(size_of::<HostFn>(), 8);
}

/// `probe(...)`: records what `Args` shows.
fn probe<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let me = global(cx, "probe");
    let rest: Vec<String> = args
        .rest(1)
        .iter()
        .map(|v| cx.to_rust_string(v))
        .collect::<JsResult<_>>()?;
    record(format!(
        "len={} this={} same_this={} construct={} nt_undefined={} same_nt={} callee_is_probe={} \
         rest1={rest:?} rest9={} a3={}",
        args.len(),
        cx.to_rust_string(args.this())?,
        this.is_same_bits(&args.this()),
        args.is_construct_call(),
        args.new_target().is_undefined(),
        nt.is_same_bits(&args.new_target()),
        args.callee().is_same_bits(&me),
        args.rest(9).len(),
        cx.to_rust_string(args.get(3))?,
    ));
    if args.is_construct_call() {
        return Ok(cx
            .ordinary_create_from_constructor(
                nt.as_object().unwrap(),
                IntrinsicId::ObjectPrototype,
            )?
            .value());
    }
    Ok(Local::undefined())
}

/// `forward(f, ...rest)`: `f.call(this, ...rest)` through `call_args` (one copy).
fn forward<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.call_args(args.get(0), this, args.rest(1))
}

fn setup_probe(iso: &mut Isolate) {
    define(iso, "probe", probe, true);
    define(iso, "forward", forward, false);
    define(iso, "Other", probe, true);
}

#[test]
fn args_show_callee_this_new_target_and_rest() {
    for stress in [false, true] {
        let seen = on_big_stack(move || {
            take_seen();
            with_cx(stress, setup_probe, |cx, _| {
                let p = global(cx, "probe");
                let this = cx.new_string("me").unwrap().value();
                let a = [Local::int32(0), Local::int32(1), Local::int32(2)];
                cx.call(p, this, &a).unwrap();
                // forward(probe, "x", "y"): probe sees this = the receiver, args ["x", "y"].
                let fw = global(cx, "forward");
                let (x, y) = (cx.new_string("x").unwrap(), cx.new_string("y").unwrap());
                cx.call(fw, this, &[p, x.value(), y.value()]).unwrap();
                // new probe(7, 8) with new.target = Other.
                let other = global(cx, "Other");
                let o = cx
                    .construct(p, &[Local::int32(7), Local::int32(8)], other)
                    .unwrap();
                let proto = cx.get_prototype_of(o).unwrap();
                let other_proto = cx.get(other.as_object().unwrap(), "prototype").unwrap();
                assert!(
                    proto.is_same_bits(&other_proto),
                    "the prototype comes from new.target"
                );
                take_seen()
            })
        });
        assert_eq!(
            seen,
            [
                "len=3 this=me same_this=true construct=false nt_undefined=true same_nt=true \
                 callee_is_probe=true rest1=[\"1\", \"2\"] rest9=0 a3=undefined",
                "len=2 this=me same_this=true construct=false nt_undefined=true same_nt=true \
                 callee_is_probe=true rest1=[\"y\"] rest9=0 a3=undefined",
                "len=2 this=undefined same_this=true construct=true nt_undefined=false \
                 same_nt=true callee_is_probe=true rest1=[\"8\"] rest9=0 a3=undefined",
            ],
            "stress={stress}"
        );
    }
}

/// f1: `function F() { return 5; }`; f2: `class A { constructor() { this.v = 7; } }`;
/// f3: `class B extends A { constructor() { super(); this.w = this.v + 1; } }`. The script
/// defines the globals `F`, `A` and `B` (as tests/calls.rs builds classes).
fn constructor_unit() -> VerifiedUnit {
    let mut f = Asm::new(meta("F", 0, 0, 0, 0));
    f.b.lda_smi(5).return_();
    let mut ma = meta("A", 0, 0, 0, 0);
    ma.kind = FunctionKind::ClassConstructor;
    ma.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
    let mut a = Asm::new(ma);
    let this = a.b.receiver();
    a.b.lda_smi(7);
    a.set_named(this, "v");
    a.b.lda_undefined().return_();
    let mut mb = meta("B", 0, 0, 0, 0);
    mb.kind = FunctionKind::DerivedConstructor;
    mb.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
    let mut b = Asm::new(mb);
    let (this, sup) = (b.b.new_register(), b.b.new_register());
    b.b.lda_the_hole().star(this);
    b.b.lda_current_function().get_super_constructor(sup);
    b.b.throw_if_not_super_constructor(sup);
    b.b.lda_new_target()
        .construct(sup, RegisterList::EMPTY, 0, b.call);
    b.b.star(this);
    b.get_named(this, "v");
    b.b.add_smi(1, b.bin);
    b.set_named(this, "w");
    b.b.ldar(this).throw_super_not_called_if_hole().return_();

    let mut s = Asm::script(0);
    let (cls, heritage) = (s.b.new_register(), s.b.new_register());
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("F");
    let def = RegisterList::new(cls.local_index().unwrap(), 2);
    assert_eq!(heritage.local_index(), Some(cls.local_index().unwrap() + 1));
    let ka = s.b.function_constant(FunctionId(2));
    s.b.create_closure(ka, s.closure, 0)
        .star(cls)
        .lda_the_hole()
        .star(heritage);
    s.b.call_runtime(RuntimeId::DefineClass, def, 2);
    s.b.ldar(cls);
    s.sta_global("A");
    let kb = s.b.function_constant(FunctionId(3));
    s.b.create_closure(kb, s.closure, 0).star(cls);
    s.lda_global("A");
    s.b.star(heritage)
        .call_runtime(RuntimeId::DefineClass, def, 2);
    s.b.ldar(cls);
    s.sta_global("B");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), f.finish(), a.finish(), b.finish()])
}

#[test]
fn construct_with_a_different_new_target() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            setup_probe(&mut iso);
            iso.run_script(constructor_unit(), "ctor.js").unwrap();
            let mut cx = Cx::new(&mut iso.cx);
            let f = global(&mut cx, "F");
            let other = global(&mut cx, "Other").as_object().unwrap();
            let other_proto = cx.get(other, "prototype").unwrap();
            let f_proto = cx.get(f.as_object().unwrap(), "prototype").unwrap();
            assert!(!other_proto.is_same_bits(&f_proto));
            // A bytecode [[Construct]] (F returns a primitive, so the new object is the result).
            let o = cx.construct(f, &[], other).unwrap();
            let p = cx.get_prototype_of(o).unwrap();
            assert!(p.is_same_bits(&other_proto), "stress={stress}");
            // A derived class: `this` comes from the base constructor, with the prototype of
            // new.target.
            let bcls = global(&mut cx, "B");
            let o = cx.construct(bcls, &[], other).unwrap();
            let p = cx.get_prototype_of(o).unwrap();
            assert!(p.is_same_bits(&other_proto), "stress={stress}");
            assert_eq!(cx.get(o, "w").unwrap().as_int32(), Some(8));
            let o = cx.construct(bcls, &[], bcls).unwrap();
            assert_eq!(cx.get(o, "v").unwrap().as_int32(), Some(7));
            // A class constructor cannot be called.
            assert!(cx.call(bcls, Local::undefined(), &[]).is_err());
            assert_eq!(
                take_error(&mut cx, &iso.rt),
                "TypeError: Class constructor B cannot be invoked without 'new'"
            );
            // new.target must be a constructor.
            let not_ctor = cx.new_object();
            assert!(cx.construct(f, &[], not_ctor).is_err());
            let rt = &iso.rt;
            assert!(rt.take_pending() != 0, "a pending TypeError");
        });
    }
}

fn returns_err_without_throw<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(Throw)
}

fn returns_ok_with_pending<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let _ = cx.throw_type_error("forgotten");
    Ok(Local::undefined())
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "returned Err without a pending exception")]
fn err_without_a_pending_exception_panics_in_debug() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    define(&mut iso, "bad", returns_err_without_throw, false);
    let mut cx = Cx::new(&mut iso.cx);
    let f = global(&mut cx, "bad");
    let _ = cx.call(f, Local::undefined(), &[]);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "returned Ok with a pending exception")]
fn ok_with_a_pending_exception_panics_in_debug() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    define(&mut iso, "bad", returns_ok_with_pending, false);
    let mut cx = Cx::new(&mut iso.cx);
    let f = global(&mut cx, "bad");
    let _ = cx.call(f, Local::undefined(), &[]);
}

#[test]
fn throw_protocol_passes_errors_through() {
    with_cx(false, setup_probe, |cx, rt| {
        let not_fn = cx.new_object();
        assert!(cx.call(not_fn, Local::undefined(), &[]).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: object is not a function");
        let t = cx.throw_range_error("boom");
        assert_eq!(t, Throw);
        assert_eq!(take_error(cx, rt), "RangeError: boom");
        let _ = cx.not_implemented("a thing");
        assert_eq!(
            take_error(cx, rt),
            "Error: not implemented in ep9 v0: a thing"
        );
    });
}

/// Pushes `n` fresh strings into `list`; each allocation is a collection under GC stress.
/// Out of line, so no Rust local of the caller holds them afterwards.
#[inline(never)]
fn fill_list(cx: &mut Cx<'_>, list: &crate::ScratchList<'_>, n: usize) {
    for i in 0..n {
        let s = cx.new_string(&format!("item {i}")).unwrap();
        list.push(s);
    }
}

#[test]
fn scratch_list_keeps_values_under_gc_stress() {
    with_cx(true, no_setup, |cx, _| {
        let list = cx.scratch_list();
        fill_list(cx, &list, 40);
        for _ in 0..8 {
            cx.new_object();
        }
        cx.collect_garbage();
        assert_eq!(list.len(), 40);
        for i in 0..40 {
            assert_eq!(cx.to_rust_string(list.get(i)).unwrap(), format!("item {i}"));
        }
        list.set(0, Local::int32(3));
        assert_eq!(list.get(0).as_int32(), Some(3));
    });
}

#[test]
#[should_panic(expected = "out of LIFO order")]
fn scratch_list_released_out_of_order_panics() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let cx = Cx::new(&mut iso.cx);
    let a = cx.scratch_list();
    let b = cx.scratch_list();
    b.push(Local::int32(1));
    drop(a);
}

/// Pushes `n` fresh strings into `b`. Out of line (see `fill_list`). Halfway it calls the
/// host function `churn`, which allocates and collects with its frame above the builder.
#[inline(never)]
fn fill_builder<'s>(cx: &mut Cx<'s>, b: &mut crate::ArgsBuilder<'s>, n: usize) {
    for i in 0..n {
        if i == n / 2 {
            let churn = global(cx, "churn");
            cx.call(churn, Local::undefined(), &[]).unwrap();
        }
        let s = cx.new_string(&format!("arg {i}")).unwrap();
        b.push(cx, s).unwrap();
    }
}

/// `churn()`: allocates and collects; its own builder must not disturb the caller's.
fn churn<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut b = cx.args_builder();
    for i in 0..8 {
        let s = cx.new_string(&format!("churn {i}"))?;
        b.push(cx, s)?;
    }
    cx.collect_garbage();
    Ok(Local::undefined())
}

/// `check(...)`: records its arguments.
fn check<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    for (i, v) in args.iter().enumerate() {
        let t = cx.to_rust_string(v)?;
        assert_eq!(t, format!("arg {i}"), "argument {i}");
    }
    record(format!("check {}", args.len()));
    Ok(Local::int32(args.len() as i32))
}

/// `build()`: builds 48 arguments on the VM stack, collects, then calls `check`.
fn build<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut b = cx.args_builder();
    fill_builder(cx, &mut b, 48);
    for _ in 0..16 {
        cx.new_object();
    }
    cx.collect_garbage();
    assert_eq!(b.len(), 48);
    let check = global(cx, "check");
    cx.call_built(check, Local::undefined(), b)
}

fn setup_builder(iso: &mut Isolate) {
    define(iso, "churn", churn, false);
    define(iso, "check", check, false);
    define(iso, "build", build, false);
}

#[test]
fn args_builder_values_survive_gc_stress_before_the_call() {
    for stress in [false, true] {
        let seen = on_big_stack(move || {
            take_seen();
            with_cx(stress, setup_builder, |cx, rt| {
                let before = rt.heap.stats().collections;
                // Inside a host function (its frame is the top frame).
                let f = global(cx, "build");
                let r = cx.call(f, Local::undefined(), &[]).unwrap();
                assert_eq!(r.as_int32(), Some(48));
                // From Rust with no frame live.
                let mut b = cx.args_builder();
                fill_builder(cx, &mut b, 48);
                cx.collect_garbage();
                let check = global(cx, "check");
                cx.call_built(check, Local::undefined(), b).unwrap();
                assert!(
                    rt.stack().scratch_top().is_null(),
                    "the builder restored the top"
                );
                if stress {
                    assert!(rt.heap.stats().collections - before > 96);
                }
                take_seen()
            })
        });
        assert_eq!(seen, ["check 48", "check 48"], "stress={stress}");
    }
}

#[test]
#[should_panic(expected = "ArgsBuilder used out of LIFO order")]
fn args_builders_interleaved_panic() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let mut cx = Cx::new(&mut iso.cx);
    let mut a = cx.args_builder();
    let mut b = cx.args_builder();
    b.push(&mut cx, Local::int32(1)).unwrap();
    a.push(&mut cx, Local::int32(2)).unwrap();
}

#[test]
fn create_list_from_array_like_feeds_a_call() {
    on_big_stack(|| {
        take_seen();
        with_cx(true, setup_builder, |cx, rt| {
            let items: Vec<String> = (0..5).map(|i| format!("arg {i}")).collect();
            let mut vals = [Local::undefined(); 5];
            for (i, t) in items.iter().enumerate() {
                vals[i] = cx.new_string(t).unwrap().value();
            }
            let a = cx.create_array_from_list(&vals).unwrap();
            let list = cx.create_list_from_array_like(a).unwrap();
            assert_eq!(list.len(), 5);
            let check = global(cx, "check");
            cx.call_built(check, Local::undefined(), list).unwrap();
            assert!(cx.create_list_from_array_like(Local::int32(1)).is_err());
            assert_eq!(
                take_error(cx, rt),
                "TypeError: CreateListFromArrayLike called on non-object"
            );
        });
        assert_eq!(take_seen(), ["check 5"]);
    });
}

#[test]
fn virtual_property_descriptors() {
    with_cx(false, setup_probe, |cx, _| {
        // gOPD([], "length") = {value: 0, writable: true, enumerable: false, configurable: false}.
        let a = cx.new_array();
        let d = cx.get_own_property(a, "length").unwrap().unwrap();
        assert_eq!(d.value.and_then(|v| v.as_int32()), Some(0));
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(true), Some(false), Some(false))
        );
        assert!(d.get.is_none() && d.set.is_none());
        // Elements are {w, e, c}; holes are absent.
        cx.set(a, "2", Local::int32(9), true).unwrap();
        let d = cx.get_own_property(a, "2").unwrap().unwrap();
        assert_eq!(
            (
                d.value.and_then(|v| v.as_int32()),
                d.writable,
                d.enumerable,
                d.configurable
            ),
            (Some(9), Some(true), Some(true), Some(true))
        );
        assert!(cx.get_own_property(a, "1").unwrap().is_none());
        assert!(cx.has_own_property(a, "2").unwrap() && !cx.has_own_property(a, "1").unwrap());
        // Function length and name: {w: false, e: false, c: true}; prototype {w: true}.
        let p = global(cx, "probe").as_object().unwrap();
        let d = cx.get_own_property(p, "length").unwrap().unwrap();
        assert_eq!(
            (
                d.value.and_then(|v| v.as_int32()),
                d.writable,
                d.enumerable,
                d.configurable
            ),
            (Some(0), Some(false), Some(false), Some(true))
        );
        let d = cx.get_own_property(p, "name").unwrap().unwrap();
        assert_eq!(cx.to_rust_string(d.value.unwrap()).unwrap(), "probe");
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(true))
        );
        let d = cx.get_own_property(p, "prototype").unwrap().unwrap();
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(true), Some(false), Some(false))
        );
        // FromPropertyDescriptor and ToPropertyDescriptor round-trip.
        let obj = cx.from_property_descriptor(Some(d)).unwrap();
        let keys = cx.own_property_keys(obj.as_object().unwrap()).unwrap();
        assert_eq!(
            texts(cx, keys),
            ["value", "writable", "enumerable", "configurable"]
        );
        let back = cx.to_property_descriptor(obj).unwrap();
        assert_eq!(
            (back.writable, back.enumerable, back.configurable),
            (Some(true), Some(false), Some(false))
        );
        assert!(back.value.unwrap().is_same_bits(&d.value.unwrap()));
        assert!(cx.from_property_descriptor(None).unwrap().is_undefined());
    });
}

#[test]
fn define_own_property_validates_and_applies() {
    with_cx(true, setup_probe, |cx, rt| {
        // A non-extensible object rejects new properties.
        let o = cx.new_object();
        assert!(cx.is_extensible(o).unwrap());
        assert!(cx.prevent_extensions(o).unwrap());
        assert!(!cx.is_extensible(o).unwrap());
        let one = Local::int32(1);
        let d = PropertyDescriptor::data(one, true, true, true);
        assert!(!cx.define_own_property(o, "x", d).unwrap());
        assert!(!cx.create_data_property(o, "x", one).unwrap());
        assert!(!cx.set(o, "x", one, false).unwrap());
        assert!(cx.set(o, "x", one, true).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot assign to read only property 'x' of object"
        );
        assert!(cx.create_data_property_or_throw(o, "x", one).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: Cannot redefine property: x");
        // ... and so does a non-extensible array (elements too).
        let a = cx.new_array();
        cx.create_data_property_or_throw(a, "0", one).unwrap();
        cx.prevent_extensions(a).unwrap();
        assert!(!cx.create_data_property(a, "1", one).unwrap());
        assert!(!cx.set(a, "1", one, false).unwrap());
        assert!(
            cx.set(a, "0", Local::int32(2), true).unwrap(),
            "existing elements stay writable"
        );

        // Non-configurable, non-writable: only compatible redefinitions pass.
        let o = cx.new_object();
        let frozen = PropertyDescriptor::data(one, false, false, false);
        assert!(cx.define_own_property(o, "k", frozen).unwrap());
        assert!(
            cx.define_own_property(o, "k", frozen).unwrap(),
            "the same descriptor"
        );
        assert!(
            cx.define_own_property(o, "k", PropertyDescriptor::default())
                .unwrap()
        );
        let other = PropertyDescriptor {
            value: Some(Local::int32(2)),
            ..PropertyDescriptor::default()
        };
        assert!(!cx.define_own_property(o, "k", other).unwrap());
        for (w, e, c) in [
            (Some(true), None, None),
            (None, Some(true), None),
            (None, None, Some(true)),
        ] {
            let d = PropertyDescriptor {
                writable: w,
                enumerable: e,
                configurable: c,
                ..PropertyDescriptor::default()
            };
            assert!(
                !cx.define_own_property(o, "k", d).unwrap(),
                "{w:?} {e:?} {c:?}"
            );
        }
        assert!(!cx.set(o, "k", Local::int32(2), false).unwrap());
        assert!(cx.delete_property_or_throw(o, "k").is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot delete property 'k' of object"
        );
        // A configurable property can change its attributes and value.
        let o = cx.new_object();
        cx.create_data_property_or_throw(o, "k", one).unwrap();
        let ro = PropertyDescriptor {
            writable: Some(false),
            enumerable: Some(false),
            ..PropertyDescriptor::default()
        };
        cx.define_property_or_throw(o, "k", ro).unwrap();
        let d = cx.get_own_property(o, "k").unwrap().unwrap();
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(true))
        );
        assert!(cx.delete(o, "k").unwrap());
        assert!(!cx.has_own_property(o, "k").unwrap());
        // Function name: redefining it materializes an own property.
        let p = global(cx, "probe").as_object().unwrap();
        let s = cx.new_string("renamed").unwrap().value();
        cx.define_property_or_throw(
            p,
            "name",
            PropertyDescriptor {
                value: Some(s),
                ..Default::default()
            },
        )
        .unwrap();
        let n = cx.get(p, "name").unwrap();
        assert_eq!(cx.to_rust_string(n).unwrap(), "renamed");
        // Array length: ArraySetLength.
        let a = cx.new_array();
        cx.set(a, "4", one, true).unwrap();
        let len = PropertyDescriptor {
            value: Some(Local::int32(2)),
            ..Default::default()
        };
        assert!(cx.define_own_property(a, "length", len).unwrap());
        assert_eq!(crate::object::array_len(a.bits()), 2);
        let bad = PropertyDescriptor {
            value: Some(Local::number(1.5)),
            ..Default::default()
        };
        assert!(cx.define_own_property(a, "length", bad).is_err());
        assert_eq!(take_error(cx, rt), "RangeError: Invalid array length");
        let enumerable = PropertyDescriptor {
            enumerable: Some(true),
            ..Default::default()
        };
        assert!(!cx.define_own_property(a, "length", enumerable).unwrap());
        // Accessors (P3-ACC; tests/accessors.rs has the full validation).
        let f = global(cx, "probe");
        let acc = PropertyDescriptor {
            get: Some(f),
            ..Default::default()
        };
        assert!(cx.define_own_property(o, "g", acc).unwrap());
        assert!(cx.get_own_property(o, "g").unwrap().unwrap().is_accessor());
    });
}

#[test]
fn own_property_keys_order() {
    with_cx(true, setup_probe, |cx, _| {
        // {b:1, 2:1, a:1, 1:1}: indices ascending, then strings in creation order.
        let o = cx.new_object();
        for k in ["b", "2", "a", "1"] {
            cx.create_data_property_or_throw(o, k, Local::int32(1))
                .unwrap();
        }
        let keys = cx.own_property_keys(o).unwrap();
        assert_eq!(texts(cx, keys), ["1", "2", "b", "a"]);
        // An array: elements, then `length`, then other strings.
        let a = cx.new_array();
        cx.create_data_property_or_throw(a, "x", Local::int32(1))
            .unwrap();
        cx.create_data_property_or_throw(a, "1", Local::int32(1))
            .unwrap();
        cx.create_data_property_or_throw(a, "0", Local::int32(1))
            .unwrap();
        let keys = cx.own_property_keys(a).unwrap();
        assert_eq!(texts(cx, keys), ["0", "1", "length", "x"]);
        // A function: length, name, prototype (constructors), then the table in order.
        let p = global(cx, "probe").as_object().unwrap();
        cx.create_data_property_or_throw(p, "z", Local::int32(1))
            .unwrap();
        cx.get(p, "prototype").unwrap();
        let keys = cx.own_property_keys(p).unwrap();
        assert_eq!(texts(cx, keys), ["length", "name", "prototype", "z"]);
        let f = global(cx, "forward").as_object().unwrap();
        let keys = cx.own_property_keys(f).unwrap();
        assert_eq!(texts(cx, keys), ["length", "name"]);
    });
}

#[test]
fn object_ops_and_conversions() {
    with_cx(true, setup_probe, |cx, rt| {
        let o = cx.new_object();
        let proto = cx.new_object();
        assert!(cx.set_prototype_of(o, proto).unwrap());
        assert!(!cx.set_prototype_of(proto, o).unwrap(), "a cycle");
        let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
        assert!(
            !cx.set_prototype_of(op, proto).unwrap(),
            "immutable prototype"
        );
        assert!(cx.set_prototype_of(op, Local::null()).unwrap(), "unchanged");
        cx.create_data_property_or_throw(proto, "inherited", Local::int32(4))
            .unwrap();
        assert!(cx.has_property(o, "inherited").unwrap());
        assert!(!cx.has_own_property(o, "inherited").unwrap());
        assert_eq!(cx.get(o, "inherited").unwrap().as_int32(), Some(4));
        let ten = cx.new_string("10").unwrap();
        assert_eq!(cx.get_v(ten, "length").unwrap().as_int32(), Some(2));
        assert!(cx.get_v(Local::undefined(), "x").is_err());
        take_error(cx, rt);
        // Conversions.
        assert_eq!(cx.to_number(ten).unwrap(), 10.0);
        assert_eq!(cx.to_int32(Local::number(4294967297.0)).unwrap(), 1);
        assert_eq!(cx.to_uint32(Local::number(-1.0)).unwrap(), u32::MAX);
        assert_eq!(cx.to_length(Local::number(-5.0)).unwrap(), 0);
        assert_eq!(
            cx.to_length(Local::number(f64::INFINITY)).unwrap(),
            (1 << 53) - 1
        );
        assert_eq!(
            cx.to_integer_or_infinity(Local::number(-2.7)).unwrap(),
            -2.0
        );
        assert_eq!(
            cx.to_integer_or_infinity(Local::number(f64::NAN)).unwrap(),
            0.0
        );
        assert!(!cx.to_boolean(Local::int32(0)) && cx.to_boolean(o));
        let s = cx.to_string(Local::number(1.5)).unwrap();
        assert_eq!(cx.to_rust_string(s).unwrap(), "1.5");
        let k = cx.to_property_key(Local::int32(7)).unwrap();
        assert_eq!(cx.to_rust_string(k).unwrap(), "7");
        assert!(
            cx.to_primitive(o, ToPrimitiveHint::String)
                .unwrap()
                .is_string()
        );
        assert!(cx.to_object(o).unwrap().is_same_bits(&o));
        assert!(cx.to_object(Local::null()).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot convert undefined or null to object"
        );
        // ToObject boxes a primitive in a wrapper with its type's prototype (P2-SYM wrappers).
        let abc = cx.new_string("abc").unwrap();
        for (v, ty) in [
            (abc.value(), PrimitiveType::String),
            (Local::int32(3), PrimitiveType::Number),
            (Local::number(1.5), PrimitiveType::Number),
            (Local::boolean(true), PrimitiveType::Boolean),
        ] {
            let w = cx.to_object(v).unwrap();
            let proto = cx.get_prototype_of(w).unwrap();
            assert!(proto.is_same_bits(&cx.intrinsic(ty.prototype())));
            let inner = cx.this_primitive_value(w, ty).unwrap();
            assert!(cx.same_value(inner, v));
        }
        let w = cx.to_object(abc).unwrap();
        let len = cx.get_own_property(w, "length").unwrap().unwrap();
        assert_eq!(len.value.and_then(|v| v.as_int32()), Some(3));
        let np = cx.intrinsic(IntrinsicId::NumberPrototype);
        let w = cx.new_wrapper(np, Local::int32(3)).unwrap();
        let inner = cx.this_primitive_value(w, PrimitiveType::Number).unwrap();
        assert_eq!(inner.as_int32(), Some(3));
        // SameValue and IsStrictlyEqual.
        let nan = Local::number(f64::NAN);
        assert!(cx.same_value(nan, nan) && !cx.is_strictly_equal(nan, nan));
        assert!(!cx.same_value(Local::number(0.0), Local::number(-0.0)));
        assert!(cx.is_strictly_equal(Local::number(0.0), Local::number(-0.0)));
        let ten2 = cx.new_string("10").unwrap();
        assert!(cx.same_value(ten, ten2) && !ten.is_same_bits(&ten2));
        // Predicates and typeof.
        let p = global(cx, "probe");
        let fw = global(cx, "forward");
        assert!(cx.is_callable(p) && cx.is_constructor(p) && !cx.is_constructor(fw));
        let arr = cx.new_array();
        assert!(cx.is_array(arr) && !cx.is_array(o));
        assert_eq!(cx.type_of(p), "function");
        assert_eq!(cx.type_of(ten), "string");
        // Downcasts.
        assert!(p.as_function().is_some() && p.as_array().is_none());
        assert!(Local::int32(1).as_object().is_none());
        // Strings.
        let flat = cx.flatten(ten).unwrap();
        assert_eq!(cx.string_len(flat), 2);
        assert_eq!(cx.code_unit_at(flat, 1), Some(u16::from(b'0')));
        assert_eq!(cx.code_unit_at(flat, 2), None);
        let u = cx.new_string_utf16(&[0xD83D, 0xDE00]).unwrap();
        assert_eq!(cx.string_len(u), 2);
        // GetPrototypeFromConstructor falls back to the intrinsic.
        let f = global(cx, "forward").as_object().unwrap();
        let fallback = cx
            .get_prototype_from_constructor(f, IntrinsicId::ArrayPrototype)
            .unwrap();
        let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
        assert!(fallback.is_same_bits(&ap));
        let made = cx
            .ordinary_create_from_constructor(f, IntrinsicId::ErrorPrototype)
            .unwrap();
        let ep = cx.intrinsic(IntrinsicId::ErrorPrototype);
        assert!(cx.get_prototype_of(made).unwrap().is_same_bits(&ep));
        // Symbols (P2-SYM).
        let s = cx.new_symbol(Local::undefined()).unwrap();
        assert!(cx.symbol_description(s).is_undefined());
        let d = cx.new_string("d").unwrap();
        let s = cx.new_symbol(d).unwrap();
        let text = cx.descriptive_string(s).unwrap();
        assert_eq!(cx.to_rust_string(text).unwrap(), "Symbol(d)");
        assert!(cx.key_for(s).is_none());
        let r = cx.symbol_for(d).unwrap();
        assert!(cx.symbol_for(d).unwrap().is_same_bits(&r));
        assert!(cx.key_for(r).is_some_and(|k| cx.same_value(k, d)));
    });
}
