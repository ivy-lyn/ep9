//! TypedArray objects at the JS level (TypedArray TA-3 TA-CTORS; `src/typed_array/`; ECMA-262
//! §23.2). R1 (the skeleton) pins the key list of each of the 26 intrinsics, the lazy group
//! (the 12 constructor keys join the group of `ArrayBuffer`, TA-D14), the `[[Prototype]]`
//! links of the constructors (the VM's `PROTO_LINKS`, decision O2), in a second realm too,
//! and `from` and `of` on `%TypedArray%` (R3 replaced their stubs). R2 (the constructors, section
//! (e)) tests each constructor form, the spec step order, sources that shrink or detach, the
//! getters, a view prototype under a hole read (MC-w26-4), subclasses and realms. Each script
//! prints the lines of node v26.13 for the same script (`print` here, `console.log` there; `createRealm()` there
//! is `vm.runInContext('globalThis', vm.createContext({}))` with `print` copied in) and runs
//! with and without GC stress.

mod common;

use std::cell::RefCell;
use std::hint::black_box;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, BuiltinModule, Cx, HostFn, IntrinsicId, Isolate, IsolateConfig, JsResult, Local,
    PropertyDescriptor,
};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The 12 constructors and their prototypes, in the row order of spec Table 71.
const VIEWS: [(IntrinsicId, IntrinsicId, &str); 12] = [
    (
        IntrinsicId::Int8Array,
        IntrinsicId::Int8ArrayPrototype,
        "Int8Array",
    ),
    (
        IntrinsicId::Uint8Array,
        IntrinsicId::Uint8ArrayPrototype,
        "Uint8Array",
    ),
    (
        IntrinsicId::Uint8ClampedArray,
        IntrinsicId::Uint8ClampedArrayPrototype,
        "Uint8ClampedArray",
    ),
    (
        IntrinsicId::Int16Array,
        IntrinsicId::Int16ArrayPrototype,
        "Int16Array",
    ),
    (
        IntrinsicId::Uint16Array,
        IntrinsicId::Uint16ArrayPrototype,
        "Uint16Array",
    ),
    (
        IntrinsicId::Int32Array,
        IntrinsicId::Int32ArrayPrototype,
        "Int32Array",
    ),
    (
        IntrinsicId::Uint32Array,
        IntrinsicId::Uint32ArrayPrototype,
        "Uint32Array",
    ),
    (
        IntrinsicId::BigInt64Array,
        IntrinsicId::BigInt64ArrayPrototype,
        "BigInt64Array",
    ),
    (
        IntrinsicId::BigUint64Array,
        IntrinsicId::BigUint64ArrayPrototype,
        "BigUint64Array",
    ),
    (
        IntrinsicId::Float16Array,
        IntrinsicId::Float16ArrayPrototype,
        "Float16Array",
    ),
    (
        IntrinsicId::Float32Array,
        IntrinsicId::Float32ArrayPrototype,
        "Float32Array",
    ),
    (
        IntrinsicId::Float64Array,
        IntrinsicId::Float64ArrayPrototype,
        "Float64Array",
    ),
];

// ---- (a) the key lists (the TA-2 form) --------------------------------------------------------

/// The own keys of `o` in [[OwnPropertyKeys]] order (a symbol as its descriptive string).
fn keys(cx: &mut Cx<'_>, id: IntrinsicId) -> JsResult<Vec<String>> {
    let o = cx.intrinsic(id);
    let keys = cx.own_property_keys(o)?;
    let n = cx.get(keys, "length")?;
    let n = cx.to_length(n)?;
    let mut texts = Vec::new();
    for i in 0..n {
        let k = cx.get(keys, i.to_string().as_str())?;
        let text = match k.as_symbol() {
            Some(sym) => cx.descriptive_string(sym)?.value(),
            None => k,
        };
        texts.push(cx.to_rust_string(text)?);
    }
    Ok(texts)
}

/// node v26.13.0's `Reflect.ownKeys(%TypedArray.prototype%)`: the full list after TA-4.
const PROTOTYPE_KEYS: &str = "constructor,buffer,byteLength,byteOffset,length,entries,keys,\
     values,at,copyWithin,every,fill,filter,find,findIndex,findLast,findLastIndex,forEach,\
     includes,indexOf,join,lastIndexOf,map,reverse,reduce,reduceRight,set,slice,some,sort,\
     subarray,toReversed,toSorted,with,toLocaleString,toString,Symbol(Symbol.toStringTag),\
     Symbol(Symbol.iterator)";

/// The keys of `%TypedArray.prototype%` that TA-3 makes (decision O3: the 4 getters).
const PROTOTYPE_KEYS_TA3: [&str; 5] = [
    "constructor",
    "buffer",
    "byteLength",
    "byteOffset",
    "length",
];

