//! Tests of the facts exporter (`facts_export.rs`, H16-Q5): each IC handler of T0 and its
//! fact, the states, the cases that are left out, the root walk, and equal bytes from two
//! isolates. The JS-level round trip on richards is in `crates/ep9_builtins/tests/
//! facts_export.rs`.
//!
//! The functions under test are hand-assembled, one site each (`f0`, `f1`, ... are globals),
//! and run with eager feedback (threshold 0). Objects come from the runtime and live in Rust
//! locals (the conservative stack scan keeps them).

use std::sync::Arc;

use ep9_bytecode::{FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionId, VerifiedUnit};
use ep9_facts::{
    Access, BindingSet, ClosureCount, ElementsKind, FactSnapshot, FieldHint, FieldRep, FunctionRef,
    IcState, KeyRef, ObjectKind, PropAttrs, PropertyFacts, ProtoRef, ShapeDesc, ShapeFlags,
    SlotFacts, TypeSet,
};

use super::ic::{SLOT, Site as IcSite, T as IcT, int, object_prototype, object_with};
use super::{Asm, config, isolate, meta, on_big_stack, profile::isolate as isolate_full, unit};
use crate::abi::{Local, export_facts};
use crate::accessor;
use crate::atoms;
use crate::builtins::IntrinsicId;
use crate::feedback;
use crate::frame::{NULL, UNDEFINED, int32_bits, int32_of};
use crate::ic::{self, KEY_STRING, State, hk, smi};
use crate::ic_ops::{self, Loc, Op, Operation, Ops, Ref, Ty, Val};
use crate::instance_ic::io;
use crate::kinds;
use crate::object::{self, attr, obj};
use crate::props;
use crate::realm;
use crate::runtime::{Root, Runtime, val};
use crate::shape;
use crate::string;
use crate::{Args, Cx, Dispatch, Isolate, JsResult, Value};

