//! The TypedArray internal-method hooks that TA-3 adds in the VM, at the JS level (TypedArray
//! TA-3 TA-CTORS R4; `crates/ep9_vm/src/typed_array.rs`, `property::try_prevent_extensions`):
//! `[[PreventExtensions]]` of a view (`sec-typedarray-preventextensions`, route TA3-PREVEXT),
//! and the facts writer of `ObjectKind::TypedArray` (`facts_export.rs` `describe`, route
//! TA3-OBJECTKIND-ROW). A script that prints compares with the lines of node v26.13 for the
//! same script (`print` here, `console.log` of the joined arguments there). Each test runs
//! with and without GC stress.

use ep9_facts::{Access, KeyRef, ObjectKind, ProtoRef, SlotFacts};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::abi::export_facts;
use ep9_vm::{ExecOptions, Feedback, IntrinsicId, Isolate, IsolateConfig};

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `[[PreventExtensions]]` of a view is false when the view is not IsTypedArrayFixedLength (a
/// length-tracking view, or any view on a resizable ArrayBuffer, also out of bounds or
/// detached), and the view stays extensible; a fixed-length view (on a fixed-length
/// ArrayBuffer or on any SharedArrayBuffer) runs OrdinaryPreventExtensions.
/// `Object.preventExtensions`, `Object.seal` and `Object.freeze` throw a TypeError on the
/// false, and a proxy with no trap forwards it.
#[test]
fn prevent_extensions_of_a_view_needs_a_fixed_length() {
    let src = r#"
// 1. Reflect.preventExtensions of each view form, then Object.isExtensible.
const rab = () => new ArrayBuffer(8, { maxByteLength: 16 });
const gsab = () => new SharedArrayBuffer(8, { maxByteLength: 16 });
const cases = [
  ["length-tracking on resizable", () => new Int8Array(rab())],
  ["fixed on resizable", () => new Float64Array(rab(), 0, 1)],
  ["fixed on fixed", () => new Int8Array(new ArrayBuffer(8), 0, 2)],
  ["fixed on growable SAB", () => new Uint16Array(gsab(), 0, 2)],
  ["length-tracking on growable SAB", () => new BigInt64Array(gsab())],
  ["no length on fixed SAB", () => new Int8Array(new SharedArrayBuffer(8))],
  ["detached length-tracking", () => { const b = rab(); const v = new Int8Array(b); b.transfer(); return v; }],
  ["out of bounds fixed on resizable", () => { const b = rab(); const v = new Int8Array(b, 4, 4); b.resize(2); return v; }],
];
for (const [n, f] of cases) {
  const v = f();
  print(n, Reflect.preventExtensions(v), Object.isExtensible(v));
}
// 2. Object.preventExtensions, seal and freeze of a view that is not fixed-length throw a
//    TypeError and leave it extensible; a proxy with no trap forwards the false.
for (const op of ["preventExtensions", "seal", "freeze"]) {
  const v = new Int8Array(rab());
  try { Object[op](v); print(op, "no error"); } catch (e) { print(op, e.constructor.name, Object.isExtensible(v)); }
}
try { Object.freeze(new Int8Array(rab(), 0, 0)); print("no error"); } catch (e) { print("freeze length 0", e.constructor.name); }
print("seal length 0 fixed", Object.isSealed(Object.seal(new Int8Array(new ArrayBuffer(0)))));
print("proxy", Reflect.preventExtensions(new Proxy(new Int8Array(rab()), {})));
try { Object.preventExtensions(new Proxy(new Int8Array(rab()), {})); } catch (e) { print("proxy", e.constructor.name); }
print("isSealed", Object.isSealed(new Int8Array(rab())), Object.isFrozen(new Int8Array(rab(), 0, 0)));
// 3. After a false answer the view still takes a new named property.
const t = new Int8Array(rab());
Reflect.preventExtensions(t);
t.tag = 1;
print("tag", Object.hasOwn(t, "tag"), t.tag);"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "length-tracking on resizable false true",
            "fixed on resizable false true",
            "fixed on fixed true false",
            "fixed on growable SAB true false",
            "length-tracking on growable SAB false true",
            "no length on fixed SAB true false",
            "detached length-tracking false true",
            "out of bounds fixed on resizable false true",
            "preventExtensions TypeError true",
            "seal TypeError true",
            "freeze TypeError true",
            "freeze length 0 TypeError",
            "seal length 0 fixed true",
            "proxy false",
            "proxy TypeError",
            "isSealed false false",
            "tag true 1",
        ])
    );
}

