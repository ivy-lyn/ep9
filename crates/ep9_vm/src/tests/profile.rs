//! Tests of T0 profile recording (`profile.rs`, M4-P1 PROFILE; H16-Q2): what each slot kind
//! records, the implicit int32 cases, the sampled calls and back edges, saturation, a shared
//! Call slot and GC. Each program runs under every dispatch expansion, with and without GC
//! stress (a collection before every allocation, then the heap verifier), with the feedback
//! threshold 0, so the script has its vector from its first instruction; every run must give
//! the same slots.

use ep9_bytecode::{
    FeedbackSlot, FeedbackSlotKind, FunctionId, Register, RegisterList, VerifiedUnit,
};
use ep9_facts::{BinaryHint, CompareHint, ForInHint, TypeSet};

use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits, native_of};
use crate::interp::BUDGET;
use crate::kinds;
use crate::profile::{self, CALL_WINDOW, CLOSURE_MANY, keyed};
use crate::{Dispatch, IsolateConfig};

/// A key half after a run, in a form that two isolates share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Undefined,
    Hole,
    /// The native word of the FunctionInfo of function `n` of the unit.
    Info(usize),
    /// A cell of this kind.
    Cell(u8),
    /// Another native word (a host function's FunctionInfo).
    Native,
}

/// The (key, value) of every slot of one function, or `None` if it has no vector. A value
/// that is not an int32 (a handler cell of an IC slot) reads [`CELL_VALUE`].
type Slots = Option<Vec<(Key, u64)>>;
/// The slots after a run and after a full GC.
type BeforeAfter = (Vec<Slots>, Vec<Slots>);

/// The value of a slot whose value half is a cell.
const CELL_VALUE: u64 = 0xCE11;

/// How to run a program.
#[derive(Clone, Copy)]
struct How {
    threshold: u32,
    /// The interrupt budget at the start (`None`: the isolate's own, `BUDGET`).
    budget: Option<i64>,
    /// The run must succeed.
    ok: bool,
}

const RUN: How = How {
    threshold: 0,
    budget: None,
    ok: true,
};

/// The slots of every function of the unit of `make` after a run. Runs under every dispatch
/// expansion, with and without GC stress; the slots must agree.
fn slots(how: How, make: fn() -> VerifiedUnit) -> Vec<Slots> {
    slots_gc(how, make).0
}