/// The site of one test function `f<i>(a, b)`; slot 0 is the site's, slot 1 a Call slot that
/// no T0 IC uses.
#[derive(Clone, Copy, Debug)]
enum Site {
    /// `return a.name` (0x21).
    Get(&'static str),
    /// `a.name = b` (0x24, sloppy).
    Set(&'static str),
    /// CreateDataPropertyOrThrow(a, name, b) (0x25).
    Define(&'static str),
    /// `return name` (0x10).
    Global(&'static str),
    /// `name = a` (0x12, sloppy).
    StoreGlobal(&'static str),
}

const SITES: [Site; 9] = [
    Site::Get("x"),
    Site::Get("length"),
    Site::Get("m"),
    Site::Set("y"),
    Site::Define("z"),
    Site::Global("print"),
    Site::Global("lx"),
    Site::StoreGlobal("g"),
    // Never called: no vector, no record.
    Site::Get("never"),
];

fn function(site: Site, i: usize) -> FunctionCode {
    let mut b = FunctionBuilder::new(meta(&format!("f{i}"), 2, 0, 0, 0));
    let (a, x) = (b.parameter(0), b.parameter(1));
    let kind = match site {
        Site::Get(_) => FeedbackSlotKind::LoadNamed,
        Site::Set(_) => FeedbackSlotKind::StoreNamed,
        Site::Define(_) => FeedbackSlotKind::DefineNamedOwn,
        Site::Global(_) => FeedbackSlotKind::LoadGlobal,
        Site::StoreGlobal(_) => FeedbackSlotKind::StoreGlobal,
    };
    let s = b.feedback_slot(kind);
    // A slot of a kind T0 v0 does not record (H16-Q2): the exporter writes no record for it.
    let _call = b.feedback_slot(FeedbackSlotKind::Call);
    match site {
        Site::Get(n) => {
            let k = b.string_constant(n);
            b.get_named_property(a, k, s);
        }
        Site::Set(n) => {
            let k = b.string_constant(n);
            b.ldar(x).set_named_property(a, k, s);
        }
        Site::Define(n) => {
            let k = b.string_constant(n);
            b.ldar(x).define_named_own_property(a, k, s);
        }
        Site::Global(n) => {
            let k = b.string_constant(n);
            b.lda_global(k, s);
        }
        Site::StoreGlobal(n) => {
            let k = b.string_constant(n);
            b.ldar(a).sta_global(k, s);
        }
    }
    b.return_();
    b.finish().unwrap()
}

/// The constructor `C` (function `SITES.len() + 1`): `function C() {}`.
fn constructor() -> FunctionCode {
    let mut b = FunctionBuilder::new(meta("C", 0, 0, 0, 0));
    b.lda_undefined().return_();
    b.finish().unwrap()
}

/// The script defines `f<i>` for each site and `C`.
fn program() -> VerifiedUnit {
    let mut s = Asm::script(0);
    for i in 0..=SITES.len() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        let name = if i == SITES.len() {
            "C".to_string()
        } else {
            format!("f{i}")
        };
        s.sta_global(&name);
    }
    s.b.lda_undefined().return_();
    let mut fns = vec![s.finish()];
    fns.extend(SITES.iter().enumerate().map(|(i, &site)| function(site, i)));
    fns.push(constructor());
    unit("", fns)
}

/// The function id of `C`.
const C_ID: u32 = SITES.len() as u32 + 1;

struct T {
    iso: Isolate,
    unit: VerifiedUnit,
}

impl T {
    fn new() -> T {
        let mut c = config(Dispatch::DEFAULT, false);
        c.feedback_threshold = 0;
        let mut iso = isolate_full(c);
        let unit = program();
        let u = unit.clone();
        iso.with_cx(|cx| cx.run_script(u, "facts.js").map(|_| ()))
            .unwrap();
        T { iso, unit }
    }

    fn rt(&self) -> &Runtime {
        &self.iso.rt
    }

    /// `f<i>(a, b)`, which must not throw.
    fn call(&mut self, i: usize, a: u64, b: u64) {
        let name = format!("f{i}");
        self.iso
            .with_cx(|cx| {
                let f = cx.global_binding(&name)?.expect("a test function");
                cx.call(
                    f,
                    Local::undefined(),
                    &[Local::from_bits(a), Local::from_bits(b)],
                )
                .map(|_| ())
            })
            .unwrap_or_else(|e| panic!("f{i} threw {}", e.message));
    }

    /// `C.prototype` (made now if it is still lazy).
    fn c_prototype(&mut self) -> u64 {
        self.iso
            .with_cx(|cx| {
                let c = cx.global_binding("C")?.expect("C");
                let c = c.as_object().expect("a function");
                Ok(cx.get(c, "prototype")?.bits())
            })
            .unwrap()
    }

    fn key(&self, name: &str) -> u64 {
        atoms::intern_str(self.rt(), name)
    }

    /// A new object with prototype `proto` and the data properties `props` {w, e, c}, added
    /// in that order.
    fn object(&self, proto: u64, props: &[&str]) -> u64 {
        let o = object::new_object(self.rt(), proto);
        for (i, &name) in props.iter().enumerate() {
            props::own_define(
                self.rt(),
                o,
                self.key(name),
                int32_bits(i as i32),
                attr::DEFAULT,
            );
        }
        o
    }

    fn object_prototype(&self) -> u64 {
        object::proto_of(object::new_plain_object(self.rt()))
    }

    fn export(&self) -> FactSnapshot {
        export_facts(&self.iso, &self.unit.content_hash()).expect("the unit is loaded")
    }

    /// The vector of `f<i>` (function `i + 1`).
    fn vector(&self, i: usize) -> Option<u64> {
        let units = self.rt().units.borrow();
        feedback::vector(&units.last().unwrap().infos[i + 1])
    }
}

/// The record of slot 0 of `f<i>`.
fn record(s: &FactSnapshot, i: usize) -> &SlotFacts {
    s.get(i as u32 + 1, 0)
        .unwrap_or_else(|| panic!("no record for f{i}"))
}

fn property(s: &FactSnapshot, i: usize) -> &PropertyFacts {
    match record(s, i) {
        SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => p,
        r => panic!("f{i}: not a property record: {r:?}"),
    }
}

fn bindings(s: &FactSnapshot, i: usize) -> BindingSet {
    match record(s, i) {
        SlotFacts::LoadGlobal(g) | SlotFacts::StoreGlobal(g) => {
            assert_eq!(g.values, TypeSet::EMPTY);
            g.bindings
        }
        r => panic!("f{i}: not a global record: {r:?}"),
    }
}

fn string_key(name: &str) -> KeyRef {
    KeyRef::String(ep9_bytecode::UnitString::new(name))
}

/// The description of an ordinary object made by `object::new_object`: 4 in-object slots
/// under Object.prototype, 8 under another prototype, no element yet (PACKED_INT32), each key
/// a field that holds one int32 value (FACT_SCHEMA 2: the shape's elements kind and the
/// fields' rep/const hints, M6-PLAN-D8).
fn plain(proto: ProtoRef, keys: &[&str]) -> ShapeDesc {
    ShapeDesc {
        proto,
        kind: ObjectKind::Ordinary,
        elements: Some(ElementsKind::PackedInt32),
        flags: ShapeFlags::EXTENSIBLE,
        capacity: if proto == object_proto() { 4 } else { 8 },
        keys: keys
            .iter()
            .map(|&k| {
                (
                    string_key(k),
                    PropAttrs::WRITABLE | PropAttrs::ENUMERABLE | PropAttrs::CONFIGURABLE,
                )
            })
            .collect(),
        fields: vec![
            FieldHint {
                rep: FieldRep::Int32,
                constant: true,
            };
            keys.len()
        ],
    }
}

fn object_proto() -> ProtoRef {
    ProtoRef::Intrinsic(IntrinsicId::ObjectPrototype as u32)
}

/// Runs the whole scenario once and returns the snapshot.
fn scenario() -> (T, FactSnapshot) {
    let mut t = T::new();
    let op = t.object_prototype();
    let p = t.c_prototype();
    props::own_define(t.rt(), p, t.key("x"), int32_bits(5), attr::DEFAULT);
    props::own_define(t.rt(), p, t.key("m"), int32_bits(6), attr::DEFAULT);

    // f0 `a.x`: an own field, a prototype field of C.prototype, a missing property: 3 cases.
    let own = t.object(op, &["w", "x"]);
    let inherits = t.object(p, &[]);
    let missing = t.object(op, &[]);
    for o in [own, inherits, missing] {
        t.call(0, o, UNDEFINED);
    }
    assert_eq!(ic::state(t.vector(0).unwrap(), 0), State::Polymorphic(3));

    // f1 `a.length`: an array (its length handler) and a string (KEY_STRING, left out).
    let arr = object::new_array(t.rt(), 0);
    let s = string::new_string(t.rt(), "abc");
    t.call(1, arr, UNDEFINED);
    t.call(1, s, UNDEFINED);
    assert_eq!(ic::state(t.vector(1).unwrap(), 0), State::Polymorphic(2));

    // f2 `a.m`: 5 shapes make it megamorphic.
    for props in [&["a"][..], &["b"], &["c"], &["d"], &["e"]] {
        let o = t.object(op, props);
        t.call(2, o, UNDEFINED);
    }
    assert_eq!(ic::state(t.vector(2).unwrap(), 0), State::Megamorphic);

    // f3 `a.y = b`: an add to {} then a store to the own field of {y}.
    let fresh = t.object(op, &[]);
    t.call(3, fresh, int32_bits(1));
    t.call(3, fresh, int32_bits(2));
    // f4 define `z`: an add.
    let fresh2 = t.object(op, &[]);
    t.call(4, fresh2, int32_bits(1));

    // f5 `print`: a data property of the global object. f6 `lx`: a lexical binding.
    t.call(5, UNDEFINED, UNDEFINED);
    let lexical = t.rt().root(Root::Lexical);
    props::own_define(t.rt(), lexical, t.key("lx"), int32_bits(7), attr::DEFAULT);
    t.call(6, UNDEFINED, UNDEFINED);
    // f7 `g = a`: the first store makes `g` (no case), the second records it.
    t.call(7, int32_bits(1), UNDEFINED);
    t.call(7, int32_bits(2), UNDEFINED);

    let snap = t.export();
    // Keep the objects (and so their shapes, which IC keys hold weakly) up to the export.
    std::hint::black_box((own, inherits, missing, arr, s, fresh, fresh2));
    (t, snap)
}

#[test]
fn each_handler_becomes_its_fact() {
    on_big_stack(|| {
        let (t, snap) = scenario();
        // `C` is a `function`, not a class: its `prototype` is writable, so the VM keeps no
        // proof that C.prototype is the initial one (Codex C-FX2) and it is `unknown`.
        let c_ref = FunctionRef::Js {
            unit: t.unit.content_hash(),
            function: C_ID,
        };
        let c_proto = plain(ProtoRef::Unknown, &[]);
        snap.records().for_each(|(_, r)| {
            r.for_each_function(&mut |f| assert_ne!(*f, c_ref));
        });

        // f0: own field (w, x: key 1), prototype field (C.prototype is {constructor, x, m}:
        // key 1, depth 1),
        // missing (Object.prototype, then null: depth 1).
        let p = property(&snap, 0);
        assert_eq!(p.state, IcState::Polymorphic);
        assert_eq!(p.values, TypeSet::EMPTY);
        let mut got: Vec<_> = p
            .cases
            .iter()
            .map(|c| ((*c.receiver).clone(), c.access.clone()))
            .collect();
        got.sort();
        let mut want = vec![
            (
                plain(object_proto(), &["w", "x"]),
                Access::OwnField { field: 1 },
            ),
            (c_proto.clone(), Access::ProtoField { depth: 1, field: 1 }),
            (plain(object_proto(), &[]), Access::Missing { depth: 1 }),
        ];
        want.sort();
        assert_eq!(got, want);

        // f1: the array case only; the string case has no shape description.
        let p = property(&snap, 1);
        assert_eq!(p.state, IcState::Polymorphic);
        assert_eq!(p.cases.len(), 1, "{:?}", p.cases);
        assert_eq!(p.cases[0].access, Access::ArrayLength);
        assert_eq!(p.cases[0].receiver.kind, ObjectKind::Array);
        assert_eq!(
            p.cases[0].receiver.proto,
            ProtoRef::Intrinsic(IntrinsicId::ArrayPrototype as u32)
        );

        // f2: megamorphic, no case.
        let p = property(&snap, 2);
        assert_eq!((p.state, p.cases.len()), (IcState::Megamorphic, 0));

        // f3: add from {} to {y}, then own field 0 of {y}.
        let p = property(&snap, 3);
        assert!(matches!(record(&snap, 3), SlotFacts::StoreNamed(_)));
        assert_eq!(p.state, IcState::Polymorphic);
        // FACT_SCHEMA 2: `y` took a second int32 (2 after 1), so its field is not const.
        let mut y = plain(object_proto(), &["y"]);
        y.fields[0].constant = false;
        let y = Arc::new(y);
        let mut want = vec![
            (
                plain(object_proto(), &[]),
                Access::AddField {
                    to: y.clone(),
                    field: 0,
                },
            ),
            ((*y).clone(), Access::OwnField { field: 0 }),
        ];
        want.sort();
        let got: Vec<_> = p
            .cases
            .iter()
            .map(|c| ((*c.receiver).clone(), c.access.clone()))
            .collect();
        assert_eq!(got, want);

        // f4: define `z`, an add.
        let p = property(&snap, 4);
        assert!(matches!(record(&snap, 4), SlotFacts::DefineNamedOwn(_)));
        assert_eq!(p.state, IcState::Monomorphic);
        assert!(
            matches!(&p.cases[0].access, Access::AddField { field: 0, to } if to.keys.len() == 1)
        );

        // Globals.
        assert_eq!(bindings(&snap, 5), BindingSet::DATA);
        assert_eq!(bindings(&snap, 6), BindingSet::LEXICAL);
        assert_eq!(bindings(&snap, 7), BindingSet::DATA);

        // f8 never ran: no vector, no record. The Call slot 1 of each `f<i>` has no site, so no
        // record. The script (function 0) has two records: its StoreGlobal slot, which the
        // `sta_global` sites of every name share (megamorphic: no binding), and its
        // CreateClosure slot (10 closures: many). Its other slots have no site.
        assert!(t.vector(8).is_none());
        assert!(snap.get(9, 0).is_none());
        for (key, facts) in snap.records() {
            if key.function == 0 {
                assert!(
                    matches!(
                        (key.slot, facts),
                        (4, SlotFacts::StoreGlobal(g)) if g.bindings.is_empty()
                    ) || matches!(
                        facts,
                        SlotFacts::CreateClosure(c) if c.closures == ClosureCount::Many
                    ),
                    "{key:?} {facts:?}"
                );
            } else {
                assert_eq!(key.slot, 0, "{key:?} {facts:?}");
            }
        }
        assert_eq!(snap.records().filter(|(k, _)| k.function == 0).count(), 2);

        // The snapshot is valid for its unit, and it encodes and decodes to itself.
        let (valid, dropped) = snap.clone().validate(&t.unit).expect("not stale");
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(valid, snap);
        let bytes = snap.encode();
        assert_eq!(FactSnapshot::decode(&bytes).unwrap(), snap);
    });
}

/// Two isolates that run the same program give the same bytes: the output holds no ShapeId,
/// address or closure identity.
#[test]
fn two_isolates_give_the_same_bytes() {
    on_big_stack(|| {
        let (_t1, a) = scenario();
        let (_t2, b) = scenario();
        assert!(!a.is_empty());
        assert_eq!(a.encode(), b.encode());
    });
}

/// A prototype change after a case was made (the epoch moves): the prototype-field and
/// missing cases miss from now on and are left out; the state stays.
#[test]
fn a_stale_prototype_case_is_left_out() {
    on_big_stack(|| {
        let (t, before) = scenario();
        assert_eq!(property(&before, 0).cases.len(), 3);
        let p = t.object_prototype();
        // A new property on Object.prototype bumps the epoch.
        props::own_define(t.rt(), p, t.key("fresh"), int32_bits(0), attr::DEFAULT);
        let after = t.export();
        let f0 = property(&after, 0);
        assert_eq!(f0.state, IcState::Polymorphic);
        assert_eq!(f0.cases.len(), 1);
        assert_eq!(f0.cases[0].access, Access::OwnField { field: 1 });
    });
}

/// The ShapeDesc keys (the first `count` descriptors) equal the keys of the parent chain from
/// the root shape: the root walk.
#[test]
fn shape_keys_are_the_root_walk() {
    on_big_stack(|| {
        let t = T::new();
        let o = t.object(t.object_prototype(), &["a", "b", "c", "d"]);
        let mut s = shape::shape_of(o);
        let mut walk = Vec::new();
        while shape::parent(s) != UNDEFINED {
            walk.push(string::to_rust_string(shape::key_at(
                s,
                shape::count(s) - 1,
            )));
            s = shape::parent(s);
        }
        walk.reverse();
        let s = shape::shape_of(o);
        let prefix: Vec<String> = (0..shape::count(s))
            .map(|i| string::to_rust_string(shape::key_at(s, i)))
            .collect();
        assert_eq!(walk, prefix);
        assert_eq!(prefix, ["a", "b", "c", "d"]);
        assert_eq!(object::proto_of(object::proto_of(o)), NULL);
    });
}

/// A `SLOW` global case for a data property under a null-prototype shape (`ic::global_case`
/// for a store to a `const`) is lexical only while its key is the lexical record's current
/// shape. After a new lexical binding the key is an old shape: the global object can have a
/// null prototype too, so the exporter cannot tell whose shape it was, and the case gives no
/// binding (Codex C-FX1). The miss path records no case for a store that throws, so the test
/// records the case that `ic::global_case` computes.
#[test]
fn an_old_slow_global_key_gives_no_binding() {
    on_big_stack(|| {
        let mut t = T::new();
        // f7 `g = a` gets a vector (the store makes the global `g`; no case yet).
        t.call(7, int32_bits(1), UNDEFINED);
        let v = t.vector(7).unwrap();
        assert_eq!(ic::state(v, 0), State::Uninitialized);
        // `const g` on the lexical record, {}.
        let lexical = t.rt().root(Root::Lexical);
        let g = t.key("g");
        props::own_define(t.rt(), lexical, g, int32_bits(1), 0);
        let case = ic::global_case(t.rt(), g, true).expect("a case");
        let ls = shape::shape_of(lexical);
        assert_eq!(case.key, ls);
        ic::record(t.rt(), v, 0, case);
        assert_eq!(ic::find(v, 0, ls), Some(ic::SLOW));
        assert_eq!(bindings(&t.export(), 7), BindingSet::LEXICAL);
        // `let h`: the record's shape moves on; the case keeps the old key.
        props::own_define(t.rt(), lexical, t.key("h"), int32_bits(2), attr::WRITABLE);
        assert_ne!(shape::shape_of(lexical), ls);
        assert_eq!(ic::find(v, 0, ls), Some(ic::SLOW));
        assert_eq!(ic::state(v, 0), State::Monomorphic);
        assert_eq!(bindings(&t.export(), 7), BindingSet::EMPTY);
        std::hint::black_box(ls);
    });
}

/// H12-Q2 (a) (Codex C-FX3): the exporter keys no table by a heap-cell Value (a shape or a
/// prototype). It describes each one again where a case names it.
#[test]
fn the_exporter_keys_no_table_by_a_cell() {
    let src = include_str!("../facts_export.rs");
    for bad in ["HashMap<u64", "HashSet<u64", "BTreeMap<u64", "BTreeSet<u64"] {
        assert!(!src.contains(bad), "facts_export.rs has a {bad} table");
    }
}

#[test]
fn an_unknown_unit_hash_is_an_error() {
    let t = T::new();
    assert!(export_facts(&t.iso, &[0xEE; 32]).is_err());
    assert!(export_facts(&t.iso, &t.unit.content_hash()).is_ok());
}

// ---- Call slots (F-R8, MC-w13-8; M4-P2 FACTS2) -------------------------------------------------

/// How a call-test function uses its one Call slot (slot 0) on its parameter `a`.
#[derive(Clone, Copy, Debug)]
enum CallUse {
    /// `a()` (0x64): `profile.rs` keys a FunctionInfo word and counts sampled calls.
    Call,
    /// `new a()` (0x69): `construct_ic.rs` keys the constructor (weak) and counts the
    /// constructs of the cold path.
    Construct,
    /// `a(); new a()` on the same slot.
    Both,
}

const CALL_USES: [CallUse; 3] = [CallUse::Call, CallUse::Construct, CallUse::Both];

/// `c<i>(a)` for each [`CALL_USES`] entry (functions 1-3), `g` (function 4: `function g()
/// {}`), all globals.
fn call_program() -> VerifiedUnit {
    let mut fns = Vec::new();
    let mut s = Asm::script(0);
    for i in 0..=CALL_USES.len() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        let name = if i == CALL_USES.len() {
            "g".to_string()
        } else {
            format!("c{i}")
        };
        s.sta_global(&name);
    }
    s.b.lda_undefined().return_();
    fns.push(s.finish());
    for (i, &how) in CALL_USES.iter().enumerate() {
        let mut b = FunctionBuilder::new(meta(&format!("c{i}"), 1, 0, 0, 0));
        let a = b.parameter(0);
        let slot = b.feedback_slot(FeedbackSlotKind::Call);
        if matches!(how, CallUse::Call | CallUse::Both) {
            b.call_undefined_receiver0(a, slot);
        }
        if matches!(how, CallUse::Construct | CallUse::Both) {
            b.ldar(a)
                .construct(a, ep9_bytecode::RegisterList::EMPTY, 0, slot);
        }
        b.lda_undefined().return_();
        fns.push(b.finish().unwrap());
    }
    let mut g = FunctionBuilder::new(meta("g", 0, 0, 0, 0));
    g.lda_undefined().return_();
    fns.push(g.finish().unwrap());
    unit("", fns)
}

/// The CallFacts of slot 0 of `c<i>` (function `i + 1`).
fn call_facts(s: &FactSnapshot, i: usize) -> &ep9_facts::CallFacts {
    match s.get(i as u32 + 1, 0) {
        Some(SlotFacts::Call(c)) => c,
        r => panic!("c{i}: not a Call record: {r:?}"),
    }
}

/// Runs `c<i>(callee)` for each `(i, callee global)` of `calls` on a fresh isolate (eager
/// feedback, every dispatch, with and without GC stress) and returns the snapshot, which
/// must give the same bytes in every run.
fn run_calls(calls: &'static [(usize, &'static str)]) -> (VerifiedUnit, FactSnapshot) {
    let mut first: Option<(VerifiedUnit, FactSnapshot)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (unit, snap) = on_big_stack(move || {
                let mut c = config(d, stress);
                c.feedback_threshold = 0;
                let mut iso = isolate_full(c);
                let unit = call_program();
                let u = unit.clone();
                iso.with_cx(|cx| cx.run_script(u, "calls.js").map(|_| ()))
                    .unwrap();
                for &(i, callee) in calls {
                    iso.with_cx(|cx| {
                        let f = cx.global_binding(&format!("c{i}"))?.expect("c<i>");
                        let a = cx.global_binding(callee)?.expect("the callee");
                        cx.call(f, Local::undefined(), &[a]).map(|_| ())
                    })
                    .unwrap_or_else(|e| panic!("c{i}({callee}) threw {}", e.message));
                }
                let snap = export_facts(&iso, &unit.content_hash()).unwrap();
                (unit, snap)
            });
            match &first {
                None => first = Some((unit, snap)),
                Some((_, s0)) => assert_eq!(snap.encode(), s0.encode(), "{d:?} stress={stress}"),
            }
        }
    }
    first.unwrap()
}

