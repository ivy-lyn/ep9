//! TypedArray views (TypedArray TA-1 TA-VM; design `docs/plan/notes/typedarray-design.md`
//! §2.2, §3.2, §4, §9): the element types and their conversions (node v26.13.0 results), out
//! of bounds and detached access, length tracking on resize, canonical numeric keys, the
//! internal methods of §10.4.5, a view as a prototype (MC-w26-4), the elements kind a view
//! keeps (needs N4), and views under GC. Rust-level tests (design §8.1): no JS constructor
//! exists yet. Each test runs with GC stress off and on, then the heap verifier.
//!
//! | Test | What it shows |
//! |---|---|
//! | `each_element_type_round_trips` | every Table 71 type: the stored value read back as node gives it (wrap, clamp, round, BigInt modulo 2^64, -0) |
//! | `a_nan_store_writes_one_pattern` | Float16, Float32, Float64 NaN bytes; a NaN payload read back boxes as the one NaN |
//! | `out_of_bounds_keys_are_not_properties` | §10.4.5.1-6 for an index past the length: no own property, no prototype walk, no store |
//! | `canonical_numeric_keys` | "-0", "1.5", "Infinity", "NaN", "-1" are view keys; "1e3", "01", " 1" are ordinary keys; the key order of `[[OwnPropertyKeys]]` |
//! | `a_set_converts_first_and_a_detach_in_valueof_stores_nothing` | TypedArraySetElement order: valueOf runs once, also out of bounds; a detach in it leaves length 0 and no store |
//! | `detached_views_read_undefined_and_store_nothing` | after DetachArrayBuffer: length none, every element undefined, stores and defines change nothing |
//! | `length_tracking_on_resize` | fixed and auto views on a resizable buffer: out of bounds after a shrink, back after a grow; the data address stays |
//! | `elements_kind_7_or_8_at_birth_and_kept` | IsTypedArrayFixedLength gives 7 or 8; stores, defines, deletes, a named key and a new prototype keep it (N2, N3, N4) |
//! | `a_view_as_a_prototype` | MC-w26-4 in Rust: a length-0 view stops an index store below it; a length-4 view hides `Object.prototype[9]`; a holey Array under a view |
//! | `views_and_buffers_under_gc` | live views keep their buffer and bytes over minor and major GCs |
//! | `a_named_ic_case_stops_at_a_view` | no named load or store IC case looks past a view, or caches a miss or an add, for "1.5", "-0", "Infinity", "NaN" (`ic::exotic_name`) |
//! | `a_view_as_a_dictionary_keeps_its_fields` | a dictionary view keeps its kind, fixed fields and elements (N3) |
//! | `a_view_never_transitions` | `elements::transition` refuses a view in debug builds (N4) |
//! | `integrity_levels_of_a_view` | SetIntegrityLevel and TestIntegrityLevel over a view's internal methods: no debug panic, the spec results (N3; TODO TA3-PREVEXT) |
//! | `no_length_on_a_fixed_buffer_computes_it` | InitializeTypedArrayFromArrayBuffer step 9.a: a fixed view with the computed length; the RangeErrors |
//! | `canonical_numeric_matches_the_spec_text` | `canonical_numeric` against the plain §7.1.21 text over a table of keys, one-byte and two-byte |
//! | `a_view_receiver_takes_its_define_hook` | OrdinarySet with a view as the receiver (Reflect.set, `super[k] = v`): CreateDataProperty is the view's `[[DefineOwnProperty]]`, so "-0", an index out of range or on a detached view gives false and stores nothing; the named IC case stays SLOW (Codex TA-1 r1 finding 2) |
//! | `integer_conversions_at_the_edges` | ToInt8 ... ToUint32 and ToUint8Clamp at NaN, ±0, ±∞, negative fractions, 2^31, 2^32, 2^53 and above, as node gives them (H36: no `fmod`, no FMA) |

use std::cell::Cell;
use std::hint::black_box;

use super::ic::{int, key};
use super::{config, isolate, on_big_stack};
use crate::array_buffer as ab;
use crate::descriptor::Desc;
use crate::elements::{self, kind as ek};
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::object::{self, attr};
use crate::runtime::{Runtime, val};
use crate::typed_array as ta;
use crate::{Dispatch, Isolate, Local, kinds, property, shape};

