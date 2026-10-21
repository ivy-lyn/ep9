//! Object builtins (§20.1): `Object.prototype.hasOwnProperty` and `propertyIsEnumerable`,
//! `Object.defineProperty`, `getOwnPropertyDescriptor`, `getOwnPropertyNames` and `keys`, and
//! the top of test262's `harness/propertyHelper.js` that needs them. Every JS check runs with
//! and without GC stress (`common::run_both`).

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
            "length,name,prototype,getOwnPropertyDescriptor,getOwnPropertyNames,defineProperty,keys",
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