/// The `FunctionRef` of the VM builtin `name` (a constructor of `natives::VM_TABLE`).
fn vm_builtin(name: &str) -> FunctionRef {
    let m = &crate::natives::VM_TABLE.modules[0];
    let i = m
        .defs
        .iter()
        .position(|d| d.name == name && d.kind == crate::builtins::BuiltinKind::Constructor)
        .unwrap();
    FunctionRef::Builtin(m.id(i as u16).bits())
}

#[test]
fn a_call_key_becomes_a_function_ref_or_megamorphic() {
    // c0 `a()`: g three times (a unit function), c1 `new a()`: g twice; then the same with
    // `Object` (a table builtin) on fresh isolates, and `print` (a host function: no
    // FunctionRef, so megamorphic). Every call runs in the first sampling window of a fresh
    // isolate, so each is a sampled call.
    let (unit, snap) = run_calls(&[(0, "g"), (0, "g"), (0, "g"), (1, "g"), (1, "g")]);
    let g = FunctionRef::Js {
        unit: unit.content_hash(),
        function: CALL_USES.len() as u32 + 1,
    };
    let c0 = call_facts(&snap, 0);
    assert_eq!(c0.callees.state, IcState::Monomorphic);
    assert_eq!(c0.callees.targets, [g]);
    assert_eq!(c0.count, 3, "sampled calls");
    assert_eq!(c0.result, TypeSet::EMPTY);
    let c1 = call_facts(&snap, 1);
    assert_eq!(c1.callees.state, IcState::Monomorphic);
    assert_eq!(c1.callees.targets, [g]);
    // Constructs of the cold path: the first one is cold; the second may hit the handler.
    assert!((1..=2).contains(&c1.count), "{c1:?}");
    // c2 never ran: no vector, no record.
    assert!(snap.get(3, 0).is_none());

    let (_, snap) = run_calls(&[(0, "Object"), (1, "Object")]);
    let c0 = call_facts(&snap, 0);
    assert_eq!(c0.callees.state, IcState::Monomorphic);
    assert_eq!(c0.callees.targets, [vm_builtin("Object")]);
    assert_eq!(c0.count, 1);
    // `new Object()` runs the host constructor in `slow::call` before `construct_ic::miss`,
    // so today its site keeps no target (uninitialized). Whatever it keeps maps the same way.
    let c1 = call_facts(&snap, 1);
    assert!(
        c1.callees.state == IcState::Uninitialized || c1.callees.targets == [vm_builtin("Object")],
        "{c1:?}"
    );

    let (_, snap) = run_calls(&[(0, "print")]);
    let c0 = call_facts(&snap, 0);
    assert_eq!(c0.callees.state, IcState::Megamorphic);
    assert!(c0.callees.targets.is_empty());
    assert_eq!(c0.count, 1);

    // A second target makes the site megamorphic.
    let (_, snap) = run_calls(&[(0, "g"), (0, "Object")]);
    let c0 = call_facts(&snap, 0);
    assert_eq!(c0.callees.state, IcState::Megamorphic);
    assert_eq!(c0.count, 2);
}

#[test]
fn a_shared_call_and_construct_slot_is_megamorphic() {
    // c2 `a(); new a()` on one slot: the key half holds a FunctionInfo word or a constructor
    // cell, and the value half sampled calls or cold-path constructs, by which site wrote
    // last. The exporter cannot branch on one opcode: megamorphic, no target, count 0 (F-R7:
    // two units never add). The raw slot is not empty, so this is not "never ran".
    let (unit, snap) = run_calls(&[(2, "g"), (2, "g"), (2, "Object")]);
    let c2 = call_facts(&snap, 2);
    assert_eq!(c2.callees.state, IcState::Megamorphic);
    assert!(c2.callees.targets.is_empty());
    assert_eq!(c2.count, 0);
    // No word of the process is in the bytes: the record round-trips and validates.
    let (valid, dropped) = snap.clone().validate(&unit).expect("not stale");
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(valid, snap);
    assert_eq!(FactSnapshot::decode(&snap.encode()).unwrap(), snap);
}