/// Runs `test` with the interpreter context of a fresh isolate, with GC stress off and on, on
/// a big stack; then a collection and the heap verifier.
fn each(test: fn(&mut Cx)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let r = iso.with_cx(|cx| {
                test(cx.inner());
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// A new ArrayBuffer (null prototype) of `len` bytes, resizable to `max` with `Some`.
pub(super) fn buffer(rt: &Runtime, len: u64, max: Option<u64>) -> u64 {
    ab::allocate(rt, NULL, len, max).unwrap()
}

/// A new TypedArray (null prototype) of element type `t` on `b`.
pub(super) fn view(rt: &Runtime, t: u8, b: u64, offset: u64, len: Option<u64>) -> u64 {
    ta::new_view(rt, NULL, t, b, offset, len).unwrap()
}

fn get(cx: &mut Cx, o: u64, name: &str) -> u64 {
    let k = key(cx.rt(), name);
    property::get(cx, o, k).unwrap()
}

/// `o[name] = v` in strict code.
fn put(cx: &mut Cx, o: u64, name: &str, v: u64) {
    let k = key(cx.rt(), name);
    property::set(cx, o, k, v, true).unwrap();
}

fn has(cx: &mut Cx, o: u64, name: &str) -> bool {
    let k = key(cx.rt(), name);
    property::try_has_property(cx, o, k).unwrap()
}

fn has_own(cx: &mut Cx, o: u64, name: &str) -> bool {
    let k = key(cx.rt(), name);
    property::get_own_property(cx, o, k).unwrap().is_some()
}

fn num(v: u64) -> f64 {
    val(v).as_number().expect("a Number")
}

/// The own keys of `o` as Rust strings.
fn own_keys(cx: &mut Cx, o: u64) -> Vec<String> {
    let keys = property::own_property_keys(cx, o);
    (0..object::array_len(keys))
        .map(|i| crate::string::to_rust_string(object::array_get(keys, i).unwrap()))
        .collect()
}

/// A value that the test stores: a Number or a BigInt (made in the isolate).
#[derive(Clone, Copy, Debug)]
enum In {
    N(f64),
    B(i128),
}

/// What a load gives: a Number (compared by bits: -0 and NaN too) or a BigInt.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Out {
    N(f64),
    B(i128),
}

fn value_of(rt: &Runtime, x: In) -> u64 {
    match x {
        In::N(d) => crate::conv::num(d),
        In::B(n) => crate::bigint::from_i128(rt, n),
    }
}

fn out_of(v: u64) -> Out {
    if kinds::is_bigint(v) {
        Out::B(crate::bigint::to_i128(v).unwrap())
    } else {
        Out::N(num(v))
    }
}

fn same(a: Out, b: Out) -> bool {
    match (a, b) {
        (Out::N(x), Out::N(y)) => x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

/// One element type: its number, its name and its (stored value, read value) pairs.
type Row = (u8, &'static str, Vec<(In, Out)>);

/// The node v26.13.0 results of `new XArray(1)[0] = x; [0]` for each element type.
fn rows() -> Vec<Row> {
    use In as I;
    use Out as O;
    let inf = f64::INFINITY;
    vec![
        (
            0,
            "Int8",
            vec![
                (I::N(127.0), O::N(127.0)),
                (I::N(128.0), O::N(-128.0)),
                (I::N(-129.0), O::N(127.0)),
                (I::N(1.9), O::N(1.0)),
                (I::N(-1.9), O::N(-1.0)),
                (I::N(f64::NAN), O::N(0.0)),
                (I::N(inf), O::N(0.0)),
                (I::N(-0.0), O::N(0.0)),
                (I::N(1e300), O::N(0.0)),
            ],
        ),
        (
            1,
            "Uint8",
            vec![
                (I::N(256.0), O::N(0.0)),
                (I::N(-1.0), O::N(255.0)),
                (I::N(3.7), O::N(3.0)),
            ],
        ),
        (
            2,
            "Uint8Clamped",
            vec![
                (I::N(1.5), O::N(2.0)),
                (I::N(2.5), O::N(2.0)),
                (I::N(-1.0), O::N(0.0)),
                (I::N(300.0), O::N(255.0)),
                (I::N(f64::NAN), O::N(0.0)),
                (I::N(254.5), O::N(254.0)),
                (I::N(0.5), O::N(0.0)),
                (I::N(1.4999), O::N(1.0)),
                (I::N(-0.0), O::N(0.0)),
            ],
        ),
        (
            3,
            "Int16",
            vec![(I::N(32768.0), O::N(-32768.0)), (I::N(-1.0), O::N(-1.0))],
        ),
        (
            4,
            "Uint16",
            vec![(I::N(-1.0), O::N(65535.0)), (I::N(65536.5), O::N(0.0))],
        ),
        (
            5,
            "Int32",
            vec![
                (I::N(2147483648.0), O::N(-2147483648.0)),
                (I::N(4294967301.0), O::N(5.0)),
                (I::N(-2147483649.0), O::N(2147483647.0)),
            ],
        ),
        (
            6,
            "Uint32",
            vec![
                (I::N(-1.0), O::N(4294967295.0)),
                (I::N(2147483653.0), O::N(2147483653.0)),
                (I::N(4294967296.0), O::N(0.0)),
                (I::N(9007199254740993.0), O::N(0.0)),
            ],
        ),
        (
            7,
            "BigInt64",
            vec![
                (I::B(1 << 63), O::B(-(1 << 63))),
                (I::B(-1), O::B(-1)),
                (I::B((1 << 64) + 7), O::B(7)),
                (I::B(-(1 << 64) - 1), O::B(-1)),
            ],
        ),
        (
            8,
            "BigUint64",
            vec![
                (I::B(-1), O::B((1 << 64) - 1)),
                (I::B(1 << 63), O::B(1 << 63)),
                (I::B(1 << 64), O::B(0)),
            ],
        ),
        (
            9,
            "Float16",
            vec![
                (I::N(1.0 / 3.0), O::N(0.333251953125)),
                (I::N(65520.0), O::N(inf)),
                (I::N(65519.0), O::N(65504.0)),
                (I::N(-0.0), O::N(-0.0)),
                (I::N(f64::NAN), O::N(f64::NAN)),
                (I::N(5.960464477539063e-8), O::N(5.960464477539063e-8)),
                (I::N(2.98e-8), O::N(0.0)),
                (I::N(6.1e-5), O::N(0.00006097555160522461)),
            ],
        ),
        (
            10,
            "Float32",
            vec![
                (I::N(0.1), O::N(0.10000000149011612)),
                (I::N(1e40), O::N(inf)),
                (I::N(-0.0), O::N(-0.0)),
                (I::N(f64::NAN), O::N(f64::NAN)),
            ],
        ),
        (
            11,
            "Float64",
            vec![
                (I::N(-0.0), O::N(-0.0)),
                (I::N(f64::NAN), O::N(f64::NAN)),
                (I::N(1e308), O::N(1e308)),
                (I::N(f64::MIN_POSITIVE / 2.0), O::N(f64::MIN_POSITIVE / 2.0)),
            ],
        ),
    ]
}

#[test]
fn each_element_type_round_trips() {
    each(|cx| {
        let rows = rows();
        assert_eq!(rows.len(), 12, "every Table 71 row");
        for (t, name, cases) in rows {
            let rt = cx.rt();
            let b = buffer(rt, 16, None);
            let v = view(rt, t, b, 8, Some(1));
            assert_eq!(ta::element_type(v), t, "{name}");
            for (x, want) in cases {
                let x = value_of(cx.rt(), x);
                put(cx, v, "0", x);
                let got = out_of(get(cx, v, "0"));
                assert!(same(got, want), "{name}: {x:?} -> {got:?}, want {want:?}");
                // The same bytes through the buffer (GetValueFromBuffer, little endian).
                let raw = ab::get_value(cx.rt(), b, 8, t, true).unwrap();
                assert!(same(out_of(raw), want), "{name}: the buffer gives {raw:#x}");
            }
            // A wrong-type store throws before anything is stored: a Number for a BigInt
            // view (ToBigInt TypeError), a BigInt for a Number view (ToNumber TypeError).
            let wrong = if ab::is_bigint_elem(t) {
                int(1)
            } else {
                crate::bigint::from_i128(cx.rt(), 1)
            };
            let k = key(cx.rt(), "0");
            assert!(property::set(cx, v, k, wrong, true).is_err(), "{name}");
            cx.rt().take_pending();
        }
    });
}

#[test]
fn a_nan_store_writes_one_pattern() {
    each(|cx| {
        let rt = cx.rt();
        let b = buffer(rt, 8, None);
        let bytes = view(rt, 1, b, 0, Some(8));
        // (element type, the NaN bytes node writes, little endian).
        for (t, want) in [
            (9u8, vec![0u8, 0x7E]),
            (10, vec![0, 0, 0xC0, 0x7F]),
            (11, vec![0, 0, 0, 0, 0, 0, 0xF8, 0x7F]),
        ] {
            let rt = cx.rt();
            let f = view(rt, t, b, 0, Some(1));
            // A NaN with another payload and the sign bit set is still the one pattern.
            let odd = crate::conv::num(f64::from_bits(0xFFF4_0000_0000_0001));
            for x in [crate::conv::num(f64::NAN), odd] {
                put(cx, f, "0", x);
                let got: Vec<u8> = (0..want.len())
                    .map(|i| num(get(cx, bytes, &i.to_string())) as u8)
                    .collect();
                assert_eq!(got, want, "type {t}");
            }
            // Bytes of another NaN, read through the float view: the one boxed NaN.
            for i in 0..8 {
                put(cx, bytes, &i.to_string(), int(0xFF));
            }
            let v = get(cx, f, "0");
            assert!(num(v).is_nan(), "type {t}");
            assert_eq!(v, crate::conv::num(f64::NAN), "type {t}: the one NaN Value");
        }
    });
}

#[test]
fn out_of_bounds_keys_are_not_properties() {
    each(|cx| {
        let rt = cx.rt();
        let op = super::ic::object_prototype(rt);
        // Object.prototype has these keys: a view never sees them.
        for name in ["9", "4", "-0", "1.5", "Infinity", "NaN", "-1"] {
            let k = key(rt, name);
            crate::props::own_define(rt, op, k, int(77), attr::DEFAULT);
        }
        let b = buffer(rt, 4, None);
        let v = ta::new_view(rt, op, 0, b, 0, Some(4)).unwrap();
        put(cx, v, "3", int(-3));
        for name in ["4", "9", "-0", "1.5", "Infinity", "NaN", "-1"] {
            assert_eq!(get(cx, v, name), UNDEFINED, "[[Get]] {name}: no walk");
            assert!(!has(cx, v, name), "[[HasProperty]] {name}");
            assert!(!has_own(cx, v, name), "[[GetOwnProperty]] {name}");
            put(cx, v, name, int(5));
            assert!(!has_own(cx, v, name), "[[Set]] {name} stores nothing");
            let k = key(cx.rt(), name);
            let d = Desc::data(int(1), attr::DEFAULT);
            assert!(
                !property::define_own_property(cx, v, k, d).unwrap(),
                "{name}"
            );
            assert!(
                property::internal_delete(cx, v, k).unwrap(),
                "[[Delete]] {name}"
            );
            assert!(
                property::define(cx, v, k, int(1)).is_err(),
                "CreateDataPropertyOrThrow {name}"
            );
            cx.rt().take_pending();
        }
        // An element is {w, e, c} and cannot be deleted; a define with other attributes fails.
        let k = key(cx.rt(), "3");
        let d = property::get_own_property(cx, v, k).unwrap().unwrap();
        assert_eq!(
            (d.value, d.writable, d.enumerable, d.configurable),
            (Some(int(-3)), Some(true), Some(true), Some(true))
        );
        assert!(!property::internal_delete(cx, v, k).unwrap());
        for d in [
            Desc::data(int(1), attr::WRITABLE | attr::ENUMERABLE),
            Desc::data(int(1), attr::WRITABLE | attr::CONFIGURABLE),
            Desc::data(int(1), attr::ENUMERABLE | attr::CONFIGURABLE),
        ] {
            assert!(!property::define_own_property(cx, v, k, d).unwrap());
        }
        let acc = Desc {
            get: Some(UNDEFINED),
            ..Desc::default()
        };
        assert!(!property::define_own_property(cx, v, k, acc).unwrap());
        assert_eq!(get(cx, v, "3"), int(-3));
        let d = Desc {
            value: Some(int(9)),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, v, k, d).unwrap());
        assert_eq!(get(cx, v, "3"), int(9));
        // Integer-index paths (no key): get, has, set, delete.
        assert_eq!(property::get_index(cx, v, 3).unwrap(), int(9));
        assert_eq!(property::get_index(cx, v, 9).unwrap(), UNDEFINED);
        assert!(property::try_has_index(cx, v, 0).unwrap());
        assert!(!property::try_has_index(cx, v, 9).unwrap());
        assert!(property::set_index(cx, v, 0, int(4)).unwrap());
        assert!(property::set_index(cx, v, 9, int(4)).unwrap());
        assert_eq!(property::get_index(cx, v, 0).unwrap(), int(4));
        assert!(!property::delete_index(cx, v, 0, false).unwrap());
        assert!(property::delete_index(cx, v, 9, false).unwrap());
        // A keyed load and store with an int32 key and a negative one.
        assert_eq!(property::get_keyed(cx, v, int(0)).unwrap(), int(4));
        property::set_keyed(cx, v, int(-1), int(3), true).unwrap();
        assert_eq!(property::get_keyed(cx, v, int(-1)).unwrap(), UNDEFINED);
        assert_eq!(own_keys(cx, v), ["0", "1", "2", "3"]);
    });
}

#[test]
fn canonical_numeric_keys() {
    each(|cx| {
        let rt = cx.rt();
        for (name, want) in [
            ("0", Some(0.0)),
            ("4294967295", Some(4294967295.0)),
            ("-0", Some(-0.0)),
            ("1.5", Some(1.5)),
            ("Infinity", Some(f64::INFINITY)),
            ("-Infinity", Some(f64::NEG_INFINITY)),
            ("-1", Some(-1.0)),
            ("1e+21", Some(1e21)),
            ("1e-7", Some(1e-7)),
            ("1e3", None),
            ("01", None),
            (" 1", None),
            ("+1", None),
            ("-", None),
            ("", None),
            ("0x1", None),
            ("1.50", None),
            ("1e21", None),
            ("Infinityx", None),
            ("-NaN", None),
            ("\u{661}", None),
        ] {
            let got = ta::canonical_numeric(key(rt, name));
            assert_eq!(got.map(f64::to_bits), want.map(f64::to_bits), "{name:?}");
        }
        assert!(ta::canonical_numeric(key(rt, "NaN")).unwrap().is_nan());
        assert_eq!(
            ta::canonical_numeric(crate::symbol::new_symbol(rt, UNDEFINED)),
            None
        );
        // On a view: numeric keys define nothing; others are ordinary properties (node:
        // `ta["-0"] = 5; ta["01"] = 5; ta["1e3"] = 7` gives the keys 0, 1, 2, 3, 01, 1e3).
        let b = buffer(rt, 4, None);
        let v = view(rt, 0, b, 0, Some(4));
        put(cx, v, "-0", int(5));
        put(cx, v, "01", int(5));
        put(cx, v, "1e3", int(7));
        put(cx, v, "foo", int(1));
        assert_eq!(own_keys(cx, v), ["0", "1", "2", "3", "01", "1e3", "foo"]);
        assert_eq!(get(cx, v, "1e3"), int(7));
        assert_eq!(get(cx, v, "0"), int(0), "\"-0\" is not \"0\"");
        assert!(has(cx, v, "01") && !has(cx, v, "-0"));
    });
}

thread_local! {
    /// The buffer that [`detach_in_valueof`] detaches, and its calls.
    static TARGET: Cell<u64> = const { Cell::new(0) };
    static CALLS: Cell<u32> = const { Cell::new(0) };
}

/// A `valueOf` host function: counts its call, detaches [`TARGET`] if set, returns 7.
fn detach_in_valueof<'s>(
    cx: &mut crate::Cx<'s>,
    _this: Local<'s>,
    _args: crate::Args<'s>,
    _nt: Local<'s>,
) -> crate::JsResult<Local<'s>> {
    CALLS.with(|c| c.set(c.get() + 1));
    let b = TARGET.with(Cell::get);
    if b != 0 {
        cx.detach_array_buffer(Local::<crate::JsValue>::from_bits(b), Local::undefined())?;
    }
    Ok(Local::number(7.0))
}

/// An object whose `valueOf` is [`detach_in_valueof`].
fn valueof_object(iso: &mut Isolate) -> u64 {
    iso.register_host_function("detachInValueOf", 0, detach_in_valueof);
    iso.with_cx(|cx| {
        let g = cx.global();
        let f = cx.get(g, "detachInValueOf")?;
        let o = cx.new_object();
        cx.set(o, "valueOf", f, true)?;
        Ok(o.bits())
    })
    .unwrap()
}

#[test]
fn a_set_converts_first_and_a_detach_in_valueof_stores_nothing() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let o = valueof_object(&mut iso);
            let r = iso.with_cx(|cx| {
                let icx = cx.inner();
                let rt = icx.rt();
                // Out of bounds: valueOf still runs once (node: `ta[5] = {valueOf}`).
                let b = buffer(rt, 2, None);
                let v = view(rt, 0, b, 0, Some(2));
                CALLS.with(|c| c.set(0));
                TARGET.with(|t| t.set(0));
                put(icx, v, "5", o);
                assert_eq!(CALLS.with(Cell::get), 1);
                put(icx, v, "1", o);
                assert_eq!(get(icx, v, "1"), int(7));
                // A valueOf that detaches: length 0, nothing stored, no throw.
                TARGET.with(|t| t.set(b));
                put(icx, v, "0", o);
                assert_eq!(CALLS.with(Cell::get), 3);
                assert!(ab::is_detached(b));
                assert_eq!(ta::length(icx.rt(), v), None);
                assert_eq!(get(icx, v, "0"), UNDEFINED);
                // The same through [[DefineOwnProperty]] with a value: true, nothing stored.
                let rt = icx.rt();
                let b2 = buffer(rt, 2, None);
                let v2 = view(rt, 0, b2, 0, Some(2));
                TARGET.with(|t| t.set(b2));
                let k = key(icx.rt(), "0");
                let d = Desc {
                    value: Some(o),
                    ..Desc::default()
                };
                assert!(property::define_own_property(icx, v2, k, d).unwrap());
                assert_eq!(get(icx, v2, "0"), UNDEFINED);
                TARGET.with(|t| t.set(0));
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

#[test]
fn detached_views_read_undefined_and_store_nothing() {
    each(|cx| {
        let rt = cx.rt();
        let b = buffer(rt, 8, None);
        let fixed = view(rt, 5, b, 0, Some(2));
        // No length on a fixed-length buffer: the length is computed, the view is fixed.
        let computed = view(rt, 1, b, 2, None);
        let rab = buffer(rt, 8, Some(16));
        let auto = view(rt, 1, rab, 2, None);
        put(cx, fixed, "1", int(9));
        assert_eq!(ta::length(cx.rt(), computed), Some(6));
        assert_eq!(ta::length(cx.rt(), auto), Some(6));
        ab::detach(cx.rt(), b, UNDEFINED).unwrap();
        ab::detach(cx.rt(), rab, UNDEFINED).unwrap();
        for v in [fixed, computed, auto] {
            assert_eq!(ta::length(cx.rt(), v), None);
            assert_eq!(get(cx, v, "0"), UNDEFINED);
            assert!(!has(cx, v, "0"));
            put(cx, v, "0", int(1));
            assert_eq!(own_keys(cx, v), Vec::<String>::new());
            let k = key(cx.rt(), "0");
            let d = Desc::data(int(1), attr::DEFAULT);
            assert!(!property::define_own_property(cx, v, k, d).unwrap());
            assert!(property::internal_delete(cx, v, k).unwrap());
        }
        // The fields after the detach (design §5.2): a null data word; a fixed view's length
        // is 0, the auto view stays auto; the byte offsets stay.
        assert_eq!(ta::fixed_length(fixed), Some(0));
        assert_eq!(ta::fixed_length(computed), Some(0));
        assert_eq!(ta::fixed_length(auto), None);
        assert_eq!((ta::byte_offset(fixed), ta::byte_offset(computed)), (0, 2));
        assert_eq!(ta::byte_offset(auto), 2);
        assert_eq!((ta::buffer_of(computed), ta::buffer_of(auto)), (b, rab));
        // A second detach does nothing; a wrong key fails first (SameValue).
        ab::detach(cx.rt(), b, UNDEFINED).unwrap();
        assert!(ab::detach(cx.rt(), b, int(1)).is_err());
        // A detached buffer gives no new view.
        assert!(ta::new_view(cx.rt(), NULL, 0, b, 0, None).is_err());
    });
}

#[test]
fn length_tracking_on_resize() {
    each(|cx| {
        let rt = cx.rt();
        let b = buffer(rt, 8, Some(16));
        let fixed = view(rt, 0, b, 0, Some(8));
        let auto = view(rt, 0, b, 0, None);
        let auto16 = view(rt, 3, b, 2, None);
        let base = ab::data_base(cx.rt(), b);
        put(cx, auto, "6", int(-1));
        put(cx, auto, "1", int(2));
        assert_eq!(ta::length(cx.rt(), auto16), Some(3));
        // node: `resize(4)` of an 8-byte buffer: the fixed view is out of bounds (length 0),
        // the auto view follows.
        ab::resize(cx.rt(), b, 4).unwrap();
        assert_eq!(ta::length(cx.rt(), fixed), None);
        assert_eq!(ta::length(cx.rt(), auto), Some(4));
        assert_eq!(ta::length(cx.rt(), auto16), Some(1));
        assert_eq!(get(cx, fixed, "1"), UNDEFINED);
        assert_eq!(get(cx, auto, "1"), int(2));
        assert_eq!(get(cx, auto, "6"), UNDEFINED);
        assert_eq!(own_keys(cx, fixed), Vec::<String>::new());
        // Grow: the fixed view is back, the bytes past the old length read 0, the base stays.
        ab::resize(cx.rt(), b, 16).unwrap();
        assert_eq!(ab::data_base(cx.rt(), b), base, "the store keeps its base");
        assert_eq!(ta::length(cx.rt(), fixed), Some(8));
        assert_eq!(ta::length(cx.rt(), auto), Some(16));
        assert_eq!(ta::length(cx.rt(), auto16), Some(7));
        assert_eq!(get(cx, fixed, "6"), int(0), "a shrink zeroed the bytes");
        assert_eq!(get(cx, fixed, "1"), int(2));
        put(cx, auto, "15", int(5));
        assert_eq!(get(cx, auto, "15"), int(5));
        // A shrink below the offset of a view puts an auto view out of bounds too.
        ab::resize(cx.rt(), b, 1).unwrap();
        assert_eq!(ta::length(cx.rt(), auto16), None);
        assert_eq!(ta::length(cx.rt(), auto), Some(1));
        ab::resize(cx.rt(), b, 0).unwrap();
        assert_eq!(ta::length(cx.rt(), auto), Some(0));
        assert_eq!(ab::data_base(cx.rt(), b), base);
    });
}

#[test]
fn elements_kind_7_or_8_at_birth_and_kept() {
    each(|cx| {
        let rt = cx.rt();
        let fixed_ab = buffer(rt, 8, None);
        let rab = buffer(rt, 8, Some(16));
        let sab = ab::allocate_shared(rt, NULL, 8, None).unwrap();
        let gsab = ab::allocate_shared(rt, NULL, 8, Some(16)).unwrap();
        // (buffer, length, the kind by IsTypedArrayFixedLength).
        let cases = [
            (fixed_ab, Some(2), ek::TYPED_FIXED),
            (fixed_ab, None, ek::TYPED_FIXED),
            (rab, Some(2), ek::TYPED_VARIABLE),
            (rab, None, ek::TYPED_VARIABLE),
            (sab, Some(2), ek::TYPED_FIXED),
            (sab, None, ek::TYPED_FIXED),
            (gsab, Some(2), ek::TYPED_FIXED),
            (gsab, None, ek::TYPED_VARIABLE),
        ];
        let mut views = Vec::new();
        for (b, len, want) in cases {
            let v = view(cx.rt(), 5, b, 0, len);
            assert_eq!(elements::kind_of(v), want);
            assert!(
                elements::storage_form_ok(v),
                "a real view passes the form check"
            );
            assert_eq!(
                elements::check(v),
                Ok(()),
                "and the elements check (needs N2)"
            );
            assert_eq!(ta::is_fixed_length(v), want == ek::TYPED_FIXED);
            views.push((v, want));
        }
        let op = super::ic::object_prototype(cx.rt());
        for (v, want) in views {
            // Element stores, defines and deletes, a named property, a new prototype (a
            // dictionary view, `object::fixed_count` of a view kind, needs N3): the kind stays
            // (no view reaches `elements::transition`, needs N4).
            put(cx, v, "0", int(1));
            put(cx, v, "1", crate::conv::num(0.5));
            put(cx, v, "9", int(1));
            let k = key(cx.rt(), "0");
            let d = Desc {
                value: Some(int(3)),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, v, k, d).unwrap());
            assert!(!property::internal_delete(cx, v, k).unwrap());
            put(cx, v, "name", int(2));
            assert!(property::set_prototype_of(cx.rt(), v, op));
            assert_eq!(elements::kind_of(v), want);
            assert!(elements::storage_form_ok(v));
            assert_eq!(elements::check(v), Ok(()));
            assert_eq!(
                object::rd(object::cell(v), object::obj::ELEMENTS),
                UNDEFINED
            );
            assert_eq!(get(cx, v, "name"), int(2));
            assert_eq!(get(cx, v, "toString"), get(cx, op, "toString"));
        }
    });
}

#[test]
fn a_view_as_a_prototype() {
    each(|cx| {
        let rt = cx.rt();
        let op = super::ic::object_prototype(rt);
        // node: `o = Object.create(new Int8Array(0)); o[0] = 1` gives no own "0".
        let empty = view(rt, 0, buffer(rt, 0, None), 0, Some(0));
        let o = object::new_object(rt, empty);
        assert!(
            shape::proto_index_keys(cx.rt()),
            "a view prototype pops the fuse"
        );
        put(cx, o, "0", int(1));
        property::set_keyed(cx, o, int(1), int(1), true).unwrap();
        assert!(!has_own(cx, o, "0") && !has_own(cx, o, "1"));
        assert_eq!(get(cx, o, "0"), UNDEFINED);
        // A valid index with another receiver: OrdinarySet, so the receiver gets the property
        // (node: `Object.create(new Int8Array(2))[0] = 1` gives own "0").
        let rt = cx.rt();
        let two = view(rt, 0, buffer(rt, 2, None), 0, Some(2));
        let o2 = object::new_object(rt, two);
        put(cx, o2, "0", int(1));
        assert!(has_own(cx, o2, "0"));
        assert_eq!(get(cx, two, "0"), int(0), "the view is unchanged");
        // A length-4 view prototype: index 9 is not looked up past the view.
        let rt = cx.rt();
        crate::props::own_define(rt, op, key(rt, "9"), int(77), attr::DEFAULT);
        let four = view(rt, 0, buffer(rt, 4, None), 0, Some(4));
        let o4 = object::new_object(rt, four);
        assert_eq!(get(cx, o4, "9"), UNDEFINED);
        assert!(!has(cx, o4, "9"));
        assert_eq!(property::get_index(cx, o4, 9).unwrap(), UNDEFINED);
        put(cx, o4, "9", int(1));
        assert!(!has_own(cx, o4, "9"));
        // A holey Array whose chain gets a view later (node: `a = []; setPrototypeOf(a, new
        // Int8Array(0)); a[1] = 1` leaves a.length 0; with a length-4 view, a[1] = 1 is own).
        let rt = cx.rt();
        let a = object::new_array(rt, 0);
        assert!(property::set_prototype_of(rt, a, empty));
        put(cx, a, "1", int(1));
        property::set_keyed(cx, a, int(2), int(1), true).unwrap();
        assert_eq!((object::array_len(a), get(cx, a, "1")), (0, UNDEFINED));
        let rt = cx.rt();
        let h = object::new_array(rt, 0);
        object::set_array_len(rt, h, 2);
        assert!(property::set_prototype_of(rt, h, four));
        property::set_keyed(cx, h, int(1), int(1), true).unwrap();
        assert_eq!((object::array_len(h), get(cx, h, "1")), (2, int(1)));
        assert_eq!(
            get(cx, h, "0"),
            int(0),
            "the hole reads the view's element 0"
        );
    });
}

/// Makes views and buffers that nothing keeps and some that the test keeps; the kept ones
/// keep their bytes over minor and major GCs.
#[test]
fn views_and_buffers_under_gc() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.generational = true;
            let mut iso = isolate(c);
            let r = iso.with_cx(|cx| {
                let icx = cx.inner();
                // The kept views live in a JS Array (a Rust `Vec` is no root); the rows in Rust.
                let keep = object::new_array(icx.rt(), 0);
                let mut kept = Vec::new();
                for i in 0..24u8 {
                    let rt = icx.rt();
                    let len = 64 + u64::from(i) * 512;
                    let max = (i % 3 == 0).then_some(len * 2);
                    let b = buffer(rt, len, max);
                    let t = i % 12;
                    let v = view(rt, t, b, 0, None);
                    // A dead view on the same buffer.
                    black_box(view(rt, t, b, 0, None));
                    let x = if ab::is_bigint_elem(t) {
                        crate::bigint::from_i128(icx.rt(), i128::from(i) - 5)
                    } else {
                        int(i32::from(i))
                    };
                    put(icx, v, "1", x);
                    if i % 2 == 0 {
                        assert!(object::array_set(icx.rt(), keep, u32::from(i / 2), v));
                        kept.push((u32::from(i / 2), t, i));
                    }
                    if i % 5 == 0 {
                        icx.rt().heap.collect_minor();
                    }
                }
                for minor in [true, false, true] {
                    if minor {
                        icx.rt().heap.collect_minor();
                    } else {
                        icx.rt().heap.collect();
                    }
                    for &(at, t, i) in &kept {
                        let v = object::array_get(keep, at).unwrap();
                        let want = if ab::is_bigint_elem(t) {
                            Out::B(i128::from(i) - 5)
                        } else {
                            Out::N(f64::from(i))
                        };
                        assert!(same(out_of(get(icx, v, "1")), want), "view {i}");
                        assert!(kinds::is_object(ta::buffer_of(v)));
                    }
                }
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// A named IC case never looks past a view for a canonical numeric key that is not an array
/// index, and never caches a miss or an add for one (`ic::exotic_name`): node gives undefined
/// for `ta["1.5"]`, `ta["-0"]` and `Object.create(ta)["1.5"]` even when `Object.prototype`
/// has the key, and `Object.create(ta)["1.5"] = 1` adds no own property (§10.4.5.5: the view
/// on the chain answers). The keyed IC has no such case: its fast case takes an Array and an
/// int32 key only (`keyed_ic::present_element`).
#[test]
fn a_named_ic_case_stops_at_a_view() {
    each(|cx| {
        let rt = cx.rt();
        let op = super::ic::object_prototype(rt);
        for (name, x) in [("1.5", 77), ("-0", 78), ("Infinity", 79), ("NaN", 80)] {
            crate::props::own_define(rt, op, key(rt, name), int(x), attr::DEFAULT);
        }
        let b = buffer(rt, 4, None);
        let v = ta::new_view(rt, op, 0, b, 0, Some(4)).unwrap();
        let o = object::new_object(rt, v);
        for name in ["1.5", "-0", "Infinity", "NaN", "-Infinity"] {
            for r in [v, o] {
                let k = key(cx.rt(), name);
                let case = crate::ic::load_case(cx.rt(), r, k).map(|c| c.handler);
                assert!(
                    !matches!(
                        case,
                        Some(
                            crate::ic::Handler::ProtoLoad { .. }
                                | crate::ic::Handler::Missing { .. }
                        )
                    ),
                    "load {name}: {case:?}"
                );
                // A define (0x25, CreateDataProperty) on `o` adds an own property: the chain
                // plays no part, so only the view itself refuses it.
                for define in [false, true].into_iter().filter(|&d| !d || r == v) {
                    let pre = crate::ic::store_pre(cx.rt(), r, k, define);
                    assert!(
                        !matches!(pre, Some(crate::ic::StorePre::Add { .. })),
                        "store {name} (define {define}): {pre:?}"
                    );
                }
                assert_eq!(get(cx, r, name), UNDEFINED, "{name}");
            }
            put(cx, o, name, int(1));
            assert!(
                !has_own(cx, o, name),
                "{name}: the view on the chain stores nothing"
            );
        }
        // An ordinary key still gets its cases.
        let k = key(cx.rt(), "toString");
        let case = crate::ic::load_case(cx.rt(), v, k).map(|c| c.handler);
        assert!(
            matches!(case, Some(crate::ic::Handler::ProtoLoad { .. })),
            "{case:?}"
        );
    });
}

/// A view that becomes a dictionary (a new prototype, `object::set_proto`; a delete of a named
/// property) keeps its kind, its fixed fields and its elements (`object::fixed_count` of a view
/// kind is 4, needs N3).
#[test]
fn a_view_as_a_dictionary_keeps_its_fields() {
    each(|cx| {
        let rt = cx.rt();
        let op = super::ic::object_prototype(rt);
        let b = buffer(rt, 8, None);
        let v = view(rt, 5, b, 0, Some(2));
        put(cx, v, "0", int(-3));
        put(cx, v, "name", int(2));
        put(cx, v, "other", int(4));
        assert!(property::set_prototype_of(cx.rt(), v, op));
        let k = key(cx.rt(), "other");
        assert!(property::internal_delete(cx, v, k).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(v)));
        assert_eq!(elements::kind_of(v), ek::TYPED_FIXED);
        assert!(elements::storage_form_ok(v));
        assert_eq!(elements::check(v), Ok(()));
        assert_eq!(ta::buffer_of(v), b);
        assert_eq!(ta::length(cx.rt(), v), Some(2));
        assert_eq!(get(cx, v, "0"), int(-3));
        assert_eq!(get(cx, v, "name"), int(2));
        assert!(!has_own(cx, v, "other"));
        assert_eq!(get(cx, v, "toString"), get(cx, op, "toString"));
    });
}

/// No transition takes a view off kind 7 or 8 (`elements::transition` asserts it in debug
/// builds; needs N4): `join(7 or 8, x)` is DICTIONARY, so a transition would break the view.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "a view never changes its elements kind")]
fn a_view_never_transitions() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let rt: &Runtime = &iso.rt;
    let v = view(rt, 0, buffer(rt, 4, None), 0, Some(4));
    elements::transition(rt, v, ek::DICTIONARY);
    black_box(&mut iso);
}

/// SetIntegrityLevel (§7.3.15) and TestIntegrityLevel (§7.3.16) over the internal methods of a
/// view, as `Object.freeze`, `Object.seal`, `Object.preventExtensions`, `Object.isFrozen` and
/// `Object.isSealed` run them (ep9_builtins `object.rs`); a debug build used to panic in
/// `object::fixed_count` (needs N3). The results are the spec's (§10.4.5.3: an element is
/// configurable, so a define with `[[Configurable]]: false` is refused, and freeze or seal
/// of a view with elements throws a TypeError; a length-0 view freezes).
#[test]
fn integrity_levels_of_a_view() {
    /// SetIntegrityLevel(o, frozen or sealed) with DefinePropertyOrThrow as a `bool`: false
    /// where the builtin throws the TypeError.
    fn set_level(cx: &mut Cx, o: u64, frozen: bool) -> bool {
        if !property::prevent_extensions(cx.rt(), o) {
            return false;
        }
        let keys = property::own_property_keys(cx, o);
        for i in 0..object::array_len(keys) {
            let k = object::array_get(keys, i).unwrap();
            let mut d = Desc {
                configurable: Some(false),
                ..Desc::default()
            };
            if frozen {
                match property::get_own_property(cx, o, k).unwrap() {
                    None => continue,
                    Some(c) if c.is_data() => d.writable = Some(false),
                    Some(_) => {}
                }
            }
            if !property::define_own_property(cx, o, k, d).unwrap() {
                return false;
            }
        }
        true
    }
    /// TestIntegrityLevel(o, frozen or sealed).
    fn test_level(cx: &mut Cx, o: u64, frozen: bool) -> bool {
        if property::is_extensible(o) {
            return false;
        }
        let keys = property::own_property_keys(cx, o);
        for i in 0..object::array_len(keys) {
            let k = object::array_get(keys, i).unwrap();
            if let Some(c) = property::get_own_property(cx, o, k).unwrap()
                && (c.configurable == Some(true)
                    || (frozen && c.is_data() && c.writable == Some(true)))
            {
                return false;
            }
        }
        true
    }
    each(|cx| {
        let rt = cx.rt();
        let fixed_ab = buffer(rt, 8, None);
        let sab = ab::allocate_shared(rt, NULL, 8, None).unwrap();
        // Fixed-length views (TYPED_FIXED) with elements: preventExtensions works; freeze and
        // seal throw at element 0 (node: "Cannot freeze array buffer views with elements").
        for (b, frozen) in [
            (fixed_ab, true),
            (fixed_ab, false),
            (sab, true),
            (sab, false),
        ] {
            let v = view(cx.rt(), 1, b, 0, Some(4));
            put(cx, v, "name", int(1));
            put(cx, v, "0", int(7));
            assert!(!set_level(cx, v, frozen), "frozen {frozen}");
            assert!(!property::is_extensible(v));
            assert!(!test_level(cx, v, false) && !test_level(cx, v, true));
            assert_eq!(elements::kind_of(v), ek::TYPED_FIXED);
            assert_eq!(elements::check(v), Ok(()));
            // The view still reads and stores its elements; no new property comes.
            assert_eq!(get(cx, v, "0"), int(7));
            put(cx, v, "1", int(8));
            assert_eq!(get(cx, v, "1"), int(8));
            let k = key(cx.rt(), "new");
            property::set(cx, v, k, int(1), false).unwrap();
            assert!(!has_own(cx, v, "new"));
        }
        // A length-0 view has no element key: freeze and seal succeed (node: `Object.freeze(new
        // Uint8Array(0))` is fine and `Object.isFrozen` gives true).
        for frozen in [true, false] {
            let rt = cx.rt();
            let v = view(rt, 1, buffer(rt, 0, None), 0, Some(0));
            put(cx, v, "name", int(1));
            assert!(set_level(cx, v, frozen));
            assert!(test_level(cx, v, false));
            assert_eq!(test_level(cx, v, true), frozen);
            assert_eq!(elements::kind_of(v), ek::TYPED_FIXED);
            assert_eq!(elements::check(v), Ok(()));
            let k = key(cx.rt(), "name");
            let d = property::get_own_property(cx, v, k).unwrap().unwrap();
            assert_eq!((d.configurable, d.writable), (Some(false), Some(!frozen)));
        }
        // A length-tracking view (TYPED_VARIABLE): its element defines with
        // `[[Configurable]]: false` are refused as above, and it keeps kind 8.
        // TODO(TA3-PREVEXT): §10.4.5 [[PreventExtensions]] returns false for a view that is not
        // IsTypedArrayFixedLength (node: `Object.preventExtensions(new Uint8Array(new
        // ArrayBuffer(4, {maxByteLength: 8})))` throws a TypeError); ep9 gives true today. The
        // fix is in the property.rs [[PreventExtensions]] entry, route TA3-PREVEXT (TA-3), so
        // this test does not call preventExtensions, freeze or seal on this view.
        let rt = cx.rt();
        let rab = buffer(rt, 4, Some(8));
        let auto = view(rt, 1, rab, 0, None);
        let k = key(cx.rt(), "0");
        let d = Desc {
            configurable: Some(false),
            ..Desc::default()
        };
        assert!(!property::define_own_property(cx, auto, k, d).unwrap());
        assert!(property::is_extensible(auto));
        assert_eq!(elements::kind_of(auto), ek::TYPED_VARIABLE);
        assert_eq!(elements::check(auto), Ok(()));
    });
}

/// InitializeTypedArrayFromArrayBuffer (§23.2.5.1.3) with no length: on a fixed-length buffer
/// (an ArrayBuffer that is not resizable, a SharedArrayBuffer that is not growable) the view
/// gets the length (byteLength - offset) / element size and is fixed-length (step 9.a; node:
/// `Reflect.preventExtensions(new Uint8Array(new ArrayBuffer(4)))` is true); a byte length that
/// is not a multiple of the element size, or an offset past it, is a RangeError. Only a
/// resizable or growable buffer gives a length-tracking view (step 8).
#[test]
fn no_length_on_a_fixed_buffer_computes_it() {
    each(|cx| {
        let rt = cx.rt();
        let fixed_ab = buffer(rt, 12, None);
        let sab = ab::allocate_shared(rt, NULL, 12, None).unwrap();
        let rab = buffer(rt, 12, Some(16));
        let gsab = ab::allocate_shared(rt, NULL, 12, Some(16)).unwrap();
        // (buffer, element type, offset, length, elements kind)
        for (b, t, off, len, want) in [
            (fixed_ab, 1, 0, 12, ek::TYPED_FIXED),
            (fixed_ab, 5, 4, 2, ek::TYPED_FIXED),
            (fixed_ab, 3, 12, 0, ek::TYPED_FIXED),
            (sab, 4, 2, 5, ek::TYPED_FIXED),
            (rab, 5, 4, 2, ek::TYPED_VARIABLE),
            (gsab, 1, 0, 12, ek::TYPED_VARIABLE),
        ] {
            let v = view(cx.rt(), t, b, off, None);
            assert_eq!(elements::kind_of(v), want, "type {t} at {off}");
            assert_eq!(ta::length(cx.rt(), v), Some(len));
            let field = (want == ek::TYPED_FIXED).then_some(len);
            assert_eq!(ta::fixed_length(v), field, "the +40 length");
            assert_eq!(ta::is_fixed_length(v), want == ek::TYPED_FIXED);
        }
        // The errors (node: `new Uint32Array(new ArrayBuffer(6))` and `new Uint8Array(new
        // ArrayBuffer(4), 5)` throw RangeErrors).
        let rt = cx.rt();
        let six = buffer(rt, 6, None);
        assert!(matches!(
            ta::new_view(rt, NULL, 6, six, 0, None),
            Err(ab::Fail::Range(_))
        ));
        assert!(matches!(
            ta::new_view(rt, NULL, 6, six, 4, None),
            Err(ab::Fail::Range(_))
        ));
        assert!(matches!(
            ta::new_view(rt, NULL, 1, six, 7, None),
            Err(ab::Fail::Range(_))
        ));
        let shared6 = ab::allocate_shared(rt, NULL, 6, None).unwrap();
        assert!(matches!(
            ta::new_view(rt, NULL, 5, shared6, 0, None),
            Err(ab::Fail::Range(_))
        ));
        // A resizable buffer takes a byte length that is not a multiple (an auto view floors).
        let r6 = buffer(rt, 6, Some(8));
        let v = view(rt, 6, r6, 0, None);
        assert_eq!(ta::length(cx.rt(), v), Some(1));
    });
}

/// CanonicalNumericIndexString as `ta::canonical_numeric` computes it, against the plain
/// form of the spec text (§7.1.21: "-0", else ToString(ToNumber(key)) == key), over a table of
/// keys: ordinary names, each start character that may begin a numeric string, Number
/// strings and their near misses, long keys and two-byte keys. The fast reject on the first
/// unit changes no result.
#[test]
fn canonical_numeric_matches_the_spec_text() {
    fn plain(text: &str) -> Option<f64> {
        if text == "-0" {
            return Some(-0.0);
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        let n = crate::numconv::string_to_number(&units);
        (crate::numconv::number_to_string(n) == text).then_some(n)
    }
    let mut keys: Vec<String> = [
        "",
        "a",
        "length",
        "name",
        "toString",
        "constructor",
        "__proto__",
        "foo",
        "Symbol",
        "-",
        "-a",
        "-0",
        "-0.0",
        "--1",
        "-1",
        "-1.5",
        "-Infinity",
        "-NaN",
        "0",
        "00",
        "01",
        "0.5",
        ".5",
        "1.",
        "1.0",
        "1.5",
        "1e3",
        "1e+21",
        "1e21",
        "1e-7",
        "1E-7",
        "1e-6",
        "0.000001",
        "0.0000001",
        "123456789012345680000",
        "1.7976931348623157e+308",
        "5e-324",
        "-5e-324",
        "4294967294",
        "4294967295",
        "4294967296",
        "9007199254740992",
        "9007199254740993",
        "I",
        "In",
        "Infinity",
        "Infinityx",
        "infinity",
        "N",
        "NaN",
        "NaNx",
        "nan",
        "+1",
        " 1",
        "1 ",
        "0x10",
        "0b1",
        "1_000",
        "\u{661}",
        "\u{ff11}",
        "-\u{661}",
        "Ι",
        "Ν",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // Number strings up to 25 units (the longest ToString of a Number: a sign, "0.", 5 zeros
    // and 17 digits; `typed_array::MAX_NUMBER_STRING`) and near misses past it.
    let longest = crate::numconv::number_to_string(-1.2345678901234567e-6);
    assert_eq!(longest, "-0.0000012345678901234567");
    keys.push(longest);
    for x in [
        1.2345678901234567e-308,
        0.000001234567890123456,
        1.2345678901234567e21,
        123456789012345680000.0,
        1.0 / 3.0,
        1180591620717411303424.0, // 2^70
        1e-300,
        f64::MAX,
        f64::MIN_POSITIVE,
    ] {
        for y in [x, -x] {
            let s = crate::numconv::number_to_string(y);
            assert!(s.len() <= 25, "{s}");
            keys.push(format!("{s}0"));
            keys.push(format!("{s}x"));
            keys.push(s);
        }
    }
    keys.push("1".repeat(26));
    keys.push(format!("-{}", "9".repeat(40)));
    keys.push("N".repeat(64));
    on_big_stack(move || {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        let rt: &Runtime = &iso.rt;
        for text in &keys {
            let want = plain(text);
            let got = ta::canonical_numeric(key(rt, text));
            assert_eq!(got.map(f64::to_bits), want.map(f64::to_bits), "{text:?}");
            // A two-byte string with the same text gives the same result.
            let units: Vec<u16> = text.encode_utf16().collect();
            let wide = crate::string::testing::new_utf16_cell(rt, &units);
            let got = ta::canonical_numeric(wide);
            assert_eq!(
                got.map(f64::to_bits),
                want.map(f64::to_bits),
                "two-byte {text:?}"
            );
        }
        assert!(ta::canonical_numeric(key(rt, "NaN")).unwrap().is_nan());
    });
}

/// OrdinarySet (§10.1.9.2) step 2.e with a view as the receiver (`Reflect.set({}, k, v, ta)`,
/// `super[k] = v`, a proxy with no `set` trap): CreateDataProperty(Receiver, P, V) is the
/// view's `[[DefineOwnProperty]]` (§10.4.5.3), so a canonical numeric key that is not a valid
/// index gives false and stores nothing, and the named load IC case of the view stays SLOW
/// (`ic::load_case` reads the storage before `exotic_name`; Codex TA-1 r1 finding 2).
#[test]
fn a_view_receiver_takes_its_define_hook() {
    each(|cx| {
        let rt = cx.rt();
        let b = buffer(rt, 4, None);
        let v = view(rt, 1, b, 0, Some(4));
        let o = object::new_object(rt, NULL);
        let refused = |cx: &mut Cx, name: &str| {
            let k = key(cx.rt(), name);
            let before = crate::ic::load_case(cx.rt(), v, k).map(|c| c.handler);
            let r = property::set_with_receiver(cx, o, k, int(7), v).unwrap();
            assert!(!r, "Reflect.set({{}}, {name:?}, 7, view) is false");
            assert!(
                crate::props::own_get(v, k).is_none(),
                "{name}: no stored key"
            );
            assert!(!has_own(cx, v, name), "{name}");
            assert_eq!(get(cx, v, name), UNDEFINED, "{name}");
            let after = crate::ic::load_case(cx.rt(), v, k).map(|c| c.handler);
            assert_eq!(after, before, "{name}: the named load case after the store");
            assert!(
                matches!(after, None | Some(crate::ic::Handler::Smi(_))),
                "{name}: {after:?}"
            );
        };
        for name in ["-0", "1.5", "4", "10", "Infinity", "-Infinity", "NaN", "-1"] {
            refused(cx, name);
        }
        assert_eq!(own_keys(cx, v), ["0", "1", "2", "3"]);
        // A valid index stores the element; an ordinary key is an ordinary property.
        let k = key(cx.rt(), "0");
        assert!(property::set_with_receiver(cx, o, k, int(7), v).unwrap());
        assert_eq!(get(cx, v, "0"), int(7));
        let k = key(cx.rt(), "name");
        assert!(property::set_with_receiver(cx, o, k, int(8), v).unwrap());
        assert_eq!(get(cx, v, "name"), int(8));
        assert_eq!(own_keys(cx, v), ["0", "1", "2", "3", "name"]);
        // A detached view: index 0 is no longer valid.
        ab::detach(cx.rt(), b, UNDEFINED).unwrap();
        refused(cx, "0");
        assert_eq!(own_keys(cx, v), ["name"]);
        assert!(!has_own(cx, o, "0") && !has_own(cx, o, "-0"));
    });
}

/// TypedArraySetElement at the conversion edges of the integer element types (ToInt8 ...
/// ToUint32 and ToUint8Clamp, §7.1.6-12): NaN, ±0, ±∞, negative fractions, 2^31, 2^32, 2^53
/// and above. The read-back values are node v26.13.0 results of `a = new XArray(1); a[0] = x;
/// a[0]`, columns Int8, Uint8, Uint8Clamped, Int16, Uint16, Int32, Uint32. H36: the
/// ToInt8 ... ToUint32 rows take `ep9_value::to_uint32` (integer operations on the bits), so no
/// target calls `fmod` or emits an FMA for them (`cargo xtask fp-check`).
#[test]
fn integer_conversions_at_the_edges() {
    #[rustfmt::skip]
    const CASES: [(f64, [f64; 7]); 26] = [
        (f64::NAN, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (f64::INFINITY, [0.0, 0.0, 255.0, 0.0, 0.0, 0.0, 0.0]),
        (f64::NEG_INFINITY, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (-0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (-0.5, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (-1.5, [-1.0, 255.0, 0.0, -1.0, 65535.0, -1.0, 4294967295.0]),
        (-2.5, [-2.0, 254.0, 0.0, -2.0, 65534.0, -2.0, 4294967294.0]),
        (65504.0, [-32.0, 224.0, 255.0, -32.0, 65504.0, 65504.0, 65504.0]),
        (2147483648.0, [0.0, 0.0, 255.0, 0.0, 0.0, -2147483648.0, 2147483648.0]),
        (-2147483648.5, [0.0, 0.0, 0.0, 0.0, 0.0, -2147483648.0, 2147483648.0]),
        (-2147483649.0, [-1.0, 255.0, 0.0, -1.0, 65535.0, 2147483647.0, 2147483647.0]),
        (4294967295.5, [-1.0, 255.0, 255.0, -1.0, 65535.0, -1.0, 4294967295.0]),
        (4294967296.0, [0.0, 0.0, 255.0, 0.0, 0.0, 0.0, 0.0]),
        (4294967297.0, [1.0, 1.0, 255.0, 1.0, 1.0, 1.0, 1.0]),
        (-4294967297.0, [-1.0, 255.0, 0.0, -1.0, 65535.0, -1.0, 4294967295.0]),
        // 2^53, 2^53 + 2, -(2^53 + 2), 2^53 + 2^32 + 6.
        (9007199254740992.0, [0.0, 0.0, 255.0, 0.0, 0.0, 0.0, 0.0]),
        (9007199254740994.0, [2.0, 2.0, 255.0, 2.0, 2.0, 2.0, 2.0]),
        (-9007199254740994.0, [-2.0, 254.0, 0.0, -2.0, 65534.0, -2.0, 4294967294.0]),
        (9007203549708294.0, [6.0, 6.0, 255.0, 6.0, 6.0, 6.0, 6.0]),
        // 2^64 + 2^12, 2^70 + 12345 * 2^18 and its negation.
        (18446744073709555712.0, [0.0, 0.0, 255.0, 4096.0, 4096.0, 4096.0, 4096.0]),
        (1180591620720647471104.0, [0.0, 0.0, 255.0, 0.0, 0.0, -1058799616.0, 3236167680.0]),
        (-1180591620720647471104.0, [0.0, 0.0, 0.0, 0.0, 0.0, 1058799616.0, 1058799616.0]),
        (1e300, [0.0, 0.0, 255.0, 0.0, 0.0, 0.0, 0.0]),
        (f64::MAX, [0.0, 0.0, 255.0, 0.0, 0.0, 0.0, 0.0]),
        (5e-324, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ];
    each(|cx| {
        for t in 0u8..=6 {
            let rt = cx.rt();
            let b = buffer(rt, 8, None);
            let v = view(rt, t, b, 0, Some(1));
            for (x, want) in CASES {
                put(cx, v, "0", crate::conv::num(x));
                let got = out_of(get(cx, v, "0"));
                let want = Out::N(want[usize::from(t)]);
                assert!(same(got, want), "type {t}: {x:e} -> {got:?}, want {want:?}");
            }
        }
    });
}
