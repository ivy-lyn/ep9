//! Inline cache tests (H16 v0, `ic.rs`): the entry states (mono, poly 2 to 4, mega), the
//! handlers of the named sites (0x21, 0x22, 0x24, 0x25) and the global sites (0x10 to 0x12),
//! the epoch, and the weak references of IC data.
//!
//! Each test runs under every dispatch expansion, with and without GC stress (a collection
//! before every allocation, then the heap verifier), with eager feedback (threshold 0: a
//! function has its vector from its first call). The functions under test are hand-assembled
//! (`f0`, `f1`, ... are globals); objects come from the runtime and live in Rust locals (the
//! conservative stack scan keeps them). A local keeps its cell only up to its last use: optimized
//! code may drop it there. An object that an assertion about weak IC data needs alive after its
//! last use is rooted by a handle ([`T::keep`]).

use std::hint::black_box;

use ep9_bytecode::{
    FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionFlags, FunctionId, VerifiedUnit,
};
use ep9_heap::GlobalHandle;

use super::{Asm, config, isolate, meta, on_big_stack, print, unit};
use crate::atoms;
use crate::feedback;
use crate::frame::{NULL, UNDEFINED, int32_bits};
use crate::ic::{self, KEY_STRING, State, hk, smi};
use crate::object::{self, attr};
use crate::property;
use crate::props;
use crate::realm;
use crate::runtime::{Runtime, val};
use crate::shape;
use crate::string;
use crate::{Dispatch, Exception, Isolate, IsolateConfig, Value};

/// The feedback slot of the site of each test function (its only slot).
const SLOT: u32 = 0;