#[test]
fn an_instanceof_slot_gets_a_record_once_it_left_its_start_state() {
    // `io(a, b)`: `a instanceof b` (InstanceOf slot 0). A slot in its start state (undefined,
    // int32 0) gives no record: a run that throws (a primitive rhs) records nothing
    // (`instance_ic.rs`, M4-P3 IC2). A run with a function rhs records it with an INSTANCEOF
    // handler cell, which the exporter must not read as an int32: TargetFacts.
    on_big_stack(|| {
        let mut s = Asm::script(0);
        for (i, name) in ["io", "g"].iter().enumerate() {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        s.b.lda_undefined().return_();
        let mut b = FunctionBuilder::new(meta("io", 2, 0, 0, 0));
        let (a, rhs) = (b.parameter(0), b.parameter(1));
        let slot = b.feedback_slot(FeedbackSlotKind::InstanceOf);
        b.ldar(rhs).test_instance_of(a, slot).return_();
        let mut g = FunctionBuilder::new(meta("g", 0, 0, 0, 0));
        g.lda_undefined().return_();
        let unit = unit(
            "",
            vec![s.finish(), b.finish().unwrap(), g.finish().unwrap()],
        );
        let mut c = config(Dispatch::DEFAULT, false);
        c.feedback_threshold = 0;
        let mut iso = isolate(c);
        let u = unit.clone();
        iso.with_cx(|cx| cx.run_script(u, "io.js").map(|_| ()))
            .unwrap();
        let export = |iso: &Isolate| export_facts(iso, &unit.content_hash()).unwrap();
        // `o instanceof 1` throws a TypeError and records nothing.
        let threw = iso.with_cx(|cx| {
            let io = cx.global_binding("io")?.expect("io");
            let o = cx.new_object();
            let one = Local::int32(1);
            cx.call(io, Local::undefined(), &[o.into(), one])?;
            Ok(())
        });
        assert!(threw.is_err());
        assert!(export(&iso).get(1, 0).is_none(), "start state: no record");
        iso.with_cx(|cx| {
            let io = cx.global_binding("io")?.expect("io");
            let g = cx.global_binding("g")?.expect("g");
            let o = cx.new_object();
            cx.call(io, Local::undefined(), &[o.into(), g])?;
            Ok(())
        })
        .unwrap();
        let want = FunctionRef::Js {
            unit: unit.content_hash(),
            function: 2,
        };
        match export(&iso).get(1, 0) {
            Some(SlotFacts::InstanceOf(t)) => {
                assert_eq!(t.state, IcState::Monomorphic);
                assert_eq!(t.targets, [want]);
            }
            r => panic!("{r:?}"),
        }
    });
}

// ---- FACT_SCHEMA 2 (M6-P0 SCHEMA6): the keyed hint and kinds set, the "ran" flag, the shape's
// elements kind, the arguments object kind ----

/// A script with keyed sites on an array `a = [1, , 3]` (PACKED -> HOLEY by the hole) and an
/// object `o`: (0) `a[0]` a present element (implicit), (1) `a[5]` out of bounds, (2) `a[1]`
/// a hole, (3) `o["1"]` an object receiver, (4) `a[0] = 1` a StoreKeyed site.
fn keyed_facts_program() -> (VerifiedUnit, [ep9_bytecode::FeedbackSlot; 5]) {
    use FeedbackSlotKind::{LoadKeyed, StoreKeyed};
    let mut s = Asm::script(0);
    let (a, o, key) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    s.b.create_empty_array_literal(s.lit).star(a);
    s.b.create_empty_object_literal().star(o);
    let st = s.b.feedback_slot(StoreKeyed);
    for (i, v) in [(0, 1), (2, 3)] {
        s.b.lda_smi(i)
            .star(key)
            .lda_smi(v)
            .set_keyed_property(a, key, st);
    }
    let present = s.b.feedback_slot(LoadKeyed);
    s.b.lda_zero().get_keyed_property(a, present);
    let oob = s.b.feedback_slot(LoadKeyed);
    s.b.lda_smi(5).get_keyed_property(a, oob);
    let hole = s.b.feedback_slot(LoadKeyed);
    s.b.lda_smi(1).get_keyed_property(a, hole);
    let obj = s.b.feedback_slot(LoadKeyed);
    s.lda_str("1");
    s.b.get_keyed_property(o, obj);
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), [present, oob, hole, obj, st])
}

fn keyed_of(s: &FactSnapshot, slot: ep9_bytecode::FeedbackSlot) -> ep9_facts::KeyedFacts {
    match s.get(0, slot.index() as u32) {
        Some(SlotFacts::LoadKeyed(k) | SlotFacts::StoreKeyed(k)) => k.clone(),
        r => panic!("slot {slot:?}: {r:?}"),
    }
}

/// The VM's keyed slot bits 4-9 become the record's `KeyedHint`, bits 10-25 its
/// `ElementsKindSet`; a LoadKeyed record also gets the implicit `INDEX` key and `ARRAY`
/// receiver. So an out-of-bounds, hole or object read no longer exports the record of a
/// present int32 read (MC-w13-9, MODES-D18).
#[test]
fn keyed_records_show_receivers_bounds_holes_and_kinds() {
    use ep9_facts::{ElementsKindSet, KeySet, KeyedHint};
    on_big_stack(|| {
        let (unit, [present, oob, hole, obj, st]) = keyed_facts_program();
        let mut c = config(Dispatch::DEFAULT, false);
        c.feedback_threshold = 0;
        let mut iso = isolate_full(c);
        let u = unit.clone();
        iso.with_cx(|cx| cx.run_script(u, "keyed.js").map(|_| ()))
            .unwrap();
        let snap = export_facts(&iso, &unit.content_hash()).unwrap();
        let k = keyed_of(&snap, present);
        assert_eq!((k.keys, k.hint), (KeySet::INDEX, KeyedHint::ARRAY), "{k:?}");
        let k = keyed_of(&snap, oob);
        assert_eq!(k.hint, KeyedHint::ARRAY | KeyedHint::OUT_OF_BOUNDS, "{k:?}");
        assert!(!k.kinds.is_empty(), "a miss records its kind: {k:?}");
        let k = keyed_of(&snap, hole);
        assert_eq!(k.hint, KeyedHint::ARRAY | KeyedHint::HOLE, "{k:?}");
        assert!(k.kinds.contains(ElementsKind::HoleyInt32), "{k:?}");
        let k = keyed_of(&snap, obj);
        assert_eq!(k.hint, KeyedHint::ARRAY | KeyedHint::OBJECT, "{k:?}");
        let k = keyed_of(&snap, st);
        assert!(k.hint.contains(KeyedHint::ARRAY), "{k:?}");
        // No T0 code writes the "ran" bit yet: every record claims nothing.
        for (_, _, m) in snap.records_with_meta() {
            assert_eq!(m, ep9_facts::RecordMeta::default());
        }

        // The slot bits as the exporter reads them: bit 26 (`profile::keyed::RAN`) is the
        // LoadKeyed record's RAN flag, and no other keyed kind's; the kinds-set bit of value
        // 15 (no named kind: an open set) is dropped.
        let v = {
            let units = iso.rt.units.borrow();
            feedback::vector(&units.last().unwrap().infos[0]).unwrap()
        };
        let top = crate::elements::kinds_set_bit(15);
        for slot in [present, st] {
            let i = slot.index() as u32;
            let bits = crate::frame::int32_of(feedback::value(v, i)) as u32;
            let more = bits | crate::profile::keyed::RAN | top;
            feedback::set_slot(&iso.rt, v, i, feedback::key(v, i), int32_bits(more as i32));
        }
        let again = export_facts(&iso, &unit.content_hash()).unwrap();
        let ran = |slot: ep9_bytecode::FeedbackSlot| {
            again
                .meta(0, slot.index() as u32)
                .unwrap()
                .flags
                .contains(ep9_facts::RecordFlags::RAN)
        };
        assert!(ran(present));
        assert!(!ran(st), "StoreKeyed has no ran bit (MODES-D17)");
        assert_eq!(
            keyed_of(&again, present).kinds,
            keyed_of(&snap, present).kinds
        );
        assert_eq!(
            keyed_of(&again, st).kinds,
            ElementsKindSet::from_bits_truncate(keyed_of(&snap, st).kinds.bits() | 1 << 15)
        );
    });
}

/// A shape's elements kind (FACT_SCHEMA 2, M6-PLAN-D8): two arrays with the same keys but
/// another elements kind export two shapes (schema 1 exported one, MC-w20-5 / FR-w20-1).
/// An arguments object exports as `ObjectKind::Arguments` (M6-PLAN-D5), no longer left out.
#[test]
fn shapes_name_their_elements_kind_and_arguments_objects_their_kind() {
    on_big_stack(|| {
        let mut t = T::new();
        let op = t.object_prototype();
        let ints = object::new_array(t.rt(), 0);
        let doubles = object::new_array(t.rt(), 0);
        for (arr, v) in [
            (ints, int32_bits(1)),
            (doubles, ep9_value::Value::double(1.5).to_bits()),
        ] {
            t.iso
                .with_cx(|cx| {
                    let a = Local::<crate::abi::JsValue>::from_bits(arr)
                        .as_object()
                        .expect("an array");
                    cx.set_index(a, 0, Local::<crate::abi::JsValue>::from_bits(v), true)
                        .map(|_| ())
                })
                .unwrap();
        }
        let args = crate::arguments::new_arguments_object(t.rt(), op);
        props::own_define(t.rt(), args, t.key("length"), int32_bits(0), attr::DEFAULT);
        for o in [ints, doubles, args] {
            t.call(1, o, UNDEFINED); // f1 `a.length`
        }
        let snap = t.export();
        let p = property(&snap, 1);
        let mut elements: Vec<_> = p
            .cases
            .iter()
            .filter(|c| c.receiver.kind == ObjectKind::Array)
            .map(|c| c.receiver.elements)
            .collect();
        elements.sort();
        assert_eq!(
            elements,
            [
                Some(ElementsKind::PackedInt32),
                Some(ElementsKind::PackedDouble)
            ],
            "{p:?}"
        );
        assert!(
            p.cases
                .iter()
                .any(|c| c.receiver.kind == ObjectKind::Arguments),
            "{p:?}"
        );
        std::hint::black_box((ints, doubles, args));
    });
}

// ---- MC-w25-5: the exported Access and `ops_of` agree for each handler kind ----------------------

/// The handler kind of a case as its stored word gives it: a smi handler kind (`ic::smi`, 1-6
/// and the representation-checked store kinds 9-14), an IC_HANDLER cell kind (`ic::hk`, 1-11),
/// or a megamorphic entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum HandlerKind {
    Smi(i32),
    Cell(i32),
    Mega,
}

fn handler_kind(h: u64) -> HandlerKind {
    if val(h).is_int32() {
        HandlerKind::Smi(int32_of(h) & smi::KIND_MASK)
    } else {
        HandlerKind::Cell(ic_ops::handler_parts(h).0)
    }
}