/// The exact key list of each of the 26 intrinsics of TA-3, read through `Cx::intrinsic`
/// (which installs the lazy group first): node v26.13.0's `Reflect.ownKeys` of the same
/// object. `%TypedArray.prototype%` is pinned in a form that TA-4's appends keep (check note
/// c5): its keys are node's full list filtered to the keys it has, and it has the 5 keys of
/// TA-3.
#[test]
fn each_intrinsic_has_the_exact_key_list() {
    let mut want = vec![
        (
            IntrinsicId::TypedArray,
            "length,name,prototype,of,from,Symbol(Symbol.species)".to_string(),
        ),
        (IntrinsicId::TypedArrayPrototype, String::new()),
    ];
    for (ctor, proto, _) in VIEWS {
        want.push((ctor, "length,name,prototype,BYTES_PER_ELEMENT".to_string()));
        want.push((proto, "constructor,BYTES_PER_ELEMENT".to_string()));
    }
    for stress in [false, true] {
        let ids: Vec<IntrinsicId> = want.iter().map(|&(id, _)| id).collect();
        let got = common::on_big_stack(move || {
            common::isolate(stress).with_cx(|cx| {
                let mut out = Vec::new();
                for id in ids {
                    out.push((id, keys(cx, id)?));
                }
                Ok(out)
            })
        })
        .expect("no exception");
        assert_eq!(got.len(), 26);
        for ((id, got), (want_id, want)) in got.into_iter().zip(&want) {
            assert_eq!(id, *want_id);
            if id == IntrinsicId::TypedArrayPrototype {
                for k in PROTOTYPE_KEYS_TA3 {
                    assert!(
                        got.iter().any(|g| g == k),
                        "stress {stress}: no {k}: {got:?}"
                    );
                }
                let node: Vec<&str> = PROTOTYPE_KEYS
                    .split(',')
                    .filter(|k| got.iter().any(|g| g == k))
                    .collect();
                assert_eq!(
                    got, node,
                    "stress {stress}: node's order of the keys it has"
                );
            } else {
                assert_eq!(got.join(","), *want, "stress {stress}: {}", id.name());
            }
        }
    }
}

/// The attributes and values of the table properties of TA-3, as node v26.13.0 prints them.
#[test]
fn the_properties_have_the_spec_attributes() {
    let src = r#"
const names = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
  "Int32Array", "Uint32Array", "BigInt64Array", "BigUint64Array", "Float16Array",
  "Float32Array", "Float64Array"];
const TA = Object.getPrototypeOf(Int8Array);
const d = (o, k) => {
  const x = Object.getOwnPropertyDescriptor(o, k);
  return ("value" in x ? "d" + (x.writable ? "w" : "-") : "a") + (x.enumerable ? "e" : "-") +
    (x.configurable ? "c" : "-");
};
print(TA.name, TA.length, d(TA, "prototype"), d(TA, "of"), d(TA, "from"), d(TA, Symbol.species));
print(TA.of.name, TA.of.length, TA.from.name, TA.from.length,
  Object.getOwnPropertyDescriptor(TA, Symbol.species).get.name);
for (const k of ["buffer", "byteLength", "byteOffset", "length"]) {
  const g = Object.getOwnPropertyDescriptor(TA.prototype, k);
  print(k, d(TA.prototype, k), g.get.name, g.get.length, g.set);
}
print(d(TA.prototype, "constructor"), TA.prototype.constructor === TA);
print(names.map((n) => {
  const C = globalThis[n];
  return [n, C.name, C.length, C.BYTES_PER_ELEMENT, C.prototype.BYTES_PER_ELEMENT,
    d(C, "BYTES_PER_ELEMENT"), d(C.prototype, "BYTES_PER_ELEMENT"), d(C, "prototype"),
    d(C.prototype, "constructor"), d(globalThis, n), C.prototype.constructor === C].join(" ");
}).join("\n"));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "TypedArray 0 d--- dw-c dw-c a-c",
            "of 0 from 1 get [Symbol.species]",
            "buffer a-c get buffer 0 undefined",
            "byteLength a-c get byteLength 0 undefined",
            "byteOffset a-c get byteOffset 0 undefined",
            "length a-c get length 0 undefined",
            "dw-c true",
            "Int8Array Int8Array 3 1 1 d--- d--- d--- dw-c dw-c true\n\
             Uint8Array Uint8Array 3 1 1 d--- d--- d--- dw-c dw-c true\n\
             Uint8ClampedArray Uint8ClampedArray 3 1 1 d--- d--- d--- dw-c dw-c true\n\
             Int16Array Int16Array 3 2 2 d--- d--- d--- dw-c dw-c true\n\
             Uint16Array Uint16Array 3 2 2 d--- d--- d--- dw-c dw-c true\n\
             Int32Array Int32Array 3 4 4 d--- d--- d--- dw-c dw-c true\n\
             Uint32Array Uint32Array 3 4 4 d--- d--- d--- dw-c dw-c true\n\
             BigInt64Array BigInt64Array 3 8 8 d--- d--- d--- dw-c dw-c true\n\
             BigUint64Array BigUint64Array 3 8 8 d--- d--- d--- dw-c dw-c true\n\
             Float16Array Float16Array 3 2 2 d--- d--- d--- dw-c dw-c true\n\
             Float32Array Float32Array 3 4 4 d--- d--- d--- dw-c dw-c true\n\
             Float64Array Float64Array 3 8 8 d--- d--- d--- dw-c dw-c true",
        ])
    );
}

// ---- (b) the lazy group (TA-D14) ---------------------------------------------------------------

/// The live cells after the last collection (out of line: the `GcStats` copy stays in this
/// frame, `common::conservative_roots`).
#[inline(never)]
fn live_cells(i: &Isolate) -> usize {
    i.gc_stats().last_marked_cells
}