/// The site of one test function `f<i>(a, b)`.
#[derive(Clone, Copy, Debug)]
enum Site {
    /// `return a.name` (0x21).
    Get(&'static str),
    /// `a.name = b` (0x24), in strict code if the flag is set.
    Set(&'static str, bool),
    /// CreateDataPropertyOrThrow(a, name, b) (0x25).
    Define(&'static str),
    /// `return super.name` with the home object `a` and the receiver `b` (0x22).
    Super(&'static str),
    /// `return name` (0x10).
    Global(&'static str),
    /// `return name` inside `typeof` (0x11: no ReferenceError).
    GlobalTypeof(&'static str),
    /// `name = a` (0x12, sloppy).
    StoreGlobal(&'static str),
    /// `a.x; return a.y` with one slot for both sites.
    Shared,
}

/// Function `f<i>`: two parameters and the one site.
fn function(site: Site, i: usize) -> FunctionCode {
    let mut m = meta(&format!("f{i}"), 2, 0, 0, 0);
    if let Site::Set(_, true) = site {
        m.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
    }
    let mut b = FunctionBuilder::new(m);
    let (a, x) = (b.parameter(0), b.parameter(1));
    let slot_kind = match site {
        Site::Get(_) | Site::Super(_) | Site::Shared => FeedbackSlotKind::LoadNamed,
        Site::Set(..) => FeedbackSlotKind::StoreNamed,
        Site::Define(_) => FeedbackSlotKind::DefineNamedOwn,
        Site::Global(_) => FeedbackSlotKind::LoadGlobal,
        Site::GlobalTypeof(_) => FeedbackSlotKind::LoadGlobalInsideTypeof,
        Site::StoreGlobal(_) => FeedbackSlotKind::StoreGlobal,
    };
    let s = b.feedback_slot(slot_kind);
    assert_eq!(s.index(), SLOT as usize);
    match site {
        Site::Get(n) => {
            let k = b.string_constant(n);
            b.get_named_property(a, k, s);
        }
        Site::Set(n, _) => {
            let k = b.string_constant(n);
            b.ldar(x).set_named_property(a, k, s);
        }
        Site::Define(n) => {
            let k = b.string_constant(n);
            b.ldar(x).define_named_own_property(a, k, s);
        }
        Site::Super(n) => {
            let k = b.string_constant(n);
            b.ldar(a).get_named_property_from_super(x, k, s);
        }
        Site::Global(n) => {
            let k = b.string_constant(n);
            b.lda_global(k, s);
        }
        Site::GlobalTypeof(n) => {
            let k = b.string_constant(n);
            b.lda_global_inside_typeof(k, s);
        }
        Site::StoreGlobal(n) => {
            let k = b.string_constant(n);
            b.ldar(a).sta_global(k, s);
        }
        Site::Shared => {
            let (kx, ky) = (b.string_constant("x"), b.string_constant("y"));
            let r = b.new_register();
            b.get_named_property(a, kx, s).star(r);
            b.get_named_property(a, ky, s);
        }
    }
    b.return_();
    b.finish().unwrap()
}

/// The script defines the global `f<i>` for each of `sites`.
fn program(sites: &[Site]) -> VerifiedUnit {
    let mut s = Asm::script(0);
    for i in 0..sites.len() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(&format!("f{i}"));
    }
    s.b.lda_undefined().return_();
    let mut fns = vec![s.finish()];
    fns.extend(sites.iter().enumerate().map(|(i, &site)| function(site, i)));
    unit("", fns)
}

/// An isolate that ran [`program`].
struct T {
    iso: Isolate,
}

impl T {
    fn new(c: IsolateConfig, sites: &[Site]) -> T {
        let mut iso = isolate(c);
        iso.run_script(program(sites), "ic.js").unwrap();
        T { iso }
    }

    fn rt(&self) -> &Runtime {
        &self.iso.rt
    }

    fn f(&self, i: usize) -> Value {
        self.iso.global(&format!("f{i}")).unwrap()
    }

    /// `f<i>(a, b)`.
    fn call(&mut self, i: usize, a: u64, b: u64) -> Result<u64, Exception> {
        let f = self.f(i);
        self.iso
            .call(f, Value::UNDEFINED, &[val(a), val(b)])
            .map(|v| v.to_bits())
    }

    /// `f<i>(a, b)`, which must not throw.
    fn ok(&mut self, i: usize, a: u64, b: u64) -> u64 {
        self.call(i, a, b)
            .unwrap_or_else(|e| panic!("f{i} threw {}", e.message))
    }

    /// `f<i>(a, b)`, which must throw an error made by `ctor`.
    fn throws(&mut self, i: usize, a: u64, b: u64, ctor: &str) {
        match self.call(i, a, b) {
            Ok(v) => panic!("f{i} returned {v:#x}, not a {ctor}"),
            Err(e) => assert_eq!(e.constructor.as_deref(), Some(ctor), "f{i}: {}", e.message),
        }
    }

    /// The feedback vector of `f<i>` (function `i + 1`).
    fn vector(&self, i: usize) -> u64 {
        let units = self.iso.rt.units.borrow();
        feedback::vector(&units.last().unwrap().infos[i + 1]).expect("eager feedback")
    }

    fn state(&self, i: usize) -> State {
        ic::state(self.vector(i), SLOT)
    }

    /// The handler `f<i>` keeps for the case key `key` (a shape or KEY_STRING).
    fn handler(&self, i: usize, key: u64) -> Option<u64> {
        ic::find(self.vector(i), SLOT, key)
    }

    /// (kind, epoch, reference, location) of that handler ([`ic::describe_handler`]).
    fn describe(&self, i: usize, key: u64) -> Option<(i32, u32, u64, i32)> {
        self.handler(i, key).map(ic::describe_handler)
    }

    fn kind(&self, i: usize, key: u64) -> Option<i32> {
        self.describe(i, key).map(|d| d.0)
    }

    fn epoch(&self) -> u32 {
        self.rt().shapes.proto_epoch.get()
    }

    fn collect(&mut self) {
        self.iso.collect_garbage();
        self.iso.verify_heap().unwrap();
    }

    /// Roots `v` (a strong handle) until [`T::release`].
    fn keep(&self, v: u64) -> GlobalHandle {
        self.rt().heap.new_global(val(v))
    }

    fn release(&self, h: GlobalHandle) {
        self.rt().heap.drop_global(h);
    }
}

/// The kinds [`ic::describe_handler`] gives for IC_HANDLER cells.
const PROTO_LOAD: i32 = hk::PROTO_LOAD | 0x100;
const MISSING: i32 = hk::MISSING | 0x100;
const ADD: i32 = hk::ADD | 0x100;
const GLOBAL: i32 = hk::GLOBAL | 0x100;

/// Runs `test` under every dispatch expansion, with and without GC stress, with eager
/// feedback, on a big stack.
fn each(test: fn(IsolateConfig)) {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut c = config(d, stress);
                c.feedback_threshold = 0;
                c.heap.verify_after_gc = true;
                c.heap.record_conservative_roots = true;
                test(c);
            });
        }
    }
}

fn key(rt: &Runtime, name: &str) -> u64 {
    atoms::intern_str(rt, name)
}

fn int(i: i32) -> u64 {
    int32_bits(i)
}

/// Object.prototype.
fn object_prototype(rt: &Runtime) -> u64 {
    object::proto_of(object::new_plain_object(rt))
}

/// A new object with prototype `proto` and the data properties `props` {w, e, c}, added in
/// that order.
fn object_with(rt: &Runtime, proto: u64, props: &[(&str, u64)]) -> u64 {
    let o = object::new_object(rt, proto);
    for &(name, v) in props {
        props::own_define(rt, o, key(rt, name), v, attr::DEFAULT);
    }
    o
}

/// The own property `name` of `o`.
fn own(rt: &Runtime, o: u64, name: &str) -> Option<u64> {
    props::own_get(o, key(rt, name)).map(|(v, _)| v)
}

/// A new function object (a host function), a "method".
fn method(rt: &Runtime, name: &str) -> u64 {
    realm::new_host_function(rt, print, name, 0, false)
}

// ---- dead cells: the stack must not keep them ----------------------------------------------

const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;

/// Overwrites 256 KiB of stack below the caller, so stale words of returned frames cannot
/// keep a dead cell alive through the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Panics if the last conservative scan kept one of `cells` (masked addresses): the check
/// that follows would not be about IC data then.
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

/// Loads `x` through `f0` from a new object `{pad: 0, x: 1}` whose shape no other object has,
/// in a frame of its own. Returns the masked addresses of the object and its shape.
#[inline(never)]
fn load_from_a_lone_shape(t: &mut T, pad: &str) -> [usize; 2] {
    let p = object_prototype(t.rt());
    let o = object_with(t.rt(), p, &[(pad, int(0)), ("x", int(1))]);
    assert_eq!(t.ok(0, o, UNDEFINED), int(1));
    let s = shape::shape_of(o);
    [black_box(o as usize) ^ MASK, black_box(s as usize) ^ MASK]
}

// ---- states --------------------------------------------------------------------------------

#[test]
fn a_load_and_a_store_go_mono_then_poly_2_3_4_then_mega() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Set("x", false)]);
        let p = object_prototype(t.rt());
        // Five shapes with `x`: {x}, {a, x}, {b, x}, {c, x}, {d, x}.
        let o0 = object_with(t.rt(), p, &[("x", int(10))]);
        let o1 = object_with(t.rt(), p, &[("a", int(0)), ("x", int(11))]);
        let o2 = object_with(t.rt(), p, &[("b", int(0)), ("x", int(12))]);
        let o3 = object_with(t.rt(), p, &[("c", int(0)), ("x", int(13))]);
        let o4 = object_with(t.rt(), p, &[("d", int(0)), ("x", int(14))]);
        let objs = [o0, o1, o2, o3, o4];
        let want = [
            State::Monomorphic,
            State::Polymorphic(2),
            State::Polymorphic(3),
            State::Polymorphic(4),
            State::Megamorphic,
        ];
        for (i, &o) in objs.iter().enumerate() {
            assert_eq!(t.ok(0, o, UNDEFINED), int(10 + i as i32));
            t.ok(1, o, int(20 + i as i32));
            assert_eq!(own(t.rt(), o, "x"), Some(int(20 + i as i32)));
            assert_eq!((t.state(0), t.state(1)), (want[i], want[i]), "shape {i}");
            // Every shape seen so far loads and stores its own `x`.
            for (j, &q) in objs[..=i].iter().enumerate() {
                t.ok(1, q, int(30 + j as i32));
                assert_eq!(t.ok(0, q, UNDEFINED), int(30 + j as i32), "shape {j}");
                if i < 4 {
                    let s = shape::shape_of(q);
                    assert_eq!(t.kind(0, s), Some(smi::FIELD_IN_OBJECT));
                    assert_eq!(t.kind(1, s), Some(smi::FIELD_IN_OBJECT));
                }
            }
        }
        // Megamorphic is sticky: a sixth shape takes the generic path and records nothing.
        let o5 = object_with(t.rt(), p, &[("e", int(0)), ("x", int(15))]);
        assert_eq!(t.ok(0, o5, UNDEFINED), int(15));
        t.ok(1, o5, int(25));
        assert_eq!(own(t.rt(), o5, "x"), Some(int(25)));
        assert_eq!(
            (t.state(0), t.state(1)),
            (State::Megamorphic, State::Megamorphic)
        );
        t.collect();
        assert_eq!(t.ok(0, o0, UNDEFINED), int(30));
    });
}

