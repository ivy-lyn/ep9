//! The global functions isNaN, isFinite and the URI functions (§19.2), Error.isError
//! (§20.5.2.1), and the key order of this crate's intrinsics.
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

/// encodeURI and encodeURIComponent (§19.2.6.3-5): the unescaped sets, UTF-8 escapes in
/// upper case, surrogate pairs, lone surrogates (URIError), ToString first.
#[test]
fn encode_uri_functions() {
    assert_eq!(
        common::run_both(
            r#"
            function r(f) { try { return f(); } catch (e) { return e.name; } }
            var all = '', i;
            for (i = 32; i < 128; i++) all += String.fromCharCode(i);
            print(encodeURI(all));
            print(encodeURIComponent(all));
            print(encodeURI('\u00e9\u0800\uffff\ud83d\ude00\u0000'), encodeURIComponent('\u07ff\ud800\udc00'));
            print([r(() => encodeURI('\ud800')), r(() => encodeURI('a\udc00')), r(() => encodeURIComponent('\ud800x')), r(() => encodeURIComponent('\ude00\ud83d'))].join());
            print(encodeURI(), encodeURIComponent(null), encodeURI(12.5), encodeURI({ toString: function () { return 'a b'; } }), encodeURI(''));
            print(encodeURI.length, encodeURI.name, encodeURIComponent.length, encodeURIComponent.name);
"#
        ),
        lines(&[
            "%20!%22#$%25&'()*+,-./0123456789:;%3C=%3E?@ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~%7F",
            "%20!%22%23%24%25%26'()*%2B%2C-.%2F0123456789%3A%3B%3C%3D%3E%3F%40ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~%7F",
            "%C3%A9%E0%A0%80%EF%BF%BF%F0%9F%98%80%00 %DF%BF%F0%90%80%80",
            "URIError,URIError,URIError,URIError",
            "undefined null 12.5 a%20b ",
            "1 encodeURI 1 encodeURIComponent",
        ])
    );
}

/// decodeURI and decodeURIComponent (§19.2.6.1-2, 6): decodeURI keeps the escapes of
/// ";/?:@&=+$,#", hex in either case, multi-byte UTF-8, and a URIError for each malformed
/// form (short escape, bad hex, a lone continuation byte, a bad lead byte, an overlong form,
/// a surrogate, above U+10FFFF, a missing continuation).
#[test]
fn decode_uri_functions() {
    assert_eq!(
        common::run_both(
            r#"
            function r(f) { try { return f(); } catch (e) { return e.name; } }
            print(decodeURI('%3B%2F%3F%3A%40%26%3D%2B%24%2C%23%20%41%7e%25'), decodeURIComponent('%3B%2F%3F%3A%40%26%3D%2B%24%2C%23%20%41%7e%25'));
            print(decodeURI('%c3%A9%E0%A0%80%F0%9F%98%80') === '\u00e9\u0800\ud83d\ude00', decodeURIComponent('%EF%BF%BF%F4%8F%BF%BF') === '\uffff\udbff\udfff', decodeURI('%00').charCodeAt(0), decodeURI('\ud800%41') === '\ud800A');
            print(['%', '%4', '%G0', '%0G', 'a%2', '%80', '%BF', '%C0%80', '%C1%BF', '%E0%80%80', '%ED%A0%80', '%F0%80%80%80', '%F4%90%80%80', '%F8%80%80%80%80', '%FF', '%C3', '%C3%41', '%C3A9', '%E0%A0', '%C3%%A9'].map(function (s) { return r(() => decodeURIComponent(s)); }).join());
            print(decodeURI(), decodeURIComponent(null), decodeURI(12), decodeURI(''), decodeURI('%%41'.slice(1)));
            var all = ''; for (var i = 0; i < 0x800; i += 7) all += String.fromCharCode(i);
            print(decodeURIComponent(encodeURIComponent(all)) === all, decodeURI(encodeURI(all)) === all);
            print(decodeURI.length, decodeURI.name, decodeURIComponent.length, decodeURIComponent.name);
"#
        ),
        lines(&[
            "%3B%2F%3F%3A%40%26%3D%2B%24%2C%23 A~% ;/?:@&=+$,# A~%",
            "true true 0 true",
            "URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError,URIError",
            "undefined null 12  A",
            "true true",
            "1 decodeURI 1 decodeURIComponent",
        ])
    );
}

/// The global keys in V8's order (node 24 `Object.getOwnPropertyNames(globalThis)`, the
/// names this crate's global module installs).
#[test]
fn global_function_key_order() {
    assert_eq!(
        common::run_both(
            r#"
            var want = ['decodeURI', 'decodeURIComponent', 'encodeURI', 'encodeURIComponent', 'isFinite', 'isNaN'];
            print(Object.getOwnPropertyNames(globalThis).filter(function (k) { return want.indexOf(k) >= 0; }).join());
"#
        ),
        lines(&["decodeURI,decodeURIComponent,encodeURI,encodeURIComponent,isFinite,isNaN"])
    );
}

