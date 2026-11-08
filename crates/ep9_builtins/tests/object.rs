//! Object builtins (§20.1, Annex B §B.2.2): `Object.prototype.hasOwnProperty`,
//! `propertyIsEnumerable` and `isPrototypeOf`, the `Object` statics of the `object` module, the
//! Annex B `__proto__` and `__define/lookup{Getter,Setter}__`, and the top of test262's
//! `harness/propertyHelper.js` that needs them. Every JS check runs with and without GC
//! stress (`common::run_both`). Expected outputs come from node v24.22.1, except where a
//! test says V8 differs from the spec.

mod common;

use std::path::PathBuf;

use common::run_both;
use ep9_vm::{IntrinsicId, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// harness/propertyHelper.js lines 27-35 (test262 at the pin in third_party/test262.commit).
const PROPERTY_HELPER_27_35: &str = "\
var __isArray = Array.isArray;
var __defineProperty = Object.defineProperty;
var __getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
var __getOwnPropertyNames = Object.getOwnPropertyNames;
var __join = Function.prototype.call.bind(Array.prototype.join);
var __push = Function.prototype.call.bind(Array.prototype.push);
var __hasOwnProperty = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
var __propertyIsEnumerable = Function.prototype.call.bind(Object.prototype.propertyIsEnumerable);
var nonIndexNumericPropertyName = Math.pow(2, 32) - 1;
";

/// Lines 27-34 of the helper (without the `Math.pow` line).
fn property_helper_27_34() -> &'static str {
    let end = PROPERTY_HELPER_27_35
        .find("var nonIndexNumericPropertyName")
        .unwrap();
    &PROPERTY_HELPER_27_35[..end]
}

#[test]
fn the_embedded_property_helper_lines_match_the_pinned_harness() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/test262/harness/propertyHelper.js");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "skipped: {} is not there (scripts/sync-test262.sh)",
            path.display()
        );
        return;
    };
    let want: Vec<&str> = text.lines().skip(26).take(9).collect();
    let have: Vec<&str> = PROPERTY_HELPER_27_35.lines().collect();
    assert_eq!(have, want);
}

#[test]
fn property_helper_lines_27_to_34_run() {
    let src = format!(
        "{}
         var a = [];
         __push(a, 'x', 'y');
         var o = {{}};
         __defineProperty(o, 'k', {{value: 1, enumerable: false}});
         var d = __getOwnPropertyDescriptor(o, 'k');
         print(__isArray(a), __join(a, '|'), __hasOwnProperty(o, 'k'), __hasOwnProperty(o, 'toString'));
         print(__propertyIsEnumerable(o, 'k'), __propertyIsEnumerable(a, 0), __getOwnPropertyNames(o).join());
         print(d.value, d.writable, d.enumerable, d.configurable);",
        property_helper_27_34()
    );
    let src: &'static str = Box::leak(src.into_boxed_str());
    assert_eq!(
        run_both(src),
        lines(&["true x|y true false", "false true k", "1 false false false"])
    );
}

/// Line 35 needs `Math.pow` (P3-BLTB): until `Math` exists the only error is that.
#[test]
fn property_helper_line_35_needs_only_math() {
    let r = run_both(PROPERTY_HELPER_27_35);
    if r != Ok(vec![]) {
        assert_eq!(r, Err("ReferenceError: Math is not defined".to_string()));
    }
}

#[test]
fn the_functions_are_installed() {
    assert_eq!(
        run_both(
            "var p = Object.prototype;
             print(p.hasOwnProperty.name, p.hasOwnProperty.length, p.propertyIsEnumerable.name, p.propertyIsEnumerable.length);
             print(Object.defineProperty.length, Object.getOwnPropertyDescriptor.length, Object.getOwnPropertyNames.length, Object.keys.length);
             print(Object.getOwnPropertyNames(Object).join());
             print(Object.keys(Object).length, p.propertyIsEnumerable('hasOwnProperty'));"
        ),
        lines(&[
            "hasOwnProperty 1 propertyIsEnumerable 1",
            "3 2 1 1",
            "length,name,prototype,getOwnPropertyDescriptor,getOwnPropertyNames,defineProperty,keys,\
             assign,create,defineProperties,entries,freeze,getOwnPropertyDescriptors,\
             getOwnPropertySymbols,getPrototypeOf,hasOwn,is,isExtensible,isFrozen,isSealed,\
             preventExtensions,seal,setPrototypeOf,values,fromEntries,groupBy",
            "0 false",
        ])
    );
}

#[test]
fn has_own_property() {
    assert_eq!(
        run_both(
            "function F() { this.own = 1; } F.prototype.inherited = 2;
             var o = new F();
             print(o.hasOwnProperty('own'), o.hasOwnProperty('inherited'), o.hasOwnProperty('nope'));
             print([1].hasOwnProperty(0), [1].hasOwnProperty('0'), [1].hasOwnProperty(1), [].hasOwnProperty('length'));
             print(F.hasOwnProperty('name'), F.hasOwnProperty('length'), F.hasOwnProperty('prototype'));
             print(Object.prototype.hasOwnProperty.call('ab', 1), Object.prototype.hasOwnProperty.call('ab', 'length'));
             var k = {toString: function () { return 'own'; }};
             print(o.hasOwnProperty(k));"
        ),
        lines(&[
            "true false false",
            "true true false true",
            "true true true",
            "true true",
            "true",
        ])
    );
}