#[test]
fn a_slot_two_names_share_is_megamorphic_from_the_start() {
    each(|c| {
        // BytecodeGen gives each named site its own slot; hand-built code may share one, and a
        // case for `x` must not answer the load of `y`.
        let mut t = T::new(c, &[Site::Shared]);
        let p = object_prototype(t.rt());
        let o = object_with(t.rt(), p, &[("x", int(1)), ("y", int(2))]);
        for _ in 0..3 {
            assert_eq!(t.ok(0, o, UNDEFINED), int(2));
            assert_eq!(t.state(0), State::Megamorphic);
        }
    });
}

// ---- receivers -----------------------------------------------------------------------------

#[test]
fn lengths_use_the_kind_byte_and_dictionary_receivers_take_the_slow_path() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("length")]);
        // A string: the KEY_STRING case.
        let s = string::new_string(t.rt(), "abcd");
        assert_eq!(t.ok(0, s, UNDEFINED), int(4));
        assert_eq!(t.kind(0, KEY_STRING), Some(smi::STRING_LENGTH));
        let s2 = string::new_string(t.rt(), "a longer string");
        assert_eq!(t.ok(0, s2, UNDEFINED), int(15));
        // An array: its length field, which the hit reads every time.
        let a = object::new_array(t.rt(), 0);
        assert!(object::array_set(t.rt(), a, 2, int(1)));
        assert_eq!(t.ok(0, a, UNDEFINED), int(3));
        assert_eq!(t.kind(0, shape::shape_of(a)), Some(smi::ARRAY_LENGTH));
        assert!(object::array_set(t.rt(), a, 5, int(1)));
        assert_eq!(t.ok(0, a, UNDEFINED), int(6));
        assert_eq!(t.state(0), State::Polymorphic(2));
        // A dictionary-mode object (a delete that is not the last, OM-10): SLOW keeps its
        // place, and the generic path reads the value every time.
        let p = object_prototype(t.rt());
        let d = object_with(t.rt(), p, &[("pad", int(0)), ("length", int(7))]);
        let pad = key(t.rt(), "pad");
        assert!(property::delete(&mut t.iso.cx, d, pad, true).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(d)));
        assert_eq!(t.ok(0, d, UNDEFINED), int(7));
        assert_eq!(t.kind(0, shape::shape_of(d)), Some(smi::SLOW));
        assert_eq!(t.state(0), State::Polymorphic(3));
        let length = key(t.rt(), "length");
        props::own_define(t.rt(), d, length, int(8), attr::DEFAULT);
        assert_eq!(t.ok(0, d, UNDEFINED), int(8));
        assert_eq!(t.ok(0, s, UNDEFINED), int(4));
    });
}

