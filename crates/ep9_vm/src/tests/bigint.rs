//! BigInt value tests (spec-temporal B2; design §2.1-§2.3): the BIGINT cell (kind 9), the
//! conversions and operators of `conv.rs` on BigInts (each §2.3 row, both operand orders),
//! `===` of two equal cells in T0, `typeof`, the unary operators, Map keys by value, the size
//! and string-length limits, the not-implemented error with no builtin table, and a BigInt
//! held only by a register across collections (GC stress). JS-level tests with the builtins
//! are in `crates/ep9_builtins/tests/bigint.rs`.

use ep9_bytecode::{BigIntConst, FeedbackSlotKind, Register, TypeOfLiteral, VerifiedUnit};

use super::{Asm, config, isolate, ok, on_big_stack, run_all, unit};
use crate::abi::ObjectClass;
use crate::atoms;
use crate::bigint;
use crate::conv::{self, BinOp, CompareOp, Numeric};
use crate::frame::{FALSE, NULL, TRUE, UNDEFINED};
use crate::interp::Cx;
use crate::kinds;
use crate::object;
use crate::ordered_table::{self as table, MAP_WIDTH};
use crate::property;
use crate::runtime::{Atom, Root, Runtime};
use crate::{Dispatch, string};

// ---- helpers --------------------------------------------------------------------------------

/// Runs `f` with the interpreter context of a fresh isolate with no builtin table, then
/// collects and verifies the heap. `stress`: a collection before every allocation.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx) -> R + Send + 'static) -> R
where
    R: Send + 'static,
{
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        let r = f(&mut iso.cx);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
        r
    })
}

/// A BIGINT cell from a StringIntegerLiteral.
fn big(cx: &mut Cx, s: &str) -> u64 {
    let b = ep9_bigint::parse_literal(s.as_bytes()).unwrap().unwrap();
    bigint::alloc(cx.rt(), &b)
}

fn s(cx: &mut Cx, text: &str) -> u64 {
    object::new_string(cx.rt(), text)
}

/// ToString of `v` as Rust text.
fn text(cx: &mut Cx, v: u64) -> String {
    conv::to_rust_string(cx, v).unwrap()
}

/// The pending exception as "Name: message".
fn err(cx: &mut Cx) -> String {
    let rt = cx.rt();
    let e = rt.take_pending();
    let name = property::get(cx, e, atoms::intern_str(rt, "name")).unwrap();
    let msg = property::get(cx, e, atoms::intern_str(rt, "message")).unwrap();
    format!(
        "{}: {}",
        string::to_rust_string(name),
        string::to_rust_string(msg)
    )
}

fn num(d: f64) -> u64 {
    conv::num(d)
}

// ---- the cell -------------------------------------------------------------------------------

#[test]
fn the_cell_is_kind_9_with_shape_id_0_and_the_digits() {
    with_cx(true, |cx| {
        let b = big(cx, "-340282366920938463463374607431768211457"); // -(2^128 + 1)
        assert!(kinds::is_bigint(b));
        assert!(!kinds::is_object(b) && !kinds::is_string(b) && !kinds::is_symbol(b));
        let c = object::cell(b);
        // SAFETY: `b` is a live cell of this isolate.
        let h = unsafe { c.header() };
        assert_eq!(h.kind(), 9);
        assert_eq!(h.shape_id().get(), 0);
        assert_eq!(h.flags(), 0);
        // SAFETY: a BIGINT cell has the +8 word and 3 digits.
        let (w, d) = unsafe {
            (
                c.field::<u32>(bigint::bf::LEN).read(),
                core::slice::from_raw_parts(c.field::<u64>(bigint::bf::DATA), 3).to_vec(),
            )
        };
        assert_eq!(w, 3 | bigint::bf::SIGN);
        assert_eq!(d, [1, 0, 1]);
        // SAFETY: `c` is a live cell of this heap.
        assert_eq!(unsafe { cx.rt().heap.cell_size(c) }, 16 + 24);
        let z = big(cx, "-0");
        assert!(bigint::is_zero(z));
        // SAFETY: `z` is a live BIGINT cell; +8 is inside its 16-byte size.
        let w = unsafe { object::cell(z).field::<u32>(bigint::bf::LEN).read() };
        assert_eq!(w, 0);
        assert_eq!(
            conv::describe(b),
            "-340282366920938463463374607431768211457n"
        );
    });
}

