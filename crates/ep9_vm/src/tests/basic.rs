//! Values, operators, conversions, objects, arrays and strings (gate "object-model-v0").
//! The expected outputs are what node prints for the same JS (checked 2026-10-21).

use ep9_bytecode::{
    FeedbackSlot, FeedbackSlotKind, FunctionBuilder, Register, RegisterList, TypeOfLiteral,
    VerifiedUnit,
};

use super::{Asm, ok, run_all, unit};

/// An operand of a test expression.
#[derive(Clone, Copy)]
enum V {
    N(f64),
    S(&'static str),
    Null,
    Undef,
    True,
}

fn load(s: &mut Asm, v: V) {
    match v {
        V::N(d) => {
            s.lda_num(d);
        }
        V::S(t) => {
            s.lda_str(t);
        }
        V::Null => {
            s.b.lda_null();
        }
        V::Undef => {
            s.b.lda_undefined();
        }
        V::True => {
            s.b.lda_true();
        }
    }
}

type BinEmit =
    for<'a> fn(&'a mut FunctionBuilder, Register, FeedbackSlot) -> &'a mut FunctionBuilder;

/// `print(l OP r)`.
fn print_bin(s: &mut Asm, t: Register, l: V, op: BinEmit, r: V, slot: FeedbackSlot) {
    load(s, l);
    s.b.star(t);
    load(s, r);
    op(&mut s.b, t, slot);
    s.print_acc();
}

