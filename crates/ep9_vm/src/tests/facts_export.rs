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
    Access, BindingSet, FactSnapshot, FunctionRef, IcState, KeyRef, ObjectKind, PropAttrs,
    PropertyFacts, ProtoRef, ShapeDesc, ShapeFlags, SlotFacts, TypeSet,
};

use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::abi::{Local, export_facts};
use crate::atoms;
use crate::builtins::IntrinsicId;
use crate::feedback;
use crate::frame::{NULL, UNDEFINED, int32_bits};
use crate::ic::{self, State};
use crate::object::{self, attr};
use crate::props;
use crate::runtime::{Root, Runtime};
use crate::shape;
use crate::string;
use crate::{Dispatch, Isolate};

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
        let mut iso = isolate(c);
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
/// under Object.prototype, 8 under another prototype.
fn plain(proto: ProtoRef, keys: &[&str]) -> ShapeDesc {
    ShapeDesc {
        proto,
        kind: ObjectKind::Ordinary,
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
        let y = Arc::new(plain(object_proto(), &["y"]));
        let mut want = vec![
            (
                plain(object_proto(), &[]),
                Access::AddField {
                    to: y.clone(),
                    field: 0,
                },
            ),
            (plain(object_proto(), &["y"]), Access::OwnField { field: 0 }),
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

        // f8 never ran: no vector, no record. No Call-kind record anywhere (H16-Q2). The
        // script (function 0) has one record: its StoreGlobal slot, which the `sta_global`
        // sites of every name share (megamorphic: no binding). Its other slots have no T0 IC
        // site.
        assert!(t.vector(8).is_none());
        assert!(snap.get(9, 0).is_none());
        for (key, facts) in snap.records() {
            if key.function == 0 {
                assert_eq!(key.slot, 4, "{facts:?}");
                assert!(
                    matches!(facts, SlotFacts::StoreGlobal(g) if g.bindings.is_empty()),
                    "{facts:?}"
                );
            } else {
                assert_eq!(key.slot, 0, "{key:?} {facts:?}");
            }
        }

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