/// [`slots`] right after the run and after a full GC (then the heap verifier).
fn slots_gc(how: How, make: fn() -> VerifiedUnit) -> BeforeAfter {
    let mut first: Option<(String, BeforeAfter)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let got = on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: how.threshold,
                    ..config(d, stress)
                });
                if let Some(b) = how.budget {
                    iso.cx.budget = b;
                }
                let r =
                    iso.with_cx_checkpoint(|cx| cx.run_script(make(), "profile.js").map(|_| ()));
                assert_eq!(r.is_ok(), how.ok, "{r:?}");
                let before = read(&iso);
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (before, read(&iso))
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, got)),
                Some((l0, g0)) => assert_eq!(&got, g0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// The slots of every function of the last unit `iso` loaded.
fn read(iso: &crate::Isolate) -> Vec<Slots> {
    let units = iso.rt.units.borrow();
    let infos = &units.last().expect("a loaded unit").infos;
    // Native FunctionInfo words (the Call keys), not heap cells.
    let words: Vec<u64> = infos.iter().map(|fi| native_of(fi)).collect();
    let key = |k: u64| {
        if k == UNDEFINED {
            Key::Undefined
        } else if k == HOLE {
            Key::Hole
        } else if let Some(n) = words.iter().position(|&w| w == k) {
            Key::Info(n)
        } else if kinds::is_cell_bits(k) {
            Key::Cell(kinds::cell_kind(k))
        } else {
            Key::Native
        }
    };
    let val = |v: u64| {
        if kinds::is_cell_bits(v) {
            CELL_VALUE
        } else {
            v
        }
    };
    infos
        .iter()
        .map(|fi| {
            let v = feedback::vector(fi)?;
            let n = feedback::slot_count(v);
            Some(
                (0..n)
                    .map(|i| (key(feedback::key(v, i)), val(feedback::value(v, i))))
                    .collect(),
            )
        })
        .collect()
}

/// The value half of slot `s` of function `f` as int32 bits.
fn value(slots: &[Slots], f: usize, s: FeedbackSlot) -> u32 {
    let (_, v) = slots[f].as_ref().expect("a vector")[s.index()];
    assert!(v & int32_bits(0) == int32_bits(0), "not an int32: {v:#x}");
    v as u32
}

/// The key half of slot `s` of function `f`.
fn key(slots: &[Slots], f: usize, s: FeedbackSlot) -> Key {
    slots[f].as_ref().expect("a vector")[s.index()].0
}

fn bin(h: BinaryHint) -> u32 {
    u32::from(h.bits())
}

fn cmp(h: CompareHint) -> u32 {
    u32::from(h.bits())
}

fn ty(t: TypeSet) -> u32 {
    u32::from(t.bits())
}

/// `acc = lhs OP acc` through `emit` at a new slot of `kind`, with `lhs` and the accumulator
/// loaded by `l` and `r`. Returns the slot.
fn site(
    s: &mut Asm,
    kind: FeedbackSlotKind,
    l: impl FnOnce(&mut Asm),
    r: impl FnOnce(&mut Asm),
    emit: impl FnOnce(&mut Asm, Register, FeedbackSlot),
) -> FeedbackSlot {
    let slot = s.b.feedback_slot(kind);
    let lhs = s.b.new_register();
    l(s);
    s.b.star(lhs);
    r(s);
    emit(s, lhs, slot);
    slot
}

/// `for (i = 0; i < n; i++) body(i)` (an int32 loop: its compare and `++` are implicit).
fn counted_loop(s: &mut Asm, n: i32, body: impl FnOnce(&mut Asm, Register)) {
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    s.b.lda_smi(n).test_less_than(i, s.cmp).jump_if_false(exit);
    body(s, i);
    s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
}

// ---- BinaryOp ------------------------------------------------------------------------------

/// The BinaryOp slots of `binary_program`, in order.
const BINARY_CASES: usize = 17;

fn binary_program() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    use FeedbackSlotKind::BinaryOp as B;
    let mut s = Asm::script(0);
    let mut v = Vec::new();
    let smi = |n: i32| {
        move |s: &mut Asm| {
            s.b.lda_smi(n);
        }
    };
    let num = |d: f64| {
        move |s: &mut Asm| {
            s.lda_num(d);
        }
    };
    // 0: 2 + 3, int32 (implicit).
    v.push(site(&mut s, B, smi(2), smi(3), |s, l, k| {
        s.b.add(l, k);
    }));
    // 1: int32 overflow of `+`: the fast path's double arm; implicit (V9-D6, `profile.rs`).
    v.push(site(&mut s, B, num(2147483647.0), smi(1), |s, l, k| {
        s.b.add(l, k);
    }));
    // 2: `*` past int32 (the fast path boxes a double).
    v.push(site(&mut s, B, num(65536.0), num(65536.0), |s, l, k| {
        s.b.mul(l, k);
    }));
    // 3: 0 * -5 = -0.
    v.push(site(&mut s, B, smi(0), smi(-5), |s, l, k| {
        s.b.mul(l, k);
    }));
    // 4: 1 / 2 = 0.5.
    v.push(site(&mut s, B, smi(1), smi(2), |s, l, k| {
        s.b.div(l, k);
    }));
    // 5: 6 / 3 = 2 (implicit).
    v.push(site(&mut s, B, smi(6), smi(3), |s, l, k| {
        s.b.div(l, k);
    }));
    // 6: -1 >>> 0 = 4294967295.
    v.push(site(&mut s, B, smi(-1), smi(0), |s, l, k| {
        s.b.shift_right_logical(l, k);
    }));
    // 7: -4 % 2 = -0 (cold path).
    v.push(site(
        &mut s,
        B,
        |_| {},
        smi(-4),
        |s, _, k| {
            s.b.mod_smi(2, k);
        },
    ));
    // 8: -5 % 3 = -2 (cold path, an int32: implicit).
    v.push(site(
        &mut s,
        B,
        |_| {},
        smi(-5),
        |s, _, k| {
            s.b.mod_smi(3, k);
        },
    ));
    // 9: 1.5 + 2: a number pair.
    v.push(site(&mut s, B, num(1.5), smi(2), |s, l, k| {
        s.b.add(l, k);
    }));
    // 10: "a" + 1.
    v.push(site(
        &mut s,
        B,
        |s| {
            s.lda_str("a");
        },
        smi(1),
        |s, l, k| {
            s.b.add(l, k);
        },
    ));
    // 11: undefined - 1.
    v.push(site(
        &mut s,
        B,
        |s| {
            s.b.lda_undefined();
        },
        smi(1),
        |s, l, k| {
            s.b.sub(l, k);
        },
    ));
    // 12: {} * 2.
    v.push(site(
        &mut s,
        B,
        |s| {
            s.b.create_empty_object_literal();
        },
        smi(2),
        |s, l, k| {
            s.b.mul(l, k);
        },
    ));
    // 13: a Wide `AddSmi` that overflows (the prefixed fast path goes cold; `AddSmi`'s own
    // fast body then takes its double arm): implicit, as case 1.
    v.push(site(
        &mut s,
        B,
        |_| {},
        num(2147483000.0),
        |s, _, k| {
            s.b.add_smi(1000, k);
        },
    ));
    // 14: a Wide `AddSmi` that fits (implicit).
    v.push(site(
        &mut s,
        B,
        |_| {},
        smi(5),
        |s, _, k| {
            s.b.add_smi(1000, k);
        },
    ));
    // 15: `++` on 1.5, then on "a" (one unary site, two executions).
    let k15 = s.b.feedback_slot(B);
    s.lda_num(1.5);
    s.b.inc(k15);
    s.lda_str("a");
    s.b.inc(k15);
    v.push(k15);
    // 16: `-` on int32 0 (-0, but the unary hook sees only an int32: implicit), ToNumber(true).
    let k16 = s.b.feedback_slot(B);
    s.b.lda_zero().negate(k16).lda_true().to_number(k16);
    v.push(k16);
    assert_eq!(v.len(), BINARY_CASES);
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), v)
}

#[test]
fn binary_sites_record_their_hints_and_int32_pairs_are_implicit() {
    let got = slots(RUN, || binary_program().0);
    let k = binary_program().1;
    let ovf = bin(BinaryHint::INT32_OVERFLOW);
    let want = [
        0,
        0,
        ovf,
        ovf,
        ovf,
        0,
        ovf,
        ovf,
        0,
        bin(BinaryHint::NUMBER),
        bin(BinaryHint::STRING) | bin(BinaryHint::NUMBER),
        bin(BinaryHint::ODDBALL) | bin(BinaryHint::NUMBER),
        bin(BinaryHint::OTHER) | bin(BinaryHint::NUMBER),
        0,
        0,
        bin(BinaryHint::NUMBER) | bin(BinaryHint::STRING),
        bin(BinaryHint::ODDBALL),
    ];
    for (i, (&slot, &w)) in k.iter().zip(&want).enumerate() {
        assert_eq!(value(&got, 0, slot), w, "binary case {i}");
        assert_eq!(
            key(&got, 0, slot),
            Key::Undefined,
            "binary case {i}: key untouched"
        );
    }
}

