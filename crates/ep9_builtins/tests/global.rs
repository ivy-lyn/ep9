//! The global functions isNaN and isFinite (§19.2), and the key order of this crate's
//! intrinsics.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

use ep9_vm::IntrinsicId;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn is_nan_and_is_finite() {
    assert_eq!(
        common::run_both(
            r#"print(isNaN('x'), isNaN(1), isNaN(), isNaN(null), isNaN('  12 '), isNaN(NaN), isFinite('1'), isFinite(Infinity), isFinite(), isFinite(null), isFinite(-0), isFinite('Infinity'))"#
        ),
        lines(&["true false true false false true true false false true true false"])
    );
}

#[test]
fn is_nan_and_is_finite_convert() {
    assert_eq!(
        common::run_both(
            r#"var n = 0; var o = { valueOf: function () { n++; return 5; } }; print(isNaN(o), isFinite(o), n); try { isNaN(Symbol()); } catch (e) { print(e.name); } try { isFinite(Symbol()); } catch (e) { print(e.name); }"#
        ),
        lines(&["false true 2", "TypeError", "TypeError"])
    );
}

#[test]
fn global_functions_and_bindings() {
    assert_eq!(
        common::run_both(
            r#"print(typeof isNaN, isNaN.length, isNaN.name, isFinite.length, isFinite.name, globalThis.Number === Number, globalThis.Math === Math, typeof globalThis.String, typeof globalThis.Symbol, typeof globalThis.Boolean)"#
        ),
        lines(&["function 1 isNaN 1 isFinite true true function function function"])
    );
}

/// The own keys of the intrinsic `id` in `[[OwnPropertyKeys]]` order; a symbol key as its
/// descriptive string.
fn keys(id: IntrinsicId) -> Vec<String> {
    common::on_big_stack(move || {
        common::isolate(false).with_cx(|cx| {
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
        })
    })
    .expect("no exception")
}

/// The keys of this crate's intrinsics are in V8's order (node 24 `Reflect.ownKeys`), with
/// the keys that are not built yet left out.
#[test]
fn intrinsic_keys_follow_v8s_order() {
    let wk = |names: &[&str]| -> Vec<String> {
        names
            .iter()
            .map(|n| format!("Symbol(Symbol.{n})"))
            .collect()
    };
    let s = |names: &[&str]| -> Vec<String> { names.iter().map(|n| n.to_string()).collect() };
    let cases: Vec<(IntrinsicId, Vec<String>)> = vec![
        (
            IntrinsicId::Number,
            s(&[
                "length",
                "name",
                "prototype",
                "MAX_VALUE",
                "MIN_VALUE",
                "NaN",
                "NEGATIVE_INFINITY",
                "POSITIVE_INFINITY",
                "MAX_SAFE_INTEGER",
                "MIN_SAFE_INTEGER",
                "EPSILON",
            ]),
        ),
        (
            IntrinsicId::NumberPrototype,
            s(&["constructor", "toString", "valueOf"]),
        ),
        (IntrinsicId::Boolean, s(&["length", "name", "prototype"])),
        (
            IntrinsicId::BooleanPrototype,
            s(&["constructor", "toString", "valueOf"]),
        ),
        (
            IntrinsicId::String,
            s(&["length", "name", "prototype", "fromCharCode"]),
        ),
        (
            IntrinsicId::StringPrototype,
            s(&["length", "constructor", "charCodeAt", "toString", "valueOf"]),
        ),
        (
            IntrinsicId::Symbol,
            s(&[
                "length",
                "name",
                "prototype",
                "for",
                "keyFor",
                "asyncIterator",
                "hasInstance",
                "isConcatSpreadable",
                "iterator",
                "match",
                "matchAll",
                "replace",
                "search",
                "species",
                "split",
                "toPrimitive",
                "toStringTag",
                "unscopables",
                "dispose",
                "asyncDispose",
            ]),
        ),
        (IntrinsicId::SymbolPrototype, {
            let mut k = s(&["constructor", "toString", "valueOf"]);
            k.extend(wk(&["toStringTag", "toPrimitive"]));
            k
        }),
        (IntrinsicId::Math, {
            let mut k = s(&[
                "abs", "ceil", "floor", "fround", "max", "min", "pow", "round", "sign", "sqrt",
                "trunc", "E", "LN10", "LN2", "LOG10E", "LOG2E", "PI", "SQRT1_2", "SQRT2",
            ]);
            k.extend(wk(&["toStringTag"]));
            k
        }),
    ];
    for (id, want) in cases {
        assert_eq!(keys(id), want, "{}", id.name());
    }
}