// ---- conversions (§7.1) ---------------------------------------------------------------------

#[test]
fn conversions() {
    with_cx(true, |cx| {
        let rt: &Runtime = cx.rt();
        let zero = big(cx, "0");
        let one = big(cx, "1");
        let neg = big(cx, "-123");
        assert_eq!(conv::type_of(one), Atom::Bigint);
        assert!(!conv::to_boolean(zero));
        assert!(conv::to_boolean(one));
        assert!(conv::to_boolean(neg));
        assert!(conv::to_number(cx, one).is_err());
        assert_eq!(
            err(cx),
            "TypeError: Cannot convert a BigInt value to a number"
        );
        assert_eq!(conv::to_numeric(cx, one).unwrap(), Numeric::BigInt(one));
        assert_eq!(text(cx, neg), "-123");
        assert_eq!(text(cx, zero), "0");
        // ToPropertyKey: o[1n] is o["1"].
        let k = conv::to_property_key(cx, one).unwrap();
        assert_eq!(k, atoms::intern_str(rt, "1"));
        // Int32 conversions go through ToNumber: TypeError.
        assert!(conv::to_int32_of(cx, one).is_err());
        assert_eq!(
            err(cx),
            "TypeError: Cannot convert a BigInt value to a number"
        );
        assert_eq!(crate::key::key_text(rt, one), "1n");
    });
}

/// With no builtin table there is no %BigInt.prototype%: ToObject and a property access
/// throw "not implemented", never use undefined as a prototype.
#[test]
fn no_builtin_table_gives_not_implemented() {
    with_cx(false, |cx| {
        let one = big(cx, "1");
        assert!(conv::to_object(cx, one).is_err());
        assert_eq!(
            err(cx),
            "Error: not implemented in ep9 v0: BigInt.prototype"
        );
        let key = atoms::intern_str(cx.rt(), "toString");
        assert!(property::get(cx, one, key).is_err());
        assert_eq!(
            err(cx),
            "Error: not implemented in ep9 v0: BigInt.prototype"
        );
    });
}

// ---- operators (§6.1.6.2, §13.15.3) ---------------------------------------------------------