#[test]
fn the_int32_overflow_rule_matches_each_operator() {
    use crate::conv::BinOp;
    let f = profile::int32_pair_overflows;
    // `**` throws "not implemented" in T0 until M4-P1 MATH1, so its rule is checked here.
    assert!(!f(BinOp::Exp, 2, 3));
    assert!(!f(BinOp::Exp, -3, 3));
    assert!(!f(BinOp::Exp, 7, 0));
    assert!(f(BinOp::Exp, 2, 31), "2 ** 31 is past int32");
    assert!(!f(BinOp::Exp, -2, 31), "-2 ** 31 is i32::MIN");
    assert!(f(BinOp::Exp, 2, -1), "a fraction");
    assert!(!f(BinOp::Exp, 1, -5));
    assert!(!f(BinOp::Exp, -1, -5));
    assert!(f(BinOp::Exp, 0, -1), "Infinity");
    assert!(f(BinOp::Mod, 5, 0), "NaN");
    assert!(f(BinOp::Mod, i32::MIN, -1), "-0");
    assert!(f(BinOp::Mod, -4, 2), "-0");
    assert!(!f(BinOp::Mod, -7, 2));
    assert!(!f(BinOp::Mod, 7, 2));
    assert!(f(BinOp::Add, i32::MAX, 1));
    assert!(f(BinOp::Sub, i32::MIN, 1));
    assert!(!f(BinOp::Add, 2, 3));
    assert!(f(BinOp::Mul, 0, -5), "-0");
    assert!(f(BinOp::Mul, 65536, 65536));
    assert!(!f(BinOp::Mul, -3, 7));
    assert!(f(BinOp::Div, 1, 2));
    assert!(f(BinOp::Div, 1, 0), "Infinity");
    assert!(!f(BinOp::Div, -6, 3));
    assert!(!f(BinOp::Shr, 8, 1));
    assert!(f(BinOp::Shr, -1, 0));
    assert!(!f(BinOp::Shl, 1, 31), "i32::MIN");
}

// ---- CompareOp and TypeOf ------------------------------------------------------------------

fn compare_program() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    use FeedbackSlotKind::CompareOp as C;
    let mut s = Asm::script(0);
    let mut v = Vec::new();
    let lt = |s: &mut Asm, l: Register, k: FeedbackSlot| {
        s.b.test_less_than(l, k);
    };
    let seq = |s: &mut Asm, l: Register, k: FeedbackSlot| {
        s.b.test_equal_strict(l, k);
    };
    let eq = |s: &mut Asm, l: Register, k: FeedbackSlot| {
        s.b.test_equal(l, k);
    };
    // 0: 1 < 2 (implicit).
    v.push(site(
        &mut s,
        C,
        |s| {
            s.b.lda_smi(1);
        },
        |s| {
            s.b.lda_smi(2);
        },
        lt,
    ));
    // 1: 1.5 < 2.
    v.push(site(
        &mut s,
        C,
        |s| {
            s.lda_num(1.5);
        },
        |s| {
            s.b.lda_smi(2);
        },
        lt,
    ));
    // 2: 1.5 < 2.5.
    v.push(site(
        &mut s,
        C,
        |s| {
            s.lda_num(1.5);
        },
        |s| {
            s.lda_num(2.5);
        },
        lt,
    ));
    // 3: "a" === "a" (constants are atoms).
    v.push(site(
        &mut s,
        C,
        |s| {
            s.lda_str("a");
        },
        |s| {
            s.lda_str("a");
        },
        seq,
    ));
    // 4: "ab" === "a" + "b" (an atom and a new string).
    v.push(site(
        &mut s,
        C,
        |s| {
            s.lda_str("ab");
        },
        |s| {
            let t = s.b.new_register();
            let k = s.b.feedback_slot(FeedbackSlotKind::BinaryOp);
            s.lda_str("a");
            s.b.star(t);
            s.lda_str("b");
            s.b.add(t, k);
        },
        seq,
    ));
    // 5: {} == null.
    v.push(site(
        &mut s,
        C,
        |s| {
            s.b.create_empty_object_literal();
        },
        |s| {
            s.b.lda_null();
        },
        eq,
    ));
    // 6: true === false.
    v.push(site(
        &mut s,
        C,
        |s| {
            s.b.lda_true();
        },
        |s| {
            s.b.lda_false();
        },
        seq,
    ));
    // 7: null == undefined.
    v.push(site(
        &mut s,
        C,
        |s| {
            s.b.lda_null();
        },
        |s| {
            s.b.lda_undefined();
        },
        eq,
    ));
    // 8: "1" < 2.
    v.push(site(
        &mut s,
        C,
        |s| {
            s.lda_str("1");
        },
        |s| {
            s.b.lda_smi(2);
        },
        lt,
    ));
    // 9: {} === {} (two receivers).
    v.push(site(
        &mut s,
        C,
        |s| {
            s.b.create_empty_object_literal();
        },
        |s| {
            s.b.create_empty_object_literal();
        },
        seq,
    ));
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), v)
}