/// Error.isError (§20.5.2.1): true for every object with [[ErrorData]] (each native error,
/// AggregateError, a subclass instance, an error thrown by the VM; test262
/// errors-other-realm.js covers another realm),
/// false for primitives and for objects that only look like errors; length, name, {w, c}, no
/// [[Construct]].
#[test]
fn error_is_error() {
    assert_eq!(
        common::run_both(
            r#"
            print(Error.isError(), Error.isError(undefined), Error.isError(null), Error.isError(1), Error.isError('Error'), Error.isError(true), Error.isError(Symbol()));
            class E extends TypeError {}
            var caught; try { null.x; } catch (e) { caught = e; }
            print([new Error('x'), new TypeError(), RangeError(), new ReferenceError(), new SyntaxError(), new EvalError(), new URIError(), new AggregateError([]), new E(), caught].map(Error.isError).join());
            var fake = { __proto__: Error.prototype, name: 'Error', message: '' };
            fake[Symbol.toStringTag] = 'Error';
            print([fake, Error.prototype, TypeError.prototype, Object.create(new Error()), {}, [], function () {}, Error].map(Error.isError).join());
            var d = Object.getOwnPropertyDescriptor(Error, 'isError');
            print(Error.isError.length, Error.isError.name, d.writable, d.enumerable, d.configurable);
            try { new Error.isError(1); } catch (e) { print(e.name); }
"#
        ),
        lines(&[
            "false false false false false false false",
            "true,true,true,true,true,true,true,true,true,true",
            "false,false,false,false,false,false,false,false",
            "1 isError true false true",
            "TypeError",
        ])
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

/// The keys that this crate's modules give their intrinsics (and a function's `length` and
/// `name`) are all there, in V8's order (node 24 `Reflect.ownKeys`; keys not built yet left
/// out). Keys from other tables (the VM table's `String.prototype[@@iterator]`, say) may sit
/// between them.
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
                "isFinite",
                "isInteger",
                "isNaN",
                "isSafeInteger",
                "parseFloat",
                "parseInt",
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
            s(&[
                "constructor",
                "toExponential",
                "toFixed",
                "toPrecision",
                "toString",
                "valueOf",
            ]),
        ),
        (
            IntrinsicId::Error,
            s(&["length", "name", "prototype", "isError"]),
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
            let mut k = s(&["constructor", "toString", "valueOf", "description"]);
            k.extend(wk(&["toStringTag", "toPrimitive"]));
            k
        }),
        (
            IntrinsicId::Reflect,
            s(&[
                "defineProperty",
                "deleteProperty",
                "apply",
                "construct",
                "get",
                "getOwnPropertyDescriptor",
                "getPrototypeOf",
                "has",
                "isExtensible",
                "ownKeys",
                "preventExtensions",
                "set",
                "setPrototypeOf",
                "Symbol(Symbol.toStringTag)",
            ]),
        ),
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
        let got: Vec<String> = keys(id).into_iter().filter(|k| want.contains(k)).collect();
        assert_eq!(got, want, "{}", id.name());
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
                (IntrinsicId::GlobalObject, "decodeURI"),
                (IntrinsicId::GlobalObject, "encodeURIComponent"),
                (IntrinsicId::Error, "isError"),
                (IntrinsicId::NumberPrototype, "toFixed"),
                (IntrinsicId::GlobalObject, "parseFloat"),
                (IntrinsicId::GlobalObject, "parseInt"),
                (IntrinsicId::GlobalObject, "Reflect"),
                (IntrinsicId::Number, "prototype"),
                (IntrinsicId::Number, "MAX_VALUE"),
                (IntrinsicId::Number, "EPSILON"),
                (IntrinsicId::Number, "isSafeInteger"),
                (IntrinsicId::Reflect, "get"),
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
                (IntrinsicId::Reflect, "toStringTag"),
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
        ("globalThis.decodeURI", METHOD),
        ("globalThis.encodeURIComponent", METHOD),
        ("%Error%.isError", METHOD),
        ("%Number.prototype%.toFixed", METHOD),
        ("globalThis.parseFloat", METHOD),
        ("globalThis.parseInt", METHOD),
        ("globalThis.Reflect", METHOD),
        ("%Number%.prototype", NONE),
        ("%Number%.MAX_VALUE", NONE),
        ("%Number%.EPSILON", NONE),
        ("%Number%.isSafeInteger", METHOD),
        ("%Reflect%.get", METHOD),
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
        ("%Reflect%[@@toStringTag]", C),
    ]
    .iter()
    .map(|(k, a)| (k.to_string(), Some(*a)))
    .collect();
    assert_eq!(got, want);
}
