//! ArrayBuffer, SharedArrayBuffer and DataView at the JS level (TypedArray TA-2 TA-BUFFERS;
//! `src/array_buffer.rs`, `src/data_view.rs`; ECMA-262 §25.1-§25.3). Each script prints the
//! lines of node v26.13.0 for the same script (`print` here, `console.log` there) and runs with
//! and without GC stress (`common::run_both`). A user call (a valueOf, a `prototype` getter, a
//! species constructor) detaches a buffer through `transfer()` (the `$262.detachArrayBuffer`
//! hook is TA-HARNESS's). The Rust tests below pin the key lists of the six intrinsics, the
//! lazy group of the keys `ArrayBuffer`, `SharedArrayBuffer` and `DataView` (TA-D14), and the
//! brand: an object of a buffer class that the generic slot API made is no buffer here.

mod common;

use std::hint::black_box;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_vm::abi::ObjectClass;
use ep9_vm::{Cx, IntrinsicId, Isolate, JsResult, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

// ---- the key lists (the e2d4f004 form) -------------------------------------------------------

/// The own keys of `o` in [[OwnPropertyKeys]] order (a symbol as its descriptive string).
fn keys(cx: &mut Cx<'_>, id: IntrinsicId) -> JsResult<String> {
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
    Ok(texts.join(","))
}

/// The exact key list of each intrinsic of TA-2, read through `Cx::intrinsic` (which installs
/// the lazy group first): node v26.13.0's `Reflect.ownKeys` of the same object.
#[test]
fn each_intrinsic_has_the_exact_key_list() {
    let want = [
        (
            IntrinsicId::ArrayBuffer,
            "length,name,prototype,isView,Symbol(Symbol.species)",
        ),
        (
            IntrinsicId::ArrayBufferPrototype,
            "constructor,byteLength,slice,maxByteLength,resizable,resize,transfer,\
             transferToFixedLength,detached,Symbol(Symbol.toStringTag)",
        ),
        (
            IntrinsicId::SharedArrayBuffer,
            "length,name,prototype,Symbol(Symbol.species)",
        ),
        (
            IntrinsicId::SharedArrayBufferPrototype,
            "constructor,byteLength,slice,maxByteLength,growable,grow,Symbol(Symbol.toStringTag)",
        ),
        (IntrinsicId::DataView, "length,name,prototype"),
        (
            IntrinsicId::DataViewPrototype,
            "constructor,buffer,byteLength,byteOffset,getInt8,setInt8,getUint8,setUint8,getInt16,\
             setInt16,getUint16,setUint16,getInt32,setInt32,getUint32,setUint32,getFloat16,\
             setFloat16,getFloat32,setFloat32,getFloat64,setFloat64,getBigInt64,setBigInt64,\
             getBigUint64,setBigUint64,Symbol(Symbol.toStringTag)",
        ),
    ];
    for stress in [false, true] {
        let got = common::on_big_stack(move || {
            common::isolate(stress).with_cx(|cx| {
                let mut out = Vec::new();
                for id in [
                    IntrinsicId::ArrayBuffer,
                    IntrinsicId::ArrayBufferPrototype,
                    IntrinsicId::SharedArrayBuffer,
                    IntrinsicId::SharedArrayBufferPrototype,
                    IntrinsicId::DataView,
                    IntrinsicId::DataViewPrototype,
                ] {
                    out.push((id, keys(cx, id)?));
                }
                Ok(out)
            })
        })
        .expect("no exception");
        let want: Vec<(IntrinsicId, String)> =
            want.iter().map(|&(id, k)| (id, k.to_string())).collect();
        assert_eq!(got, want, "stress {stress}");
    }
}

// ---- the lazy group (TA-D14) -----------------------------------------------------------------

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

/// Realm init makes no object of the group: a script that reads `ArrayBuffer` once leaves more
/// live cells than one that reads nothing, by at least one cell per definition and intrinsic
/// of the two modules. A read of any of the three keys (also `typeof`) and a use of a group
/// intrinsic from Rust install the whole group (the same cells as the `ArrayBuffer` read);
/// `Cx::has_intrinsic` answers true for each group intrinsic and installs nothing.
#[test]
fn the_three_keys_are_one_lazy_group() {
    let size: usize = [
        ep9_builtins::array_buffer::MODULE,
        ep9_builtins::data_view::MODULE,
    ]
    .iter()
    .map(|m| m.defs.len() + m.intrinsics.len())
    .sum();
    let base = min_cells("", nothing);
    let group = min_cells("ArrayBuffer;", nothing).saturating_sub(base);
    assert!(
        group >= size,
        "the group adds {group} cells, fewer than its {size} definitions and intrinsics"
    );
    // The installs of the group that a run made, rounded.
    let installs = |live: usize| (live.saturating_sub(base) + group / 2) / group;
    for src in [
        "SharedArrayBuffer;",
        "DataView;",
        "typeof DataView;",
        "new DataView(new ArrayBuffer(1));",
    ] {
        assert_eq!(installs(min_cells(src, nothing)), 1, "{src}");
    }
    fn has(cx: &mut Cx<'_>) -> JsResult<()> {
        for id in [
            IntrinsicId::ArrayBuffer,
            IntrinsicId::ArrayBufferPrototype,
            IntrinsicId::SharedArrayBuffer,
            IntrinsicId::SharedArrayBufferPrototype,
            IntrinsicId::DataView,
            IntrinsicId::DataViewPrototype,
        ] {
            assert!(cx.has_intrinsic(id), "{}", id.name());
        }
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", has)),
        0,
        "has_intrinsic installs nothing"
    );
    fn use_one(cx: &mut Cx<'_>) -> JsResult<()> {
        black_box(cx.intrinsic(IntrinsicId::DataViewPrototype));
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", use_one)),
        1,
        "Cx::intrinsic installs"
    );
}

// ---- the brand (BRAND-T; judge 4.7) ----------------------------------------------------------

/// An object of the class ArrayBuffer, SharedArrayBuffer or DataView that the generic slot API
/// makes (with a slot count and slot values of a real one, and the real prototype) is no
/// buffer and no DataView for the builtins: each getter, method and constructor that takes a
/// buffer or a DataView throws a TypeError, and `ArrayBuffer.isView` is false.
#[test]
fn a_caller_object_of_a_buffer_class_is_no_buffer() {
    for stress in [false, true] {
        let out = common::on_big_stack(move || {
            let src = r#"
var out = [];
function t(f) { try { out.push(String(f())); } catch (e) { out.push(e.constructor.name); } }
function getter(o, k) { return Object.getOwnPropertyDescriptor(o, k).get; }
var AP = ArrayBuffer.prototype, SP = SharedArrayBuffer.prototype, DP = DataView.prototype;
['byteLength', 'maxByteLength', 'resizable', 'detached'].forEach(function (k) { t(function () { return getter(AP, k).call(fakeAb); }); });
['slice', 'resize', 'transfer', 'transferToFixedLength'].forEach(function (k) { t(function () { return AP[k].call(fakeAb, 1); }); });
['byteLength', 'maxByteLength', 'growable'].forEach(function (k) { t(function () { return getter(SP, k).call(fakeSab); }); });
['slice', 'grow'].forEach(function (k) { t(function () { return SP[k].call(fakeSab, 1); }); });
t(function () { return new DataView(fakeAb); });
t(function () { return new DataView(fakeSab); });
['buffer', 'byteLength', 'byteOffset'].forEach(function (k) { t(function () { return getter(DP, k).call(fakeDv); }); });
t(function () { return DP.getInt8.call(fakeDv, 0); });
t(function () { return DP.setFloat64.call(fakeDv, 0, 1); });
t(function () { return ArrayBuffer.isView(fakeDv); });
t(function () { return fakeAb instanceof ArrayBuffer; });
out = out.join();
"#;
            let unit = ep9_frontend::compile(
                src,
                SourceOpts::script(Strictness::Sloppy),
                CompileOptions::default(),
            )
            .expect("the script compiles");
            let mut iso = common::isolate(stress);
            iso.with_cx(|cx| {
                let global = cx.global();
                let ap = cx.intrinsic(IntrinsicId::ArrayBufferPrototype);
                let sp = cx.intrinsic(IntrinsicId::SharedArrayBufferPrototype);
                let dp = cx.intrinsic(IntrinsicId::DataViewPrototype);
                // A real buffer's slot values: the store, byteLength 8, maxByteLength 16, no
                // detach key, flags (resizable), a table index 0.
                let real = cx.allocate_array_buffer(ap, 8, Some(16))?;
                let fake_ab = cx.new_object_with_slots(ap, ObjectClass::ArrayBuffer, 6);
                let fake_sab = cx.new_object_with_slots(sp, ObjectClass::SharedArrayBuffer, 6);
                let fake_dv = cx.new_object_with_slots(dp, ObjectClass::DataView, 5);
                for (o, class) in [
                    (fake_ab, ObjectClass::ArrayBuffer),
                    (fake_sab, ObjectClass::SharedArrayBuffer),
                ] {
                    cx.set_slot(o, class, 1, Local::number(8.0))?;
                    cx.set_slot(o, class, 2, Local::number(16.0))?;
                    cx.set_slot(o, class, 4, Local::int32(2))?;
                    cx.set_slot(o, class, 5, Local::int32(0))?;
                }
                cx.set_slot(fake_dv, ObjectClass::DataView, 0, real)?;
                cx.set_slot(fake_dv, ObjectClass::DataView, 1, Local::int32(0))?;
                cx.set_slot(fake_dv, ObjectClass::DataView, 2, Local::int32(8))?;
                cx.set_slot(fake_dv, ObjectClass::DataView, 4, Local::int32(0))?;
                cx.set(global, "fakeAb", fake_ab, true)?;
                cx.set(global, "fakeSab", fake_sab, true)?;
                cx.set(global, "fakeDv", fake_dv, true)?;
                Ok(())
            })
            .expect("no exception");
            iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()))
                .expect("no exception");
            let out = common::global_text(&mut iso, "out");
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            out
        });
        let te = "TypeError";
        let mut want = vec![te; 4 + 4 + 3 + 2 + 2 + 3 + 2];
        want.extend(["false", "true"]);
        assert_eq!(out, Some(want.join(",")), "stress {stress}");
    }
}