#[test]
fn compare_sites_record_their_hints_and_int32_pairs_are_implicit() {
    let got = slots(RUN, || compare_program().0);
    let k = compare_program().1;
    let want = [
        0,
        cmp(CompareHint::NUMBER),
        cmp(CompareHint::NUMBER),
        cmp(CompareHint::ATOM),
        cmp(CompareHint::STRING),
        cmp(CompareHint::RECEIVER) | cmp(CompareHint::NULLISH),
        cmp(CompareHint::BOOLEAN),
        cmp(CompareHint::NULLISH),
        cmp(CompareHint::OTHER),
        cmp(CompareHint::RECEIVER),
    ];
    assert_eq!(k.len(), want.len());
    for (i, (&slot, &w)) in k.iter().zip(&want).enumerate() {
        assert_eq!(value(&got, 0, slot), w, "compare case {i}");
        assert_eq!(
            key(&got, 0, slot),
            Key::Undefined,
            "compare case {i}: key untouched"
        );
    }
}

#[test]
fn typeof_sites_record_the_type_of_each_operand() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let k = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
        let one = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
        let f = s.b.new_register();
        let fk = s.b.function_constant(FunctionId(1));
        s.b.create_closure(fk, s.closure, 0).star(f);
        s.b.lda_smi(1).type_of(k);
        s.lda_num(0.5);
        s.b.type_of(k)
            .lda_undefined()
            .type_of(k)
            .lda_null()
            .type_of(k);
        s.b.lda_true().type_of(k);
        s.lda_str("s");
        s.b.type_of(k).create_empty_object_literal().type_of(k);
        s.b.create_empty_array_literal(s.lit)
            .type_of(k)
            .ldar(f)
            .type_of(k);
        // One site that sees one type.
        s.lda_str("t");
        s.b.type_of(one).lda_undefined().return_();
        let mut g = Asm::new(meta("g", 0, 0, 0, 0));
        g.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    let got = slots(RUN, make);
    let all = [
        TypeSet::INT32,
        TypeSet::DOUBLE,
        TypeSet::UNDEFINED,
        TypeSet::NULL,
        TypeSet::BOOLEAN,
        TypeSet::STRING,
        TypeSet::PLAIN_OBJECT,
        TypeSet::ARRAY,
        TypeSet::FUNCTION,
    ]
    .iter()
    .fold(0, |a, &t| a | ty(t));
    // The slots of the script: s.call .. s.loop_ come first (12), then the two TypeOf slots.
    let mut s = Asm::script(0);
    let k = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
    let one = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
    assert_eq!(value(&got, 0, k), all);
    assert_eq!(value(&got, 0, one), ty(TypeSet::STRING));
}

// ---- keyed ---------------------------------------------------------------------------------

fn keyed_program() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    use FeedbackSlotKind::{DefineKeyedOwn, HasKeyed, LoadKeyed, StoreInArrayLiteral, StoreKeyed};
    let mut s = Asm::script(0);
    let mut v = Vec::new();
    let (a, o) = (s.b.new_register(), s.b.new_register());
    let key = s.b.new_register();
    s.b.create_empty_array_literal(s.lit).star(a);
    s.b.create_empty_object_literal().star(o);
    // 0: a[0] = 1 on [] (out of bounds: the array grows).
    let k0 = s.b.feedback_slot(StoreKeyed);
    s.b.lda_zero()
        .star(key)
        .lda_smi(1)
        .set_keyed_property(a, key, k0);
    v.push(k0);
    // 1: a[0] read: a present element (implicit).
    let k1 = s.b.feedback_slot(LoadKeyed);
    s.b.lda_zero().get_keyed_property(a, k1);
    v.push(k1);
    // 2: a[5]: out of bounds.
    let k2 = s.b.feedback_slot(LoadKeyed);
    s.b.lda_smi(5).get_keyed_property(a, k2);
    v.push(k2);
    // 3: a[3] = 1 (out of bounds), then a[1]: a hole.
    let k3s = s.b.feedback_slot(StoreKeyed);
    s.b.lda_smi(3)
        .star(key)
        .lda_smi(1)
        .set_keyed_property(a, key, k3s);
    let k3 = s.b.feedback_slot(LoadKeyed);
    s.b.lda_smi(1).get_keyed_property(a, k3);
    v.push(k3);
    // 4: o["x"]; 5: o["1"] (an index string); 6: "abc"[1]; 7: a[-1]; 8: a[1.5];
    // 9: a[3e9] (an index double); 10: (7)[0].
    let k4 = s.b.feedback_slot(LoadKeyed);
    s.lda_str("x");
    s.b.get_keyed_property(o, k4);
    v.push(k4);
    let k5 = s.b.feedback_slot(LoadKeyed);
    s.lda_str("1");
    s.b.get_keyed_property(o, k5);
    v.push(k5);
    let k6 = s.b.feedback_slot(LoadKeyed);
    let str_ = s.b.new_register();
    s.lda_str("abc");
    s.b.star(str_).lda_smi(1).get_keyed_property(str_, k6);
    v.push(k6);
    let k7 = s.b.feedback_slot(LoadKeyed);
    s.b.lda_smi(-1).get_keyed_property(a, k7);
    v.push(k7);
    let k8 = s.b.feedback_slot(LoadKeyed);
    s.lda_num(1.5);
    s.b.get_keyed_property(a, k8);
    v.push(k8);
    let k9 = s.b.feedback_slot(LoadKeyed);
    s.lda_num(3e9);
    s.b.get_keyed_property(a, k9);
    v.push(k9);
    let k10 = s.b.feedback_slot(LoadKeyed);
    let seven = s.b.new_register();
    s.b.lda_smi(7)
        .star(seven)
        .lda_zero()
        .get_keyed_property(seven, k10);
    v.push(k10);
    // 11: "x" in o.
    let k11 = s.b.feedback_slot(HasKeyed);
    s.lda_str("x");
    s.b.star(key).ldar(o).test_in(key, k11);
    v.push(k11);
    // 12: a define of o["y"] (a class field or a computed literal key); 13: a store into a
    // new array literal at index 0.
    let k12 = s.b.feedback_slot(DefineKeyedOwn);
    s.lda_str("y");
    s.b.star(key)
        .lda_smi(1)
        .define_keyed_own_property(o, key, 0, k12);
    v.push(k12);
    let k13 = s.b.feedback_slot(StoreInArrayLiteral);
    let lit = s.b.new_register();
    s.b.create_empty_array_literal(s.lit).star(lit);
    s.b.lda_zero()
        .star(key)
        .lda_smi(1)
        .sta_in_array_literal(lit, key, k13);
    v.push(k13);
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), v)
}

