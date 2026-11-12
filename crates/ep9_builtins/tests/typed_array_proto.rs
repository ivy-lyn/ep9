//! The methods of `%TypedArray.prototype%` at the JS level (TypedArray TA-4;
//! `src/typed_array/proto.rs`, `proto_iter.rs`, `proto_copy.rs`; ECMA-262
//! `sec-properties-of-the-%typedarray%-prototype-object`). R1 (the skeleton) pins the TA-4 keys
//! with their attributes, names and lengths, `@@iterator` = `values`, the stub TypeError of each
//! method that is not built yet (R3 PROTO-A and R4 PROTO-B flip it), the two methods R1 built
//! (`toLocaleString`, get `@@toStringTag`) and `Cx::create_array_iterator`. R3 and then R4
//! append. Each script prints the lines of node v26.13.1 for the same script (`print` here,
//! `console.log` there), except the stub lines, and runs with and without GC stress.

mod common;

use ep9_vm::{ArrayIterKind, Cx, IntrinsicId, JsResult, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Each TA-4 key of `%TypedArray.prototype%` (after the 5 keys of TA-3, which
/// `tests/typed_array.rs` pins): its attributes, its function's `name` and `length` (node
/// v26.13.1, the spec's), and `@@iterator` is the function object of `values`.
#[test]
fn the_prototype_methods_have_the_spec_attributes() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
const d = (o, k) => {
  const x = Object.getOwnPropertyDescriptor(o, k);
  return ("value" in x ? "d" + (x.writable ? "w" : "-") : "a") + (x.enumerable ? "e" : "-") +
    (x.configurable ? "c" : "-");
};
const out = [];
for (const k of Reflect.ownKeys(P)) {
  if (["constructor", "buffer", "byteLength", "byteOffset", "length"].includes(k)) continue;
  const x = Object.getOwnPropertyDescriptor(P, k);
  const f = "value" in x ? x.value : x.get;
  out.push(String(k) + " " + d(P, k) + " " + f.name + " " + f.length);
}
print(out.join("\n"));
print(P[Symbol.iterator] === P.values, P.keys !== P.values, P.entries !== P.values);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "entries dw-c entries 0\n\
             keys dw-c keys 0\n\
             values dw-c values 0\n\
             at dw-c at 1\n\
             copyWithin dw-c copyWithin 2\n\
             every dw-c every 1\n\
             fill dw-c fill 1\n\
             filter dw-c filter 1\n\
             find dw-c find 1\n\
             findIndex dw-c findIndex 1\n\
             findLast dw-c findLast 1\n\
             findLastIndex dw-c findLastIndex 1\n\
             forEach dw-c forEach 1\n\
             includes dw-c includes 1\n\
             indexOf dw-c indexOf 1\n\
             join dw-c join 1\n\
             lastIndexOf dw-c lastIndexOf 1\n\
             map dw-c map 1\n\
             reverse dw-c reverse 0\n\
             reduce dw-c reduce 1\n\
             reduceRight dw-c reduceRight 1\n\
             set dw-c set 1\n\
             slice dw-c slice 2\n\
             some dw-c some 1\n\
             sort dw-c sort 1\n\
             subarray dw-c subarray 2\n\
             toReversed dw-c toReversed 0\n\
             toSorted dw-c toSorted 1\n\
             with dw-c with 2\n\
             toLocaleString dw-c toLocaleString 0\n\
             toString dw-c toString 0\n\
             Symbol(Symbol.toStringTag) a-c get [Symbol.toStringTag] 0\n\
             Symbol(Symbol.iterator) dw-c values 0",
            "true true true",
        ])
    );
}

/// Each method that is not built yet throws the stub TypeError (m8-stub, TypedArray TA-4 R1):
/// R3 PROTO-A and R4 PROTO-B replace these lines with node's results. `[Symbol.iterator]` is
/// `values`, so it gives the stub of `values`.
#[test]
fn each_stub_throws_a_type_error() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
const names = ["entries", "keys", "values", "at", "copyWithin", "every", "fill", "filter",
  "find", "findIndex", "findLast", "findLastIndex", "forEach", "includes", "indexOf", "join",
  "lastIndexOf", "map", "reverse", "reduce", "reduceRight", "set", "slice", "some", "sort",
  "subarray", "toReversed", "toSorted", "with", "toString"];