// ---- the scripts --------------------------------------------------------------------------------

const AB_CTOR_JS: &str = r#"
function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name); } }
var log = [];
function v(name, x) { return { valueOf() { log.push(name); return x; } }; }
t(function () { return ArrayBuffer(1); });
t(function () { var b = new ArrayBuffer(); return [b.byteLength, b.maxByteLength, b.resizable, b.detached].join(); });
t(function () { return new ArrayBuffer(-1); });
t(function () { return new ArrayBuffer(2 ** 53); });
t(function () { return new ArrayBuffer(1.9).byteLength + ' ' + new ArrayBuffer('3').byteLength + ' ' + new ArrayBuffer(-0.5).byteLength; });
t(function () { return new ArrayBuffer(4, { maxByteLength: 2 }); });
t(function () { return new ArrayBuffer(4, { maxByteLength: -1 }); });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: undefined }); return b.resizable; });
t(function () { var b = new ArrayBuffer(4, 8); return b.resizable; });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); return [b.byteLength, b.maxByteLength, b.resizable].join(); });
// The order: ToIndex(length), then options.maxByteLength (Get, then ToIndex), then the RangeError, then prototype.
log = [];
t(function () {
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { log.push('proto'); return ArrayBuffer.prototype; } });
  return Reflect.construct(ArrayBuffer, [v('len', 4), { get maxByteLength() { log.push('get max'); return v('max', 2); } }], nt);
});
print(log.join());
log = [];
t(function () {
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { log.push('proto'); return null; } });
  var b = Reflect.construct(ArrayBuffer, [v('len', 4), { get maxByteLength() { log.push('get max'); return v('max', 8); } }], nt);
  return Object.getPrototypeOf(b) === ArrayBuffer.prototype;
});
print(log.join());
// A prototype from another value.
t(function () { class B extends ArrayBuffer {} var b = new B(2); return [b instanceof B, b.byteLength, Object.prototype.toString.call(b)].join(); });
t(function () { return [ArrayBuffer.isView(new DataView(new ArrayBuffer(1))), ArrayBuffer.isView(new ArrayBuffer(1)), ArrayBuffer.isView({}), ArrayBuffer.isView(), ArrayBuffer.isView(1)].join(); });
t(function () { return ArrayBuffer[Symbol.species] === ArrayBuffer && SharedArrayBuffer[Symbol.species] === SharedArrayBuffer; });
t(function () { var g = Object.getOwnPropertyDescriptor(ArrayBuffer, Symbol.species).get; return [g.name, g.length, g.call(7)].join(); });
"#;

