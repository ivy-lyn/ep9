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

/// The 12 methods of R4 PROTO-B (`proto_copy.rs`; they were m8-stubs before R4) on a small
/// view with the same two arguments (a comparator and 0), then the TypeError of each one on a
/// value that is no TypedArray, and on a detached view (node v26.13.1).
#[test]
fn proto_b_methods_on_a_small_view() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
const names = ["copyWithin", "fill", "filter", "map", "reverse", "set", "slice", "sort",
  "subarray", "toReversed", "toSorted", "with"];
const show = (r) => r instanceof Object.getPrototypeOf(Int8Array) ? r.constructor.name + "[" + r.join(",") + "]" : String(r);
for (const n of names) {
  const ta = new Uint8Array([3, 1, 2]);
  try {
    const r = P[n].call(ta, (a, b) => b - a, 0);
    print(n, show(r), show(ta));
  } catch (e) {
    print(n, e.constructor.name);
  }
}
for (const n of names) {
  const r = [];
  for (const bad of [{}, [], 1, undefined, new DataView(new ArrayBuffer(1))]) {
    try { P[n].call(bad, () => 0, 0); r.push("no throw"); } catch (e) { r.push(e.constructor.name); }
  }
  const ab = new ArrayBuffer(2);
  const d = new Uint8Array(ab);
  ab.transfer();
  try { P[n].call(d, () => 0, 0); r.push("detached no throw"); } catch (e) { r.push("detached " + e.constructor.name); }
  print(n, r.join(" "));
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "copyWithin Uint8Array[3,1,2] Uint8Array[3,1,2]",
            "fill Uint8Array[0,0,0] Uint8Array[0,0,0]",
            "filter Uint8Array[3] Uint8Array[3,1,2]",
            "map Uint8Array[253,0,0] Uint8Array[3,1,2]",
            "reverse Uint8Array[2,1,3] Uint8Array[2,1,3]",
            "set undefined Uint8Array[0,0,2]",
            "slice Uint8Array[] Uint8Array[3,1,2]",
            "sort Uint8Array[3,2,1] Uint8Array[3,2,1]",
            "subarray Uint8Array[] Uint8Array[3,1,2]",
            "toReversed Uint8Array[2,1,3] Uint8Array[3,1,2]",
            "toSorted Uint8Array[3,2,1] Uint8Array[3,1,2]",
            "with Uint8Array[0,1,2] Uint8Array[3,1,2]",
            "copyWithin TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "fill TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "filter TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "map TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "reverse TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "set TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "slice TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "sort TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "subarray TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "toReversed TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "toSorted TypeError TypeError TypeError TypeError TypeError detached TypeError",
            "with TypeError TypeError TypeError TypeError TypeError detached TypeError",
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

/// `entries`, `keys`, `values` and `@@iterator` (`sec-%typedarray%.prototype.values`,
/// `sec-createarrayiterator`): ValidateTypedArray first; then each `next` reads
/// TypedArrayLength (the `natives.rs` view arm), never a `length` property: a grow during the
/// walk is seen, a shrink ends it, and a view out of bounds or detached throws a TypeError.
/// node v26.13.1 prints the same lines but one: after that TypeError the spec's iterator is
/// done (`{"done":true}`), where V8 throws again.
#[test]
fn iterators_of_a_view() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
const show = (it) => { const out = []; for (const x of it) out.push(Array.isArray(x) ? "[" + x.join(",") + "]" : String(x)); return out.join(" "); };
const ta = new Int16Array([7, -8, 9]);
print(show(ta.keys()), "|", show(ta.values()), "|", show(ta.entries()), "|", show(ta));
print(Object.getPrototypeOf(ta.values()) === Object.getPrototypeOf([].values()), ta.values().toString());
print(show(new BigInt64Array([1n, -2n]).entries()), show(new Float16Array([1.5, -0, NaN])));
print([...new Float64Array([-0])].map(x => Object.is(x, -0)).join());
// An own "length" is not read (TypedArrayLength).
const own = new Uint8Array([1, 2]);
Object.defineProperty(own, "length", { value: 5 });
print(show(own.values()));
// A length-tracking view: a grow during the walk is seen; a shrink ends it.
const rab = new ArrayBuffer(2, { maxByteLength: 6 });
const auto = new Uint8Array(rab);
auto[0] = 10; auto[1] = 11;
const it = auto.values();
const seen = [];
for (const x of it) { seen.push(x); if (seen.length === 1) rab.resize(4); if (seen.length === 3) rab.resize(3); }
print(seen.join(","));
// A fixed view out of bounds after a shrink: next throws a TypeError, then the iterator is done.
const rab2 = new ArrayBuffer(4, { maxByteLength: 8 });
const fixed = new Uint8Array(rab2, 0, 4);
const it2 = fixed.keys();
print(JSON.stringify(it2.next()));
rab2.resize(2);
try { it2.next(); print("no throw"); } catch (e) { print(e.constructor.name); }
// The spec closes the iterator after the throw (done); V8 throws again (node prints TypeError).
try { print(JSON.stringify(it2.next())); } catch (e) { print(e.constructor.name); }
// Detached during iteration.
const ab = new ArrayBuffer(3);
const d = new Uint8Array(ab);
const it3 = d.entries();
it3.next();
ab.transfer();
try { it3.next(); print("no throw"); } catch (e) { print(e.constructor.name); }
// Detached or out of bounds before the call: ValidateTypedArray throws.
for (const m of ["keys", "values", "entries"]) {
  try { d[m](); print("no throw"); } catch (e) { print(m, e.constructor.name); }
}
for (const bad of [{}, [], 1, undefined, new DataView(new ArrayBuffer(1))]) {
  try { P.values.call(bad); print("no throw"); } catch (e) { print(e.constructor.name); }
}
// Array.prototype.values on a view uses the same iterator (the natives.rs arm).
const it4 = Array.prototype.values.call(fixed);
try { it4.next(); print("no throw"); } catch (e) { print("Array values", e.constructor.name); }
// Spread and destructuring of a view.
const [a, , c] = new Uint32Array([4, 5, 6]);
print(a, c, Math.max(...new Int8Array([3, -1, 9])));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "0 1 2 | 7 -8 9 | [0,7] [1,-8] [2,9] | 7 -8 9",
            "true [object Array Iterator]",
            "[0,1] [1,-2] 1.5 0 NaN",
            "true",
            "1 2",
            "10,11,0",
            "{\"value\":0,\"done\":false}",
            "TypeError",
            "{\"done\":true}",
            "TypeError",
            "keys TypeError",
            "values TypeError",
            "entries TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "Array values TypeError",
            "4 6 9",
        ])
    );
}