/// ToPropertyKey(V) runs before ToObject(this value) (§20.1.3.2 steps 1-2).
#[test]
fn has_own_property_converts_the_key_first() {
    assert_eq!(
        run_both(
            "var k = {toString: function () { throw new RangeError('key'); }};
             try { Object.prototype.hasOwnProperty.call(undefined, k); } catch (e) { print(e.name); }
             try { Object.prototype.hasOwnProperty.call(undefined, 'x'); } catch (e) { print(e.name); }
             try { Object.prototype.propertyIsEnumerable.call(null, k); } catch (e) { print(e.name); }
             try { Object.prototype.propertyIsEnumerable.call(null, 'x'); } catch (e) { print(e.name); }"
        ),
        lines(&["RangeError", "TypeError", "RangeError", "TypeError"])
    );
}

#[test]
fn property_is_enumerable() {
    assert_eq!(
        run_both(
            "function F() { this.own = 1; } F.prototype.inherited = 2;
             var o = new F();
             print(o.propertyIsEnumerable('own'), o.propertyIsEnumerable('inherited'), o.propertyIsEnumerable('x'));
             print([5].propertyIsEnumerable(0), [5].propertyIsEnumerable('length'), F.propertyIsEnumerable('name'));
             print(Object.prototype.propertyIsEnumerable.call('ab', 0), Object.prototype.propertyIsEnumerable.call('ab', 'length'));"
        ),
        lines(&["true false false", "true false false", "true false"])
    );
}

#[test]
fn define_property_defines_and_returns_the_object() {
    assert_eq!(
        run_both(
            "var o = {};
             var r = Object.defineProperty(o, 'x', {value: 1});
             var d = Object.getOwnPropertyDescriptor(o, 'x');
             print(r === o, d.value, d.writable, d.enumerable, d.configurable);
             Object.defineProperty(o, 'y', {value: 2, writable: true, enumerable: true, configurable: true});
             print(Object.keys(o).join(), o.y);
             o.x = 5; print(o.x);
             Object.defineProperty(o, 'y', {enumerable: false});
             print(Object.keys(o).length, o.y);
             var a = [1, 2, 3];
             Object.defineProperty(a, 'length', {value: 1});
             print(a.length, a.join());
             Object.defineProperty(a, 5, {value: 9, writable: true, enumerable: true, configurable: true});
             print(a.length, a[5]);
             var f = function g() {};
             Object.defineProperty(f, 'name', {value: 'h'});
             print(f.name);"
        ),
        lines(&[
            "true 1 false false false",
            "y 2",
            "1",
            "0 2",
            "1 1",
            "6 9",
            "h",
        ])
    );
}

#[test]
fn define_property_type_errors() {
    for src in [
        "Object.defineProperty(1, 'x', {})",
        "Object.defineProperty(undefined, 'x', {})",
        "Object.defineProperty({}, 'x', 1)",
        "Object.defineProperty({}, 'x')",
        "var o = {}; Object.defineProperty(o, 'x', {value: 1}); Object.defineProperty(o, 'x', {value: 2})",
        "var o = {}; Object.defineProperty(o, 'x', {value: 1}); Object.defineProperty(o, 'x', {enumerable: true})",
        "Object.defineProperty({}, 'x', {get: 1})",
        "Object.defineProperty({}, 'x', {value: 1, get: undefined})",
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ")),
            "{src}: {r:?}"
        );
    }
}

/// ToPropertyDescriptor reads each field with HasProperty and Get, so inherited fields count.
#[test]
fn define_property_reads_inherited_descriptor_fields() {
    assert_eq!(
        run_both(
            "function D() {} D.prototype.value = 7; D.prototype.enumerable = 1;
             var o = Object.defineProperty({}, 'x', new D());
             var d = Object.getOwnPropertyDescriptor(o, 'x');
             print(d.value, d.writable, d.enumerable, d.configurable);
             var k = {toString: function () { return 'computed'; }};
             Object.defineProperty(o, k, {value: 'c'});
             print(o.computed);"
        ),
        lines(&["7 false true false", "c"])
    );
}

/// An accessor descriptor needs accessor properties (P3-ACC): before that it throws "not
/// implemented"; after, it defines the accessor.
#[test]
fn define_property_with_an_accessor_descriptor() {
    let r = run_both(
        "var o = {};
         Object.defineProperty(o, 'x', {get: function () { return 3; }, configurable: true});
         var d = Object.getOwnPropertyDescriptor(o, 'x');
         print(o.x, typeof d.get, d.set, d.enumerable, d.configurable, 'value' in d);",
    );
    if r != lines(&["3 function undefined false true false"]) {
        assert_eq!(
            r,
            Err("Error: not implemented in ep9 v0: accessor properties".to_string())
        );
    }
}