/// The ArrayBuffer constructor (§25.1.4.1): NewTarget, ToIndex of the length, the options bag
/// (`maxByteLength`: a Get, then ToIndex), the RangeError of a length above the maximum before
/// the `prototype` read, a subclass, `ArrayBuffer.isView` and `@@species`.
#[test]
fn the_array_buffer_constructor_follows_the_spec_order() {
    assert_eq!(
        common::run_both(AB_CTOR_JS),
        lines(&[
            "TypeError",
            "0,0,false,false",
            "RangeError",
            "RangeError",
            "1 3 0",
            "RangeError",
            "RangeError",
            "false",
            "false",
            "4,8,true",
            "RangeError",
            "len,get max,max",
            "true",
            "len,get max,max,proto",
            "true,2,[object ArrayBuffer]",
            "true,false,false,false,false",
            "true",
            "get [Symbol.species],0,7",
        ])
    );
}

const AB_PROTO_JS: &str = r#"
function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name); } }
var log = [];
var AP = ArrayBuffer.prototype, SP = SharedArrayBuffer.prototype;
function getter(o, k) { return Object.getOwnPropertyDescriptor(o, k).get; }
// IsSharedArrayBuffer: each ArrayBuffer method and getter on a SAB throws, and the other way.
var sab = new SharedArrayBuffer(4, { maxByteLength: 8 });
var ab = new ArrayBuffer(4, { maxByteLength: 8 });
['byteLength', 'maxByteLength', 'resizable', 'detached'].forEach(function (k) { t(function () { return getter(AP, k).call(sab); }); });
['slice', 'resize', 'transfer', 'transferToFixedLength'].forEach(function (k) { t(function () { return AP[k].call(sab, 1); }); });
['byteLength', 'maxByteLength', 'growable'].forEach(function (k) { t(function () { return getter(SP, k).call(ab); }); });
['slice', 'grow'].forEach(function (k) { t(function () { return SP[k].call(ab, 1); }); });
t(function () { return getter(AP, 'byteLength').call({}); });
t(function () { return getter(AP, 'byteLength').call(new DataView(ab)); });
// resize: the brand and resizable checks come before ToIndex; the detach check after it.
t(function () { return new ArrayBuffer(4).resize({ valueOf() { log.push('fixed'); return 1; } }); });
print(log.join()); log = [];
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); return b.resize({ valueOf() { b.transfer(); return 2; } }); });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); return b.resize(9); });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); return b.resize(-1); });
t(function () {
  var b = new ArrayBuffer(4, { maxByteLength: 8 }); var u = new DataView(b);
  u.setUint8(3, 7); b.resize(2); b.resize(8);
  return [b.byteLength, u.getUint8(1), u.getUint8(3), u.byteLength, b.resize(8)].join();
});
// slice: ranges, species, the checks of the result, a detach in a valueOf.
t(function () {
  var b = new ArrayBuffer(8); var d = new DataView(b); for (var i = 0; i < 8; i++) d.setUint8(i, i + 1);
  function bytes(x) { var r = []; var e = new DataView(x); for (var i = 0; i < x.byteLength; i++) r.push(e.getUint8(i)); return r.join(''); }
  return [bytes(b.slice()), bytes(b.slice(2)), bytes(b.slice(-3)), bytes(b.slice(2, -2)), bytes(b.slice(5, 2)), bytes(b.slice(-Infinity, Infinity)), bytes(b.slice(NaN, 3))].join(' ');
});
t(function () { var b = new ArrayBuffer(8); return b.slice({ valueOf() { b.transfer(); return 0; } }); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { return b; } }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { return new ArrayBuffer(n - 1); } }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { return new SharedArrayBuffer(n); } }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { return {}; } }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { var x = new ArrayBuffer(n); x.transfer(); return x; } }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { b.transfer(); return new ArrayBuffer(n); } }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: function (n) { log.push('n=' + n); return new ArrayBuffer(n + 2); } }; return b.slice(1, 4).byteLength; });
print(log.join()); log = [];
t(function () {
  // The source shrinks in the species constructor: only the bytes that are left are copied.
  var b = new ArrayBuffer(8, { maxByteLength: 8 }); var d = new DataView(b); for (var i = 0; i < 8; i++) d.setUint8(i, i + 1);
  b.constructor = { [Symbol.species]: function (n) { b.resize(3); return new ArrayBuffer(n); } };
  var r = b.slice(1, 6); var e = new DataView(r); var out = []; for (var i = 0; i < r.byteLength; i++) out.push(e.getUint8(i));
  return out.join('');
});
t(function () { var b = new ArrayBuffer(8); b.constructor = 1; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: 1 }; return b.slice(0); });
t(function () { var b = new ArrayBuffer(8); b.constructor = { [Symbol.species]: null }; return Object.getPrototypeOf(b.slice(0)) === AP; });
// transfer and transferToFixedLength.
t(function () {
  var b = new ArrayBuffer(4, { maxByteLength: 16 }); new DataView(b).setUint8(3, 9);
  var c = b.transfer(); var f = c.transferToFixedLength(6);
  return [b.detached, b.byteLength, b.maxByteLength, b.resizable, c.detached, c.byteLength, c.maxByteLength, c.resizable, f.byteLength, f.resizable, f.maxByteLength, new DataView(f).getUint8(3), new DataView(f).getUint8(5)].join();
});
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 16 }); return b.transfer(17); });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 16 }); var c = b.transfer(10); return [c.byteLength, c.maxByteLength, b.detached].join(); });
t(function () { var b = new ArrayBuffer(4); var c = b.transfer(2); return [c.byteLength, c.resizable, b.detached].join(); });
t(function () { var b = new ArrayBuffer(4); b.transfer(); return b.transfer(); });
t(function () { var b = new ArrayBuffer(4); return b.transfer({ valueOf() { b.transfer(); return 1; } }); });
t(function () { var b = new ArrayBuffer(4); b.transfer(); return b.transferToFixedLength(-1); });
t(function () { var b = new ArrayBuffer(4); return b.transfer(-1); });
t(function () { class B extends ArrayBuffer {} var c = new B(4).transfer(); return Object.getPrototypeOf(c) === AP; });
t(function () { var b = new ArrayBuffer(4); b.transfer(); return [b.byteLength, b.maxByteLength, b.resizable, b.detached, Object.prototype.toString.call(b)].join(); });
t(function () { var b = new ArrayBuffer(4); b.transfer(); return b.slice(0); });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); b.transfer(); return b.resize(1); });
"#;

