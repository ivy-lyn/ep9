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
/// R4 PROTO-B replaces these lines with node's results. R3 PROTO-A built the methods of
/// `proto_iter.rs`; the tests below pin them.
#[test]
fn each_stub_throws_a_type_error() {
    let src = r#"
const P = Object.getPrototypeOf(Int8Array.prototype);
const names = ["copyWithin", "fill", "filter", "map", "reverse", "set", "slice", "sort",
  "subarray", "toReversed", "toSorted", "with"];
const ta = new Uint8Array(2);
for (const n of names) {
  try {
    P[n].call(ta, () => 0, 0);
    print(n, "no throw");
  } catch (e) {
    print(n, e instanceof TypeError, e.message);
  }
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "copyWithin true TypedArray.prototype.copyWithin: not built yet (m8-stub, TypedArray TA-4)",
            "fill true TypedArray.prototype.fill: not built yet (m8-stub, TypedArray TA-4)",
            "filter true TypedArray.prototype.filter: not built yet (m8-stub, TypedArray TA-4)",
            "map true TypedArray.prototype.map: not built yet (m8-stub, TypedArray TA-4)",
            "reverse true TypedArray.prototype.reverse: not built yet (m8-stub, TypedArray TA-4)",
            "set true TypedArray.prototype.set: not built yet (m8-stub, TypedArray TA-4)",
            "slice true TypedArray.prototype.slice: not built yet (m8-stub, TypedArray TA-4)",
            "sort true TypedArray.prototype.sort: not built yet (m8-stub, TypedArray TA-4)",
            "subarray true TypedArray.prototype.subarray: not built yet (m8-stub, TypedArray TA-4)",
            "toReversed true TypedArray.prototype.toReversed: not built yet (m8-stub, TypedArray TA-4)",
            "toSorted true TypedArray.prototype.toSorted: not built yet (m8-stub, TypedArray TA-4)",
            "with true TypedArray.prototype.with: not built yet (m8-stub, TypedArray TA-4)",
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