#[test]
fn get_own_property_descriptor() {
    assert_eq!(
        run_both(
            "function show(d) { return d === undefined ? 'undefined' : [d.value, d.writable, d.enumerable, d.configurable, Object.keys(d).join('/')].join(); }
             print(show(Object.getOwnPropertyDescriptor({a: 1}, 'a')));
             print(show(Object.getOwnPropertyDescriptor({a: 1}, 'b')));
             print(show(Object.getOwnPropertyDescriptor({}, 'toString')));
             print(show(Object.getOwnPropertyDescriptor([7], 'length')));
             print(show(Object.getOwnPropertyDescriptor([7], 0)));
             print(show(Object.getOwnPropertyDescriptor(function f(a, b) {}, 'length')));
             print(show(Object.getOwnPropertyDescriptor('ab', 1)));
             print(show(Object.getOwnPropertyDescriptor('ab', 'length')));
             print(show(Object.getOwnPropertyDescriptor(Object, 'prototype')));"
        ),
        lines(&[
            "1,true,true,true,value/writable/enumerable/configurable",
            "undefined",
            "undefined",
            "1,true,false,false,value/writable/enumerable/configurable",
            "7,true,true,true,value/writable/enumerable/configurable",
            "2,false,false,true,value/writable/enumerable/configurable",
            "b,false,true,false,value/writable/enumerable/configurable",
            "2,false,false,false,value/writable/enumerable/configurable",
            "[object Object],false,false,false,value/writable/enumerable/configurable",
        ])
    );
    for src in [
        "Object.getOwnPropertyDescriptor(undefined, 'x')",
        "Object.getOwnPropertyDescriptor(null)",
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ")),
            "{src}: {r:?}"
        );
    }
}

#[test]
fn get_own_property_names_lists_string_keys_in_order() {
    assert_eq!(
        run_both(
            "print(Object.getOwnPropertyNames({b: 1, 2: 1, a: 1, 1: 1}).join());
             print(Object.getOwnPropertyNames([5, 6]).join());
             print(Object.getOwnPropertyNames('ab').join());
             print(Object.getOwnPropertyNames(function f(x) {}).join());
             var o = {x: 1}; Object.defineProperty(o, 'hidden', {value: 2});
             print(Object.getOwnPropertyNames(o).join(), Object.getOwnPropertyNames(5).length);"
        ),
        lines(&[
            "1,2,b,a",
            "0,1,length",
            "0,1,length",
            "length,name,prototype",
            "x,hidden 0"
        ])
    );
    let r = run_both("Object.getOwnPropertyNames(null)");
    assert!(
        matches!(&r, Err(m) if m.starts_with("TypeError: ")),
        "{r:?}"
    );
}

#[test]
fn keys_lists_enumerable_string_keys() {
    assert_eq!(
        run_both(
            "print(Object.keys({b: 1, 2: 1, a: 1, 1: 1}).join());
             var o = {x: 1, y: 2}; Object.defineProperty(o, 'hidden', {value: 3});
             print(Object.keys(o).join(), Object.keys([7, 8]).join(), Object.keys('ab').join());
             function F() { this.own = 1; } F.prototype.inherited = 2;
             print(Object.keys(new F()).join(), Object.keys(F).length, Object.keys(1).length);
             var big = {}; for (var i = 0; i < 40; i++) big['k' + i] = i;
             var ks = Object.keys(big); print(ks.length, ks[0], ks[39]);"
        ),
        lines(&["1,2,b,a", "x,y 0,1 0,1", "own 0 0", "40 k0 k39"])
    );
    let r = run_both("Object.keys(undefined)");
    assert!(
        matches!(&r, Err(m) if m.starts_with("TypeError: ")),
        "{r:?}"
    );
}

/// Symbol keys are left out of getOwnPropertyNames and keys, and a symbol `V` works in
/// hasOwnProperty and propertyIsEnumerable (symbols are made through the ABI: the `Symbol`
/// global is P3-BLTB's).
#[test]
fn symbol_keys_are_skipped_or_found() {
    for stress in [false, true] {
        let r = common::on_big_stack(move || {
            let mut iso = common::isolate(stress);
            let r = iso.with_cx(|cx| {
                let o = cx.new_object();
                let sym = cx.new_symbol(Local::undefined())?;
                cx.create_data_property_or_throw(o, sym, Local::int32(1))?;
                cx.create_data_property_or_throw(o, "s", Local::int32(2))?;
                let object = cx.intrinsic(IntrinsicId::Object);
                let mut out = Vec::new();
                for name in ["getOwnPropertyNames", "keys"] {
                    let f = cx.get(object, name)?;
                    let a = cx.call(f, Local::undefined(), &[o.value()])?;
                    let s = cx.to_rust_string(a)?;
                    out.push(s);
                }
                let proto = cx.intrinsic(IntrinsicId::ObjectPrototype);
                for name in ["hasOwnProperty", "propertyIsEnumerable"] {
                    let f = cx.get(proto, name)?;
                    let b = cx.call(f, o, &[sym.into()])?;
                    out.push(cx.to_rust_string(b)?);
                }
                Ok(out)
            });
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            r.map_err(|e| e.message)
        });
        assert_eq!(
            r,
            Ok(vec![
                "s".to_string(),
                "s".to_string(),
                "true".to_string(),
                "true".to_string()
            ]),
            "stress {stress}"
        );
    }
}

// ---- P4-B-OBJECT: the rest of the Object statics, isPrototypeOf and Annex B ----------------

fn is_type_error(r: &Result<Vec<String>, String>) -> bool {
    matches!(r, Err(m) if m.starts_with("TypeError: "))
}