#[test]
fn binary_operators() {
    with_cx(true, |cx| {
        let cases: &[(BinOp, &str, &str, &str)] = &[
            (BinOp::Add, "7", "-2", "5"),
            (BinOp::Sub, "7", "-2", "9"),
            (BinOp::Mul, "-7", "2", "-14"),
            (BinOp::Div, "-7", "2", "-3"),
            (BinOp::Mod, "-7", "2", "-1"),
            (BinOp::Mod, "7", "-2", "1"),
            (BinOp::Exp, "-3", "3", "-27"),
            (BinOp::Exp, "0", "0", "1"),
            (BinOp::BitAnd, "-5", "3", "3"),
            (BinOp::BitOr, "-5", "3", "-5"),
            (BinOp::BitXor, "-5", "3", "-8"),
            (BinOp::Shl, "-5", "2", "-20"),
            (BinOp::Shl, "-5", "-1", "-3"),
            (BinOp::Sar, "-5", "1", "-3"),
            (BinOp::Sar, "5", "-2", "20"),
            (BinOp::Exp, "2", "64", "18446744073709551616"),
        ];
        for &(op, a, b, want) in cases {
            let (x, y) = (big(cx, a), big(cx, b));
            let r = conv::binary(cx, op, x, y).unwrap();
            assert!(kinds::is_bigint(r), "{op:?}");
            assert_eq!(text(cx, r), want, "{a} {op:?} {b}");
        }
        let one = big(cx, "1");
        let zero = big(cx, "0");
        let errors: &[(BinOp, u64, u64, &str)] = &[
            (
                BinOp::Shr,
                one,
                one,
                "TypeError: BigInts have no unsigned right shift, use >> instead",
            ),
            (BinOp::Div, one, zero, "RangeError: Division by zero"),
            (BinOp::Mod, one, zero, "RangeError: Division by zero"),
            (
                BinOp::Add,
                one,
                num(1.0),
                "TypeError: Cannot mix BigInt and other types, use explicit conversions",
            ),
            (
                BinOp::Mul,
                num(1.0),
                one,
                "TypeError: Cannot mix BigInt and other types, use explicit conversions",
            ),
            (
                BinOp::Sub,
                one,
                UNDEFINED,
                "TypeError: Cannot mix BigInt and other types, use explicit conversions",
            ),
            (
                BinOp::BitOr,
                TRUE,
                one,
                "TypeError: Cannot mix BigInt and other types, use explicit conversions",
            ),
            (
                BinOp::Shr,
                num(1.0),
                one,
                "TypeError: Cannot mix BigInt and other types, use explicit conversions",
            ),
        ];
        for &(op, a, b, want) in errors {
            assert!(conv::binary(cx, op, a, b).is_err(), "{op:?}");
            assert_eq!(err(cx), want, "{op:?}");
        }
        let m1 = big(cx, "-1");
        assert!(conv::binary(cx, BinOp::Exp, one, m1).is_err());
        assert_eq!(err(cx), "RangeError: Exponent must be non-negative");
        // `+` with a String side concatenates, in both orders.
        let a = s(cx, "a");
        let r = conv::binary(cx, BinOp::Add, one, a).unwrap();
        assert_eq!(text(cx, r), "1a");
        let r = conv::binary(cx, BinOp::Add, a, m1).unwrap();
        assert_eq!(text(cx, r), "a-1");
    });
}

#[test]
fn the_size_and_string_length_limits() {
    with_cx(false, |cx| {
        let one = big(cx, "1");
        let max = big(cx, "1073741824"); // MAX_BITS
        assert!(conv::binary(cx, BinOp::Shl, one, max).is_err());
        assert_eq!(err(cx), "RangeError: Maximum BigInt size exceeded");
        let two = big(cx, "2");
        assert!(conv::binary(cx, BinOp::Exp, two, max).is_err());
        assert_eq!(err(cx), "RangeError: Maximum BigInt size exceeded");
        // (1n << 600000000n).toString(2) has 600,000,001 chars: above MAX_STRING_LENGTH.
        let n = big(cx, "600000000");
        let b = conv::binary(cx, BinOp::Shl, one, n).unwrap();
        assert!(bigint::to_string(cx, b, 2).is_err());
        assert_eq!(err(cx), "RangeError: Invalid string length");
        // Radix 16 of the same value fits (150,000,001 chars) but is not made here; a small
        // value converts in every radix.
        let v = big(cx, "-255");
        let r = bigint::to_string(cx, v, 16).unwrap();
        assert_eq!(string::to_rust_string(r), "-ff");
    });
}

#[test]
fn equality() {
    with_cx(true, |cx| {
        let a = big(cx, "18446744073709551616");
        let b = big(cx, "18446744073709551616");
        assert_ne!(a, b, "two cells");
        assert!(conv::strict_equals(a, b));
        assert!(conv::strict_equals(b, a));
        let one = big(cx, "1");
        let neg_one = big(cx, "-1");
        assert!(!conv::strict_equals(one, num(1.0)));
        assert!(!conv::strict_equals(num(1.0), one));
        assert!(!conv::strict_equals(one, neg_one));
        let one_s = s(cx, "1");
        let hex = s(cx, " 0x1 ");
        let bad = s(cx, "1n");
        let empty = s(cx, "");
        let zero = big(cx, "0");
        let p64 = num(18446744073709551616.0);
        // (a, b, a == b); each pair is also tested as (b, a).
        let cases: &[(u64, u64, bool)] = &[
            (a, b, true),
            (one, num(1.0), true),
            (one, num(1.5), false),
            (one, num(f64::NAN), false),
            (one, num(f64::INFINITY), false),
            (a, p64, true),
            (one, one_s, true),
            (one, hex, true),
            (one, bad, false),
            (zero, empty, true),
            (one, TRUE, true),
            (zero, FALSE, true),
            (one, UNDEFINED, false),
            (zero, NULL, false),
            (one, neg_one, false),
        ];
        for &(x, y, want) in cases {
            assert_eq!(
                conv::loose_equals(cx, x, y).unwrap(),
                want,
                "{}",
                conv::describe(y)
            );
            assert_eq!(
                conv::loose_equals(cx, y, x).unwrap(),
                want,
                "{}",
                conv::describe(y)
            );
        }
        // SameValue and SameValueZero follow IsStrictlyEqual for BigInts.
        assert!(property::same_value(a, b));
        assert!(!property::same_value(zero, num(0.0)));
    });
}