const ta = new Uint8Array(2);
for (const n of names) {
  try {
    P[n].call(ta, () => 0, 0);
    print(n, "no throw");
  } catch (e) {
    print(n, e instanceof TypeError, e.message);
  }
}
try { ta[Symbol.iterator](); print("@@iterator no throw"); } catch (e) { print("@@iterator", e instanceof TypeError, e.message); }
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "entries true TypedArray.prototype.entries: not built yet (m8-stub, TypedArray TA-4)",
            "keys true TypedArray.prototype.keys: not built yet (m8-stub, TypedArray TA-4)",
            "values true TypedArray.prototype.values: not built yet (m8-stub, TypedArray TA-4)",
            "at true TypedArray.prototype.at: not built yet (m8-stub, TypedArray TA-4)",
            "copyWithin true TypedArray.prototype.copyWithin: not built yet (m8-stub, TypedArray TA-4)",
            "every true TypedArray.prototype.every: not built yet (m8-stub, TypedArray TA-4)",
            "fill true TypedArray.prototype.fill: not built yet (m8-stub, TypedArray TA-4)",
            "filter true TypedArray.prototype.filter: not built yet (m8-stub, TypedArray TA-4)",
            "find true TypedArray.prototype.find: not built yet (m8-stub, TypedArray TA-4)",
            "findIndex true TypedArray.prototype.findIndex: not built yet (m8-stub, TypedArray TA-4)",
            "findLast true TypedArray.prototype.findLast: not built yet (m8-stub, TypedArray TA-4)",
            "findLastIndex true TypedArray.prototype.findLastIndex: not built yet (m8-stub, TypedArray TA-4)",
            "forEach true TypedArray.prototype.forEach: not built yet (m8-stub, TypedArray TA-4)",
            "includes true TypedArray.prototype.includes: not built yet (m8-stub, TypedArray TA-4)",
            "indexOf true TypedArray.prototype.indexOf: not built yet (m8-stub, TypedArray TA-4)",
            "join true TypedArray.prototype.join: not built yet (m8-stub, TypedArray TA-4)",
            "lastIndexOf true TypedArray.prototype.lastIndexOf: not built yet (m8-stub, TypedArray TA-4)",
            "map true TypedArray.prototype.map: not built yet (m8-stub, TypedArray TA-4)",
            "reverse true TypedArray.prototype.reverse: not built yet (m8-stub, TypedArray TA-4)",
            "reduce true TypedArray.prototype.reduce: not built yet (m8-stub, TypedArray TA-4)",
            "reduceRight true TypedArray.prototype.reduceRight: not built yet (m8-stub, TypedArray TA-4)",
            "set true TypedArray.prototype.set: not built yet (m8-stub, TypedArray TA-4)",
            "slice true TypedArray.prototype.slice: not built yet (m8-stub, TypedArray TA-4)",
            "some true TypedArray.prototype.some: not built yet (m8-stub, TypedArray TA-4)",
            "sort true TypedArray.prototype.sort: not built yet (m8-stub, TypedArray TA-4)",
            "subarray true TypedArray.prototype.subarray: not built yet (m8-stub, TypedArray TA-4)",
            "toReversed true TypedArray.prototype.toReversed: not built yet (m8-stub, TypedArray TA-4)",
            "toSorted true TypedArray.prototype.toSorted: not built yet (m8-stub, TypedArray TA-4)",
            "with true TypedArray.prototype.with: not built yet (m8-stub, TypedArray TA-4)",
            "toString true TypedArray.prototype.toString: not built yet (m8-stub, TypedArray TA-4)",
            "@@iterator true TypedArray.prototype.values: not built yet (m8-stub, TypedArray TA-4)",
        ])
    );
}

/// The two methods that R1 built so that the test262 files which pass before TA-4 keep
/// passing: get `@@toStringTag` (`sec-get-%typedarray%.prototype-%symbol.tostringtag%`: the
/// `[[TypedArrayName]]`, else undefined, no throw; a detached or zero-length view keeps its
/// name) and `toLocaleString` (`sec-%typedarray%.prototype.tolocalestring`: ValidateTypedArray,
/// TypedArrayLength (no `length` read), `Invoke(element, "toLocaleString")` per element,
/// undefined after a shrink gives the empty String).
#[test]
fn to_string_tag_and_to_locale_string() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
const tag = Object.getOwnPropertyDescriptor(P, Symbol.toStringTag).get;
print(tag.call(new Uint8Array(1)), tag.call(new Float16Array(0)), tag.call(new BigInt64Array(1)),
  String(tag.call({})), String(tag.call(3)), String(tag.call(undefined)),
  String(tag.call(new DataView(new ArrayBuffer(1)))), String(P[Symbol.toStringTag]),
  Object.prototype.toString.call(new Int32Array(2)));