/// `at`, `includes`, `indexOf`, `lastIndexOf`, `join` and `toString` (node v26.13.1): the
/// `fromIndex` forms, `-0` and NaN (SameValueZero vs IsStrictlyEqual), BigInt and Float16
/// views, no length re-read after a `valueOf` that shrinks or detaches, the TypeError of a
/// value that is no TypedArray, detached or out of bounds (before any argument is read), and
/// no `length` property read.
#[test]
fn at_includes_index_of_join_to_string() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
// `set` is PROTO-B work: store element by element.
const put = (v, a) => { for (let i = 0; i < a.length; i++) v[i] = a[i]; };
const ta = new Int8Array([5, -3, 7, -3, 0]);
// at
print(ta.at(0), ta.at(-1), ta.at(-5), String(ta.at(5)), String(ta.at(-6)), ta.at("1"), ta.at(1.9), ta.at(-0.5), String(ta.at(Infinity)));
// includes, indexOf, lastIndexOf with fromIndex forms
print(ta.includes(-3), ta.includes(-3, 4), ta.includes(5, -5), ta.includes(5, -4), ta.includes(0, Infinity), ta.includes(7, -Infinity));
print(ta.indexOf(-3), ta.indexOf(-3, 2), ta.indexOf(-3, -1), ta.indexOf(9), ta.indexOf(5, -Infinity), ta.indexOf(5, Infinity));
print(ta.lastIndexOf(-3), ta.lastIndexOf(-3, 2), ta.lastIndexOf(-3, -3), ta.lastIndexOf(5, -6), ta.lastIndexOf(5, -Infinity), ta.lastIndexOf(0, Infinity), ta.lastIndexOf(-3, undefined));
// -0, NaN: includes uses SameValueZero, indexOf uses IsStrictlyEqual
const f = new Float32Array([NaN, -0, 1.5]);
print(f.includes(NaN), f.indexOf(NaN), f.lastIndexOf(NaN), f.includes(0), f.indexOf(0), f.lastIndexOf(+0), f.includes("1.5"), f.indexOf(1.5));
const h = new Float16Array([NaN, -0, 65504]);
print(h.includes(NaN), h.indexOf(-0), h.includes(65504), h.indexOf(65504));
// BigInt views
const b = new BigInt64Array([1n, -2n, 3n]);
print(b.includes(-2n), b.includes(-2), b.indexOf(3n), b.lastIndexOf(1n), String(b.at(-1)), b.join("|"));
// empty views
const e = new Uint8Array(0);
print(e.includes(undefined), e.indexOf(undefined), e.lastIndexOf(undefined), String(e.at(0)), JSON.stringify(e.join()));
// a valueOf of fromIndex that shrinks: no length re-read; includes(undefined) is true past it, indexOf skips
const rab = new ArrayBuffer(4, { maxByteLength: 4 });
const u = new Uint8Array(rab);
put(u, [1, 2, 3, 4]);
const shrink = { valueOf() { rab.resize(2); return 0; } };
print(u.includes(undefined, shrink));
rab.resize(4);
print(u.indexOf(0, { valueOf() { rab.resize(2); return 0; } }), u.length);
rab.resize(4);
print(u.lastIndexOf(0, { valueOf() { rab.resize(2); return 3; } }));
rab.resize(4); put(u, [1, 2, 3, 4]);
print(String(u.at({ valueOf() { rab.resize(1); return 2; } })));
// a detach in valueOf
const dab = new ArrayBuffer(4);
const dv = new Uint8Array(dab);
print(dv.includes(undefined, { valueOf() { dab.transfer(); return 0; } }), dv.length);
// errors: not a TypedArray, detached, out of bounds
const bads = [{}, [1], 1, undefined, new DataView(new ArrayBuffer(1))];
for (const m of ["at", "includes", "indexOf", "lastIndexOf", "join"]) {
  const r = [];
  for (const x of bads) { try { P[m].call(x); r.push("ok"); } catch (err) { r.push(err.constructor.name); } }
  try { P[m].call(dv); r.push("ok"); } catch (err) { r.push(err.constructor.name); }
  const rab3 = new ArrayBuffer(4, { maxByteLength: 8 });
  const oob = new Uint8Array(rab3, 2, 2);
  rab3.resize(1);
  try { P[m].call(oob, { valueOf() { throw new Error("arg read"); } }); r.push("ok"); } catch (err) { r.push(err.constructor.name); }
  print(m, r.join(" "));
}
// join
print(ta.join(), ta.join(""), ta.join(undefined), ta.join(null), ta.join({ toString() { return "--"; } }));
print(new Float64Array([1.5, -0, NaN, Infinity]).join(), new Uint8ClampedArray([300, -1]).join(";"));
const rab4 = new ArrayBuffer(3, { maxByteLength: 3 });
const v4 = new Uint8Array(rab4);
put(v4, [1, 2, 3]);
print(JSON.stringify(v4.join({ toString() { rab4.resize(1); return "+"; } })));
try { new Uint8Array(2).join({ toString() { throw new RangeError("sep"); } }); } catch (err) { print(err.constructor.name, err.message); }
// toString
print(ta.toString(), P.toString.call({ join() { return "J"; } }), P.toString.call({ join: 1 }), P.toString.call([1, 2]));
print(P.toString.call(new Uint8Array([9, 8])), String(new Uint16Array([1, 2])), `${new Int8Array([3])}`);
try { P.toString.call(undefined); } catch (err) { print(err.constructor.name); }
const t2 = new Uint8Array([1, 2]);
t2.join = undefined;
print(t2.toString());
// length getters are not read
const s5 = new Uint8Array([5, 6]);
const bad = { get() { throw new Error("length read"); }, configurable: true };
Object.defineProperty(Uint8Array.prototype, "length", bad);
Object.defineProperty(s5, "length", bad);
const r5 = [s5.at(-1), s5.includes(6), s5.indexOf(6), s5.lastIndexOf(5), s5.join("-"), s5.toString()].join(" ");
delete Uint8Array.prototype.length;
print(r5);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "5 0 5 undefined undefined -3 -3 5 undefined",
            "true false true false false true",
            "1 3 -1 -1 0 -1",
            "3 1 1 -1 -1 4 -1",
            "true -1 -1 true 1 1 false 2",
            "true 1 true 2",
            "true false 2 0 3 1|-2|3",
            "false -1 -1 undefined \"\"",
            "true",
            "-1 2",
            "-1",
            "undefined",
            "true 0",
            "at TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "includes TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "indexOf TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "lastIndexOf TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "join TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "5,-3,7,-3,0 5-37-30 5,-3,7,-3,0 5null-3null7null-3null0 5---3--7---3--0",
            "1.5,0,NaN,Infinity 255;0",
            "\"1++\"",
            "RangeError sep",
            "5,-3,7,-3,0 J [object Object] 1,2",
            "9,8 1,2 3",
            "TypeError",
            "[object Uint8Array]",
            "6 true 1 0 5-6 5,6",
        ])
    );
}