#[test]
fn relational() {
    with_cx(true, |cx| {
        let one = big(cx, "1");
        let two = big(cx, "2");
        let p53 = big(cx, "9007199254740993"); // 2^53 + 1, not a double
        let two_s = s(cx, "2");
        let x = s(cx, "x");
        // (a, b, a < b): undefined (None) for NaN and a text that is no StringIntegerLiteral.
        let cases: &[(u64, u64, Option<bool>)] = &[
            (one, two, Some(true)),
            (two, one, Some(false)),
            (one, num(2.0), Some(true)),
            (num(2.0), one, Some(false)),
            (one, num(1.5), Some(true)),
            (num(1.5), two, Some(true)),
            (one, num(1.0), Some(false)),
            (one, num(f64::NAN), None),
            (num(f64::NAN), one, None),
            (one, num(f64::INFINITY), Some(true)),
            (num(f64::NEG_INFINITY), one, Some(true)),
            (num(9007199254740992.0), p53, Some(true)),
            (p53, num(9007199254740992.0), Some(false)),
            (one, two_s, Some(true)),
            (two_s, one, Some(false)),
            (one, x, None),
            (x, one, None),
        ];
        for &(a, b, want) in cases {
            assert_eq!(
                conv::less_than(cx, a, b, true).unwrap(),
                want,
                "{} < {}",
                conv::describe(a),
                conv::describe(b)
            );
        }
        assert!(conv::compare(cx, CompareOp::Le, one, num(1.0)).unwrap());
        assert!(conv::compare(cx, CompareOp::Ge, two_s, two).unwrap());
        assert!(!conv::compare(cx, CompareOp::Gt, one, x).unwrap());
        assert!(!conv::compare(cx, CompareOp::Le, one, x).unwrap());
    });
}

/// Map and Set keys are SameValueZero: two cells with one value are one key, and the hash
/// comes from the value (H12-Q2), not the address.
#[test]
fn map_keys_by_value() {
    with_cx(true, |cx| {
        let rt: &Runtime = cx.rt();
        let o =
            object::new_slot_object(rt, rt.root(Root::ObjectPrototype), ObjectClass::Map.id(), 1);
        let h = rt.heap.new_global(crate::runtime::val(o));
        let t = table::new_empty(rt, MAP_WIDTH, false);
        object::slot_set(rt, o, table::TABLE_SLOT, t);
        let k1 = big(cx, "-123456789012345678901234567890");
        table::set(rt, o, k1, num(1.0)).unwrap();
        let k2 = big(cx, "-123456789012345678901234567890");
        assert_ne!(k1, k2);
        assert_eq!(bigint::hash(k1), bigint::hash(k2));
        let t = table::table_of(o);
        assert_eq!(
            table::lookup(rt, t, k2).map(|i| table::value_at(t, i)),
            Some(num(1.0))
        );
        let k3 = big(cx, "123456789012345678901234567890");
        assert_eq!(table::lookup(rt, t, k3), None);
        assert_eq!(table::lookup(rt, t, num(-1.2345678901234568e29)), None);
        rt.heap.drop_global(h);
    });
}

// ---- T0 (bytecode) --------------------------------------------------------------------------

fn bigc(v: &[u64], neg: bool) -> BigIntConst {
    BigIntConst::new(neg, v)
}