/// The prototype of each of the 12 TypedArray constructors, in Table 71 order.
const VIEW_PROTOS: [(&str, IntrinsicId); 12] = [
    ("Int8Array", IntrinsicId::Int8ArrayPrototype),
    ("Uint8Array", IntrinsicId::Uint8ArrayPrototype),
    ("Uint8ClampedArray", IntrinsicId::Uint8ClampedArrayPrototype),
    ("Int16Array", IntrinsicId::Int16ArrayPrototype),
    ("Uint16Array", IntrinsicId::Uint16ArrayPrototype),
    ("Int32Array", IntrinsicId::Int32ArrayPrototype),
    ("Uint32Array", IntrinsicId::Uint32ArrayPrototype),
    ("BigInt64Array", IntrinsicId::BigInt64ArrayPrototype),
    ("BigUint64Array", IntrinsicId::BigUint64ArrayPrototype),
    ("Float16Array", IntrinsicId::Float16ArrayPrototype),
    ("Float32Array", IntrinsicId::Float32ArrayPrototype),
    ("Float64Array", IntrinsicId::Float64ArrayPrototype),
];

/// A named load site on a view records the receiver shape as `ObjectKind::TypedArray` (H16
/// "declared fields (shrink-only)": the item has its writer, `facts_export.rs` `describe`),
/// for each of the 12 view kinds: `l<r>` reads the `length` getter of a view of row `r` (a
/// Generic case: the getter has no typed fact yet), `own` reads an own named property of a
/// length-tracking Uint8Array (an OwnField case). The receiver names its constructor's
/// prototype, has no elements kind name (a view kind 7 or 8 has none, O1) and is extensible.
/// Feedback FULL, threshold 0; the snapshot round-trips and the validator keeps each record.
#[test]
fn a_view_receiver_is_exported_as_object_kind_typed_array() {
    let mut src = String::new();
    for (r, (name, _)) in VIEW_PROTOS.iter().enumerate() {
        src += &format!("function l{r}(v) {{ return v.length; }}\nconst v{r} = new {name}(2);\n");
    }
    src += "function own(v) { return v.tag; }\n";
    src += "const t = new Uint8Array(new ArrayBuffer(2, { maxByteLength: 4 }));\nt.tag = 1;\n";
    src += "for (let i = 0; i < 4; i++) {\n  own(t);\n";
    for r in 0..VIEW_PROTOS.len() {
        src += &format!("  l{r}(v{r});\n");
    }
    src += "}\n";
    for stress in [false, true] {
        let src = src.clone();
        common::on_big_stack(move || check_view_facts(&src, stress));
    }
}

fn check_view_facts(src: &str, stress: bool) {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the script compiles");
    let mut iso = Isolate::new_with(
        IsolateConfig {
            builtins: Some(&ep9_builtins::TABLE),
            feedback_threshold: 0,
            heap: HeapConfig {
                gc_stress: u32::from(stress),
                verify_after_gc: stress,
                ..HeapConfig::default()
            },
            ..IsolateConfig::default()
        },
        ExecOptions::default().feedback(Feedback::FULL),
    )
    .expect("an isolate");
    let u = unit.clone();
    iso.with_cx(|cx| cx.run_script(u, "views.js").map(|_| ()))
        .unwrap_or_else(|e| panic!("the script threw: {}", e.message));
    let facts = export_facts(&iso, &unit.content_hash()).expect("the unit is loaded");
    let bytes = facts.encode();
    let back = ep9_facts::FactSnapshot::decode(&bytes).expect("the exporter's bytes decode");
    assert_eq!(back, facts);
    let (valid, dropped) = back.validate(&unit).expect("not stale");
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(valid, facts);
    let named = |f: &str| {
        let id = unit
            .functions
            .iter()
            .position(|x| {
                x.name
                    .is_some_and(|n| unit.strings[n.index()] == ep9_bytecode::UnitString::new(f))
            })
            .unwrap_or_else(|| panic!("no function {f}")) as u32;
        let cases: Vec<_> = facts
            .records()
            .filter(|(k, _)| k.function == id)
            .filter_map(|(_, r)| match r {
                SlotFacts::LoadNamed(p) => Some(p.cases.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(cases.len(), 1, "{f}: one LoadNamed record");
        assert_eq!(cases[0].len(), 1, "{f}: one case: {:?}", cases[0]);
        cases[0][0].clone()
    };
    for (r, (name, proto)) in VIEW_PROTOS.iter().enumerate() {
        let c = named(&format!("l{r}"));
        let s = &c.receiver;
        assert_eq!(s.kind, ObjectKind::TypedArray, "{name}");
        assert_eq!(s.proto, ProtoRef::Intrinsic(*proto as u32), "{name}");
        assert_eq!(s.elements, None, "{name}");
        assert!(s.keys.is_empty(), "{name}");
        assert_eq!(c.access, Access::Generic, "{name}: the length getter");
    }
    let c = named("own");
    assert_eq!(c.receiver.kind, ObjectKind::TypedArray);
    assert_eq!(
        c.receiver.proto,
        ProtoRef::Intrinsic(IntrinsicId::Uint8ArrayPrototype as u32)
    );
    assert_eq!(c.receiver.elements, None);
    assert_eq!(c.receiver.keys.len(), 1);
    assert_eq!(
        c.receiver.keys[0].0,
        KeyRef::String(ep9_bytecode::UnitString::new("tag"))
    );
    assert_eq!(c.access, Access::OwnField { field: 0 });
}