/// A getter or setter for the accessor cases: returns undefined.
fn nop<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> {
    Ok(Local::undefined())
}

/// Defines the accessor `name` {get, set, enumerable, configurable} of `o`. GC point: `get`
/// and `set` live in Rust locals.
fn define_accessor(rt: &Runtime, o: u64, name: &str, get: u64, set: u64) {
    let pair = accessor::new_pair(rt, get, set);
    let a = attr::ACCESSOR | attr::ENUMERABLE | attr::CONFIGURABLE;
    props::own_define(rt, o, atoms::intern_str(rt, name), pair, a);
}

/// True if key `field` of `d` is the string key `name`.
fn names(d: &ShapeDesc, field: u16, name: &str) -> bool {
    d.keys
        .get(usize::from(field))
        .is_some_and(|(k, _)| *k == string_key(name))
}

/// MC-w25-5: the exported access of one case (`None`: the exporter left the case out) agrees
/// with the case's `ops_of` list at a site of operation `op`, by kind. A typed access needs the
/// list of that access kind (its guard form and its action op); operands (`field`, `depth`, the
/// action's `Loc`, the store `Ty`) are checked by [`operands_agree`] (MC-P3-OPAGREE), and by
/// `each_handler_becomes_its_fact` and `tests::ic_ops` (`eval`). A `[Generic]` list allows only Generic
/// or no case; Generic comes only from a `[Generic]` list or from the accessor and
/// dictionary-holder kinds GETTER, SETTER and PROTO_DICT_LOAD, which export as Generic until a
/// typed writer exists (H16 "Declared, no writer"). The exporter leaves out a case whose key
/// is not a shape (KEY_STRING) and one whose handler is no named-site kind (cell kinds 4-8:
/// their list is `[Generic]`).
fn check_agree(
    what: &str,
    kind: HandlerKind,
    key: u64,
    op: Operation,
    access: Option<&Access>,
    receiver: Option<&ShapeDesc>,
    ops: &Ops,
) {
    let list = ops.as_slice();
    let generic = list == [Op::Generic];
    let guards = &list[..list.len() - 1];
    let checks_chain = guards
        .iter()
        .any(|g| matches!(g, Op::GuardProtoValidity(_)));
    let shape_guard = guards.first() == Some(&Op::GuardShape(Val::Receiver, Ref::CaseKey));
    let accessor_kind = matches!(
        kind,
        HandlerKind::Cell(hk::GETTER | hk::SETTER | hk::PROTO_DICT_LOAD)
    );
    let load = op == Operation::LoadNamed;
    let ok = match access {
        None => generic || key == KEY_STRING,
        Some(Access::Generic) => generic || accessor_kind,
        Some(_) if generic || accessor_kind => false,
        Some(Access::OwnField { field }) => {
            receiver.is_some_and(|r| names(r, *field, "x"))
                && shape_guard
                && guards[1..]
                    .iter()
                    .all(|g| !load && matches!(g, Op::GuardType(Val::Value, _)))
                && matches!(
                    (load, ops.action()),
                    (true, Op::LoadField(Val::Receiver, _))
                        | (false, Op::StoreField(Val::Receiver, _))
                )
        }
        Some(Access::ArrayLength) => load && ops.action() == Op::LoadArrayLength,
        Some(Access::ProtoField { .. }) => {
            load && shape_guard
                && checks_chain
                && matches!(ops.action(), Op::LoadHolderField(Ref::HandlerRef, _))
        }
        Some(Access::Missing { .. }) => {
            load && shape_guard && checks_chain && ops.action() == Op::LoadUndefined
        }
        Some(Access::AddField { to, field }) => {
            !load
                && names(to, *field, "x")
                && shape_guard
                && checks_chain == (op == Operation::StoreNamed)
                && matches!(
                    ops.action(),
                    Op::AddField(Val::Receiver, Ref::HandlerRef, _)
                )
        }
        // Getter and Setter have no writer (H16 "Declared, no writer"); Element is keyed only.
        Some(_) => false,
    };
    assert!(
        ok,
        "{what}: handler kind {kind:?} at {op:?}: the facts give {access:?}, ops_of gives {list:?}"
    );
}

/// MC-w25-5 (MODES-CHECK-w25; the SM b3e803f64871 class: a second reader of the cache
/// drifts): for each handler kind, made by T0 in a named slot (smi kinds 1-5 and 9-14, cell
/// kinds 1-3 and 9-11, a megamorphic entry) or put in one (cell kinds 4-8, which no named site
/// makes), the access `facts_export` gives and the list `ic_ops::ops_of` gives agree
/// ([`check_agree`]); GETTER, SETTER and PROTO_DICT_LOAD give Generic.
#[test]
fn each_handler_kind_has_an_access_that_agrees_with_ops_of() {
    on_big_stack(|| {
        use IcSite::{Define, Get, Set};
        const LOADS: usize = 9; // f0-f8
        const STORES: usize = 9; // f9-f17
        const DEFINE: usize = LOADS + STORES; // f18
        const PUT: usize = DEFINE + 1; // f19-f23: cell kinds 4-8
        const MEGA: usize = PUT + 5; // f24
        let mut sites = vec![Get("x"); LOADS];
        sites[2] = Get("length");
        sites[3] = Get("length");
        sites.extend([Set("x", false); STORES]);
        sites.push(Define("x"));
        sites.extend([Get("x"); 6]);
        let mut c = config(Dispatch::DEFAULT, false);
        c.feedback_threshold = 0;
        let mut t = IcT::new(c, &sites);
        let rt = t.rt();
        let op = object_prototype(rt);
        let num = |d: f64| Value::number(d).to_bits();
        // `x` after 12 fields: out of line. The tag names a shape family of its own, so one
        // family's representation does not widen another's.
        let wide = |rt: &Runtime, tag: &str, x: u64| {
            let mut ps: Vec<(String, u64)> =
                (0..12).map(|k| (format!("{tag}{k}"), int(k))).collect();
            ps.push(("x".into(), x));
            let ps: Vec<(&str, u64)> = ps.iter().map(|(n, v)| (n.as_str(), *v)).collect();
            object_with(rt, op, &ps)
        };
        let get = realm::new_host_function(rt, nop, "get", 0, false);
        let set = realm::new_host_function(rt, nop, "set", 1, false);
        // Loads: own field in object, out of line, array and string length, a dictionary
        // receiver (SLOW), a prototype field, a missing name, an own getter, a field of a
        // dictionary-mode prototype.
        let dict = object_with(rt, op, &[("w", int(0)), ("x", int(1))]);
        props::to_dictionary(rt, dict);
        let holder = object_with(rt, op, &[("x", int(2))]);
        let dict_holder = object_with(rt, op, &[("x", int(3)), ("pad", int(0))]);
        let getter = object::new_object(rt, op);
        define_accessor(rt, getter, "x", get, UNDEFINED);
        let loads = [
            object_with(rt, op, &[("x", int(1))]),
            wide(rt, "p", int(1)),
            object::new_array(rt, 0),
            string::new_string(rt, "abc"),
            dict,
            object_with(rt, holder, &[("b", int(0))]),
            object_with(rt, op, &[("z", int(0))]),
            getter,
            object_with(rt, dict_holder, &[("b", int(0))]),
        ];
        props::to_dictionary(rt, dict_holder);
        // Stores of `x`: an add, then int32, double and heap fields in object and out of line
        // (the checked kinds 9-14 on the second store of one representation), an own setter
        // and a read-only field (SLOW).
        let setter = object::new_object(rt, op);
        define_accessor(rt, setter, "x", UNDEFINED, set);
        let read_only = object::new_object(rt, op);
        let ro = attr::DEFAULT & !attr::WRITABLE;
        props::own_define(rt, read_only, atoms::intern_str(rt, "x"), int(1), ro);
        let heap = |rt: &Runtime| object::new_plain_object(rt);
        let stores: [(u64, u64); STORES] = [
            (object::new_plain_object(rt), int(1)),
            (object_with(rt, op, &[("i", int(0)), ("x", int(1))]), int(2)),
            (wide(rt, "i", int(1)), int(2)),
            (
                object_with(rt, op, &[("d", int(0)), ("x", num(0.5))]),
                num(1.5),
            ),
            (wide(rt, "d", num(0.5)), num(1.5)),
            (
                object_with(rt, op, &[("h", int(0)), ("x", heap(rt))]),
                heap(rt),
            ),
            (wide(rt, "h", heap(rt)), heap(rt)),
            (setter, int(1)),
            (read_only, int(2)),
        ];
        let define = object::new_plain_object(rt);
        let mut keep: Vec<u64> = loads.to_vec();
        keep.extend(stores.iter().flat_map(|&(o, v)| [o, v]));
        keep.extend([define, holder, dict_holder, get, set]);
        let handles = t.keep_all(&keep);
        for (i, &o) in loads.iter().enumerate() {
            t.ok(i, o, UNDEFINED);
        }
        for (i, &(o, v)) in stores.iter().enumerate() {
            t.ok(LOADS + i, o, v);
        }
        t.ok(DEFINE, define, int(1));
        // Cell kinds 4-8 put in a named load slot, keyed on the shape of `{x}`: no named site
        // makes them, so the exporter has no case for them and `ops_of` gives `[Generic]`.
        let plain_x = object_with(t.rt(), op, &[("x", int(1))]);
        keep.push(plain_x);
        let more = t.keep_all(&[plain_x]);
        for (j, k) in (hk::GLOBAL..=hk::ALLOC_SITE).enumerate() {
            t.ok(PUT + j, plain_x, UNDEFINED);
            let rt = t.rt();
            let h = rt.alloc(kinds::IC_HANDLER, io::SIZE);
            object::init(h, io::KIND, int32_bits(k));
            object::init(h, io::CELL, int32_bits(0));
            object::init(h, io::SHAPE, UNDEFINED);
            object::init(h, io::FIELD, int32_bits(0));
            let v = t.vector(PUT + j);
            feedback::set_slot(
                rt,
                v,
                SLOT,
                shape::shape_of(plain_x),
                h.to_value().to_bits(),
            );
        }
        // Megamorphic: 5 shapes.
        for n in 0..5 {
            let o = object_with(t.rt(), op, &[(&format!("m{n}"), int(0)), ("x", int(n))]);
            t.ok(MEGA, o, UNDEFINED);
        }
        let hash = t.iso.rt.units.borrow().last().unwrap().unit.content_hash();
        let snap = export_facts(&t.iso, &hash).expect("the unit is loaded");

        let mut seen = Vec::new();
        let (mut typed, mut controls) = (0, 0);
        for (i, site) in sites.iter().enumerate() {
            let op = match site {
                Get(_) => Operation::LoadNamed,
                Set(..) => Operation::StoreNamed,
                _ => Operation::DefineNamed,
            };
            let facts = property(&snap, i);
            let cs = ic_ops::cases(t.vector(i), SLOT, op);
            if i == MEGA {
                assert_eq!(cs, [ic_ops::Case::Mega], "f{i}");
                assert_eq!(ops_of_list(&cs[0], op), [Op::Generic]);
                assert_eq!((facts.state, facts.cases.len()), (IcState::Megamorphic, 0));
                seen.push(HandlerKind::Mega);
                continue;
            }
            assert_eq!(facts.state, IcState::Monomorphic, "f{i}: {facts:?}");
            let [ic_ops::Case::Handler { key, handler }] = cs[..] else {
                panic!("f{i}: {cs:?}");
            };
            let kind = handler_kind(handler);
            assert!(facts.cases.len() <= 1, "f{i}: {facts:?}");
            let case = facts.cases.first();
            let ops = ic_ops::ops_of(&cs[0], op);
            check_agree(
                &format!("f{i}"),
                kind,
                key,
                op,
                case.map(|c| &c.access),
                case.map(|c| &*c.receiver),
                &ops,
            );
            // MC-P3-OPAGREE: the operands agree too, and each changed operand is refused (the
            // negative control).
            if let Some(c) = case {
                let rt = t.rt();
                let o = if i < LOADS {
                    loads[i]
                } else if i < DEFINE {
                    let (o, v) = stores[i - LOADS];
                    if !matches!(c.access, Access::Generic) {
                        let x = atoms::intern_str(rt, "x");
                        assert_eq!(props::own_get(o, x).map(|p| p.0), Some(v), "f{i}: stored");
                    }
                    o
                } else {
                    define
                };
                let list = ops.as_slice();
                operands_agree(rt, o, handler, &c.access, &c.receiver, list)
                    .unwrap_or_else(|e| panic!("f{i}: {e}"));
                for bad in changed_accesses(&c.access) {
                    assert!(
                        operands_agree(rt, o, handler, &bad, &c.receiver, list).is_err(),
                        "f{i}: {bad:?} agrees with {list:?}"
                    );
                    controls += 1;
                }
                let operands = !matches!(c.access, Access::Generic | Access::ArrayLength);
                for bad in changed_lists(list).into_iter().filter(|_| operands) {
                    assert!(
                        operands_agree(rt, o, handler, &c.access, &c.receiver, &bad).is_err(),
                        "f{i}: {:?} agrees with {bad:?}",
                        c.access
                    );
                    controls += 1;
                }
                typed += usize::from(!matches!(c.access, Access::Generic));
            }
            seen.push(kind);
        }
        // Every kind is in the test.
        seen.sort();
        seen.dedup();
        let mut want: Vec<HandlerKind> = [1, 2, 3, 4, 5, 9, 10, 11, 12, 13, 14]
            .map(HandlerKind::Smi)
            .to_vec();
        want.extend((hk::PROTO_LOAD..=hk::PROTO_DICT_LOAD).map(HandlerKind::Cell));
        want.push(HandlerKind::Mega);
        want.sort();
        assert_eq!(seen, want);
        // The typed cases: 2 own loads, length, prototype field, missing, add, 6 checked
        // stores and define (13). The controls: 13 changed accesses (the prototype field has
        // 2), 11 lists with a lower Loc and 6 with another store Ty.
        assert_eq!(
            (typed, controls),
            (13, 30),
            "MC-P3-OPAGREE cases and controls"
        );
        // The accessor cases are in this snapshot; no declared field has a value (MC-w24-5).
        assert_no_declared_field(&snap);
        t.release_all(handles);
        t.release_all(more);
        std::hint::black_box(keep);
    });
}