#[test]
fn loop_and_arithmetic() {
    fn make() -> VerifiedUnit {
        // var s = 0; for (var i = 0; i < 1000; i++) s = s + i; return s * 2 + 0.5;
        let mut s = Asm::script(0);
        let (sum, i, t) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        s.b.lda_zero().star(sum).lda_zero().star(i);
        let header = s.b.bound_label();
        let exit = s.b.new_label();
        s.b.lda_smi(1000)
            .test_less_than(i, s.cmp)
            .jump_if_false(exit);
        s.b.ldar(i).add(sum, s.bin).star(sum);
        s.b.ldar(i).inc(s.bin).star(i);
        s.b.jump_loop(header, 0, s.loop_);
        s.b.bind(exit);
        s.lda_num(0.5);
        s.b.star(t);
        s.b.ldar(sum).mul_smi(2, s.bin).add(t, s.bin).return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    assert_eq!(ok(&r), "999000.5");
}

#[test]
fn number_operators() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let t = s.b.new_register();
        let bin = s.bin;
        let cases: &[(V, BinEmit, V)] = &[
            (V::N(-7.0), FunctionBuilder::r#mod, V::N(3.0)),
            (V::N(7.0), FunctionBuilder::r#mod, V::N(-3.0)),
            (V::N(5.5), FunctionBuilder::r#mod, V::N(2.0)),
            (V::N(5.0), FunctionBuilder::div, V::N(2.0)),
            (V::N(1.0), FunctionBuilder::div, V::N(0.0)),
            (V::N(-1.0), FunctionBuilder::shift_right_logical, V::N(0.0)),
            (V::N(1.0), FunctionBuilder::shift_left, V::N(31.0)),
            (V::N(-16.0), FunctionBuilder::shift_right, V::N(2.0)),
            (V::N(2.0), FunctionBuilder::exp, V::N(10.0)),
            (V::N(0.1), FunctionBuilder::add, V::N(0.2)),
            (V::N(2147483647.0), FunctionBuilder::add, V::N(1.0)),
            (V::N(-2147483648.0), FunctionBuilder::sub, V::N(1.0)),
            (V::N(65536.0), FunctionBuilder::mul, V::N(65536.0)),
            (V::S("5"), FunctionBuilder::mul, V::S("2")),
            (V::S("a"), FunctionBuilder::add, V::N(1.0)),
            (V::N(1.5), FunctionBuilder::add, V::S("b")),
            (V::S(" 42 "), FunctionBuilder::sub, V::N(0.0)),
            (V::S("0x1f"), FunctionBuilder::mul, V::N(1.0)),
            (V::S("1e3"), FunctionBuilder::add, V::N(0.0)),
            (V::S("abc"), FunctionBuilder::mul, V::N(1.0)),
            (V::Null, FunctionBuilder::add, V::N(1.0)),
            (V::Undef, FunctionBuilder::add, V::N(1.0)),
            (V::True, FunctionBuilder::add, V::True),
            (V::N(6.0), FunctionBuilder::bitwise_and, V::N(3.0)),
            (V::N(6.0), FunctionBuilder::bitwise_or, V::N(3.0)),
            (V::N(6.0), FunctionBuilder::bitwise_xor, V::N(3.0)),
            (V::N(1e21), FunctionBuilder::add, V::N(0.0)),
            (V::N(4294967296.5), FunctionBuilder::bitwise_or, V::N(0.0)),
            (V::N(0.000001), FunctionBuilder::add, V::N(0.0)),
            (V::N(1e-7), FunctionBuilder::add, V::N(0.0)),
        ];
        for &(l, op, r) in cases {
            print_bin(&mut s, t, l, op, r, bin);
        }
        // -0: 1 / -(0) is -Infinity; String(-0) is "0".
        s.b.lda_zero().negate(bin).star(t);
        s.print_acc();
        s.b.lda_zero()
            .negate(bin)
            .star(t)
            .lda_smi(1)
            .star(s.a.get(1));
        s.b.ldar(t).div(s.a.get(1), bin);
        s.print_acc();
        // ~5, ++/-- at the int32 edges.
        s.b.lda_smi(5).bitwise_not(bin);
        s.print_acc();
        s.b.lda_smi(i32::MAX).inc(bin);
        s.print_acc();
        s.b.lda_smi(i32::MIN).dec(bin);
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    let want = [
        "-1",
        "1",
        "1.5",
        "2.5",
        "Infinity",
        "4294967295",
        "-2147483648",
        "-4",
        "1024",
        "0.30000000000000004",
        "2147483648",
        "-2147483649",
        "4294967296",
        "10",
        "a1",
        "1.5b",
        "42",
        "31",
        "1e30",
        "NaN",
        "1",
        "NaN",
        "2",
        "2",
        "7",
        "5",
        "1e+21",
        "0",
        "0.000001",
        "1e-7",
        "0",
        "-Infinity",
        "-6",
        "2147483648",
        "-2147483649",
    ];
    assert_eq!(r.output, want);
}

#[test]
fn equality_relational_and_typeof() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let t = s.b.new_register();
        let cmp = s.cmp;
        let cases: &[(V, BinEmit, V)] = &[
            (V::Null, FunctionBuilder::test_equal, V::Undef),
            (V::Null, FunctionBuilder::test_equal_strict, V::Undef),
            (
                V::N(f64::NAN),
                FunctionBuilder::test_equal_strict,
                V::N(f64::NAN),
            ),
            (V::S("1"), FunctionBuilder::test_equal, V::N(1.0)),
            (V::S(""), FunctionBuilder::test_equal, V::N(0.0)),
            (V::True, FunctionBuilder::test_equal, V::N(1.0)),
            (V::Null, FunctionBuilder::test_equal, V::N(0.0)),
            (V::S("b"), FunctionBuilder::test_less_than, V::S("a")),
            (V::S("a"), FunctionBuilder::test_less_than, V::S("b")),
            (V::N(2.0), FunctionBuilder::test_less_than, V::S("10")),
            (V::S("2"), FunctionBuilder::test_less_than, V::S("10")),
            (
                V::N(1.0),
                FunctionBuilder::test_greater_than_or_equal,
                V::N(1.0),
            ),
            (
                V::N(f64::NAN),
                FunctionBuilder::test_less_than_or_equal,
                V::N(1.0),
            ),
            (V::Undef, FunctionBuilder::test_greater_than, V::N(0.0)),
        ];
        for &(l, op, r) in cases {
            print_bin(&mut s, t, l, op, r, cmp);
        }
        // "ab" + "c" === "abc" (a new string against the constant).
        s.lda_str("ab");
        s.b.star(t);
        s.lda_str("c");
        s.b.add(t, s.bin).star(t);
        s.lda_str("abc");
        s.b.test_equal_strict(t, cmp);
        s.print_acc();
        // !"" and !!"x".
        s.lda_str("");
        s.b.to_boolean_logical_not();
        s.print_acc();
        s.lda_str("x");
        s.b.to_boolean_logical_not().logical_not();
        s.print_acc();
        // typeof of each kind, and TestTypeOf.
        let ts = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
        for v in [V::N(1.0), V::S("s"), V::Undef, V::Null, V::True] {
            load(&mut s, v);
            s.b.type_of(ts);
            s.print_acc();
        }
        s.lda_global("print");
        s.b.type_of(ts);
        s.print_acc();
        s.b.create_empty_object_literal().type_of(ts);
        s.print_acc();
        s.lda_global("print");
        s.b.test_type_of(TypeOfLiteral::Function);
        s.print_acc();
        s.b.lda_null().test_type_of(TypeOfLiteral::Object);
        s.print_acc();
        // typeof of an undeclared global is "undefined", not a ReferenceError.
        let k = s.b.string_constant("nope");
        let slot = s.b.feedback_slot(FeedbackSlotKind::LoadGlobalInsideTypeof);
        s.b.lda_global_inside_typeof(k, slot).type_of(ts);
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    let want = [
        "true",
        "false",
        "false",
        "true",
        "true",
        "true",
        "false",
        "false",
        "true",
        "true",
        "false",
        "true",
        "false",
        "false",
        "true",
        "true",
        "true",
        "number",
        "string",
        "undefined",
        "object",
        "boolean",
        "function",
        "object",
        "true",
        "true",
        "undefined",
    ];
    assert_eq!(r.output, want);
}

#[test]
fn objects_arrays_and_for_in() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (o, arr, k, idx) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let cache = s.b.new_register_list(3);
        let bin = s.bin;
        // o = {a: 1, b: "x"}; o.c = 3; print(o.a + o.c, o.b, o.missing)
        let (ka, kb, k1, kx) = (
            s.b.string_constant("a"),
            s.b.string_constant("b"),
            s.b.number_constant(1.0),
            s.b.string_constant("x"),
        );
        let bp = s.b.object_boilerplate(vec![(ka, Some(k1)), (kb, Some(kx))]);
        s.b.create_object_literal(bp, s.lit, 0).star(o);
        s.b.lda_smi(3);
        s.set_named(o, "c");
        s.get_named(o, "a");
        s.b.star(k);
        s.get_named(o, "c");
        s.b.add(k, bin);
        s.print_acc();
        s.get_named(o, "b");
        s.print_acc();
        s.get_named(o, "missing");
        s.print_acc();
        // arr = [1, , "x"]; print(arr.length, String(arr)); arr[5] = 7; arr.push(8)
        let hole = s.b.oddball_constant(ep9_bytecode::Oddball::Hole);
        let abp = s.b.array_boilerplate(vec![Some(k1), Some(hole), Some(kx)]);
        s.b.create_array_literal(abp, s.lit, 0).star(arr);
        s.get_named(arr, "length");
        s.print_acc();
        s.b.ldar(arr);
        s.print_acc();
        s.b.lda_smi(5)
            .star(k)
            .lda_smi(7)
            .set_keyed_property(arr, k, s.set_keyed);
        s.get_named(arr, "length");
        s.print_acc();
        let push_args = RegisterList::new(s.a.get(1).local_index().unwrap(), 2);
        s.get_named(arr, "push");
        s.b.star(s.f)
            .mov(arr, push_args.get(0))
            .lda_smi(8)
            .star(push_args.get(1));
        s.b.call_property(s.f, push_args, 2, s.call);
        s.print_acc();
        let jk = s.b.string_constant("join");
        s.b.get_named_property(arr, jk, s.named).star(s.f);
        s.lda_str("-");
        s.b.star(s.a.get(2));
        s.b.call_property1(s.f, arr, s.a.get(2), s.call);
        s.print_acc();
        // keyed read with a computed index and an out-of-range index
        s.b.lda_smi(2).get_keyed_property(arr, s.keyed);
        s.print_acc();
        s.b.lda_smi(100).get_keyed_property(arr, s.keyed);
        s.print_acc();
        // "c" in o; delete o.a; o.a; o instanceof Object
        let hs = s.b.feedback_slot(FeedbackSlotKind::HasKeyed);
        s.lda_str("c");
        s.b.star(k).ldar(o).test_in(k, hs);
        s.print_acc();
        s.lda_str("a");
        s.b.delete_property_sloppy(o);
        s.print_acc();
        s.get_named(o, "a");
        s.print_acc();
        let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
        s.lda_global("Object");
        s.b.test_instance_of(o, is);
        s.print_acc();
        // for (k in o) print(k)
        let fs = s.b.feedback_slot(FeedbackSlotKind::ForIn);
        let done = s.b.new_label();
        let next = s.b.new_label();
        s.b.for_in_enumerate(o)
            .for_in_prepare(cache, fs)
            .lda_zero()
            .star(idx);
        let head = s.b.bound_label();
        s.b.jump_if_for_in_done(done, idx, cache.get(2));
        let pair = RegisterList::new(cache.first().local_index().unwrap(), 2);
        s.b.for_in_next(o, idx, pair, fs).jump_if_undefined(next);
        s.print_acc();
        s.b.bind(next);
        s.b.for_in_step(idx).jump_loop(head, 0, s.loop_);
        s.b.bind(done);
        s.b.ldar(arr).return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    // `display` runs no JS, so an array shows as "array" (the joined text is printed above).
    assert_eq!(ok(&r), "array");
    let want = [
        "4",
        "x",
        "undefined",
        "3",
        "1,,x",
        "6",
        "7",
        "1--x---7-8",
        "x",
        "undefined",
        "true",
        "true",
        "undefined",
        "true",
        "b",
        "c",
    ];
    assert_eq!(r.output, want);
}

#[test]
fn strings_latin1_and_utf16() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let t = s.b.new_register();
        // "café" + "€" has 5 UTF-16 units; [4] is "€"; "😀".length is 2.
        s.lda_str("caf\u{e9}");
        s.b.star(t);
        s.lda_str("\u{20ac}");
        s.b.add(t, s.bin).star(t);
        s.print_acc();
        s.get_named(t, "length");
        s.print_acc();
        s.b.lda_smi(4).get_keyed_property(t, s.keyed);
        s.print_acc();
        s.lda_str("\u{1F600}");
        s.b.star(t);
        s.get_named(t, "length");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["caf\u{e9}\u{20ac}", "5", "\u{20ac}", "2"]);
}