#[test]
fn keyed_sites_record_keys_receivers_and_misses() {
    let got = slots(RUN, || keyed_program().0);
    let k = keyed_program().1;
    let ia = keyed::INDEX | keyed::ARRAY;
    let want = [
        ia | keyed::OUT_OF_BOUNDS,
        0,
        ia | keyed::OUT_OF_BOUNDS,
        ia | keyed::HOLE,
        keyed::NAME | keyed::OBJECT,
        keyed::INDEX | keyed::OBJECT,
        keyed::INDEX | keyed::STRING,
        keyed::OTHER_KEY | keyed::ARRAY,
        keyed::OTHER_KEY | keyed::ARRAY,
        ia | keyed::OUT_OF_BOUNDS,
        keyed::INDEX | keyed::PRIMITIVE,
        keyed::NAME | keyed::OBJECT,
        keyed::NAME | keyed::OBJECT,
        ia | keyed::OUT_OF_BOUNDS,
    ];
    assert_eq!(k.len(), want.len());
    for (i, (&slot, &w)) in k.iter().zip(&want).enumerate() {
        assert_eq!(value(&got, 0, slot), w, "keyed case {i}");
        assert_eq!(
            key(&got, 0, slot),
            Key::Undefined,
            "keyed case {i}: key untouched"
        );
    }
}

/// One key form of `keyed_index_program`.
#[derive(Clone, Copy)]
enum IndexKey {
    Int(i32),
    Num(f64),
    Str(&'static str),
    /// The string `head + tail` made at run time (a flat string: shorter than a rope), so
    /// its hash is not computed when the profile reads it.
    Concat(&'static str, &'static str),
}

/// The site of each `keyed_index_program` case and its keyed bits: an op on the array
/// `a = [, , , 1]` (length 4: holes at 0 to 2, present at 3).
const INDEX_CASES: [(FeedbackSlotKind, IndexKey, u32); 21] = {
    use FeedbackSlotKind::{HasKeyed, LoadKeyed, StoreKeyed};
    use IndexKey::{Concat, Int, Num, Str};
    let ia = keyed::INDEX | keyed::ARRAY;
    let (hole, oob) = (ia | keyed::HOLE, ia | keyed::OUT_OF_BOUNDS);
    let (name, other) = (keyed::NAME | keyed::ARRAY, keyed::OTHER_KEY | keyed::ARRAY);
    [
        // int32 keys (the reference: the element case was already recorded).
        (LoadKeyed, Int(1), hole),
        (LoadKeyed, Int(4), oob),
        // Index strings.
        (LoadKeyed, Str("3"), ia),
        (LoadKeyed, Str("1"), hole),
        (LoadKeyed, Str("4"), oob),
        (LoadKeyed, Str("4294967294"), oob),
        (LoadKeyed, Concat("42949", "67294"), oob),
        (LoadKeyed, Str("4294967295"), name),
        (LoadKeyed, Str("01"), name),
        // Integral doubles; -0 is the index 0.
        (LoadKeyed, Num(-0.0), hole),
        // An integral double constant in the int32 range loads as an int32: the implicit
        // fast case (a present element), so nothing is recorded.
        (LoadKeyed, Num(3.0), 0),
        (LoadKeyed, Num(2_147_483_648.0), oob),
        (LoadKeyed, Num(3e9), oob),
        (LoadKeyed, Num(4_294_967_294.0), oob),
        (LoadKeyed, Num(4_294_967_295.0), other),
        // A store and `in` record the same element cases.
        (StoreKeyed, Str("9"), oob),
        (StoreKeyed, Num(2_147_483_648.0), oob),
        (HasKeyed, Str("2"), hole),
        (HasKeyed, Num(-0.0), hole),
        (HasKeyed, Num(3e9), oob),
        (HasKeyed, Str("4294967294"), oob),
    ]
};

/// Each [`INDEX_CASES`] op at its own slot, on a fresh `a = [, , , 1]` (a store does not
/// change the array of a later case).
fn keyed_index_program() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let (a, key, val) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    let mut v = Vec::new();
    for (kind, k, _) in INDEX_CASES {
        s.b.create_empty_array_literal(s.lit).star(a);
        s.b.lda_smi(1).star(val).lda_smi(3).star(key).ldar(val);
        let k3 = s.b.feedback_slot(FeedbackSlotKind::StoreKeyed);
        s.b.set_keyed_property(a, key, k3);
        match k {
            IndexKey::Int(i) => {
                s.b.lda_smi(i);
            }
            IndexKey::Num(d) => {
                s.lda_num(d);
            }
            IndexKey::Str(t) => {
                s.lda_str(t);
            }
            IndexKey::Concat(head, tail) => {
                let lhs = s.b.new_register();
                s.lda_str(head);
                s.b.star(lhs);
                s.lda_str(tail);
                s.b.add(lhs, s.bin);
            }
        }
        let slot = s.b.feedback_slot(kind);
        match kind {
            FeedbackSlotKind::LoadKeyed => {
                s.b.get_keyed_property(a, slot);
            }
            FeedbackSlotKind::StoreKeyed => {
                s.b.star(key).ldar(val).set_keyed_property(a, key, slot);
            }
            _ => {
                s.b.star(key).ldar(a).test_in(key, slot);
            }
        }
        v.push(slot);
    }
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), v)
}

