//! TypedArray objects at the JS level (TypedArray TA-3 TA-CTORS; `src/typed_array/`; ECMA-262
//! §23.2). R1 (the skeleton) pins the key list of each of the 26 intrinsics, the lazy group
//! (the 12 constructor keys join the group of `ArrayBuffer`, TA-D14), the `[[Prototype]]`
//! links of the constructors (the VM's `PROTO_LINKS`, decision O2), in a second realm too,
//! and the stub TypeError of `from` and `of` that R3 replaces. Each script prints the lines
//! of node v26.13.0 for the same script (`print` here, `console.log` there; `createRealm()`
//! there is `vm.runInContext('globalThis', vm.createContext({}))` with `print` copied in) and runs
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

// ---- (d) the stubs (R3 replaces them) -----------------------------------------------------------

/// TA-3 R2 built the constructors and `%TypedArray%` (an abstract class: a TypeError with and
/// without `new`); `%TypedArray%.from` and `of` stay stubs that throw a TypeError until R3
/// (node: `new Int8Array(1)` makes a view of length 1; `TA.from([])` and `TA.of(1)` make
/// views).
#[test]
fn the_constructors_are_built_and_from_and_of_are_stubs() {
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
            "true TypedArray.from: not built yet (m8-stub, TypedArray TA-3)",
            "true TypedArray.of: not built yet (m8-stub, TypedArray TA-3)",
        ])
    );
}