/// The live cells after the sloppy script `src` (no GC stress) and a scrubbed full collection;
/// `rust` runs first, in the same isolate. Verifies the heap.
fn cells_after(src: &'static str, rust: fn(&mut Cx<'_>) -> JsResult<()>) -> usize {
    common::on_big_stack(move || {
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .expect("the script compiles");
        let mut iso = common::isolate(false);
        iso.with_cx(rust).expect("no exception");
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()))
            .expect("no exception");
        common::scrub_and_collect(&mut iso);
        let live = live_cells(&iso);
        black_box(&mut iso);
        iso.verify_heap().expect("a valid heap");
        live
    })
}

/// [`cells_after`], the smallest of 3 runs (a stale stack word does not count).
fn min_cells(src: &'static str, rust: fn(&mut Cx<'_>) -> JsResult<()>) -> usize {
    (0..3)
        .map(|_| cells_after(src, rust))
        .min()
        .expect("3 runs")
}

fn nothing(_cx: &mut Cx<'_>) -> JsResult<()> {
    Ok(())
}

/// Realm init makes no object of the TypedArray group: a script that reads `Int8Array` once
/// leaves more live cells than one that reads nothing, by at least one cell per definition and
/// intrinsic of the three modules `array_buffer`, `data_view` and `typed_array`, and the read
/// installs the whole group, `ArrayBuffer` included (the same cells as an `ArrayBuffer` read;
/// a later `ArrayBuffer` read installs nothing more). Each constructor key (also `typeof`) and
/// a use of a TA-3 intrinsic from Rust install the group once; `Cx::has_intrinsic` answers
/// true for each TA-3 intrinsic and installs nothing.
#[test]
fn the_constructor_keys_join_the_lazy_group() {
    let size: usize = [
        ep9_builtins::array_buffer::MODULE,
        ep9_builtins::data_view::MODULE,
        ep9_builtins::typed_array::MODULE,
    ]
    .iter()
    .map(|m: &BuiltinModule| m.defs.len() + m.intrinsics.len())
    .sum();
    let base = min_cells("", nothing);
    let group = min_cells("Int8Array;", nothing).saturating_sub(base);
    assert!(
        group >= size,
        "the group adds {group} cells, fewer than its {size} definitions and intrinsics"
    );
    assert_eq!(
        min_cells("ArrayBuffer;", nothing).saturating_sub(base),
        group,
        "an ArrayBuffer read installs the same group"
    );
    // The installs of the group that a run made, rounded.
    let installs = |live: usize| (live.saturating_sub(base) + group / 2) / group;
    for src in [
        "Int8Array; ArrayBuffer; DataView;",
        "Float64Array;",
        "typeof Uint8ClampedArray;",
        "Object.getPrototypeOf(BigInt64Array);",
    ] {
        assert_eq!(installs(min_cells(src, nothing)), 1, "{src}");
    }
    fn has(cx: &mut Cx<'_>) -> JsResult<()> {
        assert!(cx.has_intrinsic(IntrinsicId::TypedArray));
        assert!(cx.has_intrinsic(IntrinsicId::TypedArrayPrototype));
        for (ctor, proto, _) in VIEWS {
            assert!(cx.has_intrinsic(ctor), "{}", ctor.name());
            assert!(cx.has_intrinsic(proto), "{}", proto.name());
        }
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", has)),
        0,
        "has_intrinsic installs nothing"
    );
    fn use_one(cx: &mut Cx<'_>) -> JsResult<()> {
        black_box(cx.intrinsic(IntrinsicId::TypedArrayPrototype));
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", use_one)),
        1,
        "Cx::intrinsic installs"
    );
}

// ---- (c) the [[Prototype]] links (decision O2), in two realms ----------------------------------

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    OUT.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// The host functions of every realm.
const HOST: [(&str, u32, HostFn); 2] = [("print", 1, print), ("createRealm", 0, create_realm)];

/// `createRealm()`: a new realm with the host functions, made in it; returns its global.
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let global = cx.create_realm()?;
    cx.with_realm(global, |cx| {
        for (name, length, f) in HOST {
            let func = cx.new_builtin_closure(f, name, length, Local::undefined());
            let desc = PropertyDescriptor::data(func.into(), true, false, true);
            cx.define_property_or_throw(global, name, desc)?;
        }
        Ok(())
    })?;
    Ok(global.into())
}

/// Runs the sloppy script `src` on a fresh isolate with the table and [`HOST`]: the printed
/// lines, or the uncaught exception as "Name: message". Then collects and verifies the heap.
fn run_realms(src: &'static str, stress: bool) -> Result<Vec<String>, String> {
    common::on_big_stack(move || {
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .map_err(|e| format!("compile error: {e:?}"))?;
        let mut iso = Isolate::new(IsolateConfig {
            heap: HeapConfig {
                gc_stress: u32::from(stress),
                verify_after_gc: stress,
                ..HeapConfig::default()
            },
            builtins: Some(&ep9_builtins::TABLE),
            ..IsolateConfig::default()
        })
        .expect("an isolate");
        for (name, length, f) in HOST {
            iso.register_host_function(name, length, f);
        }
        OUT.with(|o| o.borrow_mut().clear());
        let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
        let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
        iso.collect_garbage();
        iso.verify_heap().expect("a valid heap");
        r.map(|_| out).map_err(|e| e.message)
    })
}