/// `every`, `some`, `find`, `findIndex`, `findLast`, `findLastIndex`, `forEach`, `reduce` and
/// `reduceRight` (node v26.13.1): the callback arguments and `thisArg`, the order, the empty
/// view, a callback that detaches the buffer or shrinks a length-tracking view (later
/// elements read undefined, no throw), the TypeErrors (ValidateTypedArray before the callback
/// check), and no `length` property read.
#[test]
fn callback_methods() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
// `set` is PROTO-B work: store element by element.
const put = (v, a) => { for (let i = 0; i < a.length; i++) v[i] = a[i]; };
const ta = new Int16Array([4, -5, 6, -7]);
const log = [];
const cb = function (v, k, o) { log.push(v + "@" + k + (o === ta ? "" : "!") + (this === undefined ? "" : ":" + this.t)); return v < 0; };
print(ta.every(cb), ta.some(cb), ta.find(cb), ta.findIndex(cb), ta.findLast(cb), ta.findLastIndex(cb));
print(log.join(" "));
log.length = 0;
const ctx = { t: "T" };
print(String(ta.forEach(cb, ctx)), log.join(" "));
print(ta.every(v => true), ta.some(v => false), String(ta.find(v => v > 9)), ta.findIndex(v => v > 9), String(ta.findLast(v => v > 9)), ta.findLastIndex(v => v > 9));
print(ta.every(() => 1), ta.some(() => ""), ta.findIndex(() => ({})), ta.findLastIndex(() => 0n));
// reduce, reduceRight
print(ta.reduce((a, v) => a + v), ta.reduce((a, v) => a + "," + v, "s"), ta.reduceRight((a, v) => a + v), ta.reduceRight((a, v) => a + "," + v, "s"));
const steps = [];
ta.reduce(function (a, v, k, o) { steps.push(a + ":" + v + "@" + k + (o === ta) + typeof this); return v; });
ta.reduceRight(function (a, v, k) { steps.push(a + ":" + v + "@" + k); return v; }, undefined);
print(steps.join(" "));
print(new Uint8Array(1).reduce(() => 9), new Uint8Array(0).reduce(() => 9, "init"), new Uint8Array(0).reduceRight(() => 9, "init"));
for (const m of ["reduce", "reduceRight"]) {
  try { new Uint8Array(0)[m](() => 0); print("no throw"); } catch (e) { print(m, e.constructor.name); }
  try { new Uint8Array(0)[m](() => 0, undefined); print(m, "undefined initial ok"); } catch (e) { print(m, e.constructor.name); }
}
// BigInt and Float16 elements
print(String(new BigUint64Array([1n, 2n, 3n]).reduce((a, v) => a + v)), String(new BigInt64Array([5n, -6n]).find(v => v < 0n)), new Float16Array([0.1, 2.5]).findLast(() => true), new Float16Array([0.1]).at(0));
// a callback that detaches: the later Gets give undefined, no throw
const ab = new ArrayBuffer(3);
const d = new Uint8Array(ab);
put(d, [1, 2, 3]);
const got = [];
d.forEach((v, k) => { if (k === 0) ab.transfer(); got.push(String(v)); });
print(got.join(","), d.length);
for (const m of ["every", "some", "find", "findIndex", "findLast", "findLastIndex", "reduce", "reduceRight"]) {
  const ab2 = new ArrayBuffer(3);
  const x = new Uint8Array(ab2);
  put(x, [1, 2, 3]);
  const vals = [];
  let r;
  try {
    r = x[m]((a, b) => { const v = m.startsWith("reduce") ? b : a; vals.push(String(v)); if (vals.length === 1) ab2.transfer(); return m === "every"; }, ...(m.startsWith("reduce") ? [0] : []));
    print(m, String(r), vals.join(","));
  } catch (e) { print(m, e.constructor.name); }
}
// a callback that shrinks a length-tracking view: Get gives undefined past the new end
const rab = new ArrayBuffer(4, { maxByteLength: 4 });
const auto = new Uint8Array(rab);
put(auto, [1, 2, 3, 4]);
const seen = [];
auto.forEach((v, k) => { if (k === 0) rab.resize(2); seen.push(String(v)); });
print(seen.join(","), auto.length);
// errors: not callable (after ValidateTypedArray), not a TypedArray, detached, out of bounds
const bads = [{}, [1], 1, undefined, new DataView(new ArrayBuffer(1))];
for (const m of ["every", "some", "find", "findIndex", "findLast", "findLastIndex", "forEach", "reduce", "reduceRight"]) {
  const r = [];
  for (const cbv of [undefined, null, {}, 1]) { try { ta[m](cbv); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
  for (const x of bads) { try { P[m].call(x, () => 0); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
  const ab3 = new ArrayBuffer(1);
  const v3 = new Uint8Array(ab3);
  ab3.transfer();
  // ValidateTypedArray throws before the callback check
  try { P[m].call(v3, undefined); r.push("ok"); } catch (e) { r.push(e.constructor.name + (e.message.includes("callback") ? "-cb" : "")); }
  print(m, r.join(" "));
}
// length getters are not read
const s5 = new Uint8Array([5, 6]);
const bad = { get() { throw new Error("length read"); }, configurable: true };
Object.defineProperty(Uint8Array.prototype, "length", bad);
Object.defineProperty(s5, "length", bad);
const r5 = [s5.every(v => v > 0), s5.some(v => v > 5), s5.find(v => v > 5), s5.findIndex(v => v > 5), s5.findLast(v => v), s5.findLastIndex(v => v), s5.reduce((a, v) => a + v), s5.reduceRight((a, v) => a + v)].join(" ");
let n5 = 0; s5.forEach(() => n5++);
delete Uint8Array.prototype.length;
print(r5, n5);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "false true -5 1 -7 3",
            "4@0:undefined 4@0:undefined -5@1:undefined 4@0:undefined -5@1:undefined 4@0:undefined -5@1:undefined -7@3:undefined -7@3:undefined",
            "undefined 4@0:T -5@1:T 6@2:T -7@3:T",
            "true false undefined -1 undefined -1",
            "true false 0 -1",
            "-2 s,4,-5,6,-7 -2 s,-7,6,-5,4",
            "4:-5@1trueobject -5:6@2trueobject 6:-7@3trueobject undefined:-7@3 -7:6@2 6:-5@1 -5:4@0",
            "0 init init",
            "reduce TypeError",
            "reduce undefined initial ok",
            "reduceRight TypeError",
            "reduceRight undefined initial ok",
            "6 -6 2.5 0.0999755859375",
            "1,undefined,undefined 0",
            "every true 1,undefined,undefined",
            "some false 1,undefined,undefined",
            "find undefined 1,undefined,undefined",
            "findIndex -1 1,undefined,undefined",
            "findLast undefined 3,undefined,undefined",
            "findLastIndex -1 3,undefined,undefined",
            "reduce false 1,undefined,undefined",
            "reduceRight false 3,undefined,undefined",
            "1,2,undefined,undefined 2",
            "every TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "some TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "find TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "findIndex TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "findLast TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "findLastIndex TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "forEach TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "reduce TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "reduceRight TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "true true 6 1 6 1 11 11 2",
        ])
    );
}