#[test]
fn keyed_sites_record_the_element_case_of_every_index_key_form() {
    // An index string, -0 or an integral double up to 2^32 - 2 is the same array index as
    // the int32 key: a hole or an index past the length records HOLE or OUT_OF_BOUNDS.
    let got = slots(RUN, || keyed_index_program().0);
    let k = keyed_index_program().1;
    assert_eq!(k.len(), INDEX_CASES.len());
    let wrong: Vec<(usize, u32, u32)> = k
        .iter()
        .zip(&INDEX_CASES)
        .enumerate()
        .map(|(i, (&slot, &(_, _, w)))| (i, value(&got, 0, slot), w))
        .filter(|&(_, g, w)| g != w)
        .collect();
    assert!(wrong.is_empty(), "(case, got, want): {wrong:?}");
}

/// Keyed loads whose key is a rope that is not flattened (a CONS from a string `+` of 13 or
/// more units): `o[k]` on an object and `a[k]` on an array, with a letter rope and a digit
/// rope (13 digits: not an array index).
fn keyed_rope_program() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let (a, o) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_array_literal(s.lit).star(a);
    s.b.create_empty_object_literal().star(o);
    let mut v = Vec::new();
    for (head, tail, receiver) in [
        ("abcdefg", "hijklm", o),
        ("1234567", "890123", o),
        ("abcdefg", "hijklm", a),
        ("1234567", "890123", a),
    ] {
        let k = s.b.feedback_slot(FeedbackSlotKind::LoadKeyed);
        let lhs = s.b.new_register();
        s.lda_str(head);
        s.b.star(lhs);
        s.lda_str(tail);
        s.b.add(lhs, s.bin).get_keyed_property(receiver, k);
        v.push(k);
    }
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), v)
}

#[test]
fn keyed_sites_read_rope_keys_in_place() {
    // `keyed_key` calls `string::array_index` on the key before the generic operation, so
    // the key is still a CONS here (M4-P1 PROFILE need 5): the rope reads as a name.
    let got = slots(RUN, || keyed_rope_program().0);
    let k = keyed_rope_program().1;
    let want = [
        keyed::NAME | keyed::OBJECT,
        keyed::NAME | keyed::OBJECT,
        keyed::NAME | keyed::ARRAY,
        keyed::NAME | keyed::ARRAY,
    ];
    for (i, (&slot, &w)) in k.iter().zip(&want).enumerate() {
        assert_eq!(value(&got, 0, slot), w, "rope key case {i}");
    }
}

// ---- calls ---------------------------------------------------------------------------------

/// The script slots of `call_program`: the mono site, the mega site, the closures site, the
/// host site and the non-callable site.
struct CallSites {
    mono: FeedbackSlot,
    mega: FeedbackSlot,
    closures: FeedbackSlot,
    host: FeedbackSlot,
    bad: FeedbackSlot,
}

/// Functions 1 (`f`) and 2 (`g`) return undefined. The script calls `f` `n` times at the
/// mono site; `f` and `g` in turn `n` times at the mega site; two closures of `f` at the
/// closures site; `print()` at the host site; and last `undefined()` (a TypeError) at the
/// bad site if `bad`.
fn call_program(n: i32, bad: bool) -> (VerifiedUnit, CallSites) {
    use FeedbackSlotKind::Call;
    let mut s = Asm::script(0);
    let sites = CallSites {
        mono: s.b.feedback_slot(Call),
        mega: s.b.feedback_slot(Call),
        closures: s.b.feedback_slot(Call),
        host: s.b.feedback_slot(Call),
        bad: s.b.feedback_slot(Call),
    };
    let (f, g, f2, h) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    let (kf, kg) = (
        s.b.function_constant(FunctionId(1)),
        s.b.function_constant(FunctionId(2)),
    );
    s.b.create_closure(kf, s.closure, 0).star(f);
    s.b.create_closure(kg, s.closure, 0).star(g);
    s.b.create_closure(kf, s.closure, 0).star(f2);
    counted_loop(&mut s, n, |s, _| {
        s.b.call_undefined_receiver0(f, sites.mono);
    });
    counted_loop(&mut s, n, |s, i| {
        let odd = s.b.feedback_slot(FeedbackSlotKind::BinaryOp);
        let other = s.b.new_label();
        let call = s.b.new_label();
        s.b.ldar(i)
            .bitwise_and_smi(1, odd)
            .jump_if_to_boolean_true(other);
        s.b.ldar(f).star(h).jump(call);
        s.b.bind(other);
        s.b.ldar(g).star(h);
        s.b.bind(call);
        s.b.call_undefined_receiver0(h, sites.mega);
    });
    for c in [f, f2] {
        s.b.call_undefined_receiver0(c, sites.closures);
    }
    s.lda_global("print");
    s.b.star(h).call_undefined_receiver0(h, sites.host);
    if bad {
        s.b.lda_undefined()
            .star(h)
            .call_undefined_receiver0(h, sites.bad);
    }
    s.b.lda_undefined().return_();
    let mut fa = Asm::new(meta("f", 0, 0, 0, 0));
    fa.b.lda_undefined().return_();
    let mut ga = Asm::new(meta("g", 0, 0, 0, 0));
    ga.b.lda_undefined().return_();
    (unit("", vec![s.finish(), fa.finish(), ga.finish()]), sites)
}