/// §23.2.6 and §23.2.7: the `[[Prototype]]` of each constructor is `%TypedArray%` (one object,
/// whose `[[Prototype]]` is `%Function.prototype%`), and of each constructor's prototype is
/// `%TypedArray.prototype%` (whose `[[Prototype]]` is `%Object.prototype%`). A second realm
/// has its own: the first use there (a constructor read) installs the group into that realm,
/// with its links to its own `%TypedArray%`.
#[test]
fn each_constructor_inherits_from_typed_array_in_each_realm() {
    let src = r#"
const names = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
  "Int32Array", "Uint32Array", "BigInt64Array", "BigUint64Array", "Float16Array",
  "Float32Array", "Float64Array"];
function check(g) {
  const TA = Object.getPrototypeOf(g.Int8Array);
  return [
    TA === Object.getPrototypeOf(g.Float64Array), TA.name, TA.length,
    Object.getPrototypeOf(TA) === g.Function.prototype,
    Object.getPrototypeOf(TA.prototype) === g.Object.prototype,
    names.every((n) => Object.getPrototypeOf(g[n]) === TA),
    names.every((n) => Object.getPrototypeOf(g[n].prototype) === TA.prototype),
    names.every((n) => g[n].prototype.__proto__ === TA.prototype),
  ].join(" ");
}
print(check(globalThis));
const other = createRealm();
print(check(other));
const TA = Object.getPrototypeOf(Int8Array), OTA = Object.getPrototypeOf(other.Int8Array);
print(TA === OTA, other.Int8Array === Int8Array, OTA.prototype === TA.prototype,
  Object.getPrototypeOf(other.Float32Array) === OTA);
"#;
    let want = lines(&[
        "true TypedArray 0 true true true true true",
        "true TypedArray 0 true true true true true",
        "false false false true",
    ]);
    let plain = run_realms(src, false);
    assert_eq!(plain, want);
    assert_eq!(run_realms(src, true), plain, "GC stress");
}

// ---- (d) the former stubs (R2 and R3 replaced them) ---------------------------------------------

/// TA-3 R2 built the constructors and `%TypedArray%` (an abstract class: a TypeError with and
/// without `new`); R3 built `%TypedArray%.from` and `of`, which construct their this value, so
/// `TA.from([])` and `TA.of(1)` on `%TypedArray%` itself throw the TypeError of the abstract
/// class (node: the same 6 lines; `tests/typed_array_from_of.rs` tests `from` and `of`).
#[test]
fn the_constructors_are_built_and_from_and_of_construct_this() {
    let src = r#"
const TA = Object.getPrototypeOf(Int8Array);
for (const f of [() => new Int8Array(1), () => Int8Array(1), () => new Float64Array(),
  () => new TA(), () => TA.from([]), () => TA.of(1)]) {
  try { const t = f(); print(t.constructor.name, t.length); }
  catch (e) { print(e.constructor === TypeError, e.message); }
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "Int8Array 1",
            "true Constructor Int8Array requires 'new'",
            "Float64Array 0",
            "true Abstract class TypedArray not directly constructable",
            "true Abstract class TypedArray not directly constructable",
            "true Abstract class TypedArray not directly constructable",
        ])
    );
}

// ---- (e) the constructors, the getters and @@species (TA-3 R2) ---------------------------------

/// TypedArray ( ...args ) (`sec-typedarray`) in each form: no argument, a length (ToIndex of a
/// primitive), a TypedArray of the same type (a copy of the bytes, a new buffer) and of
/// another type (element by element: Uint8Clamped, BigInt, Float16 and Float32 conversions),
/// an ArrayBuffer with and without byteOffset and length, an iterable (a Set, a generator), an
/// array-like, a SharedArrayBuffer (fixed and growable) and a view on one as the source; each
/// of the 12 constructors makes a view of its own prototype on a new `%ArrayBuffer%`.
#[test]
fn each_constructor_form_makes_a_view() {
    let src = r#"
const show = (t) => [t.constructor.name, t.length, t.byteLength, t.byteOffset,
  t.buffer.byteLength, Array.prototype.join.call(t)].join(" ");
print(show(new Int8Array()));
print(show(new Uint8Array(3)), show(new Int8Array("2")), show(new Int8Array(null)),
  show(new Int8Array(undefined)), show(new Float32Array(2.9)));
const src = new Int8Array([1, -2, 3]);
const same = new Int8Array(src);
print(show(same), same.buffer !== src.buffer, show(new Int16Array(src)),
  show(new Uint8Array(src)), show(new Float64Array(src)));
const buf = new ArrayBuffer(32);
print(show(new Float64Array(buf, 8, 2)), show(new Float64Array(buf, 8)),
  show(new Float64Array(buf)), new Float64Array(buf, 8).buffer === buf);
print(show(new Uint8ClampedArray([300, -5, 1.5, 2.5, 0.5, NaN])));
function* gen() { yield 1; yield "2"; yield { valueOf() { return 3; } }; }
print(show(new Int32Array(new Set([1, 2, 3]))), show(new Int32Array(gen())),
  show(new Uint16Array({ length: 3, 0: 1, 1: 65537, 2: "7" })),
  show(new Int8Array({ length: "2", 1: 5 })));
print(show(new BigInt64Array([1n, -1n])), show(new BigUint64Array(new BigInt64Array([-1n, 2n]))),
  show(new BigInt64Array({ length: 1, 0: 2n ** 63n })));
print(show(new Float16Array([1.5, 65520, 1e-8, -0, 0.1, 2049])), Object.is(new Float16Array([-0])[0], -0));
print(show(new Float32Array(new Float64Array([0.1, 1e40]))), show(new Int8Array(new Float16Array([300.5]))));
const names = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
  "Int32Array", "Uint32Array", "BigInt64Array", "BigUint64Array", "Float16Array",
  "Float32Array", "Float64Array"];