#[test]
fn out_of_line_fields_load_store_and_add() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Set("x", true)]);
        let p = object_prototype(t.rt());
        let names = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9"];
        let pads: Vec<(&str, u64)> = names.iter().map(|&n| (n, int(0))).collect();
        // `x` after ten others: past the in-object capacity.
        let o = object_with(t.rt(), p, &pads);
        props::own_define(t.rt(), o, key(t.rt(), "x"), int(1), attr::DEFAULT);
        let s = shape::shape_of(o);
        assert!(shape::field_layout(s).0 < 11);
        assert_eq!(t.ok(0, o, UNDEFINED), int(1));
        assert_eq!(t.kind(0, s), Some(smi::FIELD_OUT_OF_LINE));
        t.ok(1, o, int(2));
        t.ok(1, o, int(3));
        assert_eq!(t.kind(1, s), Some(smi::FIELD_OUT_OF_LINE));
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        // Adds of an out-of-line `x`: the ADD case stores into room the SLOTS cell has, and
        // growth takes the slow path. Either way the value lands.
        for v in 0..6 {
            let q = object_with(t.rt(), p, &pads);
            t.ok(1, q, int(v));
            assert_eq!(own(t.rt(), q, "x"), Some(int(v)));
            assert_eq!(shape::shape_of(q), s);
            assert_eq!(t.ok(0, q, UNDEFINED), int(v));
        }
        assert_eq!(t.kind(1, shape::parent(s)), Some(ADD));
    });
}