/// The methods and getters of `ArrayBuffer.prototype` and `SharedArrayBuffer.prototype` on the
/// other kind of buffer (IsSharedArrayBuffer: a TypeError each), `resize` (the brand and
/// resizable checks before ToIndex, the detach check after it, the bounds), `slice` (the ranges,
/// the species constructor and each check of its result, a detach or a shrink in user code),
/// `transfer` (it keeps resizability) and `transferToFixedLength`, and a detached buffer.
#[test]
fn array_buffer_prototype_methods_check_the_brand_and_detach() {
    assert_eq!(
        common::run_both(AB_PROTO_JS),
        lines(&[
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "",
            "TypeError",
            "RangeError",
            "RangeError",
            "8,0,0,8,",
            "12345678 345678 678 3456  12345678 123",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "5",
            "n=3",
            "23000",
            "TypeError",
            "TypeError",
            "true",
            "true,0,0,true,true,0,0,true,6,false,6,9,0",
            "RangeError",
            "10,16,true",
            "2,false,true",
            "TypeError",
            "TypeError",
            "RangeError",
            "RangeError",
            "true",
            "0,0,false,true,[object ArrayBuffer]",
            "TypeError",
            "TypeError",
        ])
    );
}

const SAB_JS: &str = r#"
function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name); } }
var log = [];
function v(name, x) { return { valueOf() { log.push(name); return x; } }; }
t(function () { return SharedArrayBuffer(1); });
t(function () { var s = new SharedArrayBuffer(3); return [s.byteLength, s.maxByteLength, s.growable, Object.prototype.toString.call(s)].join(); });
t(function () { return new SharedArrayBuffer(4, { maxByteLength: 3 }); });
t(function () { return new SharedArrayBuffer(-1); });
t(function () {
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { log.push('proto'); return SharedArrayBuffer.prototype; } });
  return Reflect.construct(SharedArrayBuffer, [v('len', 4), { get maxByteLength() { log.push('get max'); return v('max', 2); } }], nt);
});
print(log.join()); log = [];
t(function () { var s = new SharedArrayBuffer(4, { maxByteLength: 8 }); s.grow(4); s.grow(7); return [s.byteLength, s.maxByteLength, s.growable].join(); });
t(function () { var s = new SharedArrayBuffer(4, { maxByteLength: 8 }); return s.grow(3); });
t(function () { var s = new SharedArrayBuffer(4, { maxByteLength: 8 }); return s.grow(9); });
t(function () { var s = new SharedArrayBuffer(4); return s.grow({ valueOf() { log.push('fixed'); return 4; } }); });
print(log.join()); log = [];
t(function () {
  var s = new SharedArrayBuffer(4, { maxByteLength: 8 }); var d = new DataView(s); var fixed = new DataView(s, 1, 2);
  d.setUint8(3, 5); s.grow(8);
  return [d.byteLength, d.getUint8(3), d.getUint8(7), fixed.byteLength].join();
});
t(function () {
  var s = new SharedArrayBuffer(6); var d = new DataView(s); for (var i = 0; i < 6; i++) d.setUint8(i, i + 1);
  var r = s.slice(1, -1); var e = new DataView(r); var out = []; for (var i = 0; i < r.byteLength; i++) out.push(e.getUint8(i));
  return [out.join(''), r.growable, Object.getPrototypeOf(r) === SharedArrayBuffer.prototype].join();
});
t(function () { var s = new SharedArrayBuffer(6); s.constructor = { [Symbol.species]: function (n) { return s; } }; return s.slice(); });
t(function () { var s = new SharedArrayBuffer(6); s.constructor = { [Symbol.species]: function (n) { return new SharedArrayBuffer(n - 1); } }; return s.slice(); });
t(function () { var s = new SharedArrayBuffer(6); s.constructor = { [Symbol.species]: function (n) { return new ArrayBuffer(n); } }; return s.slice(); });
t(function () { var s = new SharedArrayBuffer(6); s.constructor = { [Symbol.species]: function (n) { return new SharedArrayBuffer(n + 3); } }; return s.slice(2).byteLength; });
t(function () { return Object.getOwnPropertyDescriptor(SharedArrayBuffer.prototype, 'growable').get.call(new SharedArrayBuffer(1, { maxByteLength: 2 })); });
"#;