#[test]
fn calls_in_the_window_record_target_and_count() {
    // 6 calls at each loop site: every unit of the run is inside the first CALL_WINDOW units
    // after the isolate start.
    const N: i32 = 6;
    assert!(i64::from(4 * N + 8) < CALL_WINDOW);
    let got = slots(RUN, || call_program(N, false).0);
    let k = call_program(N, false).1;
    let n = N as u32;
    assert_eq!(
        (key(&got, 0, k.mono), value(&got, 0, k.mono)),
        (Key::Info(1), n)
    );
    assert_eq!(
        (key(&got, 0, k.mega), value(&got, 0, k.mega)),
        (Key::Hole, n),
        "f and g at one site: megamorphic"
    );
    assert_eq!(
        (key(&got, 0, k.closures), value(&got, 0, k.closures)),
        (Key::Info(1), 2),
        "two closures of f are one target"
    );
    assert_eq!(
        (key(&got, 0, k.host), value(&got, 0, k.host)),
        (Key::Native, 1),
        "a host function: its own FunctionInfo"
    );
    assert_eq!(key(&got, 0, k.bad), Key::Undefined, "never ran");
}

#[test]
fn a_callee_that_is_not_a_function_cell_makes_the_site_megamorphic() {
    let how = How { ok: false, ..RUN };
    let got = slots(how, || call_program(1, true).0);
    let k = call_program(1, true).1;
    assert_eq!((key(&got, 0, k.bad), value(&got, 0, k.bad)), (Key::Hole, 1));
}

#[test]
fn calls_outside_the_window_are_not_recorded_until_a_poll() {
    // Start well below the window: 6 calls per site record nothing.
    let low = How {
        budget: Some(BUDGET / 2),
        ..RUN
    };
    let got = slots(low, || call_program(6, false).0);
    let k = call_program(6, false).1;
    for s in [k.mono, k.mega, k.closures, k.host] {
        assert_eq!((key(&got, 0, s), value(&got, 0, s)), (Key::Undefined, 0));
    }
    // A poll resets the budget: the calls of the next CALL_WINDOW units are recorded. The
    // mono loop takes 2 units per iteration (the call and the back edge).
    let near = How {
        budget: Some(10),
        ..RUN
    };
    let got = slots(near, || call_program(200, false).0);
    let mono = value(&got, 0, k.mono);
    assert_eq!(key(&got, 0, k.mono), Key::Info(1));
    assert!(
        (1..=(CALL_WINDOW as u32) / 2 + 6).contains(&mono),
        "about one window of calls: {mono}"
    );
}

// ---- JumpLoop, CreateClosure, ForIn ----------------------------------------------------------

#[test]
fn back_edges_are_counted_at_the_budget_polls() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        counted_loop(&mut s, 2 * BUDGET as i32 + 10, |_, _| {});
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let got = slots(RUN, make);
    let s = Asm::script(0);
    // One back edge per unit and no call: the loop polls twice.
    assert_eq!(value(&got, 0, s.loop_), 2);
    assert_eq!(key(&got, 0, s.loop_), Key::Undefined);
    // The loop's compare and `++` are int32: implicit.
    assert_eq!(value(&got, 0, s.cmp), 0);
    assert_eq!(value(&got, 0, s.bin), 0);
}

#[test]
fn closures_count_none_one_many() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let one = s.b.feedback_slot(FeedbackSlotKind::CreateClosure);
        let many = s.b.feedback_slot(FeedbackSlotKind::CreateClosure);
        let _none = s.b.feedback_slot(FeedbackSlotKind::CreateClosure);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, one, 0);
        counted_loop(&mut s, 5, |s, _| {
            s.b.create_closure(k, many, 0);
        });
        s.b.lda_undefined().return_();
        let mut f = Asm::new(meta("f", 0, 0, 0, 0));
        f.b.lda_undefined().return_();
        unit("", vec![s.finish(), f.finish()])
    }
    let got = slots(RUN, make);
    let mut s = Asm::script(0);
    let one = s.b.feedback_slot(FeedbackSlotKind::CreateClosure);
    let many = s.b.feedback_slot(FeedbackSlotKind::CreateClosure);
    let none = s.b.feedback_slot(FeedbackSlotKind::CreateClosure);
    assert_eq!(value(&got, 0, one), 1);
    assert_eq!(value(&got, 0, many), CLOSURE_MANY, "saturates at many");
    assert_eq!(value(&got, 0, none), 0);
    assert_eq!(value(&got, 0, s.closure), 0, "the Asm slot is unused");
}

/// `for (k in o) {}` at the ForIn slot `fs`.
fn for_in(s: &mut Asm, o: Register, fs: FeedbackSlot) {
    let cache = s.b.new_register_list(3);
    let idx = s.b.new_register();
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
    s.b.bind(next);
    s.b.for_in_step(idx).jump_loop(head, 0, s.loop_);
    s.b.bind(done);
}