const rab = new ArrayBuffer(4, { maxByteLength: 8 });
const v = new Uint8Array(rab);
rab.resize(0);
print(tag.call(v));
const ab = new ArrayBuffer(2);
const w = new Uint8Array(ab);
ab.transfer();
print(tag.call(w));
print(new Uint8Array([1, 2, 3]).toLocaleString(), new Float64Array([1.5, 2.25]).toLocaleString(),
  new BigInt64Array([1n, -2n]).toLocaleString(), new Int8Array(0).toLocaleString() === "");
let calls = [];
const save = Number.prototype.toLocaleString;
Number.prototype.toLocaleString = function (a, b) {
  calls.push(String(this));
  return "<" + save.call(this) + ">";
};
print(new Int16Array([7, 8]).toLocaleString("en", {}), calls.join(" "));
Number.prototype.toLocaleString = function () { return { toString() { return "obj"; } }; };
print(new Int8Array([1, 2]).toLocaleString());
Number.prototype.toLocaleString = 42;
try { new Int8Array([1]).toLocaleString(); } catch (e) { print(e.constructor.name); }
Number.prototype.toLocaleString = save;
// A shrink in a user toLocaleString: later elements read undefined (the empty String).
const rab2 = new ArrayBuffer(3, { maxByteLength: 3 });
const u = new Uint8Array(rab2);
u[0] = 1; u[1] = 2; u[2] = 3;
Number.prototype.toLocaleString = function () { rab2.resize(1); return "x"; };
print(JSON.stringify(u.toLocaleString()));
Number.prototype.toLocaleString = save;
// The length getters are not read (TypedArrayLength).
const s5 = new Uint8Array([5, 6]);
const bad = { get() { throw new Error("length read"); }, configurable: true };
Object.defineProperty(Uint8Array.prototype, "length", bad);
Object.defineProperty(s5, "length", bad);
const r5 = s5.toLocaleString();
delete Uint8Array.prototype.length;
print(r5);
for (const bad of [{}, [], 1, undefined, new DataView(new ArrayBuffer(1))]) {
  try { P.toLocaleString.call(bad); print("no throw"); } catch (e) { print(e.constructor.name); }
}
const dab = new ArrayBuffer(1);
const dv = new Uint8Array(dab);
dab.transfer();
try { dv.toLocaleString(); print("no throw"); } catch (e) { print(e.constructor.name); }
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "Uint8Array Float16Array BigInt64Array undefined undefined undefined undefined undefined [object Int32Array]",
            "Uint8Array",
            "Uint8Array",
            "1,2,3 1.5,2.25 1,-2 true",
            "<7>,<8> 7 8",
            "obj,obj",
            "TypeError",
            "\"x,,\"",
            "5,6",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
        ])
    );
}

/// The values of `Cx::create_array_iterator(view, kind)` through `%ArrayIteratorPrototype%.next`
/// (an Array Iterator object of the realm), for each kind: key, value, key+value.
fn iterate(cx: &mut Cx<'_>) -> JsResult<Vec<String>> {
    let ctor = cx.intrinsic(IntrinsicId::Int16Array);
    let view = cx.construct(ctor, &[Local::int32(3)], ctor)?;
    for (i, v) in [(0, 7), (1, -8), (2, 9)] {
        cx.set_index(view, i, Local::int32(v), true)?;
    }
    let proto = cx.intrinsic(IntrinsicId::ArrayIteratorPrototype);
    let mut out = Vec::new();
    for kind in [
        ArrayIterKind::Keys,
        ArrayIterKind::Values,
        ArrayIterKind::Entries,
    ] {
        let it = cx.create_array_iterator(view, kind);
        let p = cx.get_prototype_of(it)?;
        assert!(
            p.value().is_same_bits(&proto.value()),
            "%ArrayIteratorPrototype%"
        );
        let mut rec = cx.get_iterator(it)?;
        let mut seen = Vec::new();
        while let Some(v) = cx.iterator_step_value(&mut rec)? {
            seen.push(cx.to_rust_string(v)?);
        }
        out.push(format!("{kind:?}: {}", seen.join(" ")));
    }
    Ok(out)
}

/// `Cx::create_array_iterator` (`sec-createarrayiterator`) on a TypedArray: the Array Iterator
/// gives the keys, the elements, or the `[key, element]` pairs (`String` of an entry array is
/// "key,element"), with and without GC stress.
#[test]
fn create_array_iterator_iterates_a_view() {
    for stress in [false, true] {
        let got = common::on_big_stack(move || common::isolate(stress).with_cx(iterate));
        assert_eq!(
            got.expect("no exception"),
            ["Keys: 0 1 2", "Values: 7 -8 9", "Entries: 0,7 1,-8 2,9"],
            "stress {stress}"
        );
    }
}