/// Property attributes follow §18 and each property's own clause: functions and data
/// properties {w, c}, value properties and `prototype` {}, `@@toStringTag` and
/// `Symbol.prototype[@@toPrimitive]` {c}.
#[test]
fn property_attributes() {
    type Wec = (bool, bool, bool);
    const METHOD: Wec = (true, false, true);
    const NONE: Wec = (false, false, false);
    const C: Wec = (false, false, true);
    let got = common::on_big_stack(|| {
        common::isolate(false).with_cx(|cx| {
            let mut out: Vec<(String, Option<Wec>)> = Vec::new();
            let global = cx.global();
            for (id, key) in [
                (IntrinsicId::GlobalObject, "Number"),
                (IntrinsicId::GlobalObject, "Boolean"),
                (IntrinsicId::GlobalObject, "String"),
                (IntrinsicId::GlobalObject, "Symbol"),
                (IntrinsicId::GlobalObject, "Math"),
                (IntrinsicId::GlobalObject, "isNaN"),
                (IntrinsicId::GlobalObject, "isFinite"),
                (IntrinsicId::Number, "prototype"),
                (IntrinsicId::Number, "MAX_VALUE"),
                (IntrinsicId::Number, "EPSILON"),
                (IntrinsicId::NumberPrototype, "constructor"),
                (IntrinsicId::NumberPrototype, "toString"),
                (IntrinsicId::String, "fromCharCode"),
                (IntrinsicId::StringPrototype, "charCodeAt"),
                (IntrinsicId::Symbol, "iterator"),
                (IntrinsicId::Symbol, "for"),
                (IntrinsicId::Math, "PI"),
                (IntrinsicId::Math, "max"),
            ] {
                let o = if id == IntrinsicId::GlobalObject {
                    global
                } else {
                    cx.intrinsic(id)
                };
                let d = cx.get_own_property(o, key)?;
                let wec = d.map(|d| {
                    (
                        d.writable == Some(true),
                        d.enumerable == Some(true),
                        d.configurable == Some(true),
                    )
                });
                out.push((format!("{}.{key}", id.name()), wec));
            }
            for (id, name) in [
                (IntrinsicId::SymbolPrototype, "toPrimitive"),
                (IntrinsicId::SymbolPrototype, "toStringTag"),
                (IntrinsicId::Math, "toStringTag"),
            ] {
                let s = cx.intrinsic(IntrinsicId::Symbol);
                let sym = cx.get(s, name)?;
                let sym = sym.as_symbol().expect("a well-known symbol");
                let o = cx.intrinsic(id);
                let d = cx.get_own_property(o, sym)?;
                let wec = d.map(|d| {
                    (
                        d.writable == Some(true),
                        d.enumerable == Some(true),
                        d.configurable == Some(true),
                    )
                });
                out.push((format!("{}[@@{name}]", id.name()), wec));
            }
            Ok(out)
        })
    })
    .expect("no exception");
    let want: Vec<(String, Option<Wec>)> = [
        ("globalThis.Number", METHOD),
        ("globalThis.Boolean", METHOD),
        ("globalThis.String", METHOD),
        ("globalThis.Symbol", METHOD),
        ("globalThis.Math", METHOD),
        ("globalThis.isNaN", METHOD),
        ("globalThis.isFinite", METHOD),
        ("%Number%.prototype", NONE),
        ("%Number%.MAX_VALUE", NONE),
        ("%Number%.EPSILON", NONE),
        ("%Number.prototype%.constructor", METHOD),
        ("%Number.prototype%.toString", METHOD),
        ("%String%.fromCharCode", METHOD),
        ("%String.prototype%.charCodeAt", METHOD),
        ("%Symbol%.iterator", NONE),
        ("%Symbol%.for", METHOD),
        ("%Math%.PI", NONE),
        ("%Math%.max", METHOD),
        ("%Symbol.prototype%[@@toPrimitive]", C),
        ("%Symbol.prototype%[@@toStringTag]", C),
        ("%Math%[@@toStringTag]", C),
    ]
    .iter()
    .map(|(k, a)| (k.to_string(), Some(*a)))
    .collect();
    assert_eq!(got, want);
}