fn ops_of_list(case: &ic_ops::Case, op: Operation) -> Vec<Op> {
    ic_ops::ops_of(case, op).as_slice().to_vec()
}

// ---- MC-P3-OPAGREE: the operands of the exported access and of `ops_of` agree -------------------

/// The word that the field `loc` names in the object `o` (in the object cell or its SLOTS
/// cell), read as T0 reads it.
fn read_at(o: u64, loc: Loc) -> u64 {
    if loc.in_object() {
        object::rd(object::cell(o), loc.offset())
    } else {
        object::rd(
            object::cell(object::rd(object::cell(o), obj::SLOTS)),
            loc.offset(),
        )
    }
}

/// MC-P3-OPAGREE (Codex M6-P3 r2 X1): the operands of the exported access of one case agree
/// with the operands of its `ops_of` list `list`. The check reads the live objects, not the
/// exporter's own index map (`facts_export::key_index`): the property is `x` at every site.
/// - `OwnField { field }`: key `field` of the receiver `o`'s shape is `x`, and the action's
///   `Loc` reads the value of `x` (`props::own_get`). A checked store's `GuardType` is the
///   field's exported representation (int32: Int32, double: Number, heap: Heap).
/// - `ProtoField { depth, field }`: `depth` prototype steps from `o` reach the holder at the
///   handler's +24, key `field` of the holder's shape is `x`, and the `Loc` reads its `x`.
/// - `Missing { depth }`: `o` has `depth` prototypes up to null.
/// - `AddField { field, .. }`: key `field` of `o`'s shape after the add is `x`, and the `Loc`
///   reads it.
/// - `ArrayLength` and `Generic` have no operands.
fn operands_agree(
    rt: &Runtime,
    o: u64,
    handler: u64,
    access: &Access,
    receiver: &ShapeDesc,
    list: &[Op],
) -> Result<(), String> {
    let x = atoms::intern_str(rt, "x");
    let key_is_x = |obj: u64, field: u16| {
        let s = shape::shape_of(obj);
        usize::from(field) < shape::count(s) && shape::key_at(s, usize::from(field)) == x
    };
    let reads_x =
        |obj: u64, loc: Loc| props::own_get(obj, x).map(|p| p.0) == Some(read_at(obj, loc));
    let ty = list.iter().find_map(|op| match op {
        Op::GuardType(Val::Value, t) => Some(*t),
        _ => None,
    });
    let ok = match (access, list.last()) {
        (
            Access::OwnField { field },
            Some(&(Op::LoadField(Val::Receiver, loc) | Op::StoreField(Val::Receiver, loc))),
        ) => {
            let rep = receiver.fields.get(usize::from(*field)).map(|h| h.rep);
            let ty_ok = match ty {
                None => true,
                Some(Ty::Int32) => rep == Some(FieldRep::Int32),
                Some(Ty::Number) => rep == Some(FieldRep::Double),
                Some(Ty::Heap) => rep == Some(FieldRep::Heap),
            };
            key_is_x(o, *field) && reads_x(o, loc) && ty_ok
        }
        (Access::ProtoField { depth, field }, Some(&Op::LoadHolderField(Ref::HandlerRef, loc))) => {
            let holder = ic_ops::handler_parts(handler).2;
            let mut p = o;
            for _ in 0..*depth {
                if p != NULL {
                    p = object::proto_of(p);
                }
            }
            p == holder && key_is_x(holder, *field) && reads_x(holder, loc)
        }
        (Access::Missing { depth }, Some(Op::LoadUndefined)) => {
            let (mut n, mut p) = (0u32, object::proto_of(o));
            while p != NULL {
                n += 1;
                p = object::proto_of(p);
            }
            n == u32::from(*depth)
        }
        (
            Access::AddField { field, .. },
            Some(&Op::AddField(Val::Receiver, Ref::HandlerRef, loc)),
        ) => key_is_x(o, *field) && reads_x(o, loc),
        (Access::ArrayLength | Access::Generic, Some(_)) => true,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "the operands of {access:?} do not agree with {list:?}"
        ))
    }
}

/// The negative control of [`operands_agree`]: `access` with one operand changed (a field
/// index or a depth one more).
fn changed_accesses(access: &Access) -> Vec<Access> {
    match access {
        Access::OwnField { field } => vec![Access::OwnField { field: field + 1 }],
        Access::ProtoField { depth, field } => vec![
            Access::ProtoField {
                depth: depth + 1,
                field: *field,
            },
            Access::ProtoField {
                depth: *depth,
                field: field + 1,
            },
        ],
        Access::Missing { depth } => vec![Access::Missing { depth: depth + 1 }],
        Access::AddField { to, field } => vec![Access::AddField {
            to: to.clone(),
            field: field + 1,
        }],
        _ => Vec::new(),
    }
}

/// The negative control of [`operands_agree`]: `list` with the action's `Loc` one word lower
/// (still inside the cell: a field is never the first word), or with another store `Ty`.
fn changed_lists(list: &[Op]) -> Vec<Vec<Op>> {
    let lower = |l: Loc| Loc(l.0 - (8 << smi::SHIFT));
    let mut out = Vec::new();
    let last = list.len() - 1;
    let moved = match list[last] {
        Op::LoadField(v, l) => Some(Op::LoadField(v, lower(l))),
        Op::StoreField(v, l) => Some(Op::StoreField(v, lower(l))),
        Op::LoadHolderField(r, l) => Some(Op::LoadHolderField(r, lower(l))),
        Op::AddField(v, r, l) => Some(Op::AddField(v, r, lower(l))),
        _ => None,
    };
    if let Some(op) = moved {
        let mut l = list.to_vec();
        l[last] = op;
        out.push(l);
    }
    for (i, op) in list.iter().enumerate() {
        if let Op::GuardType(v, t) = *op {
            let other = match t {
                Ty::Int32 => Ty::Heap,
                Ty::Number => Ty::Int32,
                Ty::Heap => Ty::Number,
            };
            let mut l = list.to_vec();
            l[i] = Op::GuardType(v, other);
            out.push(l);
        }
    }
    out
}