#[test]
fn the_new_functions_have_their_names_and_lengths() {
    assert_eq!(
        run_both(
            "var out = [];
             var names = ['create', 'defineProperties', 'getPrototypeOf', 'setPrototypeOf', 'freeze',
              'isFrozen', 'seal', 'isSealed', 'preventExtensions', 'isExtensible', 'entries',
              'values', 'assign', 'is', 'hasOwn', 'getOwnPropertySymbols',
              'getOwnPropertyDescriptors'];
             for (var i = 0; i < names.length; i++) {
               var f = Object[names[i]];
               out.push(f.name + ':' + f.length + ':' + Object.prototype.propertyIsEnumerable.call(Object, names[i]));
             }
             print(out.join(' '));
             out = [];
             var p = Object.prototype;
             names = ['isPrototypeOf', '__defineGetter__', '__defineSetter__', '__lookupGetter__', '__lookupSetter__'];
             for (var i = 0; i < names.length; i++) out.push(p[names[i]].name + ':' + p[names[i]].length);
             print(out.join(' '));
             var d = Object.getOwnPropertyDescriptor(p, '__proto__');
             print(d.get.name, d.get.length, d.set.name, d.set.length, d.enumerable, d.configurable);"
        ),
        lines(&[
            "create:2:false defineProperties:2:false getPrototypeOf:1:false setPrototypeOf:2:false \
             freeze:1:false isFrozen:1:false seal:1:false isSealed:1:false \
             preventExtensions:1:false isExtensible:1:false entries:1:false values:1:false \
             assign:2:false is:2:false hasOwn:2:false getOwnPropertySymbols:1:false \
             getOwnPropertyDescriptors:1:false",
            "isPrototypeOf:1 __defineGetter__:2 __defineSetter__:2 __lookupGetter__:1 \
             __lookupSetter__:1",
            "get __proto__ 0 set __proto__ 1 false true",
        ])
    );
}

#[test]
fn create_and_define_properties() {
    assert_eq!(
        run_both(
            "var p = {a: 1};
             var o = Object.create(p, {x: {value: 2, enumerable: true}, y: {get: function () { return 3; }}});
             print(Object.getPrototypeOf(o) === p, o.x, o.y, o.a, Object.keys(o).join());
             var n = Object.create(null);
             print(Object.getPrototypeOf(n), n instanceof Object, Object.create(p, undefined).a);
             var q = Object.defineProperties({}, {a: {value: 1, writable: true, enumerable: true, configurable: true}, b: {value: 2}});
             var d = Object.getOwnPropertyDescriptor(q, 'b');
             print(Object.keys(q).join(), q.b, d.writable, d.enumerable, d.configurable);
             var hidden = Object.defineProperty({}, 'h', {value: {value: 1}, enumerable: false});
             print(Object.getOwnPropertyNames(Object.defineProperties({}, hidden)).length);
             var s = Symbol('k'); var sp = {}; sp[s] = {value: 5, enumerable: true};
             var r = Object.defineProperties({}, sp); print(r[s]);"
        ),
        lines(&[
            "true 2 3 1 x",
            "null false 1",
            "a 2 false false false",
            "0",
            "5",
        ])
    );
    for src in [
        "Object.create(1)",
        "Object.create(undefined)",
        "Object.create({}, null)",
        "Object.defineProperties(1, {})",
        "Object.defineProperties({}, {a: 1})",
        "Object.defineProperties({}, undefined)",
    ] {
        let r = run_both(src);
        assert!(is_type_error(&r), "{src}: {r:?}");
    }
}

/// ObjectDefineProperties reads every descriptor (running user getters in key order) before it
/// defines the first property (§20.1.2.3.1 steps 4-5): a throw while reading leaves the target
/// unchanged. The descriptors live in a `ScratchList` across those calls (GC stress).
#[test]
fn define_properties_reads_every_descriptor_before_the_first_define() {
    assert_eq!(
        run_both(
            "var order = [];
             var props = {
               get a() { order.push('get a'); return {value: 1}; },
               get b() { order.push('get b'); return {get value() { order.push('b.value'); throw new RangeError('b'); }}; },
             };
             var t = {};
             try { Object.defineProperties(t, props); } catch (e) { print(e.name, order.join(), Object.getOwnPropertyNames(t).length); }
             var m = Object.defineProperties({}, {
               v: {value: 'v', writable: true},
               g: {get: function () { return 'g'; }, set: function (x) { this.v = x; }, enumerable: true},
             });
             m.g = 'set';
             var dg = Object.getOwnPropertyDescriptor(m, 'g');
             print(m.v, m.g, typeof dg.get, typeof dg.set, dg.enumerable, dg.configurable);"
        ),
        lines(&["RangeError get a,get b,b.value 0", "set g function function true false"])
    );
}

#[test]
fn get_and_set_prototype_of() {
    assert_eq!(
        run_both(
            "var p = {}; var o = {};
             print(Object.getPrototypeOf(o) === Object.prototype, Object.getPrototypeOf(1) === Number.prototype);
             print(Object.setPrototypeOf(o, p) === o, Object.getPrototypeOf(o) === p);
             print(Object.setPrototypeOf(1, null), Object.setPrototypeOf(o, null) === o, Object.getPrototypeOf(o));
             print(Object.setPrototypeOf(Object.prototype, null) === Object.prototype);"
        ),
        lines(&["true true", "true true", "1 true null", "true"])
    );
    for src in [
        "Object.getPrototypeOf(undefined)",
        "Object.setPrototypeOf(undefined, null)",
        "Object.setPrototypeOf({}, 1)",
        "Object.setPrototypeOf(Object.preventExtensions({}), {})",
        "var a = {}; var b = Object.create(a); Object.setPrototypeOf(a, b)",
        "Object.setPrototypeOf(Object.prototype, {})",
    ] {
        let r = run_both(src);
        assert!(is_type_error(&r), "{src}: {r:?}");
    }
}