for (const n of names) {
  const C = globalThis[n];
  const t = new C(2);
  print(n, t.length, t.byteLength, t.byteOffset, C.BYTES_PER_ELEMENT,
    Object.getPrototypeOf(t) === C.prototype, t.buffer.constructor === ArrayBuffer);
}
const sab = new SharedArrayBuffer(8);
const sv = new Int16Array(sab, 2, 2);
sv[0] = -1; sv[1] = 513;
print(show(sv), sv.buffer === sab, show(new Int16Array(sv)), show(new Uint8Array(sv)),
  new Int16Array(sv).buffer.constructor === ArrayBuffer);
const gsab = new SharedArrayBuffer(4, { maxByteLength: 8 });
const gl = new Uint8Array(gsab);
gsab.grow(6);
print(show(gl), show(new Uint8Array(gsab, 1)), show(new Uint8Array(gsab, 1, 2)));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "Int8Array 0 0 0 0 ",
            "Uint8Array 3 3 0 3 0,0,0 Int8Array 2 2 0 2 0,0 Int8Array 0 0 0 0  Int8Array 0 0 0 0  Float32Array 2 8 0 8 0,0",
            "Int8Array 3 3 0 3 1,-2,3 true Int16Array 3 6 0 6 1,-2,3 Uint8Array 3 3 0 3 1,254,3 Float64Array 3 24 0 24 1,-2,3",
            "Float64Array 2 16 8 32 0,0 Float64Array 3 24 8 32 0,0,0 Float64Array 4 32 0 32 0,0,0,0 true",
            "Uint8ClampedArray 6 6 0 6 255,0,2,2,0,0",
            "Int32Array 3 12 0 12 1,2,3 Int32Array 3 12 0 12 1,2,3 Uint16Array 3 6 0 6 1,1,7 Int8Array 2 2 0 2 0,5",
            "BigInt64Array 2 16 0 16 1,-1 BigUint64Array 2 16 0 16 18446744073709551615,2 BigInt64Array 1 8 0 8 -9223372036854775808",
            "Float16Array 6 12 0 12 1.5,Infinity,0,0,0.0999755859375,2048 true",
            "Float32Array 2 8 0 8 0.10000000149011612,Infinity Int8Array 1 1 0 1 44",
            "Int8Array 2 2 0 1 true true",
            "Uint8Array 2 2 0 1 true true",
            "Uint8ClampedArray 2 2 0 1 true true",
            "Int16Array 2 4 0 2 true true",
            "Uint16Array 2 4 0 2 true true",
            "Int32Array 2 8 0 4 true true",
            "Uint32Array 2 8 0 4 true true",
            "BigInt64Array 2 16 0 8 true true",
            "BigUint64Array 2 16 0 8 true true",
            "Float16Array 2 4 0 2 true true",
            "Float32Array 2 8 0 4 true true",
            "Float64Array 2 16 0 8 true true",
            "Int16Array 2 4 2 8 -1,513 true Int16Array 2 4 0 4 -1,513 Uint8Array 2 2 0 2 255,1 true",
            "Uint8Array 6 6 0 6 0,0,0,0,0,0 Uint8Array 5 5 1 6 0,0,0,0,0 Uint8Array 2 2 1 6 0,0",
        ])
    );
}