/// `copyWithin`, `fill`, `reverse`, `toReversed` and `set` (node v26.13.1): overlaps in both
/// directions, the conversion order, a shrink or a detach in a conversion (the length re-read
/// of `copyWithin` and `fill`; the ignored writes of `set`), the RangeErrors of the offset, the
/// content type checks, a source on the same buffer or on the same SharedArrayBuffer of another
/// type (read before any write), and resizable buffers.
#[test]
fn copy_within_fill_reverse_set() {
    let src = r#"
const show = (v) => "[" + Array.from(v, (x) => Object.is(x, -0) ? "-0" : String(x)).join(",") + "]";
// copyWithin: overlap both ways, negative and missing arguments, end undefined.
print(show(new Int16Array([1, 2, 3, 4, 5]).copyWithin(1, 0)), show(new Int16Array([1, 2, 3, 4, 5]).copyWithin(0, 2)),
  show(new Int16Array([1, 2, 3, 4, 5]).copyWithin(-2, -4, -3)), show(new Int16Array([1, 2, 3, 4, 5]).copyWithin(3)),
  show(new Float64Array([1, -0, NaN, 4]).copyWithin(2, 0, undefined)), show(new BigInt64Array([1n, 2n, 3n]).copyWithin(0, 1)));
// copyWithin: a shrink in a conversion cuts the copy; a fixed view out of bounds throws.
{
  const rab = new ArrayBuffer(8, { maxByteLength: 16 });
  const v = new Uint16Array(rab);
  for (let i = 0; i < 4; i++) v[i] = i + 10;
  v.copyWithin({ valueOf() { rab.resize(6); return 1; } }, 0);
  print(show(v));
  const fixed = new Uint16Array(rab, 0, 3);
  try { fixed.copyWithin(0, { valueOf() { rab.resize(2); return 1; } }); print("no throw"); } catch (e) { print(e.constructor.name); }
}
// fill: the value is converted first, then start and end; a BigInt view wants a BigInt.
{
  const log = [];
  const t = new Int8Array(5);
  t.fill({ valueOf() { log.push("value"); return 7; } }, { valueOf() { log.push("start"); return 1; } }, { valueOf() { log.push("end"); return -1; } });
  print(show(t), log.join(" "));
  print(show(new Uint8ClampedArray(3).fill(300.5)), show(new Float16Array(2).fill(1.0009765625)), show(new BigUint64Array(2).fill(-1n)));
  try { new BigInt64Array(1).fill(1); } catch (e) { print(e.constructor.name); }
  try { new Int8Array(1).fill(1n); } catch (e) { print(e.constructor.name); }
  const rab = new ArrayBuffer(4, { maxByteLength: 8 });
  const auto = new Uint8Array(rab);
  auto.fill(9, 0, { valueOf() { rab.resize(2); return 4; } });
  print(show(auto), rab.byteLength);
  const fixed = new Uint8Array(rab, 0, 2);
  try { fixed.fill({ valueOf() { rab.resize(1); return 1; } }); print("no throw"); } catch (e) { print(e.constructor.name); }
}
// reverse, toReversed.
{
  const a = new Float32Array([1, 2, 3, 4, 5]);
  print(show(a.reverse()), show(new Int8Array(0).reverse()), show(new BigInt64Array([1n, 2n]).reverse()));
  const b = new Uint8Array([1, 2, 3]);
  const r = b.toReversed();
  print(show(r), show(b), r.constructor.name, r.buffer !== b.buffer);
}
// set: an array-like (Get and ToNumber in order), a primitive source, the offset checks.
{
  const t = new Int16Array(5);
  const log = [];
  const src = { length: 2, get 0() { log.push("get 0"); return { valueOf() { log.push("num 0"); return 7; } }; },
    get 1() { log.push("get 1"); return "8"; } };
  print(String(t.set(src, 2)), show(t), log.join(" "));
  t.set([1, 2, 3]); print(show(t));
  t.set("45", 3); print(show(t));
  t.set(5); print(show(t));
  for (const [s, o] of [[[1], -1], [[1, 2], 4], [[1], Infinity], [new Int8Array(6), 0]]) {
    try { t.set(s, o); print("no throw"); } catch (e) { print(e.constructor.name); }
  }
  try { t.set(null); } catch (e) { print(e.constructor.name); }
  try { new Int8Array(2).set([1n]); } catch (e) { print(e.constructor.name); }
  try { new Int8Array(2).set(new BigInt64Array(1)); } catch (e) { print(e.constructor.name); }
  // A conversion that detaches the target: later writes are ignored, no throw.
  const ab = new ArrayBuffer(3);
  const u = new Uint8Array(ab);
  u.set([1, { valueOf() { ab.transfer(); return 2; } }, 3]);
  print(u.length, ab.detached);
}
// set: a TypedArray source on the same buffer, another type (the source is read first).
{
  const ab = new ArrayBuffer(8);
  const bytes = new Uint8Array(ab);
  for (let i = 0; i < 8; i++) bytes[i] = i + 1;
  const words = new Uint16Array(ab, 2, 3);
  const src = new Uint8Array(ab, 0, 3);
  words.set(src);
  print(show(bytes));
  // The same type on one buffer, overlapping both ways.
  const v = new Int8Array([1, 2, 3, 4, 5, 6]);
  v.set(v.subarray(0, 4), 2); print(show(v));
  const w = new Int8Array([1, 2, 3, 4, 5, 6]);
  w.set(w.subarray(2), 0); print(show(w));
  // Other types, no shared buffer: conversions per element.
  const f = new Float64Array([1.5, -1, 300, NaN]);
  const c = new Uint8ClampedArray(4); c.set(f); const i8 = new Int8Array(4); i8.set(f);
  print(show(c), show(i8));
  const big = new BigUint64Array(2); big.set(new BigInt64Array([-1n, 2n])); print(show(big));
}
// set on a SharedArrayBuffer: same block, other types.
{
  const sab = new SharedArrayBuffer(8);
  const bytes = new Uint8Array(sab);
  for (let i = 0; i < 8; i++) bytes[i] = i + 1;
  new Uint16Array(sab, 2, 3).set(new Uint8Array(sab, 0, 3));
  print(show(bytes));
  const v = new Int32Array(sab);
  v.set([7, 8]);
  print(show(v));
}
// set: a resizable source and target.
{
  const rab = new ArrayBuffer(4, { maxByteLength: 8 });
  const auto = new Uint8Array(rab);
  auto.set([1, 2, 3, 4]);
  rab.resize(6);
  auto.set([5, 6], 4);
  print(show(auto));
  const fixed = new Uint8Array(rab, 4, 2);
  rab.resize(4);
  try { fixed.set([1]); print("no throw"); } catch (e) { print(e.constructor.name); }
  try { auto.set(fixed); print("no throw"); } catch (e) { print(e.constructor.name); }
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "[1,1,2,3,4] [3,4,5,4,5] [1,2,3,2,5] [1,2,3,1,2] [1,-0,1,-0] [2,3,3]",
            "[10,10,11]",
            "TypeError",
            "[0,7,7,7,0] value start end",
            "[255,255,255] [1.0009765625,1.0009765625] [18446744073709551615,18446744073709551615]",
            "TypeError",
            "TypeError",
            "[9,9] 2",
            "TypeError",
            "[5,4,3,2,1] [] [2,1]",
            "[3,2,1] [1,2,3] Uint8Array true",
            "undefined [0,0,7,8,0] get 0 num 0 get 1",
            "[1,2,3,8,0]",
            "[1,2,3,4,5]",
            "[1,2,3,4,5]",
            "RangeError",
            "RangeError",
            "RangeError",
            "RangeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "0 true",
            "[1,2,1,0,2,0,3,0]",
            "[1,2,1,2,3,4]",
            "[3,4,5,6,5,6]",
            "[2,0,255,0] [1,-1,44,0]",
            "[18446744073709551615,2]",
            "[1,2,1,0,2,0,3,0]",
            "[7,8]",
            "[1,2,3,4,5,6]",
            "TypeError",
            "TypeError",
        ])
    );
}