fn for_in_program() -> (VerifiedUnit, [FeedbackSlot; 3]) {
    let mut s = Asm::script(0);
    let mono = s.b.feedback_slot(FeedbackSlotKind::ForIn);
    let mega = s.b.feedback_slot(FeedbackSlotKind::ForIn);
    let empty = s.b.feedback_slot(FeedbackSlotKind::ForIn);
    let (o, p, q) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    // o = {x: 1}; p = {y: 1}: two shapes. q = {}: no key, so ForInNext never runs.
    s.b.create_empty_object_literal().star(o);
    s.b.lda_smi(1);
    s.set_named(o, "x");
    s.b.create_empty_object_literal().star(p);
    s.b.lda_smi(1);
    s.set_named(p, "y");
    s.b.create_empty_object_literal().star(q);
    for_in(&mut s, o, mono);
    for_in(&mut s, o, mono);
    for (r, fs) in [(o, mega), (p, mega)] {
        for_in(&mut s, r, fs);
    }
    for_in(&mut s, q, empty);
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), [mono, mega, empty])
}

#[test]
fn for_in_sites_record_the_receiver_shape_weakly() {
    let (got, after) = slots_gc(RUN, || for_in_program().0);
    let [mono, mega, empty] = for_in_program().1;
    let any = u32::from(ForInHint::ANY.bits());
    assert_eq!(
        (key(&got, 0, mono), value(&got, 0, mono)),
        (Key::Cell(kinds::SHAPE.get()), any)
    );
    assert_eq!((key(&got, 0, mega), value(&got, 0, mega)), (Key::Hole, any));
    assert_eq!(
        (key(&got, 0, empty), value(&got, 0, empty)),
        (Key::Undefined, any)
    );
    // The objects die with the run; the key is weak, so the GC clears it and the next
    // receiver starts the site again. The bits and the hole stay.
    assert_eq!(
        (key(&after, 0, mono), value(&after, 0, mono)),
        (Key::Undefined, any)
    );
    assert_eq!(key(&after, 0, mega), Key::Hole);
}

// ---- no vector, GC, the count rules ---------------------------------------------------------

#[test]
fn a_function_without_a_vector_records_nothing() {
    let never = How {
        threshold: u32::MAX,
        ..RUN
    };
    let got = slots(never, || binary_program().0);
    assert!(got[0].is_none());
}

#[test]
fn call_keys_survive_gc_and_need_no_barrier() {
    // The key is a native word, not a cell: a GC neither clears nor moves it, the verifier
    // accepts it, and a closure made after the GC is still the same target. `slots` runs
    // every program under GC stress (a GC before every allocation) and verifies the heap.
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let site = s.b.feedback_slot(FeedbackSlotKind::Call);
        let k = s.b.function_constant(FunctionId(1));
        let f = s.b.new_register();
        for _ in 0..3 {
            // A new closure each time; the old one is garbage at the next allocation.
            s.b.create_closure(k, s.closure, 0).star(f);
            s.b.create_empty_object_literal();
            s.b.call_undefined_receiver0(f, site);
        }
        s.b.lda_undefined().return_();
        let mut fa = Asm::new(meta("f", 0, 0, 0, 0));
        fa.b.lda_undefined().return_();
        unit("", vec![s.finish(), fa.finish()])
    }
    let got = slots(RUN, make);
    let mut s = Asm::script(0);
    let site = s.b.feedback_slot(FeedbackSlotKind::Call);
    assert_eq!(
        (key(&got, 0, site), value(&got, 0, site)),
        (Key::Info(1), 3)
    );
}

#[test]
fn counts_saturate_and_leave_a_handler_alone() {
    // White box: the count rule on a real vector, with values no program reaches in a test.
    on_big_stack(|| {
        let mut iso = isolate(IsolateConfig {
            feedback_threshold: 0,
            ..config(Dispatch::ALL[0], false)
        });
        iso.with_cx_checkpoint(|cx| cx.run_script(binary_program().0, "profile.js").map(|_| ()))
            .unwrap();
        {
            let units = iso.rt.units.borrow();
            let fi = &units.last().unwrap().infos[0];
            let v = feedback::vector(fi).unwrap();
            let rt: &crate::runtime::Runtime = &iso.rt;
            let near = int32_bits(i32::MAX - 1);
            feedback::set_slot(rt, v, 0, UNDEFINED, near);
            profile::count(rt, v, 0, i32::MAX as u32);
            assert_eq!(feedback::value(v, 0), int32_bits(i32::MAX));
            profile::count(rt, v, 0, i32::MAX as u32);
            assert_eq!(feedback::value(v, 0), int32_bits(i32::MAX), "saturated");
            profile::count(rt, v, 0, CLOSURE_MANY);
            assert_eq!(
                feedback::value(v, 0),
                int32_bits(i32::MAX),
                "above the limit: no store"
            );
            // A value half that is not an int32 (a construct-site handler of a shared Call slot):
            // the count leaves it.
            feedback::set_slot(rt, v, 0, UNDEFINED, v);
            profile::count(rt, v, 0, i32::MAX as u32);
            assert_eq!(feedback::value(v, 0), v);
            feedback::set_slot(rt, v, 0, UNDEFINED, int32_bits(-1));
            profile::count(rt, v, 0, i32::MAX as u32);
            assert_eq!(
                feedback::value(v, 0),
                int32_bits(-1),
                "a negative int32 is no count"
            );
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}