/// SharedArrayBuffer (§25.2): the constructor order, `grow` (it never shrinks; the maximum), a
/// length-tracking DataView after a grow, `slice` and the checks of its species result.
#[test]
fn shared_array_buffer_grow_and_slice() {
    assert_eq!(
        common::run_both(SAB_JS),
        lines(&[
            "TypeError",
            "3,3,false,[object SharedArrayBuffer]",
            "RangeError",
            "RangeError",
            "RangeError",
            "len,get max,max",
            "7,8,true",
            "RangeError",
            "RangeError",
            "TypeError",
            "",
            "8,5,0,2",
            "2345,false,true",
            "TypeError",
            "TypeError",
            "TypeError",
            "7",
            "true",
        ])
    );
}

const DV_JS: &str = r#"
function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name); } }
var log = [];
function v(name, x) { return { valueOf() { log.push(name); return x; } }; }
var DP = DataView.prototype;
function getter(k) { return Object.getOwnPropertyDescriptor(DP, k).get; }
t(function () { return DataView(new ArrayBuffer(1)); });
t(function () { return new DataView({}); });
t(function () { return new DataView(new ArrayBuffer(4), -1); });
t(function () { return new DataView(new ArrayBuffer(4), 5); });
t(function () { return new DataView(new ArrayBuffer(4), 4).byteLength; });
t(function () { return new DataView(new ArrayBuffer(4), 1, 4); });
t(function () { var d = new DataView(new ArrayBuffer(4), 1); return [d.byteOffset, d.byteLength, d.buffer instanceof ArrayBuffer].join(); });
// The order: buffer brand, ToIndex(byteOffset), the detach check, the offset check, ToIndex(byteLength), the length check, prototype, then the checks again.
t(function () { return new DataView(1, v('off', 0)); });
print(log.join()); log = [];
t(function () { var b = new ArrayBuffer(4); return new DataView(b, { valueOf() { b.transfer(); return 0; } }); });
t(function () { var b = new ArrayBuffer(4); return new DataView(b, v('off', 5), v('len', 0)); });
print(log.join()); log = [];
t(function () { var b = new ArrayBuffer(4); return new DataView(b, v('off', 1), v('len', 4)); });
print(log.join()); log = [];
t(function () {
  var b = new ArrayBuffer(4);
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { log.push('proto'); b.transfer(); return DP; } });
  return Reflect.construct(DataView, [b, v('off', 1), v('len', 2)], nt);
});
print(log.join()); log = [];
t(function () {
  var b = new ArrayBuffer(8, { maxByteLength: 16 });
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { b.resize(2); return DP; } });
  return Reflect.construct(DataView, [b, 1, 2], nt);
});
t(function () {
  var b = new ArrayBuffer(8, { maxByteLength: 16 });
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { b.resize(2); return DP; } });
  return Reflect.construct(DataView, [b, 3], nt);
});
t(function () {
  var b = new ArrayBuffer(8, { maxByteLength: 16 });
  var nt = function () {}.bind();
  Object.defineProperty(nt, 'prototype', { get() { b.resize(4); return DP; } });
  var d = Reflect.construct(DataView, [b, 1], nt); return d.byteLength;
});
// Every type, both byte orders, against bytes written one by one.
t(function () {
  var b = new ArrayBuffer(16); var d = new DataView(b);
  var out = [];
  [['Int8', 1], ['Uint8', 1], ['Int16', 2], ['Uint16', 2], ['Int32', 4], ['Uint32', 4], ['Float16', 2], ['Float32', 4], ['Float64', 8], ['BigInt64', 8], ['BigUint64', 8]].forEach(function (p) {
    for (var i = 0; i < 16; i++) d.setUint8(i, 0x81 + 7 * i);
    out.push(p[0] + ':' + d['get' + p[0]](3) + '/' + d['get' + p[0]](3, true));
  });
  return out.join(' ');
});
t(function () {
  var b = new ArrayBuffer(8); var d = new DataView(b);
  function hex() { var s = ''; for (var i = 0; i < 8; i++) s += d.getUint8(i).toString(16).padStart(2, '0'); return s; }
  var out = [];
  d.setInt16(0, -2); d.setInt16(2, -2, true); out.push(hex());
  d.setUint32(0, 0xdeadbeef); d.setInt32(4, 0xdeadbeef, true); out.push(hex());
  d.setFloat64(0, -0); out.push(hex() + ' ' + Object.is(d.getFloat64(0), -0));
  d.setFloat64(0, NaN); out.push(hex());
  d.setFloat32(0, 1 / 3); out.push(hex() + ' ' + d.getFloat32(0));
  d.setFloat16(0, 1 / 3); out.push(hex() + ' ' + d.getFloat16(0));
  d.setFloat16(0, 65520); out.push(d.getFloat16(0));
  d.setFloat16(0, 65519.99); out.push(d.getFloat16(0));
  d.setFloat16(0, 5.960464477539063e-8 / 2); out.push(d.getFloat16(0));
  d.setFloat16(0, 5.960464477539063e-8 * 1.5); out.push(d.getFloat16(0));
  d.setFloat16(2, NaN); out.push(Number.isNaN(d.getFloat16(2)));
  d.setBigInt64(0, -1n); out.push(d.getBigUint64(0) + ' ' + d.getBigInt64(0, true));
  d.setBigUint64(0, 2n ** 64n + 5n, true); out.push(d.getBigInt64(0, true));
  d.setUint8(0, 300); d.setInt8(1, 200); out.push(d.getUint8(0) + ' ' + d.getInt8(1));
  d.setUint16(0, -1.5); out.push(d.getUint16(0));
  return out.join(' | ');
});
// The conversions: ToIndex first, then the value, then ToBoolean; the bounds after them.
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.setInt8(v('i', 0), v('x', 1)); });
print(log.join()); log = [];
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.setInt8(v('i', 9), v('x', 1)); });
print(log.join()); log = [];
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.setInt32(1, 0); });
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.getInt32(-1); });
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.getInt32(1); });
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.getInt8(4); });
t(function () { var d = new DataView(new ArrayBuffer(4)); return d.getInt8(2 ** 53); });
t(function () { var d = new DataView(new ArrayBuffer(8)); return d.setBigInt64(0, 1); });
t(function () { var d = new DataView(new ArrayBuffer(8)); return d.setFloat64(0, 1n); });
t(function () { var d = new DataView(new ArrayBuffer(8)); d.setBigInt64(0, { valueOf() { return 3n; } }); return d.getBigInt64(0); });
t(function () { var d = new DataView(new ArrayBuffer(8)); return d.setUint8(0, Symbol()); });
t(function () { var b = new ArrayBuffer(4); var d = new DataView(b); return d.setUint8(0, { valueOf() { b.transfer(); return 1; } }); });
t(function () { var b = new ArrayBuffer(4); var d = new DataView(b); return d.getUint8({ valueOf() { b.transfer(); return 0; } }); });
t(function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); var d = new DataView(b); return d.setUint32(0, { valueOf() { b.resize(2); return 1; } }); });
t(function () { return DP.getInt8.call({}, 0); });
t(function () { return DP.setInt8.call(new ArrayBuffer(4), 0, 0); });
t(function () { return DP.getInt8.call(new DataView(new ArrayBuffer(1)), { valueOf() { log.push('never'); return 0; } }); });
print(log.join()); log = [];
t(function () { return DP.getInt8.call({}, { valueOf() { log.push('index'); return 0; } }); });
print(log.join()); log = [];
// The getters: detached and out of bounds, length tracking.
t(function () { var b = new ArrayBuffer(4); var d = new DataView(b, 1); b.transfer(); return d.buffer === b; });
t(function () { var b = new ArrayBuffer(4); var d = new DataView(b, 1); b.transfer(); return d.byteLength; });
t(function () { var b = new ArrayBuffer(4); var d = new DataView(b, 1); b.transfer(); return d.byteOffset; });
t(function () { var b = new ArrayBuffer(4); var d = new DataView(b, 1); b.transfer(); return d.getInt8(0); });
t(function () {
  var b = new ArrayBuffer(8, { maxByteLength: 16 });
  var auto = new DataView(b, 2), fixed = new DataView(b, 2, 4), out = [];
  [8, 16, 6, 2, 1, 12].forEach(function (n) {
    b.resize(n);
    var a = []; [auto, fixed].forEach(function (d) { try { a.push(d.byteLength + '@' + d.byteOffset); } catch (e) { a.push(e.constructor.name); } });
    try { auto.setUint8(0, n); a.push(auto.getUint8(0)); } catch (e) { a.push(e.constructor.name); }
    out.push(n + ':' + a.join('/'));
  });
  return out.join(' ');
});
t(function () { return [getter('buffer').call(1)].join(); });
t(function () { return getter('byteLength').call(new ArrayBuffer(1)); });
t(function () { return getter('byteOffset').call({}); });
t(function () { var d = new DataView(new ArrayBuffer(2)); d.x = 1; d[0] = 5; return Object.keys(d).join() + ' ' + d.getInt8(0) + ' ' + Object.prototype.toString.call(d); });
t(function () { class D extends DataView {} var d = new D(new ArrayBuffer(4), 1); return [d instanceof D, d.byteLength].join(); });
"#;