/// The step order of TypedArray ( ...args ) (check note b1): a first argument that is not an
/// Object runs ToIndex first (RangeError for -1 and 2^53, TypeError for a Symbol or a BigInt)
/// and GetPrototypeFromConstructor after it; no argument and an Object first argument run
/// GetPrototypeFromConstructor first; the buffer path then reads byteOffset, checks its
/// alignment, reads length, and checks detach and range after the conversions. The other
/// lines: the errors of the iterable, array-like and TypedArray paths, an allocation past
/// the heap (RangeError), BigInt and Number mixes, `%TypedArray%` and a call without `new`.
/// node v26.13 prints these lines, except two: V8 reads `@@iterator` and `length` of an
/// Object argument before the prototype ("object: EvalError [length,proto]", "object
/// logging: ok 1 [iter,length,proto]"); the spec (step 6.b.i, AllocateTypedArray first) and
/// test262 (`ctors/object-arg/custom-proto-access-throws.js`) give the lines below.
#[test]
fn the_constructor_steps_run_in_spec_order() {
    let src = r#"
const log = [];
const err = (f) => {
  try { return "ok " + f().length; } catch (e) { return e.constructor.name; }
};
const throwing = function () {}.bind(null);
Object.defineProperty(throwing, "prototype", {
  get() { log.push("proto"); throw new EvalError("proto"); },
});
const logging = function () {}.bind(null);
Object.defineProperty(logging, "prototype", {
  get() { log.push("proto"); return Int8Array.prototype; },
});
const value = (name, v) => ({ valueOf() { log.push(name); return v; } });
const run = (args, nt, C = Int8Array) => {
  log.length = 0;
  const r = err(() => Reflect.construct(C, args, nt));
  return r + " [" + log.join(",") + "]";
};
print("no args:", run([], throwing));
print("-1:", run([-1], throwing));
print("Symbol:", run([Symbol()], throwing));
print("1n:", run([1n], throwing));
print("1:", run([1], throwing));
print("valueOf len:", run([value("len", 2)], logging));
print("2**53:", run([2 ** 53], throwing));
print("object:", run([{ get length() { log.push("length"); return 1; } }], throwing));
print("object logging:", run([{ get length() { log.push("length"); return 1; },
  get [Symbol.iterator]() { log.push("iter"); return undefined; } }], logging));
const buf = new ArrayBuffer(8);
print("buffer:", run([buf, value("off", 0), value("len", 1)], logging));
print("buffer throwing:", run([buf, value("off", 0), value("len", 1)], throwing));
print("buffer align:", run([buf, value("off", 1), value("len", 1)], logging, Int16Array));
print("buffer off -1:", run([buf, value("off", -1), value("len", 1)], logging));
print("buffer len -1:", run([buf, value("off", 0), value("len", -1)], logging));
const d = new ArrayBuffer(8);
print("detach in len:", run([d, value("off", 0), { valueOf() { log.push("len"); d.transfer(); return 1; } }], logging));
print("detach first:", run([d, 0, 1], logging));
print("off past:", run([new ArrayBuffer(8), 9], logging), run([new ArrayBuffer(8), 8], logging));
print("len past:", run([new ArrayBuffer(8), 4, 5], logging), run([new ArrayBuffer(8), 4, 4], logging));
print("size:", run([new ArrayBuffer(7)], logging, Int16Array), run([new ArrayBuffer(7), 2, 2], logging, Int16Array));
const rb = new ArrayBuffer(7, { maxByteLength: 16 });
print("resizable:", run([rb], logging, Int16Array), run([rb, 8], logging, Int16Array),
  run([rb, 6], logging, Int16Array));
print("ta src:", run([new Int8Array(3)], throwing), run([new Int8Array(3)], logging));
print("iter throws:", err(() => new Int8Array({ [Symbol.iterator]() { throw new EvalError("i"); } })));
print("iter not callable:", err(() => new Int8Array({ [Symbol.iterator]: 1 })));
print("iter null:", err(() => new Int8Array({ [Symbol.iterator]: null, length: 2 })));
print("next throws:", err(() => new Int8Array({ [Symbol.iterator]() {
  return { next() { throw new EvalError("n"); } }; } })));
print("value throws:", err(() => new Int8Array([1, { valueOf() { throw new EvalError("v"); } }])));
print("arraylike get throws:", err(() => new Int8Array({ length: 2, get 1() { throw new EvalError("g"); } })));
print("length throws:", err(() => new Int8Array({ get length() { throw new EvalError("l"); } })));
print("huge:", err(() => new Int8Array({ length: 2 ** 53 - 1 })), err(() => new Float64Array(2 ** 50)));
print("bigint mix:", err(() => new BigInt64Array(new Int8Array(2))), err(() => new Int8Array(new BigInt64Array(2))),
  err(() => new BigInt64Array([1])), err(() => new Int8Array([1n])));
print("abstract:", err(() => Object.getPrototypeOf(Int8Array)()), err(() => new (Object.getPrototypeOf(Int8Array))()));
print("call:", err(() => Int8Array()), err(() => Float64Array(1)));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "no args: EvalError [proto]",
            "-1: RangeError []",
            "Symbol: TypeError []",
            "1n: TypeError []",
            "1: EvalError [proto]",
            "valueOf len: ok 0 [proto]",
            "2**53: RangeError []",
            "object: EvalError [proto]",
            "object logging: ok 1 [proto,iter,length]",
            "buffer: ok 1 [proto,off,len]",
            "buffer throwing: EvalError [proto]",
            "buffer align: RangeError [proto,off]",
            "buffer off -1: RangeError [proto,off]",
            "buffer len -1: RangeError [proto,off,len]",
            "detach in len: TypeError [proto,off,len]",
            "detach first: TypeError [proto]",
            "off past: RangeError [proto] ok 0 [proto]",
            "len past: RangeError [proto] ok 4 [proto]",
            "size: RangeError [proto] ok 2 [proto]",
            "resizable: ok 3 [proto] RangeError [proto] ok 0 [proto]",
            "ta src: EvalError [proto] ok 3 [proto]",
            "iter throws: EvalError",
            "iter not callable: TypeError",
            "iter null: ok 2",
            "next throws: EvalError",
            "value throws: EvalError",
            "arraylike get throws: EvalError",
            "length throws: EvalError",
            "huge: RangeError RangeError",
            "bigint mix: TypeError TypeError TypeError TypeError",
            "abstract: TypeError TypeError",
            "call: TypeError TypeError",
        ])
    );
}