/// TypedArraySpeciesCreate (`sec-typedarray-species-create`) through `slice`, `map`, `filter`
/// and `subarray` (node v26.13.1): the default constructor, a custom `constructor[@@species]`
/// and the arguments it gets, the TypeErrors of SpeciesConstructor, a short result, a result
/// that is no TypedArray and a content type mismatch, a species constructor that detaches the
/// source, a `slice` into a view on the same buffer (an ascending byte copy), and `slice` and
/// `subarray` on resizable and growable buffers. TypedArrayCreateSameType (`toReversed`,
/// `toSorted`, `with`) reads no `@@species`. node prints the same lines but one: `map` with a
/// species result of the other content type (a BigInt64Array for an Int8Array) is a TypeError
/// by TypedArraySpeciesCreate step 4, where V8 does not check it ("map no throw").
#[test]
fn species_create() {
    let src = r#"
const show = (v) => v.constructor.name + "[" + Array.from(v, String).join(",") + "]";
// The default constructor: a new buffer of the same type.
{
  const ta = new Int16Array([1, 2, 3, 4]);
  const s = ta.slice(1, 3), m = ta.map((x) => x * 2), f = ta.filter((x) => x % 2 === 0), sub = ta.subarray(1, -1);
  print(show(s), show(m), show(f), show(sub), s.buffer !== ta.buffer, sub.buffer === ta.buffer, sub.byteOffset);
}
// A custom constructor[@@species]: the arguments it gets, and its result is used.
{
  const log = [];
  class MyTA extends Uint8Array {
    static get [Symbol.species]() { log.push("species"); return Sub; }
  }
  class Sub extends Uint8Array {
    constructor(...args) { log.push("new " + args.map((a) => a instanceof ArrayBuffer ? "buffer" : String(a)).join(" ")); super(...args); }
  }
  const t = new MyTA([1, 2, 3, 4]);
  log.length = 0;
  print(show(t.slice(1)), show(t.map((x) => x + 1)), show(t.filter((x) => x > 2)), show(t.subarray(1, 3)), show(t.subarray(2)));
  print(log.join("; "));
  // toReversed, toSorted and with ignore @@species.
  log.length = 0;
  print(show(t.toReversed()), show(t.toSorted()), show(t.with(0, 9)), log.length);
}
// constructor is not an object, @@species undefined or null, not a constructor.
{
  const ta = new Int8Array([1, 2]);
  for (const c of [undefined, 1, { [Symbol.species]: undefined }, { [Symbol.species]: null }, { [Symbol.species]: 1 }, { [Symbol.species]: () => {} }]) {
    ta.constructor = c;
    try { print(show(ta.slice())); } catch (e) { print(e.constructor.name); }
  }
}
// A short result (TypeError), a result that is no TypedArray, a content type mismatch.
{
  const ta = new Int8Array([1, 2, 3]);
  const species = (f) => { ta.constructor = { [Symbol.species]: f }; };
  species(function () { return new Int8Array(1); });
  for (const m of ["slice", "map", "filter"]) {
    try { ta[m]((x) => true); print(m, "no throw"); } catch (e) { print(m, e.constructor.name); }
  }
  species(function (n) { return new Int8Array(n + 2); });
  print(show(ta.slice()), show(ta.map((x) => x * 3)));
  species(function () { return {}; });
  try { ta.slice(); } catch (e) { print(e.constructor.name); }
  species(function (n) { return new BigInt64Array(n); });
  for (const m of ["slice", "map", "filter", "subarray"]) {
    try { ta[m]((x) => true); print(m, "no throw"); } catch (e) { print(m, e.constructor.name); }
  }
  species(function (n) { return new Float64Array(n); });
  print(show(ta.slice(1)), show(ta.map((x) => x / 2)));
}
// The species constructor detaches the source: slice throws (count > 0), map reads undefined.
{
  const ab = new ArrayBuffer(3);
  const ta = new Uint8Array(ab);
  ta.set([1, 2, 3]);
  ta.constructor = { [Symbol.species]: function (n) { ab.transfer(); return new Uint8Array(n); } };
  try { ta.slice(); print("no throw"); } catch (e) { print(e.constructor.name); }
  const ab2 = new ArrayBuffer(3);
  const tb = new Uint8Array(ab2);
  tb.set([1, 2, 3]);
  tb.constructor = { [Symbol.species]: function (n) { ab2.transfer(); return new Uint8Array(n); } };
  print(show(tb.map((x) => x === undefined ? 7 : x)));
  const ab3 = new ArrayBuffer(3);
  const tc = new Uint8Array(ab3);
  tc.constructor = { [Symbol.species]: function (n) { ab3.transfer(); return new Uint8Array(n); } };
  try { print(show(tc.slice(0, 0))); } catch (e) { print(e.constructor.name); }
}
// slice into a view on the same buffer: an ascending byte copy (target after the source
// repeats; target before is a plain move).
{
  const ab = new Uint16Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).buffer;
  const ta = new Uint16Array(ab, 0, 6);
  ta.constructor = { [Symbol.species]: function (n) { return new Uint16Array(ab, 4, n); } };
  print(show(ta.slice(0)), show(new Uint16Array(ab)));
  const ab2 = new Uint16Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).buffer;
  const tb = new Uint16Array(ab2, 4, 6);
  tb.constructor = { [Symbol.species]: function (n) { return new Uint16Array(ab2, 0, n); } };
  print(show(tb.slice(0)), show(new Uint16Array(ab2)));
  // Another type: element by element.
  const tc = new Float32Array([1.5, -2.5, 3]);
  tc.constructor = { [Symbol.species]: function (n) { return new Int8Array(n); } };
  print(show(tc.slice()));
}
// slice and subarray on resizable buffers.
{
  const rab = new ArrayBuffer(4, { maxByteLength: 8 });
  const auto = new Uint8Array(rab);
  auto.set([1, 2, 3, 4]);
  print(show(auto.slice({ valueOf() { rab.resize(2); return 0; } })));
  rab.resize(4);
  auto.set([1, 2, 3, 4]);
  const s1 = auto.subarray(1), s2 = auto.subarray(1, 3);
  rab.resize(6);
  print(show(s1), show(s2), s1.length, s2.length);
  rab.resize(1);
  print(s1.length, s2.length);
  rab.resize(2);
  const fixed = new Uint8Array(rab, 0, 2);
  rab.resize(1);
  try { fixed.slice(); } catch (e) { print("slice", e.constructor.name); }
  const sub = fixed.subarray(0);
  print(show(sub), sub.length);
  const sab = new SharedArrayBuffer(4, { maxByteLength: 8 });
  const g = new Int8Array(sab);
  const gs = g.subarray(2);
  sab.grow(6);
  print(gs.length, gs.buffer === sab);
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "Int16Array[2,3] Int16Array[2,4,6,8] Int16Array[2,4] Int16Array[2,3] true true 2",
            "Sub[2,3,4] Sub[2,3,4,5] Sub[3,4] Sub[2,3] Sub[3,4]",
            "species; new 3; species; new 4; species; new 2; species; new buffer 1 2; species; new buffer 2 2",
            "Uint8Array[4,3,2,1] Uint8Array[1,2,3,4] Uint8Array[9,2,3,4] 0",
            "Int8Array[1,2]",
            "TypeError",
            "Int8Array[1,2]",
            "Int8Array[1,2]",
            "TypeError",
            "TypeError",
            "slice TypeError",
            "map TypeError",
            "filter TypeError",
            "Int8Array[1,2,3,0,0] Int8Array[3,6,9,0,0]",
            "TypeError",
            "slice TypeError",
            "map TypeError",
            "filter TypeError",
            "subarray RangeError",
            "Float64Array[2,3] Float64Array[0.5,1,1.5]",
            "TypeError",
            "Uint8Array[7,7,7]",
            "Uint8Array[]",
            "Uint16Array[1,2,1,2,1,2] Uint16Array[1,2,1,2,1,2,1,2,9,10]",
            "Uint16Array[3,4,5,6,7,8] Uint16Array[3,4,5,6,7,8,7,8,9,10]",
            "Int8Array[1,-2,3]",
            "Uint8Array[1,2,0,0]",
            "Uint8Array[2,3,4,0,0] Uint8Array[2,3] 5 2",
            "0 0",
            "slice TypeError",
            "Uint8Array[] 0",
            "4 true",
        ])
    );
}