/// DataView (§25.3): the constructor checks in spec order (with a `prototype` getter that
/// detaches or shrinks the buffer), every get and set method in both byte orders (Float16
/// rounding and its edges, BigInt64 wrap, NaN), the order of the conversions and the bounds, a
/// detach or a shrink in a valueOf, the receiver checks, and the getters of a detached, an out
/// of bounds and a length-tracking view.
#[test]
fn data_view_constructor_getters_get_and_set() {
    assert_eq!(
        common::run_both(DV_JS),
        lines(&[
            "TypeError",
            "TypeError",
            "RangeError",
            "RangeError",
            "0",
            "RangeError",
            "1,3,true",
            "TypeError",
            "",
            "TypeError",
            "RangeError",
            "off",
            "RangeError",
            "off,len",
            "TypeError",
            "off,len,proto",
            "RangeError",
            "RangeError",
            "3",
            "Int8:-106/-106 Uint8:150/150 Int16:-26979/-25194 Uint16:38557/40342 Int32:-1768053589/-1415275114 Uint32:2526913707/2879692182 Float16:-0.0016145706176757812/-0.00545501708984375 Float32:-2.546863810042555e-25/-1.1696639750508053e-12 Float64:-9.681633311434108e-200/-4.446369343851778e+37 BigInt64:-7593732339331907385/-4053035487598436970 BigUint64:10853011734377644231/14393708586111114646",
            "fffefeff00000000 | deadbeefefbeadde | 8000000000000000 true | 7ff8000000000000 | 3eaaaaab00000000 0.3333333432674408 | 3555aaab00000000 0.333251953125 | Infinity | 65504 | 0 | 1.1920928955078125e-7 | true | 18446744073709551615 -1 | 5 | 44 -56 | 65535",
            "undefined",
            "i,x",
            "RangeError",
            "i,x",
            "RangeError",
            "RangeError",
            "RangeError",
            "RangeError",
            "RangeError",
            "TypeError",
            "TypeError",
            "3",
            "TypeError",
            "TypeError",
            "TypeError",
            "RangeError",
            "TypeError",
            "TypeError",
            "0",
            "never",
            "TypeError",
            "",
            "true",
            "TypeError",
            "TypeError",
            "TypeError",
            "8:6@2/4@2/8 16:14@2/4@2/16 6:4@2/4@2/6 2:0@2/TypeError/RangeError 1:TypeError/TypeError/TypeError 12:10@2/4@2/12",
            "TypeError",
            "TypeError",
            "TypeError",
            "0,x 0 [object DataView]",
            "true,3",
        ])
    );
}