// ---- MC-w24-5: the fields that H16 declares with no writer ---------------------------------------

/// H16 "Declared, no writer" (MC-w24-5; MC-w25-7, M6-ORCH-26; Codex r1 D4): the FACT_SCHEMA 2
/// fields that no ep9_vm code writes at M1 exit. Each entry: the field; the text of that H16
/// paragraph that names it; the code a writer needs in any `src` file outside `src/tests`
/// (needles); the identifier a writer needs in a file that uses `ep9_facts` (with a `use`,
/// `EXITED` alone is enough); the module that writes the field's slot source, where that
/// identifier is a needle too; and, for a `TypeSet` field, its name: in a file that uses
/// `ep9_facts`, a line that sets that name to a value other than `TypeSet::EMPTY` writes it.
/// An entry with no needle, word or name has no code scan (its writer is a value that other
/// facts share, such as an `IcState` or an elements kind); the snapshots check it, and so does
/// the corpus test `every_field_that_claims_a_writer_is_seen` (`ep9_builtins`
/// `tests/facts_export.rs`), whose list is this list.
struct NoWriter {
    field: &'static str,
    named: &'static str,
    needles: &'static [&'static str],
    word: Option<&'static str>,
    source: Option<&'static str>,
    types: Option<&'static str>,
}

const NO_WRITER: [NoWriter; 17] = [
    NoWriter {
        field: "RecordFlags::RAN",
        named: "`RecordFlags::RAN`",
        needles: &["RecordFlags::RAN", "keyed::RAN"],
        word: Some("RAN"),
        source: Some("profile.rs"),
        types: None,
    },
    NoWriter {
        field: "RecordFlags::EXITED",
        named: "`RecordFlags::EXITED`",
        needles: &["RecordFlags::EXITED"],
        word: Some("EXITED"),
        source: None,
        types: None,
    },
    NoWriter {
        field: "ObjectKind::TypedArray",
        named: "`ObjectKind::TypedArray`",
        needles: &["ObjectKind::TypedArray"],
        word: Some("TypedArray"),
        source: None,
        types: None,
    },
    NoWriter {
        field: "Access::Getter",
        named: "the Access tags getter and setter",
        needles: &["Access::Getter"],
        word: Some("Getter"),
        source: None,
        types: None,
    },
    NoWriter {
        field: "Access::Setter",
        named: "the Access tags getter and setter",
        needles: &["Access::Setter"],
        word: Some("Setter"),
        source: None,
        types: None,
    },
    NoWriter {
        field: "Access::StringLength",
        named: "`Access::StringLength`",
        needles: &["Access::StringLength"],
        word: Some("StringLength"),
        source: None,
        types: None,
    },
    NoWriter {
        field: "Access::Element",
        named: "`Access::Element`",
        needles: &["Access::Element"],
        word: Some("Element"),
        source: None,
        types: None,
    },
    NoWriter {
        field: "PropertyFacts.values",
        named: "`PropertyFacts.values`",
        needles: &[],
        word: None,
        source: None,
        types: Some("values"),
    },
    NoWriter {
        field: "GlobalFacts.values",
        named: "`GlobalFacts.values`",
        needles: &[],
        word: None,
        source: None,
        types: Some("values"),
    },
    NoWriter {
        field: "CallFacts.result",
        named: "`CallFacts.result`",
        needles: &[],
        word: None,
        source: None,
        types: Some("result"),
    },
    NoWriter {
        field: "BindingSet::MISSING",
        named: "`BindingSet::MISSING`",
        needles: &["BindingSet::MISSING"],
        word: None,
        source: None,
        types: None,
    },
    NoWriter {
        field: "ForInHint::KEYS",
        named: "the `ForInHint` bits `enum-cache-keys` and `enum-cache-keys-and-indices`",
        // Also matches `ForInHint::KEYS_AND_INDICES`.
        needles: &["ForInHint::KEYS"],
        word: None,
        source: None,
        types: None,
    },
    NoWriter {
        field: "ForInHint::KEYS_AND_INDICES",
        named: "the `ForInHint` bits `enum-cache-keys` and `enum-cache-keys-and-indices`",
        needles: &["ForInHint::KEYS_AND_INDICES"],
        word: None,
        source: None,
        types: None,
    },
    NoWriter {
        field: "CallFacts.callees.state Polymorphic",
        named: "the `polymorphic` IC state of a Call or keyed record",
        needles: &[],
        word: None,
        source: None,
        types: None,
    },
    NoWriter {
        field: "KeyedFacts.property.state Polymorphic",
        named: "the `polymorphic` IC state of a Call or keyed record",
        needles: &[],
        word: None,
        source: None,
        types: None,
    },
    NoWriter {
        field: "FieldRep::Unset",
        named: "`FieldRep::Unset`",
        needles: &["FieldRep::Unset", "rep::NONE"],
        word: None,
        source: None,
        types: None,
    },
    NoWriter {
        field: "LiteralFacts.elements Dictionary",
        named: "`LiteralFacts.elements` `dictionary`",
        needles: &[],
        word: None,
        source: None,
        types: None,
    },
];

/// The code lines (file, trimmed text) that hold a needle today and write no field: the
/// exporter's read of the LoadKeyed slot bit 26, which no T0 code sets (H16 "Declared, no
/// writer"), the definition of that bit, a fuse name, and the representation join, which
/// gives `NONE` only for two `NONE` inputs.
const NOT_WRITERS: [(&str, &str); 5] = [
    (
        "facts_export.rs",
        "let flags = if ran && bits & crate::profile::keyed::RAN != 0 {",
    ),
    ("facts_export.rs", "RecordFlags::RAN"),
    ("profile.rs", "pub const RAN: u32 = 1 << 26;"),
    ("fuse.rs", "\"TypedArray species\","),
    ("field_rep.rs", "(rep::NONE, x) | (x, rep::NONE) => x,"),
];

/// True if `code` has `word` with no identifier character on either side.
fn has_word(code: &str, word: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    code.match_indices(word)
        .any(|(i, _)| !code[..i].ends_with(ident) && !code[i + word.len()..].starts_with(ident))
}

/// True if `code` sets the `TypeSet` field `name` to a value other than `TypeSet::EMPTY`: a
/// field init `name: v` or an assignment `.name = v` / `.name |= v`.
fn sets_types(code: &str, name: &str) -> bool {
    let init = code
        .strip_prefix(name)
        .and_then(|r| r.strip_prefix(':'))
        .is_some_and(|v| v.trim() != "TypeSet::EMPTY,");
    let assign = [" =", " |="]
        .iter()
        .any(|op| code.contains(&format!(".{name}{op}")));
    init || assign
}

/// The H16 "Declared, no writer" items that a record holds, as [`NO_WRITER`] names them. The
/// matches have no `_` arm, and each bit list is read in full: a new `SlotFacts` kind,
/// `Access` tag, `ObjectKind`, `RecordFlags` bit or `BindingSet` bit fails here (to compile,
/// or with its name) until it is sorted as written or as declared with no writer (Codex r1
/// D4). A keyed record with a case is `Access::Element`: no T0 code writes a keyed case
/// (MC-w15-1). The other items of [`NO_WRITER`] are read where they can be.
fn declared_items(facts: &SlotFacts, flags: ep9_facts::RecordFlags, out: &mut Vec<&'static str>) {
    use ep9_facts::RecordFlags;
    for &(bit, name) in RecordFlags::BITS {
        let field = match name {
            "exited" => "RecordFlags::EXITED",
            "ran" => "RecordFlags::RAN",
            _ => panic!("RecordFlags bit {name:?} is not sorted (H16 \"Declared, no writer\")"),
        };
        if flags.contains(bit) {
            out.push(field);
        }
    }
    fn shape(s: &ShapeDesc, out: &mut Vec<&'static str>) {
        if s.fields.iter().any(|f| f.rep == FieldRep::Unset) {
            out.push("FieldRep::Unset");
        }
        match s.kind {
            ObjectKind::Ordinary
            | ObjectKind::Array
            | ObjectKind::Function
            | ObjectKind::Error
            | ObjectKind::Iterator
            | ObjectKind::PrimitiveWrapper
            | ObjectKind::BoundFunction
            | ObjectKind::Arguments => {}
            ObjectKind::TypedArray => out.push("ObjectKind::TypedArray"),
        }
    }
    fn property(p: &PropertyFacts, out: &mut Vec<&'static str>) {
        if p.values != TypeSet::EMPTY {
            out.push("PropertyFacts.values");
        }
        for c in &p.cases {
            shape(&c.receiver, out);
            match &c.access {
                Access::OwnField { .. }
                | Access::ProtoField { .. }
                | Access::Missing { .. }
                | Access::ArrayLength
                | Access::Generic => {}
                Access::AddField { to, .. } => shape(to, out),
                Access::Getter { .. } => out.push("Access::Getter"),
                Access::Setter { .. } => out.push("Access::Setter"),
                Access::StringLength => out.push("Access::StringLength"),
                Access::Element { .. } => out.push("Access::Element"),
            }
        }
    }
    fn keyed(k: &ep9_facts::KeyedFacts, out: &mut Vec<&'static str>) {
        if k.property.state == IcState::Polymorphic {
            out.push("KeyedFacts.property.state Polymorphic");
        }
        if !k.property.cases.is_empty() {
            out.push("Access::Element");
        }
        property(&k.property, out);
    }
    fn global(g: &ep9_facts::GlobalFacts, out: &mut Vec<&'static str>) {
        if g.values != TypeSet::EMPTY {
            out.push("GlobalFacts.values");
        }
        for &(bit, name) in BindingSet::BITS {
            match name {
                "lexical" | "data-property" | "accessor-property" => {}
                "missing" => {
                    if g.bindings.contains(bit) {
                        out.push("BindingSet::MISSING");
                    }
                }
                _ => panic!("BindingSet bit {name:?} is not sorted (H16 \"Declared, no writer\")"),
            }
        }
    }
    match facts {
        SlotFacts::Call(c) => {
            if c.result != TypeSet::EMPTY {
                out.push("CallFacts.result");
            }
            if c.callees.state == IcState::Polymorphic {
                out.push("CallFacts.callees.state Polymorphic");
            }
        }
        SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
            property(p, out)
        }
        SlotFacts::LoadKeyed(k)
        | SlotFacts::StoreKeyed(k)
        | SlotFacts::DefineKeyedOwn(k)
        | SlotFacts::HasKeyed(k)
        | SlotFacts::StoreInArrayLiteral(k) => keyed(k, out),
        SlotFacts::LoadGlobal(g)
        | SlotFacts::LoadGlobalInsideTypeof(g)
        | SlotFacts::StoreGlobal(g) => global(g, out),
        SlotFacts::Literal(l) => {
            if let Some(s) = &l.shape {
                shape(s, out);
            }
            if l.elements == Some(ElementsKind::Dictionary) {
                out.push("LiteralFacts.elements Dictionary");
            }
        }
        SlotFacts::CloneObject(c) => {
            for case in &c.cases {
                shape(&case.source, out);
                shape(&case.result, out);
            }
        }
        // The BIGINT bits have their writers since spec-temporal B3 (`profile.rs`).
        SlotFacts::BinaryOp(_) | SlotFacts::CompareOp(_) | SlotFacts::TypeOf(_) => {}
        SlotFacts::ForIn(h) => {
            if h.contains(ep9_facts::ForInHint::KEYS) {
                out.push("ForInHint::KEYS");
            }
            if h.contains(ep9_facts::ForInHint::KEYS_AND_INDICES) {
                out.push("ForInHint::KEYS_AND_INDICES");
            }
        }
        SlotFacts::InstanceOf(_) | SlotFacts::JumpLoop(_) | SlotFacts::CreateClosure(_) => {}
    }
}