/// `sort`, `toSorted` and `with` (node v26.13.1): the numeric order (-0 before +0, NaN last)
/// of each element type, a stable sort with a comparator, a NaN or an object result of the
/// comparator, the comparator check before the this check, a throw that ends the sort before
/// any write, a detach or a shrink in the comparator (the writes are ignored), and the
/// conversion order and the RangeError of `with`.
#[test]
fn sort_to_sorted_with() {
    let src = r#"
const show = (v) => "[" + Array.from(v, (x) => Object.is(x, -0) ? "-0" : String(x)).join(",") + "]";
// The numeric order: -0 before +0, NaN last; not the string order.
print(show(new Float64Array([3, NaN, -0, 0, -Infinity, 10, 2, NaN, -1]).sort()),
  show(new Int8Array([10, 9, -1, 100 - 128, 1]).sort()), show(new Uint32Array([10, 9, 4294967295, 1]).sort()),
  show(new BigInt64Array([10n, -5n, 2n ** 63n - 1n, -(2n ** 63n)]).sort()), show(new BigUint64Array([2n ** 64n - 1n, 0n, 5n]).sort()),
  show(new Float16Array([0.5, -0.25, NaN, -0]).sort()));
// A comparator: stable, NaN results are +0, its result is converted by ToNumber.
{
  const ta = new Uint8Array([15, 21, 12, 25, 11, 32, 5]);
  print(show(ta.sort((a, b) => (a % 10) - (b % 10))));
  print(show(new Int8Array([3, 1, 2]).sort(() => NaN)), show(new Int8Array([1, 2, 3]).sort((a, b) => ({ valueOf() { return b - a; } }))));
  const calls = [];
  new Int8Array([1, 2]).sort((a, b) => { calls.push(typeof a + typeof b); return 0; });
  new BigInt64Array([1n, 2n]).sort((a, b) => { calls.push(typeof a + typeof b); return 0; });
  print(calls.join(" "));
}
// The comparator check is first: before the this value check.
for (const c of [null, 1, {}, "x"]) {
  try { Int8Array.prototype.sort.call(undefined, c); } catch (e) { print(e.constructor.name); }
}
// A throw ends the sort before any write; a detach in the comparator: the writes are ignored.
{
  const ta = new Int8Array([3, 2, 1]);
  try { ta.sort(() => { throw new RangeError("x"); }); } catch (e) { print(e.constructor.name, show(ta)); }
  const ab = new ArrayBuffer(4);
  const v = new Uint8Array(ab);
  v.set([4, 3, 2, 1]);
  print(v.sort((a, b) => { if (!ab.detached) ab.transfer(); return a - b; }) === v, v.length);
  const rab = new ArrayBuffer(4, { maxByteLength: 8 });
  const w = new Uint8Array(rab);
  w.set([4, 3, 2, 1]);
  let once = true;
  w.sort((a, b) => { if (once) { once = false; rab.resize(2); } return a - b; });
  print(show(w));
}
// toSorted: a new array of the same type; the source is not changed.
{
  const ta = new Float32Array([2, -0, 1, NaN]);
  const s = ta.toSorted();
  print(show(s), show(ta), s.constructor.name, show(new Int8Array([2, -1, 3]).toSorted((a, b) => b - a)));
  try { ta.toSorted(1); } catch (e) { print(e.constructor.name); }
}
// with: the index then the value is converted; a RangeError when the index is out of range
// after the conversions.
{
  const ta = new Int16Array([1, 2, 3]);
  print(show(ta.with(1, 9)), show(ta.with(-1, "7")), show(ta.with(0.9, 5)), show(ta));
  for (const i of [3, -4, Infinity, -Infinity]) {
    try { ta.with(i, 0); print("no throw"); } catch (e) { print(e.constructor.name); }
  }
  const log = [];
  try { ta.with({ valueOf() { log.push("index"); return 5; } }, { valueOf() { log.push("value"); return 1; } }); } catch (e) { print(e.constructor.name, log.join(" ")); }
  try { new BigInt64Array(1).with(0, 1); } catch (e) { print(e.constructor.name); }
  print(show(new BigInt64Array(2).with(1, 5n)));
  const rab = new ArrayBuffer(4, { maxByteLength: 8 });
  const auto = new Uint8Array(rab);
  try { auto.with(3, { valueOf() { rab.resize(2); return 1; } }); print("no throw"); } catch (e) { print(e.constructor.name); }
  rab.resize(4);
  print(show(auto.with(0, { valueOf() { rab.resize(6); return 1; } })), auto.length);
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "[-Infinity,-1,-0,0,2,3,10,NaN,NaN] [-28,-1,1,9,10] [1,9,10,4294967295] [-9223372036854775808,-5,10,9223372036854775807] [0,5,18446744073709551615] [-0.25,-0,0.5,NaN]",
            "[21,11,12,32,15,25,5]",
            "[3,1,2] [3,2,1]",
            "numbernumber bigintbigint",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "RangeError [3,2,1]",
            "true 0",
            "[1,2]",
            "[-0,1,2,NaN] [2,-0,1,NaN] Float32Array [3,2,-1]",
            "TypeError",
            "[1,9,3] [1,2,7] [5,2,3] [1,2,3]",
            "RangeError",
            "RangeError",
            "RangeError",
            "RangeError",
            "RangeError index value",
            "TypeError",
            "[0,5]",
            "RangeError",
            "[1,0,0,0] 6",
        ])
    );
}