const DESC_JS: &str = r#"
var keyText = function (k) { return typeof k === 'symbol' ? k.toString() : k; };
var desc = function (o) {
  return Reflect.ownKeys(o).map(function (k) {
    var d = Object.getOwnPropertyDescriptor(o, k);
    var kind = 'value' in d ? 'd' : 'a';
    var w = 'value' in d ? (d.writable ? 'w' : '-') : '-';
    var f = 'value' in d ? d.value : d.get;
    var extra = typeof f === 'function' ? ' ' + f.name + '/' + f.length : '';
    return keyText(k) + ':' + kind + w + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-') + extra;
  }).join(',');
};
[ArrayBuffer, ArrayBuffer.prototype, SharedArrayBuffer, SharedArrayBuffer.prototype, DataView, DataView.prototype].forEach(function (o) { print(desc(o)); });
['ArrayBuffer', 'SharedArrayBuffer', 'DataView'].forEach(function (k) { var d = Object.getOwnPropertyDescriptor(globalThis, k); print(k + ':' + d.writable + d.enumerable + d.configurable); });
print([Object.getPrototypeOf(ArrayBuffer) === Function.prototype, Object.getPrototypeOf(ArrayBuffer.prototype) === Object.prototype, Object.getPrototypeOf(DataView.prototype) === Object.prototype, Object.getPrototypeOf(SharedArrayBuffer.prototype) === Object.prototype].join());
"#;