// ---- prototype chain: the epoch, the holder ------------------------------------------------

#[test]
fn a_warm_proto_load_returns_the_replaced_method() {
    each(|c| {
        let mut t = T::new(
            c,
            &[Site::Get("m"), Site::Set("m", true), Site::Get("nope")],
        );
        let op = object_prototype(t.rt());
        let m1 = method(t.rt(), "m1");
        let p = object_with(t.rt(), op, &[("m", m1)]);
        let o = object::new_object(t.rt(), p);
        assert_eq!(t.ok(0, o, UNDEFINED), m1);
        let so = shape::shape_of(o);
        let h = t.handler(0, so).unwrap();
        let (kind, _, holder, _) = ic::describe_handler(h);
        assert_eq!((kind, holder), (PROTO_LOAD, p));
        assert_eq!(t.ok(0, o, UNDEFINED), m1);
        // `p.m = m2` replaces the method: a value store, so the epoch stays.
        let m2 = method(t.rt(), "m2");
        let e = t.epoch();
        t.ok(1, p, m2);
        assert_eq!(t.epoch(), e);
        assert_eq!(t.ok(0, o, UNDEFINED), m2);
        assert_eq!(
            t.handler(0, so),
            Some(h),
            "a hit: the case was not made again"
        );
        // A closer prototype gets its own `m`: a structural change, so the case misses.
        let q = object::new_object(t.rt(), p);
        let o2 = object::new_object(t.rt(), q);
        assert_eq!(t.ok(0, o2, UNDEFINED), m2);
        let m3 = method(t.rt(), "m3");
        t.ok(1, q, m3);
        assert_ne!(t.epoch(), e);
        assert_eq!(t.ok(0, o2, UNDEFINED), m3);
        assert_eq!(t.ok(0, o, UNDEFINED), m2);
        let (kind, _, holder, _) = t.describe(0, shape::shape_of(o2)).unwrap();
        assert_eq!((kind, holder), (PROTO_LOAD, q));
        // A missing property: undefined under the epoch, until a prototype gets it.
        assert_eq!(t.ok(2, o, UNDEFINED), UNDEFINED);
        assert_eq!(t.kind(2, so), Some(MISSING));
        assert_eq!(t.ok(2, o, UNDEFINED), UNDEFINED);
        props::own_define(t.rt(), p, key(t.rt(), "nope"), int(5), attr::DEFAULT);
        assert_eq!(t.ok(2, o, UNDEFINED), int(5));
    });
}