/// `print(acc)` keeping the registers.
fn print(s: &mut Asm) {
    s.print_acc();
}

/// `===` of two BigInt cells with one value takes the cold body (the fast body answered false
/// before B2), `switch` too; `typeof` literal tests; `++`, `--`, `-`, `~` on a BigInt.
#[test]
fn interpreter_ops() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (a, b, t): (Register, Register, Register) =
            (s.b.new_register(), s.b.new_register(), s.b.new_register());
        let k1 = s.b.bigint_constant(bigc(&[1], false));
        let k0 = s.b.bigint_constant(bigc(&[], false));
        let tyof = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
        // a = 1n; b = 1n + 0n (a new cell); print(a === b)
        s.b.lda_constant(k1).star(a);
        s.b.lda_constant(k0).add(a, s.bin).star(b);
        s.b.ldar(b).test_equal_strict(a, s.cmp);
        print(&mut s);
        // print(a === 1) and print(a == 1)
        s.b.lda_smi(1).test_equal_strict(a, s.cmp);
        print(&mut s);
        s.b.lda_smi(1).test_equal(a, s.cmp);
        print(&mut s);
        // print(typeof a === "bigint"), print(typeof a)
        s.b.ldar(a).test_type_of(TypeOfLiteral::BigInt);
        print(&mut s);
        s.b.ldar(a).test_type_of(TypeOfLiteral::Object);
        print(&mut s);
        s.b.ldar(a).type_of(tyof);
        print(&mut s);
        // t = a; t++ (ToNumeric, Inc); print(t); print(--t); print(-t); print(~t)
        s.b.ldar(a).to_numeric(s.bin).inc(s.bin).star(t);
        print(&mut s);
        s.b.ldar(t).dec(s.bin);
        print(&mut s);
        s.b.ldar(t).negate(s.bin);
        print(&mut s);
        s.b.ldar(t).bitwise_not(s.bin);
        print(&mut s);
        // print(!0n), print(1n < 2)
        s.b.ldar(b).sub(a, s.bin).to_boolean_logical_not();
        print(&mut s);
        s.b.lda_smi(2).test_less_than(a, s.cmp);
        print(&mut s);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "true", "false", "true", "true", "false", "bigint", "2", "1", "-2", "-3", "true",
            "true"
        ]
    );
}

/// A BigInt that only a register holds survives collections: under GC stress (`run_all`
/// runs every dispatch with a collection before every allocation and the heap verifier)
/// `r = 2n ** 100n`, then 50 allocations, then `print(r)` and `print(r + 1n)`.
#[test]
fn a_bigint_held_only_by_a_register_survives_gc() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (r, t) = (s.b.new_register(), s.b.new_register());
        let k2 = s.b.bigint_constant(bigc(&[2], false));
        let k100 = s.b.bigint_constant(bigc(&[100], false));
        let k1 = s.b.bigint_constant(bigc(&[1], false));
        s.b.lda_constant(k2).star(t);
        s.b.lda_constant(k100).exp(t, s.bin).star(r);
        for _ in 0..50 {
            // Garbage: a new object and a new BigInt each time.
            s.b.create_empty_object_literal().star(t);
            s.b.lda_constant(k1).add(r, s.bin).star(t);
        }
        s.b.ldar(r);
        print(&mut s);
        s.b.lda_constant(k1).add(r, s.bin);
        print(&mut s);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "1267650600228229401496703205376",
            "1267650600228229401496703205377"
        ]
    );
}

/// A BigInt literal in an array boilerplate is a real cell (the literal site's BigInt arms
/// went with B2).
#[test]
fn a_bigint_in_an_array_literal() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let t = s.b.new_register();
        let k = s.b.bigint_constant(bigc(&[5], true));
        let k_arr = s.b.array_boilerplate(vec![Some(k), Some(k)]);
        for _ in 0..3 {
            // Three runs of the site: the cold path, then the recorded shape.
            s.b.create_array_literal(k_arr, s.lit, 1).star(t);
        }
        s.b.lda_smi(1);
        s.b.get_keyed_property(t, s.keyed);
        print(&mut s);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["-5"]);
}