/// SetIntegrityLevel and TestIntegrityLevel on ordinary objects, functions and wrappers.
#[test]
fn freeze_seal_and_prevent_extensions() {
    assert_eq!(
        run_both(
            "var s = Object.seal({k: 1});
             print(Object.isSealed(s), Object.isFrozen(s), Object.isExtensible(s));
             s.k = 2; s.z = 3; print(s.k, s.z, delete s.k, s.k);
             var f = Object.freeze({k: 1, get g() { return 5; }});
             print(Object.isSealed(f), Object.isFrozen(f), Object.isExtensible(f), f.g);
             f.k = 9; print(f.k, Object.getOwnPropertyDescriptor(f, 'g').configurable);
             print(Object.isFrozen({}), Object.isFrozen(Object.preventExtensions({})), Object.isSealed(Object.preventExtensions({a: 1})));
             var fn = function a(x) {}; Object.freeze(fn); print(Object.isFrozen(fn));
             var w = new String('ab'); Object.freeze(w); print(Object.isFrozen(w));
             print(Object.freeze(1), Object.seal('s'), Object.preventExtensions(true));
             print(Object.isExtensible(1), Object.isSealed(1), Object.isFrozen(1));
             var e = Object.preventExtensions({a: 1}); e.b = 2; print(Object.isExtensible(e), e.b, e.a);"
        ),
        lines(&[
            "true false false",
            "2 undefined false 2",
            "true true false 5",
            "1 false",
            "false true false",
            "true",
            "true",
            "1 s true",
            "false true true",
            "false undefined 1",
        ])
    );
    let r = run_both("'use strict'; var f = Object.freeze({k: 1}); f.k = 2;");
    assert!(is_type_error(&r), "{r:?}");
}

/// A sealed empty array keeps a writable `length`, so TestIntegrityLevel(frozen) is false
/// (§7.3.16 step 5.b.ii). V8 (node v24.22.1) prints true here; ep9 follows the spec.
/// Freezing an array makes its elements and `length` non-writable: DICTIONARY elements
/// (H18, M5-P2 ELEM2); strict stores to them throw TypeError.
#[test]
fn arrays_seal_and_freeze() {
    assert_eq!(
        run_both(
            "var a = Object.seal([]); print(Object.isSealed(a), Object.isFrozen(a));
             var b = Object.freeze([1]); b[0] = 2; print(Object.isFrozen(b), b.length, b[0]);
             var c = Object.freeze([]); print(Object.isFrozen(c), c.length);
             try { (function () { 'use strict'; b[0] = 3; })(); } catch (e) { print(e.constructor.name); }
             try { (function () { 'use strict'; c.length = 1; })(); } catch (e) { print(e.constructor.name); }"
        ),
        lines(&["true false", "true 1 1", "true 0", "TypeError", "TypeError"])
    );
}