#[test]
fn a_read_only_x_on_object_prototype_makes_a_warm_add_ic_fail_a_strict_store() {
    each(|c| {
        let mut t = T::new(
            c,
            &[
                Site::Set("x", true),
                Site::Set("x", false),
                Site::Define("x"),
            ],
        );
        let op = object_prototype(t.rt());
        let root = shape::shape_of(object::new_object(t.rt(), op));
        // Warm: each site adds `x` to `{}` twice; the second add hits.
        for i in 0..3 {
            for v in 1..=2 {
                let o = object::new_object(t.rt(), op);
                t.ok(i, o, int(v));
                assert_eq!(own(t.rt(), o, "x"), Some(int(v)));
            }
            assert_eq!(t.state(i), State::Monomorphic);
            assert_eq!(t.kind(i, root), Some(ADD));
        }
        // Object.prototype gets a read-only `x`: a structural change of a prototype.
        let e = t.epoch();
        let x = key(t.rt(), "x");
        props::own_define(t.rt(), op, x, int(9), attr::CONFIGURABLE);
        assert_ne!(t.epoch(), e);
        let o = object::new_object(t.rt(), op);
        // Strict `o.x = 3`: TypeError, and `o` has no own `x`.
        t.throws(0, o, int(3), "TypeError");
        assert_eq!(own(t.rt(), o, "x"), None);
        assert_eq!(shape::shape_of(o), root);
        // Sloppy: no error and no own `x`.
        t.ok(1, o, int(3));
        assert_eq!(own(t.rt(), o, "x"), None);
        // CreateDataPropertyOrThrow does not look at the chain: its add case still applies.
        t.ok(2, o, int(4));
        assert_eq!(own(t.rt(), o, "x"), Some(int(4)));
    });
}

#[test]
fn add_property_cases_miss_on_a_non_extensible_receiver() {
    each(|c| {
        let mut t = T::new(
            c,
            &[
                Site::Set("x", true),
                Site::Define("x"),
                Site::Set("x", false),
            ],
        );
        let op = object_prototype(t.rt());
        let root = shape::shape_of(object::new_object(t.rt(), op));
        for i in 0..3 {
            for v in 1..=2 {
                let o = object::new_object(t.rt(), op);
                t.ok(i, o, int(v));
            }
            assert_eq!(t.kind(i, root), Some(ADD));
        }
        // A non-extensible `{}` has a dictionary shape (OM-10), never the root shape.
        let o = object::new_object(t.rt(), op);
        assert!(property::prevent_extensions(t.rt(), o));
        let s = shape::shape_of(o);
        assert!(shape::is_dictionary(s) && s != root);
        t.throws(0, o, int(5), "TypeError");
        t.throws(1, o, int(5), "TypeError");
        t.ok(2, o, int(5));
        assert_eq!(own(t.rt(), o, "x"), None);
        // The throwing stores recorded nothing. The sloppy one ran (and added nothing): a
        // dictionary shape gets SLOW. The add cases still serve extensible objects.
        assert_eq!(
            (t.state(0), t.state(1)),
            (State::Monomorphic, State::Monomorphic)
        );
        assert_eq!((t.handler(0, s), t.handler(1, s)), (None, None));
        assert_eq!(t.state(2), State::Polymorphic(2));
        assert_eq!(t.kind(2, s), Some(smi::SLOW));
        for i in 0..3 {
            assert_eq!(t.kind(i, root), Some(ADD));
            let q = object::new_object(t.rt(), op);
            t.ok(i, q, int(6));
            assert_eq!(own(t.rt(), q, "x"), Some(int(6)));
        }
    });
}

#[test]
fn a_super_load_is_keyed_on_the_home_objects_prototype() {
    each(|c| {
        let mut t = T::new(c, &[Site::Super("x"), Site::Set("x", true)]);
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("x", int(1))]);
        // `p` (a prototype: its shape is its own) keeps its case only while it lives, and the
        // home object stops referencing it below.
        let kept = t.keep(p);
        let home = object::new_object(t.rt(), p);
        let receiver = object_with(t.rt(), op, &[("x", int(100))]);
        assert_eq!(t.ok(0, home, receiver), int(1));
        assert_eq!(t.kind(0, shape::shape_of(p)), Some(smi::FIELD_IN_OBJECT));
        assert_eq!(t.handler(0, shape::shape_of(home)), None);
        t.ok(1, p, int(2));
        assert_eq!(t.ok(0, home, receiver), int(2));
        // Another prototype of the home object: another lookup start, another case.
        let q = object_with(t.rt(), op, &[("y", int(0)), ("x", int(3))]);
        assert!(property::set_prototype_of(t.rt(), home, q));
        assert_eq!(t.ok(0, home, receiver), int(3));
        assert_eq!(t.state(0), State::Polymorphic(2));
        t.release(kept);
        // A home object whose prototype is null: GetSuperBase gives null, and the load throws.
        assert!(property::set_prototype_of(t.rt(), home, NULL));
        t.throws(0, home, receiver, "TypeError");
    });
}