/// No record of `snap` has a field of [`NO_WRITER`]. Returns the number of records read.
fn assert_no_declared_field(snap: &FactSnapshot) -> usize {
    let mut n = 0;
    for (key, facts, meta) in snap.records_with_meta() {
        let mut found = Vec::new();
        declared_items(facts, meta.flags, &mut found);
        for f in &found {
            assert!(
                NO_WRITER.iter().any(|w| w.field == *f),
                "{f} is not in NO_WRITER"
            );
        }
        assert!(found.is_empty(), "{key:?}: {found:?} in {facts:?}");
        n += 1;
    }
    n
}

/// MC-w24-5, the negative half: each field of H16 "Declared, no writer" has no writer in
/// ep9_vm. A writer fails this test with the field's name and the line; then the field leaves
/// [`NO_WRITER`] and the H16 paragraph (an ADR change). The H16 paragraph names every field of
/// the list, and snapshots of the T0 cases ([`scenario`]), the call sites ([`run_calls`]) and
/// the keyed sites ([`keyed_facts_program`]) hold none of them. The positive half (each other
/// field is seen on a JS corpus) is `every_field_that_claims_a_writer_is_seen` (`ep9_builtins`
/// `tests/facts_export.rs`).
#[test]
fn the_fields_declared_with_no_writer_have_none() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let h16 =
        std::fs::read_to_string(root.join("../../docs/adr/H16-ics-and-feedback.md")).expect("H16");
    let para = h16
        .lines()
        .find(|l| l.starts_with("**Declared, no writer"))
        .expect("the H16 paragraph \"Declared, no writer\"");
    for w in &NO_WRITER {
        assert!(
            para.contains(w.named),
            "H16 \"Declared, no writer\" does not name {}",
            w.field
        );
    }

    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "tests") {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let src = root.join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    let mut allowed: Vec<(&str, &str)> = NOT_WRITERS.to_vec();
    let mut writers = Vec::new();
    for f in &files {
        let rel = f
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(f).unwrap();
        let facts_user = text.contains("ep9_facts");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("").trim();
            for w in &NO_WRITER {
                let by_word = w.word.is_some_and(|word| {
                    (facts_user || w.source == Some(rel.as_str())) && has_word(code, word)
                });
                let by_types = w.types.is_some_and(|t| facts_user && sets_types(code, t));
                let hit = w.needles.iter().any(|x| code.contains(x)) || by_word || by_types;
                if !hit {
                    continue;
                }
                if let Some(at) = allowed.iter().position(|&(af, at)| af == rel && at == code) {
                    allowed.remove(at);
                } else {
                    writers.push(format!("{}: src/{rel}:{} `{code}`", w.field, n + 1));
                }
            }
        }
    }
    assert!(
        writers.is_empty(),
        "H16 \"Declared, no writer\" fields with code that may write them (MC-w24-5): {writers:#?}"
    );
    assert!(allowed.is_empty(), "lines that are gone: {allowed:?}");

    // Each snapshot has records, and none holds a declared field.
    let snap = on_big_stack(|| scenario().1);
    assert!(assert_no_declared_field(&snap) > 0, "scenario: no record");
    let (_, snap) = run_calls(&[(0, "g"), (1, "g")]);
    assert!(assert_no_declared_field(&snap) > 0, "calls: no record");
    let snap = on_big_stack(|| {
        let (unit, _) = keyed_facts_program();
        let mut c = config(Dispatch::DEFAULT, false);
        c.feedback_threshold = 0;
        let mut iso = isolate_full(c);
        let u = unit.clone();
        iso.with_cx(|cx| cx.run_script(u, "keyed.js").map(|_| ()))
            .unwrap();
        export_facts(&iso, &unit.content_hash()).unwrap()
    });
    assert!(assert_no_declared_field(&snap) > 0, "keyed: no record");
}

// ---- MC-P3-KEYEDKIND: per-slot-kind evidence for the 5 keyed kinds -----------------------------

/// One site of each keyed slot kind on fresh receivers: (0) StoreKeyed `a[0] = 1` on `[]` (out
/// of bounds: the array grows), (1) LoadKeyed `a[0]` (a present element), (2) HasKeyed
/// `"x" in o`, (3) DefineKeyedOwn of `o["y"]`, (4) StoreInArrayLiteral at index 0 of a new
/// array literal (the `profile.rs` keyed program, one site per kind).
fn keyed_kinds_program() -> (
    VerifiedUnit,
    [(FeedbackSlotKind, ep9_bytecode::FeedbackSlot); 5],
) {
    use FeedbackSlotKind::{DefineKeyedOwn, HasKeyed, LoadKeyed, StoreInArrayLiteral, StoreKeyed};
    let mut s = Asm::script(0);
    let (a, o, key, lit) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    s.b.create_empty_array_literal(s.lit).star(a);
    s.b.create_empty_object_literal().star(o);
    let store = s.b.feedback_slot(StoreKeyed);
    s.b.lda_zero()
        .star(key)
        .lda_smi(1)
        .set_keyed_property(a, key, store);
    let load = s.b.feedback_slot(LoadKeyed);
    s.b.lda_zero().get_keyed_property(a, load);
    let has = s.b.feedback_slot(HasKeyed);
    s.lda_str("x");
    s.b.star(key).ldar(o).test_in(key, has);
    let define = s.b.feedback_slot(DefineKeyedOwn);
    s.lda_str("y");
    s.b.star(key)
        .lda_smi(1)
        .define_keyed_own_property(o, key, 0, define);
    let in_literal = s.b.feedback_slot(StoreInArrayLiteral);
    s.b.create_empty_array_literal(s.lit).star(lit);
    s.b.lda_zero()
        .star(key)
        .lda_smi(1)
        .sta_in_array_literal(lit, key, in_literal);
    s.b.lda_undefined().return_();
    (
        unit("", vec![s.finish()]),
        [
            (StoreKeyed, store),
            (LoadKeyed, load),
            (HasKeyed, has),
            (DefineKeyedOwn, define),
            (StoreInArrayLiteral, in_literal),
        ],
    )
}

/// MC-P3-KEYEDKIND (Codex M6-P3 r3): each of the 5 keyed slot kinds exports its own record,
/// with the payload its own site recorded, so no kind can export a default `KeyedFacts` while
/// the others supply the evidence (a pooled check misses an exporter arm that drops one kind's
/// facts). Mutation check: an exporter whose StoreInArrayLiteral arm gives
/// `KeyedFacts::default()` fails here.
#[test]
fn each_keyed_slot_kind_exports_its_own_payload() {
    use FeedbackSlotKind as K;
    use ep9_facts::{KeySet, KeyedFacts, KeyedHint};
    on_big_stack(|| {
        let (unit, sites) = keyed_kinds_program();
        let mut c = config(Dispatch::DEFAULT, false);
        c.feedback_threshold = 0;
        let mut iso = isolate_full(c);
        let u = unit.clone();
        iso.with_cx(|cx| cx.run_script(u, "keyed-kinds.js").map(|_| ()))
            .unwrap();
        let snap = export_facts(&iso, &unit.content_hash()).unwrap();
        let oob = KeyedHint::ARRAY | KeyedHint::OUT_OF_BOUNDS;
        let name = (KeySet::NAME, KeyedHint::OBJECT);
        let mut kinds = Vec::new();
        for (kind, slot) in sites {
            let record = snap.get(0, slot.index() as u32);
            let k = match (kind, record) {
                (K::LoadKeyed, Some(SlotFacts::LoadKeyed(k)))
                | (K::StoreKeyed, Some(SlotFacts::StoreKeyed(k)))
                | (K::HasKeyed, Some(SlotFacts::HasKeyed(k)))
                | (K::DefineKeyedOwn, Some(SlotFacts::DefineKeyedOwn(k)))
                | (K::StoreInArrayLiteral, Some(SlotFacts::StoreInArrayLiteral(k))) => k,
                _ => panic!("{kind:?}: {record:?}"),
            };
            assert_ne!(*k, KeyedFacts::default(), "{kind:?}: a default payload");
            let want = match kind {
                K::StoreKeyed | K::StoreInArrayLiteral => (KeySet::INDEX, oob),
                K::LoadKeyed => (KeySet::INDEX, KeyedHint::ARRAY),
                _ => name,
            };
            assert_eq!((k.keys, k.hint), want, "{kind:?}: {k:?}");
            kinds.push(kind);
        }
        kinds.sort();
        assert_eq!(
            kinds,
            [
                K::LoadKeyed,
                K::StoreKeyed,
                K::DefineKeyedOwn,
                K::HasKeyed,
                K::StoreInArrayLiteral
            ],
            "every keyed kind has its own site"
        );
    });
}