/// The own keys of the six intrinsics in [[OwnPropertyKeys]] order, with the attributes, the
/// function names and lengths, and the three global keys (node v26.13.0).
#[test]
fn the_property_keys_attributes_and_lengths_match_node() {
    assert_eq!(
        common::run_both(DESC_JS),
        lines(&[
            "length:d--c,name:d--c,prototype:d---,isView:dw-c isView/1,Symbol(Symbol.species):a--c get [Symbol.species]/0",
            "constructor:dw-c ArrayBuffer/1,byteLength:a--c get byteLength/0,slice:dw-c slice/2,maxByteLength:a--c get maxByteLength/0,resizable:a--c get resizable/0,resize:dw-c resize/1,transfer:dw-c transfer/0,transferToFixedLength:dw-c transferToFixedLength/0,detached:a--c get detached/0,Symbol(Symbol.toStringTag):d--c",
            "length:d--c,name:d--c,prototype:d---,Symbol(Symbol.species):a--c get [Symbol.species]/0",
            "constructor:dw-c SharedArrayBuffer/1,byteLength:a--c get byteLength/0,slice:dw-c slice/2,maxByteLength:a--c get maxByteLength/0,growable:a--c get growable/0,grow:dw-c grow/1,Symbol(Symbol.toStringTag):d--c",
            "length:d--c,name:d--c,prototype:d---",
            "constructor:dw-c DataView/1,buffer:a--c get buffer/0,byteLength:a--c get byteLength/0,byteOffset:a--c get byteOffset/0,getInt8:dw-c getInt8/1,setInt8:dw-c setInt8/2,getUint8:dw-c getUint8/1,setUint8:dw-c setUint8/2,getInt16:dw-c getInt16/1,setInt16:dw-c setInt16/2,getUint16:dw-c getUint16/1,setUint16:dw-c setUint16/2,getInt32:dw-c getInt32/1,setInt32:dw-c setInt32/2,getUint32:dw-c getUint32/1,setUint32:dw-c setUint32/2,getFloat16:dw-c getFloat16/1,setFloat16:dw-c setFloat16/2,getFloat32:dw-c getFloat32/1,setFloat32:dw-c setFloat32/2,getFloat64:dw-c getFloat64/1,setFloat64:dw-c setFloat64/2,getBigInt64:dw-c getBigInt64/1,setBigInt64:dw-c setBigInt64/2,getBigUint64:dw-c getBigUint64/1,setBigUint64:dw-c setBigUint64/2,Symbol(Symbol.toStringTag):d--c",
            "ArrayBuffer:truefalsetrue",
            "SharedArrayBuffer:truefalsetrue",
            "DataView:truefalsetrue",
            "true,true,true,true",
        ])
    );
}