// ---- weak references -----------------------------------------------------------------------

#[test]
fn a_dead_shape_clears_its_entry() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        let dead = load_from_a_lone_shape(&mut t, "icPadA");
        assert_eq!(t.state(0), State::Monomorphic);
        scrub_stack();
        t.collect();
        assert_not_kept(t.rt(), &dead);
        assert_eq!(feedback::key(t.vector(0), SLOT), UNDEFINED);
        assert_eq!(t.state(0), State::Uninitialized);
        // A polymorphic case: two live shapes and one that dies. Handles keep the live objects
        // (and so their shapes) to the end.
        let p = object_prototype(t.rt());
        let a = object_with(t.rt(), p, &[("x", int(1))]);
        let b = object_with(t.rt(), p, &[("icPadB", int(0)), ("x", int(2))]);
        let kept = [t.keep(a), t.keep(b)];
        assert_eq!(t.ok(0, a, UNDEFINED), int(1));
        assert_eq!(t.ok(0, b, UNDEFINED), int(2));
        let dead = load_from_a_lone_shape(&mut t, "icPadC");
        assert_eq!(t.state(0), State::Polymorphic(3));
        scrub_stack();
        t.collect();
        assert_not_kept(t.rt(), &dead);
        assert_eq!(t.state(0), State::Polymorphic(2));
        assert_eq!(t.ok(0, a, UNDEFINED), int(1));
        assert_eq!(t.ok(0, b, UNDEFINED), int(2));
        // The free place takes the next shape: four live cases before megamorphic.
        let d = object_with(t.rt(), p, &[("icPadD", int(0)), ("x", int(4))]);
        let e = object_with(t.rt(), p, &[("icPadE", int(0)), ("x", int(5))]);
        let kept = [kept, [t.keep(d), t.keep(e)]];
        assert_eq!(t.ok(0, d, UNDEFINED), int(4));
        assert_eq!(t.state(0), State::Polymorphic(3));
        assert_eq!(t.ok(0, e, UNDEFINED), int(5));
        assert_eq!(t.state(0), State::Polymorphic(4));
        for h in kept.into_iter().flatten() {
            t.release(h);
        }
    });
}

/// Builds `o → p1 → p2` with `m` = 7 on `p2`, loads `o.m` through `f0` (a prototype-load case
/// whose holder is `p2`), then cuts `p2` off the chain. Returns `o` and the masked address of
/// `p2`, which nothing references now.
#[inline(never)]
fn proto_load_through_a_lone_holder(t: &mut T) -> (u64, usize) {
    let op = object_prototype(t.rt());
    let p2 = object_with(t.rt(), op, &[("m", int(7))]);
    let p1 = object::new_object(t.rt(), p2);
    let o = object::new_object(t.rt(), p1);
    assert_eq!(t.ok(0, o, UNDEFINED), int(7));
    let (kind, _, holder, _) = t.describe(0, shape::shape_of(o)).unwrap();
    assert_eq!((kind, holder), (PROTO_LOAD, p2));
    assert!(property::set_prototype_of(t.rt(), p1, NULL));
    (o, black_box(p2 as usize) ^ MASK)
}

#[test]
fn a_proto_load_whose_holder_died_misses() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m")]);
        let (o, p2) = proto_load_through_a_lone_holder(&mut t);
        scrub_stack();
        t.collect();
        assert_not_kept(t.rt(), &[p2]);
        // The case stays (its key, the shape of `o`, lives); its weak holder is cleared.
        let (kind, made_at, holder, _) = t.describe(0, shape::shape_of(o)).unwrap();
        assert_eq!((kind, holder), (PROTO_LOAD, UNDEFINED));
        // The changed epoch alone would miss. Put the case's epoch back, so the dead holder is
        // what makes the hit path miss.
        assert_ne!(t.epoch(), made_at);
        t.rt().shapes.proto_epoch.set(made_at);
        assert_eq!(t.ok(0, o, UNDEFINED), UNDEFINED);
        assert_eq!(t.kind(0, shape::shape_of(o)), Some(MISSING));
        assert_eq!(t.ok(0, o, UNDEFINED), UNDEFINED);
    });
}