/// `%TypedArray%.prototype.toString` with a `join` that is not callable calls the intrinsic
/// `%Object.prototype.toString%` (`sec-array.prototype.tostring` step 3), as
/// `Array.prototype.toString` does: a replaced `Object.prototype.toString` (a function, a
/// getter, a value that is not callable) is never read (Codex TA-4 r1 #3).
#[test]
fn to_string_uses_the_intrinsic_object_to_string() {
    let src = r#"
const ta = new Uint8Array([1, 2]); ta.join = null;
const orig = Object.prototype.toString;
Object.prototype.toString = function () { return "wrong"; };
print(Uint8Array.prototype.toString.call(ta), Array.prototype.toString.call(ta));
Object.prototype.toString = orig;
let calls = 0;
Object.defineProperty(Object.prototype, "toString", { get() { calls++; return orig; }, configurable: true });
print(Uint8Array.prototype.toString.call(ta), calls);
Object.defineProperty(Object.prototype, "toString", { value: 1, configurable: true, writable: true });
let r; try { r = Uint8Array.prototype.toString.call(ta); } catch (e) { r = e.constructor.name; }
print(r);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "[object Uint8Array] [object Uint8Array]",
            "[object Uint8Array] 0",
            "[object Uint8Array]",
        ])
    );
}
