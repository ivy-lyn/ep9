//! The TypedArray internal-method hooks that TA-3 adds in the VM, at the JS level (TypedArray
//! TA-3 TA-CTORS R4; `crates/ep9_vm/src/typed_array.rs`, `property::try_prevent_extensions`):
//! `[[PreventExtensions]]` of a view (`sec-typedarray-preventextensions`, route TA3-PREVEXT).
//! Each script prints the lines of node v26.13 for the same script (`print` here,
//! `console.log` of the joined arguments there) and runs with and without GC stress.

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