// ---- globals -------------------------------------------------------------------------------

/// `f<i>(a)` and the number of scans of the global object's shape it took.
fn scans_of_the_global_object(t: &mut T, i: usize, a: u64) -> (u64, u32) {
    let f = t.f(i);
    let gs = shape::shape_of(t.rt().global());
    shape::SCANS.with(|c| c.set((gs, 0)));
    let r = t
        .iso
        .call(f, Value::UNDEFINED, &[val(a)])
        .unwrap()
        .to_bits();
    (r, shape::SCANS.with(|c| c.get().1))
}

#[test]
fn a_found_global_costs_one_scan_of_the_global_object() {
    each(|c| {
        let sites = [Site::Global("icG"), Site::GlobalTypeof("icG")];
        // No vector (threshold never): the generic path at every load.
        let never = IsolateConfig {
            feedback_threshold: u32::MAX,
            ..c.clone()
        };
        for config in [never, c] {
            let eager = config.feedback_threshold == 0;
            let mut t = T::new(config, &sites);
            let g = t.rt().global();
            props::own_define(t.rt(), g, key(t.rt(), "icG"), int(5), attr::DEFAULT);
            assert!(!shape::is_dictionary(shape::shape_of(g)));
            for i in 0..2 {
                // The miss that records the case reads the value from the case's lookup.
                assert_eq!(
                    scans_of_the_global_object(&mut t, i, UNDEFINED),
                    (int(5), 1)
                );
                // Then a hit scans nothing.
                let want = if eager { 0 } else { 1 };
                assert_eq!(
                    scans_of_the_global_object(&mut t, i, UNDEFINED),
                    (int(5), want)
                );
                if eager {
                    assert_eq!(t.kind(i, shape::shape_of(g)), Some(GLOBAL));
                }
            }
        }
    });
}

#[test]
fn a_global_ic_follows_stores_and_misses_after_a_delete() {
    each(|c| {
        let mut t = T::new(
            c,
            &[
                Site::Global("icG"),
                Site::GlobalTypeof("icG"),
                Site::StoreGlobal("icG"),
            ],
        );
        t.throws(0, UNDEFINED, UNDEFINED, "ReferenceError");
        assert_eq!(t.ok(1, UNDEFINED, UNDEFINED), UNDEFINED);
        // A sloppy store makes the global; the next miss records its case.
        t.ok(2, int(5), UNDEFINED);
        assert_eq!(t.ok(0, UNDEFINED, UNDEFINED), int(5));
        assert_eq!(t.ok(1, UNDEFINED, UNDEFINED), int(5));
        let g = t.rt().global();
        let gs = shape::shape_of(g);
        for i in 0..2 {
            assert_eq!(t.kind(i, gs), Some(GLOBAL));
        }
        t.ok(2, int(6), UNDEFINED);
        assert_eq!(t.kind(2, gs), Some(GLOBAL));
        t.ok(2, int(7), UNDEFINED);
        assert_eq!(t.ok(0, UNDEFINED, UNDEFINED), int(7));
        assert_eq!(t.ok(1, UNDEFINED, UNDEFINED), int(7));
        // A delete changes the global object's shape: the cases miss.
        let k = key(t.rt(), "icG");
        assert!(property::delete(&mut t.iso.cx, g, k, false).unwrap());
        assert_ne!(shape::shape_of(g), gs);
        t.throws(0, UNDEFINED, UNDEFINED, "ReferenceError");
        assert_eq!(t.ok(1, UNDEFINED, UNDEFINED), UNDEFINED);
        t.ok(2, int(8), UNDEFINED);
        assert_eq!(t.ok(0, UNDEFINED, UNDEFINED), int(8));
    });
}