/// The integrity levels of Arrays (M7-P3 ELEMKINDS; H18 "extra element kinds"): sealed and
/// frozen arrays of each storage form (INT32, DOUBLE, ELEMENTS, holey, empty) under `[[Get]]`,
/// `[[Set]]` (sloppy and strict, the keyed store sites in their monomorphic and megamorphic
/// states), `[[Delete]]`, `[[DefineOwnProperty]]`, `length` sets and cuts, the Array methods
/// that write, Reflect, a level array as a prototype, and TestIntegrityLevel. Expected output
/// from node v26.13.0; the spec agrees on each line (§7.3.15, §7.3.16, §10.4.2). The lines pass
/// with DICTIONARY elements (the form up to M7-P2) and with the shape-flag form, so they guard
/// the move from one to the other.
#[test]
fn arrays_keep_each_integrity_level() {
    assert_eq!(
        run_both(
            "function d(o, k) { var x = Object.getOwnPropertyDescriptor(o, k); return x ? [x.value, x.writable, x.enumerable, x.configurable].join(',') : 'none'; }
             function te(f) { try { f(); return 'ok'; } catch (e) { return e.constructor.name; } }
             var s = Object.seal([1, 2.5, 'x']);
             print(Object.isSealed(s), Object.isFrozen(s), Object.isExtensible(s), d(s, 0), d(s, 'length'));
             s[0] = 7; s[1] = 3.25; s[2] = {}; s[3] = 9;
             print(s[0], s[1], typeof s[2], s[3], s.length, delete s[0], s[0], 0 in s);
             s.length = 1; print(s.length, s[2] === undefined);
             s.length = 5; print(s.length, 4 in s, Object.isSealed(s));
             print(te(function () { 'use strict'; delete s[0]; }), te(function () { 'use strict'; s[9] = 1; }));
             print(te(function () { Object.defineProperty(s, 0, { value: 8 }); }), s[0], d(s, 0));
             print(te(function () { Object.defineProperty(s, 0, { configurable: true }); }));
             Object.defineProperty(s, 1, { writable: false }); print(d(s, 1), Object.isFrozen(s));
             var f = Object.freeze([1, 2, 3]);
             print(Object.isSealed(f), Object.isFrozen(f), d(f, 1), d(f, 'length'));
             f[0] = 9; f.length = 0; print(f[0], f.length, delete f[2], f.join());
             print(te(function () { 'use strict'; f[0] = 9; }), te(function () { 'use strict'; f.length = 0; }));
             print(te(function () { f.push(4); }), te(function () { f.pop(); }), te(function () { f.reverse(); }), te(function () { f.sort(); }));
             print(te(function () { f.fill(0); }), te(function () { f.copyWithin(0, 1); }), te(function () { f.splice(0, 1); }));
             print(te(function () { f.unshift(0); }), te(function () { f.shift(); }), f.join(), f.length);
             print(f.map(function (x) { return x * 2; }).join(), f.slice(1).join(), f.concat([4]).join(), f.indexOf(3), f.includes(2));
             print(te(function () { Object.defineProperty(f, 0, { value: 1 }); }), te(function () { Object.defineProperty(f, 0, { value: 2 }); }));
             var fd = Object.freeze([1.5, -0, NaN]); fd[0] = 2; print(fd[0], Object.is(fd[1], -0), fd[2] !== fd[2], Object.isFrozen(fd));
             var fe = Object.freeze([]); print(Object.isFrozen(fe), d(fe, 'length'), te(function () { fe.push(); }), te(function () { 'use strict'; fe[0] = 1; }));
             var h = [1, , 3]; Object.freeze(h); print(1 in h, h.length, Object.keys(h).join());
             var p = Object.preventExtensions([1, 2]); p[0] = 5; p[2] = 6; print(p.join(), p.length, delete p[1], p.length, Object.isSealed(p));
             var g = [1, 2]; Object.seal(g); Object.freeze(g); print(Object.isFrozen(g)); Object.seal(g); print(Object.isFrozen(g));
             var q = Object.freeze([1, 2]); var r = Object.create(q); r[0] = 5; print(r[0], Object.keys(r).length);
             var t = Object.freeze([1, 2]); var u = []; u.__proto__ = t; print(u.length, te(function () { 'use strict'; u[0] = 1; }));
             var sp = Object.seal([1, 2, 3]); var o2 = Object.create(sp); print(o2[1]);
             for (var i = 0; i < 3; i++) { var k = [i, i + 1]; if (i == 2) Object.freeze(k); k[0] = 'z'; print(k[0]); }
             var big = []; for (var j = 0; j < 100; j++) big.push(j); Object.freeze(big); var sum = 0; for (var j = 0; j < 100; j++) sum += big[j]; print(sum, Object.isFrozen(big));
             var ss = Object.seal([1, 2, 3]); ss.length = 0; print(ss.length, ss.join());
             print(JSON.stringify(Object.freeze([1, 'a', null])), Array.isArray(Object.freeze([])));
             var sd = Object.seal([1, 2]); sd[0] = 1.5; sd[1] = 'w'; print(sd.join(), Object.isSealed(sd), d(sd, 0));
             var fp = Object.freeze([10, 20]); var o3 = Object.create(fp); print(Object.getPrototypeOf(o3) === fp, fp[1]);
             print(Reflect.set(f, 0, 5), Reflect.defineProperty(f, 0, { value: 5 }), Reflect.deleteProperty(s, 0), Reflect.deleteProperty(f, 5));
             function st(a) { a[0] = 'm'; return a[0]; }
             var arrs = [[1], [1.5], ['a'], {0: 1}, Object.preventExtensions([1]), Object.seal([2]), Object.freeze([3]), Object.freeze([4.5])];
             print(arrs.map(st).join());
             function st2(a, v) { a[1] = v; return a[1]; }
             var fz = Object.freeze([0, 1]); var sl = Object.seal([0, 1]);
             for (var n = 0; n < 4; n++) { st2([0, 1], n); st2(sl, n); } print(st2(fz, 9), st2(sl, 9), st2(sl, 2.5), Object.isSealed(sl), d(sl, 1));"
        ),
        lines(&[
            "true false false 1,true,true,false 3,true,false,false",
            "7 3.25 object undefined 3 false 7 true",
            "3 false",
            "5 false true",
            "TypeError TypeError",
            "ok 8 8,true,true,false",
            "TypeError",
            "3.25,false,true,false false",
            "true true 2,false,true,false 3,false,false,false",
            "1 3 false 1,2,3",
            "TypeError TypeError",
            "TypeError TypeError TypeError TypeError",
            "TypeError TypeError TypeError",
            "TypeError TypeError 1,2,3 3",
            "2,4,6 2,3 1,2,3,4 2 true",
            "ok TypeError",
            "1.5 true true true",
            "true 0,false,false,false TypeError TypeError",
            "false 3 0,2",
            "5,2 2 true 2 false",
            "true",
            "true",
            "1 0",
            "0 TypeError",
            "2",
            "z",
            "z",
            "2",
            "4950 true",
            "3 1,2,3",
            "[1,\"a\",null] true",
            "1.5,w true 1.5,true,true,false",
            "true 20",
            "false false false true",
            "m,m,m,m,m,m,3,4.5",
            "1 9 2.5 true 2.5,true,true,false",
        ])
    );
}