/// InitializeTypedArrayFromArrayLike reads the source after the allocation: a valueOf that
/// detaches or shrinks the buffer of the source's prototype view makes the later reads
/// undefined (stored as 0). InitializeTypedArrayFromTypedArray reads the source's current
/// length: a length-tracking source on a resized buffer, a fixed one with an offset, and an
/// out-of-bounds or detached source (TypeError); a detached ArrayBuffer is a TypeError too.
#[test]
fn a_source_that_shrinks_or_detaches_is_read_again() {
    let src = r#"
// A valueOf in FromArrayLike detaches or shrinks the buffer of the source's prototype view.
const sb = new ArrayBuffer(3);
const s = new Uint8Array(sb);
s[0] = 1; s[1] = 2; s[2] = 3;
const al = Object.create(s, { length: { value: 3 }, [Symbol.iterator]: { value: undefined } });
Object.defineProperty(al, "0", { value: { valueOf() { sb.transfer(); return 9; } } });
print(Array.prototype.join.call(new Int8Array(al)), s.length);
const rb = new ArrayBuffer(3, { maxByteLength: 8 });
const r = new Uint8Array(rb);
r[0] = 1; r[1] = 2; r[2] = 3;
const al2 = Object.create(r, { length: { value: 3 }, [Symbol.iterator]: { value: undefined } });
Object.defineProperty(al2, "0", { value: { valueOf() { rb.resize(2); return 9; } } });
print(Array.prototype.join.call(new Int8Array(al2)), r.length);
// A TypedArray source: a length-tracking one on a resizable buffer, a fixed one out of bounds.
const tb = new ArrayBuffer(4, { maxByteLength: 8 });
const lt = new Uint8Array(tb);
lt[0] = 1; lt[1] = 2; lt[2] = 3; lt[3] = 4;
tb.resize(6);
const fx = new Uint8Array(tb, 2, 2);
print(Array.prototype.join.call(new Uint8Array(lt)), Array.prototype.join.call(new Int16Array(lt)),
  Array.prototype.join.call(new Uint8Array(fx)), Array.prototype.join.call(new Uint8Array(new Uint8Array(tb, 1))));
tb.resize(3);
try { new Uint8Array(fx); } catch (e) { print(e.constructor.name); }
print(Array.prototype.join.call(new Uint8Array(lt)));
const db = new ArrayBuffer(4);
const dv = new Int8Array(db);
db.transfer();
try { new Int8Array(dv); } catch (e) { print(e.constructor.name); }
try { new Int8Array(db); } catch (e) { print(e.constructor.name); }
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "9,0,0 0",
            "9,2,0 2",
            "1,2,3,4,0,0 1,2,3,4,0,0 3,4 2,3,4,0,0",
            "TypeError",
            "1,2,3",
            "TypeError",
            "TypeError",
        ])
    );
}

/// The getters `length`, `byteLength`, `byteOffset` (0 when out of bounds or detached) and
/// `buffer` (also when detached) on fixed, length-tracking and whole views of a resizable
/// buffer that shrinks and grows, on a detached buffer and on a growable SharedArrayBuffer;
/// each throws a TypeError for a receiver that is no TypedArray (a prototype included);
/// `get %TypedArray%[@@species]` gives its this value.
#[test]
fn the_getters_follow_the_buffer() {
    let src = r#"
const TA = Object.getPrototypeOf(Int8Array);
const g = (t) => [t.length, t.byteLength, t.byteOffset].join(",");
const b = new ArrayBuffer(8, { maxByteLength: 16 });
const fixed = new Int16Array(b, 2, 2), track = new Int16Array(b, 2), whole = new Float64Array(b);
print(g(fixed), g(track), g(whole), fixed.buffer === b, track.buffer === b);
b.resize(5);
print(g(fixed), g(track), g(whole), fixed.buffer === b);
b.resize(1);
print(g(fixed), g(track), g(whole), track.buffer === b);
b.resize(16);
print(g(fixed), g(track), g(whole));
const d = new ArrayBuffer(8);
const v = new Float32Array(d, 4);
d.transfer();
print(g(v), v.buffer === d, d.detached);
const desc = (k) => Object.getOwnPropertyDescriptor(TA.prototype, k).get;
for (const k of ["buffer", "byteLength", "byteOffset", "length"]) {
  const out = [];
  for (const t of [{}, new DataView(new ArrayBuffer(1)), Int8Array.prototype, TA.prototype, 1, undefined]) {
    try { desc(k).call(t); out.push("ok"); } catch (e) { out.push(e.constructor.name); }
  }
  print(k, out.join(" "));
}
const sp = Object.getOwnPropertyDescriptor(TA, Symbol.species).get;
print(TA[Symbol.species] === TA, Int8Array[Symbol.species] === Int8Array, sp.call(42), sp.call(undefined));
const sab = new SharedArrayBuffer(4, { maxByteLength: 8 });
const st = new Int16Array(sab, 2);
print(g(st));
sab.grow(8);
print(g(st), st.buffer === sab);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "2,4,2 3,6,2 1,8,0 true true",
            "0,0,0 1,2,2 0,0,0 true",
            "0,0,0 0,0,0 0,0,0 true",
            "2,4,2 7,14,2 2,16,0",
            "0,0,0 true true",
            "buffer TypeError TypeError TypeError TypeError TypeError TypeError",
            "byteLength TypeError TypeError TypeError TypeError TypeError TypeError",
            "byteOffset TypeError TypeError TypeError TypeError TypeError TypeError",
            "length TypeError TypeError TypeError TypeError TypeError TypeError",
            "true true 42 undefined",
            "1,2,2",
            "3,6,2 true",
        ])
    );
}

/// MC-w26-4 (route R:100, check note b4): an Array hole read through a prototype that is a
/// length-tracking Int8Array on a resizable buffer, in a function warmed on a plain holey
/// array first (the no-elements-prototypes fuse, `tests/fuse.rs` way 7): the read sees the
/// view's element once the buffer grows, and a hole again once it shrinks. Then the [[Set]]
/// line: a store through a TypedArray prototype with another receiver (`sec-typedarray-set`)
/// adds no own property for an index out of the view and writes the view for an index in it.
#[test]
fn a_view_prototype_answers_a_hole_read() {
    let src = r#"
// MC-w26-4 (check note b4): a hole read through a length-tracking Int8Array prototype on a
// resizable buffer, in a function warmed on a plain holey array first.
function read(a) { return a[0]; }
const plain = [, 1];
let sum = 0;
for (let i = 0; i < 200; i++) sum += read(plain) === undefined ? 1 : 0;
const b = new ArrayBuffer(0, { maxByteLength: 8 });
const a = [, 1];
Object.setPrototypeOf(a, new Int8Array(b));
print(sum, read(a), 0 in a);
b.resize(1);
print(read(a), 0 in a);
new Int8Array(b)[0] = 5;
print(read(a), read(plain));
b.resize(0);
print(read(a), 0 in a);
// The [[Set]] line: a TypedArray on the prototype chain, another receiver.
const o = Object.create(new Int8Array(0));
o[0] = 1;
print(Object.hasOwn(o, "0"), o[0]);
const p = Object.create(new Int8Array(1));
p[0] = 7;
print(Object.hasOwn(p, "0"), p[0], Object.getPrototypeOf(p)[0]);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "200 undefined false",
            "0 true",
            "5 undefined",
            "undefined false",
            "false undefined",
            "true 7 0",
        ])
    );
}

/// `evalIn(global, source)`: the sloppy script `source`, compiled here and run in the realm of
/// `global` (its global declarations go there).
fn eval_in<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let Some(global) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("evalIn: no global object"));
    };
    let source = cx.to_rust_string(args.get(1))?;
    let unit = ep9_frontend::compile(
        &source,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .map_err(|e| cx.throw_type_error(&format!("evalIn: compile error: {e:?}")))?;
    cx.with_realm(global, |cx| cx.run_script(unit, "evalIn.js"))
}

/// [`run_realms`] with the host function `evalIn` on the first realm too.
fn run_realms_eval(src: &'static str, stress: bool) -> Result<Vec<String>, String> {
    common::on_big_stack(move || {
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .map_err(|e| format!("compile error: {e:?}"))?;
        let mut iso = Isolate::new(IsolateConfig {
            heap: HeapConfig {
                gc_stress: u32::from(stress),
                verify_after_gc: stress,
                ..HeapConfig::default()
            },
            builtins: Some(&ep9_builtins::TABLE),
            ..IsolateConfig::default()
        })
        .expect("an isolate");
        for (name, length, f) in HOST {
            iso.register_host_function(name, length, f);
        }
        iso.register_host_function("evalIn", 2, eval_in);
        OUT.with(|o| o.borrow_mut().clear());
        let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
        let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
        iso.collect_garbage();
        iso.verify_heap().expect("a valid heap");
        r.map(|_| out).map_err(|e| e.message)
    })
}

/// A subclass (`class X extends Int8Array`) makes a view of its own prototype; a NewTarget of
/// another realm whose `prototype` is not an object gives that realm's `%<T>Array.prototype%`
/// (GetFunctionRealm); a constructor of another realm makes its buffer in its own realm
/// (`%ArrayBuffer%` of the current realm), also with a NewTarget of this realm; a view on a
/// buffer of another realm keeps that buffer. (node: `createRealm()` is
/// `vm.runInContext("globalThis", ctx)`, `evalIn(g, s)` is `vm.runInContext(s, ctx)`.)
#[test]
fn subclasses_and_realms_choose_the_prototype_and_the_buffer() {
    let src = r#"
class X extends Int8Array {
  constructor(...a) { super(...a); this.tag = "x"; }
  m() { return this.length; }
}
const x = new X([1, 2, 3]);
print(x instanceof X, x instanceof Int8Array, Object.getPrototypeOf(x) === X.prototype,
  x.m(), x.tag, X.BYTES_PER_ELEMENT, Object.getPrototypeOf(X) === Int8Array, Array.prototype.join.call(x));
class Y extends Float64Array {}
const y = new Y(new ArrayBuffer(16), 8);
print(y.length, y.byteOffset, y.constructor === Y);
const other = createRealm();
evalIn(other, "var C = function () {};");
const C = other.C;
C.prototype = null;
const t = Reflect.construct(Int8Array, [], C);
print(Object.getPrototypeOf(t) === other.Int8Array.prototype, t.buffer.constructor === ArrayBuffer);
const u = Reflect.construct(Uint8Array, [new ArrayBuffer(2)], C);
print(Object.getPrototypeOf(u) === other.Uint8Array.prototype);
const w = new other.Int16Array(2);
print(Object.getPrototypeOf(w) === other.Int16Array.prototype, w.buffer.constructor === other.ArrayBuffer,
  w.buffer.constructor === ArrayBuffer);
const z = Reflect.construct(other.Int16Array, [new Int8Array([1, 2])], Int16Array);
print(Object.getPrototypeOf(z) === Int16Array.prototype, z.buffer.constructor === other.ArrayBuffer,
  Array.prototype.join.call(z));
const ob = new other.ArrayBuffer(4);
const v = new Int8Array(ob, 1);
print(v.length, v.buffer === ob);
try { other.Int8Array(1); } catch (e) { print(e.constructor === other.TypeError); }
"#;
    let want = lines(&[
        "true true true 3 x 1 true 1,2,3",
        "1 8 true",
        "true true",
        "true",
        "true true false",
        "true true 1,2",
        "3 true",
        "true",
    ]);
    let plain = run_realms_eval(src, false);
    assert_eq!(plain, want);
    assert_eq!(run_realms_eval(src, true), plain, "GC stress");
}