/// M2-P2-CLSCG-NONEXT-FALSEPASS / M2-PRIVATE-EXT: freeze, seal and preventExtensions walk
/// `[[OwnPropertyKeys]]`, which never lists a private name (§6.2.12), so private fields,
/// methods and accessors stay usable, and the Object.* key lists never show them. A private
/// add on a non-extensible object is a TypeError (the stage-3 proposal
/// `nonextensible-applies-to-private`, test262 features.txt); node v24.22.1 does not throw
/// there yet, every other line matches node.
#[test]
fn integrity_levels_leave_private_names_alone() {
    assert_eq!(
        run_both(
            "class C {
               #x = 1;
               #m() { return 'm'; }
               get #a() { return this.#x * 10; }
               set #a(v) { this.#x = v; }
               pub = 2;
               bump() { this.#x++; return this.#x; }
               seta(v) { this.#a = v; return this.#a; }
               m() { return this.#m(); }
             }
             var c = new C();
             Object.freeze(c);
             print(Object.isFrozen(c), c.bump(), c.seta(4), c.m());
             c.pub = 9; print(c.pub);
             print(Object.getOwnPropertyNames(c).join(), Object.getOwnPropertySymbols(c).length,
                   Object.keys(Object.getOwnPropertyDescriptors(c)).join());
             var d = new C(); Object.seal(d); print(Object.isSealed(d), d.bump());
             var e = new C(); Object.preventExtensions(e); print(Object.isExtensible(e), e.bump());
             class Over { constructor(o) { return o; } }
             class Add extends Over { #y = 1; static y(o) { return o.#y; } }
             try { new Add(Object.preventExtensions({})); } catch (err) { print(err.name); }
             var target = {}; new Add(target); Object.freeze(target); print(Add.y(target), Object.isFrozen(target));"
        ),
        lines(&[
            "true 2 40 m",
            "2",
            "pub 0 pub",
            "true 2",
            "false 2",
            "TypeError",
            "1 true",
        ])
    );
}

#[test]
fn entries_values_assign_is_and_has_own() {
    assert_eq!(
        run_both(
            "var q = {a: 1, b: 2}; Object.defineProperty(q, 'h', {value: 3});
             print(Object.entries(q).join('|'), Object.values(q).join());
             print(Object.entries('ab').join('|'), Object.values([7, 8]).join(), Object.entries(1).length);
             var big = {}; for (var i = 0; i < 100; i++) big['k' + i] = i;
             var e = Object.entries(big); print(e.length, e[99].join(':'), Object.values(big)[50]);
             print(Object.entries({get x() { delete this.y; return 1; }, y: 2}).length);
             var log = [];
             var src = {get a() { log.push('a'); return 1; }, b: 2};
             var tgt = {set a(v) { log.push('set ' + v); }};
             print(Object.assign(tgt, src, null, undefined) === tgt, log.join(), tgt.b);
             print(Object.keys(Object.assign({}, 'xy')).join(), typeof Object.assign(1, {}));
             print(Object.is(NaN, NaN), Object.is(0, -0), Object.is('a', 'a'), Object.is({}, {}));
             print(Object.hasOwn({x: 1}, 'x'), Object.hasOwn({x: 1}, 'toString'), Object.hasOwn('ab', 1));"
        ),
        lines(&[
            "a,1|b,2 1,2",
            "0,a|1,b 7,8 0",
            "100 k99:99 50",
            "1",
            "true a,set 1 2",
            "0,1 object",
            "true false true false",
            "true false true",
        ])
    );
    for src in [
        "Object.entries(undefined)",
        "Object.values(null)",
        "Object.assign(null)",
        "Object.assign(Object.freeze({x: 1}), {x: 2})",
        "Object.hasOwn(undefined, 'x')",
    ] {
        let r = run_both(src);
        assert!(is_type_error(&r), "{src}: {r:?}");
    }
}

#[test]
fn own_property_symbols_and_descriptors() {
    assert_eq!(
        run_both(
            "var s1 = Symbol('s1'), s2 = Symbol('s2');
             var o = {n: 1}; o[s2] = 2; o[s1] = 3;
             var syms = Object.getOwnPropertySymbols(o);
             print(syms.length, String(syms[0]), String(syms[1]), Object.getOwnPropertySymbols('ab').length);
             var ds = Object.getOwnPropertyDescriptors({a: 1, get b() { return 1; }});
             print(Object.keys(ds).join(), ds.a.value, ds.a.writable, typeof ds.b.get, ds.b.set, ds.b.enumerable);
             var dd = Object.getOwnPropertyDescriptors(o); print(dd[s1].value, dd[s2].value);
             print(Object.keys(Object.getOwnPropertyDescriptors([5])).join());"
        ),
        lines(&[
            "2 Symbol(s2) Symbol(s1) 0",
            "a,b 1 true function undefined true",
            "3 2",
            "0,length",
        ])
    );
    for src in [
        "Object.getOwnPropertySymbols(undefined)",
        "Object.getOwnPropertyDescriptors(null)",
    ] {
        let r = run_both(src);
        assert!(is_type_error(&r), "{src}: {r:?}");
    }
}

#[test]
fn is_prototype_of() {
    assert_eq!(
        run_both(
            "var proto = {}; var child = Object.create(proto); var grand = Object.create(child);
             print(proto.isPrototypeOf(child), proto.isPrototypeOf(grand), Object.prototype.isPrototypeOf(grand));
             print(child.isPrototypeOf(proto), proto.isPrototypeOf(proto), Object.prototype.isPrototypeOf(Object.create(null)));
             print(Object.prototype.isPrototypeOf.call(undefined, 1), Function.prototype.isPrototypeOf(Object));"
        ),
        lines(&["true true true", "false false false", "false true"])
    );
    let r = run_both("Object.prototype.isPrototypeOf.call(undefined, {})");
    assert!(is_type_error(&r), "{r:?}");
}

/// Annex B §B.2.2: `__proto__` and `__define/lookup{Getter,Setter}__`.
#[test]
fn annex_b_proto_and_legacy_accessor_methods() {
    assert_eq!(
        run_both(
            "var p = {}; var o = Object.create(p);
             print(({}).__proto__ === Object.prototype, o.__proto__ === p, (1).__proto__ === Number.prototype);
             o.__proto__ = null; print(Object.getPrototypeOf(o), o.__proto__);
             var n = {}; n.__proto__ = 5; print(Object.getPrototypeOf(n) === Object.prototype);
             var set = Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').set;
             print(set.call(1, {}), set.call({}, 1));
             var o2 = {};
             o2.__defineGetter__('g', function () { return 'G'; });
             o2.__defineSetter__('g', function (v) { this.v = v; });
             o2.g = 7;
             var d = Object.getOwnPropertyDescriptor(o2, 'g');
             print(o2.g, o2.v, d.enumerable, d.configurable);
             print(o2.__lookupGetter__('g')(), typeof o2.__lookupSetter__('g'), o2.__lookupGetter__('v'));
             print(Object.create(o2).__lookupGetter__('g') === d.get, ({}).__lookupGetter__('__proto__').name);
             print(({}).__lookupSetter__('nope'), ({x: 1}).__lookupGetter__('x'));"
        ),
        lines(&[
            "true true true",
            "null undefined",
            "true",
            "undefined undefined",
            "G 7 true true",
            "G function undefined",
            "true get __proto__",
            "undefined undefined",
        ])
    );
    for src in [
        "Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').get.call(undefined)",
        "Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').set.call(null, {})",
        "var a = {}; var b = Object.create(a); a.__proto__ = b",
        "Object.preventExtensions({}).__proto__ = {}",
        "({}).__defineGetter__('x', 1)",
        "({}).__defineSetter__('x', {})",
        "Object.prototype.__lookupGetter__.call(null, 'x')",
        "Object.defineProperty({}, 'x', {value: 1}).__defineGetter__('x', function () {})",
    ] {
        let r = run_both(src);
        assert!(is_type_error(&r), "{src}: {r:?}");
    }
}

/// Object.prototype.toLocaleString (§20.1.3.5) is Invoke(this, "toString"): a primitive
/// `this` stays a primitive in strict code.
#[test]
fn to_locale_string_invokes_to_string() {
    assert_eq!(
        run_both(
            "var p = Object.prototype.toLocaleString;
             print(p.name, p.length, ({}).toLocaleString(), p.call(1), typeof p.call(true));
             var o = {toString: function () { return 'mine'; }}; print(o.toLocaleString());
             Boolean.prototype.toString = function () { 'use strict'; return typeof this; };
             print(p.call(false));"
        ),
        lines(&[
            "toLocaleString 0 [object Object] 1 string",
            "mine",
            "boolean"
        ])
    );
    for src in [
        "Object.prototype.toLocaleString.call(undefined)",
        "Object.prototype.toLocaleString.call({toString: 1})",
    ] {
        let r = run_both(src);
        assert!(is_type_error(&r), "{src}: {r:?}");
    }
}

/// Codex M7-P3 r2 CX2-1: the `length` of an Array frozen by `Object.freeze` (a dense Array
/// since the ELEMKINDS activation) is read-only for a `[[Set]]` whose receiver is another
/// object: `Reflect.set` with another receiver is false and keeps it; an inherited
/// `o.length = 5` throws in strict code and adds no own `length` in sloppy code; the generic
/// `push` and `pop` on `Object.create(frozen)` throw. The strict text names a read-only
/// property (node: "... of object '[object Array]'"; the object name differs, as before).
/// `Object.seal` keeps `length` writable (the control).
#[test]
fn a_frozen_array_length_is_read_only_for_another_receiver() {
    assert_eq!(
        run_both(
            "function t(f) { try { return String(f()); } catch (e) { return 'THROW ' + e.name; } }
             var r = [1, 2];
             print(Reflect.set(Object.freeze([]), 'length', 0, r), JSON.stringify(r));
             r = [1, 2];
             print(Reflect.set(Object.freeze([7]), 'length', 0, r), JSON.stringify(r));
             var p = {};
             print(Reflect.set(Object.freeze([]), 'length', 5, p), Object.hasOwn(p, 'length'));
             var o = Object.create(Object.freeze([])); o.length = 5;
             print(Object.hasOwn(o, 'length'), o.length);
             print(t(function () { 'use strict'; var o = Object.create(Object.freeze([])); o.length = 5; return 'no throw'; }));
             print(t(function () { 'use strict'; var o = Object.create(Object.freeze([1])); o.length = 5; return 'no throw'; }));
             print(t(function () { var o = Object.create(Object.freeze([])); Array.prototype.push.call(o, 9); return 'no throw'; }));
             print(t(function () { var o = Object.create(Object.freeze([1])); Array.prototype.pop.call(o); return 'no throw'; }));
             print(t(function () { return Object.create(Object.freeze([])).push(9); }));
             print(t(function () { return Object.create(Object.freeze([])).pop(); }));
             var n = 0, q = Object.freeze([]);
             for (var i = 0; i < 20; i++) { try { (function (o) { 'use strict'; o.length = 5; })(Object.create(q)); } catch (e) { n++; } }
             print('throws=' + n);
             try { (function () { 'use strict'; Object.freeze([1]).length = 0; })(); } catch (e) { print(e.name, e.message.split(' of ')[0]); }
             r = [1, 2];
             print(Reflect.set(Object.seal([]), 'length', 0, r), JSON.stringify(r));
             print(t(function () { 'use strict'; var o = Object.create(Object.seal([])); o.length = 5; return Object.hasOwn(o, 'length') + ' ' + o.length; }));"
        ),
        lines(&[
            "false [1,2]",
            "false [1,2]",
            "false false",
            "false 0",
            "THROW TypeError",
            "THROW TypeError",
            "THROW TypeError",
            "THROW TypeError",
            "THROW TypeError",
            "THROW TypeError",
            "throws=20",
            "TypeError Cannot assign to read only property 'length'",
            "true []",
            "true 5",
        ])
    );
}
