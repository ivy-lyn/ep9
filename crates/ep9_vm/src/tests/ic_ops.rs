//! Tests of `ops_of` (`ic_ops.rs`, M6-P1 CBC; V11-D20): every IC opcode has an operation, every
//! list is guards then one action, and, per handler kind and slot kind, the list matches the
//! handler's checks:
//!
//! ```text
//!  warm a site (T0) --> cases(slot) --> ops_of --> eval(list, probe)       (this file)
//!                                                     |  guards pass, action value
//!  probe receivers / operands -----------------------> compare with
//!     named, global: the T0 hit path itself (`ic::load`, `ic::store`, `ic::load_global`):
//!                    every T0 hit passes the guards and gives the same value; the guards
//!                    pass only where T0 hits (except the H16 widening of ARRAY_LENGTH)
//!     keyed, InstanceOf, Call, BinaryOp, CompareOp: soundness (guards pass -> the action's
//!                    value is what the site computes) and coverage (what warmed the site
//!                    passes the guards)
//! ```
//!
//! The two T0 speed arms of M6-P1 (V11-D20: `===` of two atoms, `+` of two strings) are
//! tested at the end.
//!
//! The named, global, keyed, InstanceOf and Call tests run under every dispatch expansion,
//! with and without GC stress (`tests::ic::each`); the arithmetic ones under every expansion
//! without stress (no cell is cached there).

use ep9_bytecode::{
    FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionId, OPCODES, OperandParam,
    VerifiedUnit,
};

use super::ic::{SLOT, Site, T, each, int, key, object_prototype, object_with};
use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::conv::BinOp;
use crate::external_ref::ExternalRef;
use crate::feedback;
use crate::field_rep;
use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_bits, int32_of};
use crate::ic::{self, KEY_STRING, hk, smi};
use crate::ic_ops::{
    Case, Class, Compare, Holder, Kinds, Loc, MAX_OPS, Op, Operation, Ops, Proto, Ref, Ty, Unary,
    Val, cases, handler_parts, ops_of,
};
use crate::instance_ic::io;
use crate::kinds::{self, is_cell_bits};
use crate::object::{self, arr, attr, cell, init, obj, rd};
use crate::property;
use crate::props;
use crate::runtime::{Atom, Root, Runtime, val};
use crate::shape::{self, slf};
use crate::string;
use crate::unit::FunctionInfo;
use crate::validity;
use crate::{Dispatch, IsolateConfig, Value};

// ---- the evaluator: what a list says for one probe -----------------------------------------------

/// The values of one probe of a site.
#[derive(Clone, Copy, Debug)]
struct Env {
    receiver: u64,
    value: u64,
    key: u64,
    lhs: u64,
    rhs: u64,
    callee: u64,
    function: u64,
}

impl Default for Env {
    fn default() -> Env {
        Env {
            receiver: UNDEFINED,
            value: UNDEFINED,
            key: UNDEFINED,
            lhs: UNDEFINED,
            rhs: UNDEFINED,
            callee: UNDEFINED,
            function: UNDEFINED,
        }
    }
}

/// What a list gives for a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    /// A guard failed or the action missed: the generic path.
    Miss,
    /// The action's value.
    Value(u64),
    /// A store action whose guards passed (nothing is stored here).
    Stored,
    /// A call action whose guards passed.
    Call,
}

fn shape_id(o: u64) -> u32 {
    // SAFETY: a cell value the test holds is live.
    unsafe { cell(o).header() }.shape_id().get()
}

/// The field `loc` of the object `o`.
fn field(o: u64, loc: Loc) -> u64 {
    if loc.in_object() {
        rd(cell(o), loc.offset())
    } else {
        rd(cell(rd(cell(o), obj::SLOTS)), loc.offset())
    }
}

fn is_i32(b: u64) -> bool {
    val(b).is_int32()
}

fn number(b: u64) -> Option<f64> {
    val(b).as_number()
}

fn num(d: f64) -> u64 {
    Value::number(d).to_bits()
}

fn boolean(b: bool) -> u64 {
    if b { TRUE } else { FALSE }
}

/// The list's answer for the probe `e` of the case `case` (the list is `ops_of(case, ..)`).
fn eval(rt: &Runtime, case: &Case, ops: &Ops, e: &Env) -> Out {
    let value_of = |v: Val| match v {
        Val::Receiver => e.receiver,
        Val::Value => e.value,
        Val::Key => e.key,
        Val::Lhs | Val::Operand => e.lhs,
        Val::Rhs => e.rhs,
        Val::Callee => e.callee,
        Val::Function => e.function,
        Val::Global => rt.global(),
        Val::Lexical => rt.root(Root::Lexical),
    };
    let (case_key, handler) = match *case {
        Case::Handler { key, handler } => (key, handler),
        Case::Keyed { key, .. } | Case::Call { key } => (key, UNDEFINED),
        _ => (UNDEFINED, UNDEFINED),
    };
    let ref_of = |r: Ref| match r {
        Ref::CaseKey => case_key,
        Ref::HandlerRef => rd(cell(handler), io::SHAPE),
    };
    let ty_ok = |x: u64, t: Ty| match t {
        Ty::Int32 => is_i32(x),
        Ty::Number => number(x).is_some(),
        Ty::Heap => number(x).is_none(),
    };
    for &op in ops.as_slice() {
        let pass = match op {
            Op::GuardShape(v, r) => {
                let (x, s) = (value_of(v), ref_of(r));
                is_cell_bits(x) && is_cell_bits(s) && shape_id(x) == s as u32
            }
            Op::GuardProtoValidity(_) => validity::valid_cell(rd(cell(handler), io::CELL)),
            Op::GuardKind(v, Class::Array) => kinds::is_array(value_of(v)),
            Op::GuardKind(v, Class::String) => kinds::is_string(value_of(v)),
            Op::GuardType(v, t) => ty_ok(value_of(v), t),
            Op::GuardTarget(Val::Callee, r) => {
                let f = value_of(Val::Callee);
                kinds::is_function(f)
                    && ExternalRef::new(object::function_info(f)).to_word() == ref_of(r)
            }
            Op::GuardTarget(v, r) => value_of(v) == ref_of(r),
            Op::GuardIndex(o, k) => {
                let (o, k) = (value_of(o), value_of(k));
                is_i32(k)
                    && int32_of(k) >= 0
                    && kinds::is_array(o)
                    && object::array_get(o, int32_of(k) as u32).is_some()
            }
            Op::GuardSlotsRoom(v, loc) => {
                let s = rd(cell(value_of(v)), obj::SLOTS);
                s != UNDEFINED
                    && loc.offset()
                        < slf::DATA + 8 * object::rd_i32(cell(s), slf::CAPACITY) as usize
            }
            Op::GuardNotHole(v, loc) => field(value_of(v), loc) != HOLE,
            // The view ops (TA-2 TA-KEYED): the spec-layer reading (`typed_array.rs`).
            Op::GuardKind(v, Class::View(t)) => {
                kinds::cell_kind(value_of(v)) == kinds::INT8_VIEW.get() + t
            }
            Op::GuardViewIndex(o, k) => {
                let (o, k) = (value_of(o), value_of(k));
                kinds::is_view(o)
                    && is_i32(k)
                    && crate::typed_array::valid_index(rt, o, f64::from(int32_of(k))).is_some()
            }
            // TA-3 (V13-D8): the view's buffer, by its class (`array_buffer::is_shared`).
            Op::GuardViewNotShared(v) => {
                let o = value_of(v);
                kinds::is_view(o)
                    && !crate::array_buffer::is_shared(crate::typed_array::buffer_of(o))
            }
            action => return act(rt, action, e, &value_of, &ref_of, &ty_ok),
        };
        if !pass {
            return Out::Miss;
        }
    }
    panic!("a list with no action: {ops:?}");
}

/// The action `op` of a list whose guards passed (the meanings in `ic_ops::Op`).
fn act(
    rt: &Runtime,
    op: Op,
    e: &Env,
    value_of: &dyn Fn(Val) -> u64,
    ref_of: &dyn Fn(Ref) -> u64,
    ty_ok: &dyn Fn(u64, Ty) -> bool,
) -> Out {
    match op {
        Op::LoadField(v, loc) => Out::Value(field(value_of(v), loc)),
        Op::StoreField(..) | Op::StoreGlobalField(_) => Out::Stored,
        Op::LoadHolderField(r, loc) => match ref_of(r) {
            UNDEFINED => Out::Miss,
            h => Out::Value(field(h, loc)),
        },
        Op::AddField(_, r, _) => {
            let target = ref_of(r);
            if target == UNDEFINED || !field_rep::fits(shape::last_field_rep(target), e.value) {
                Out::Miss
            } else {
                Out::Stored
            }
        }
        Op::LoadArrayLength => Out::Value(rd(cell(e.receiver), arr::LENGTH)),
        Op::LoadStringLength => Out::Value(int32_bits(string::str_len(e.receiver) as i32)),
        Op::LoadUndefined => Out::Value(UNDEFINED),
        Op::LoadGlobalField(loc) => Out::Value(field(rt.global(), loc)),
        Op::LoadElement(_) => Out::Value(
            object::array_get(e.receiver, int32_of(e.key) as u32).expect("GuardIndex passed"),
        ),
        Op::StoreElement(_) => {
            let k = crate::elements::kind_of(e.receiver);
            let fits = if k == crate::elements::kind::PACKED_DOUBLE
                || k == crate::elements::kind::HOLEY_DOUBLE
            {
                number(e.value).is_some()
            } else {
                is_i32(e.value) || k > crate::elements::kind::HOLEY_INT32
            };
            if fits { Out::Stored } else { Out::Miss }
        }
        Op::LoadViewOrUndefined(t) => {
            assert_eq!(
                kinds::cell_kind(e.receiver),
                kinds::INT8_VIEW.get() + t,
                "GuardKind passed"
            );
            Out::Value(crate::typed_array::get_element(
                rt,
                e.receiver,
                f64::from(int32_of(e.key)),
            ))
        }
        Op::StoreView(_) => Out::Stored,
        Op::CallTarget(_) | Op::CallGetter(..) | Op::CallSetter(..) => Out::Call,
        Op::LoadHolderDict(_) | Op::Generic => Out::Miss,
        Op::BinaryArith(o, t) => binary(o, t, e.lhs, e.rhs),
        Op::UnaryArith(u, t) => unary(u, t, e.lhs),
        Op::CompareNumbers(c, t) => {
            assert!(ty_ok(e.lhs, t) && ty_ok(e.rhs, t));
            let (x, y) = (number(e.lhs).unwrap(), number(e.rhs).unwrap());
            Out::Value(boolean(match c {
                Compare::Eq | Compare::StrictEq => x == y,
                Compare::Lt => x < y,
                Compare::Gt => x > y,
                Compare::Le => x <= y,
                Compare::Ge => x >= y,
            }))
        }
        Op::OrdinaryHasInstance(_) => {
            let o = e.lhs;
            if !kinds::is_object(o) {
                return Out::Value(FALSE);
            }
            let p = props::own_get(e.function, rt.atom(Atom::Prototype))
                .expect("an own prototype")
                .0;
            if !kinds::is_object(p) {
                return Out::Miss;
            }
            let mut q = o;
            loop {
                q = object::proto_of(q);
                if q == NULL {
                    return Out::Value(FALSE);
                }
                if q == p {
                    return Out::Value(TRUE);
                }
            }
        }
        g => panic!("not an action: {g:?}"),
    }
}

/// [`Op::BinaryArith`] as its doc says.
fn binary(op: BinOp, t: Ty, l: u64, r: u64) -> Out {
    if t == Ty::Int32 {
        let (a, b) = (int32_of(l), int32_of(r));
        let fits = |x: Option<i32>, d: f64| Out::Value(x.map_or(num(d), int32_bits));
        let (x, y) = (f64::from(a), f64::from(b));
        return match op {
            BinOp::Add => fits(a.checked_add(b), x + y),
            BinOp::Sub => fits(a.checked_sub(b), x - y),
            BinOp::Mul => Out::Value(num(x * y)),
            BinOp::Div => Out::Value(num(x / y)),
            BinOp::Mod if a >= 0 && b != 0 => Out::Value(int32_bits(a % b)),
            BinOp::Mod => Out::Miss,
            BinOp::BitOr => Out::Value(int32_bits(a | b)),
            BinOp::BitXor => Out::Value(int32_bits(a ^ b)),
            BinOp::BitAnd => Out::Value(int32_bits(a & b)),
            BinOp::Shl => Out::Value(int32_bits(a.wrapping_shl(b as u32 & 31))),
            BinOp::Sar => Out::Value(int32_bits(a >> (b as u32 & 31))),
            BinOp::Shr => Out::Value(num(f64::from((a as u32) >> (b as u32 & 31)))),
            BinOp::Exp => panic!("no int32 list for **"),
        };
    }
    let (x, y) = (number(l).unwrap(), number(r).unwrap());
    Out::Value(num(match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => x / y,
        _ => panic!("no number list for {op:?}"),
    }))
}

/// [`Op::UnaryArith`] as its doc says.
fn unary(u: Unary, t: Ty, v: u64) -> Out {
    if t == Ty::Int32 {
        let a = int32_of(v);
        let x = f64::from(a);
        return Out::Value(match u {
            Unary::Inc => a.checked_add(1).map_or(num(x + 1.0), int32_bits),
            Unary::Dec => a.checked_sub(1).map_or(num(x - 1.0), int32_bits),
            Unary::Negate => num(-x),
            Unary::BitNot => int32_bits(!a),
            Unary::ToNumber | Unary::ToNumeric => v,
        });
    }
    let x = number(v).unwrap();
    Out::Value(num(match u {
        Unary::Inc => x + 1.0,
        Unary::Dec => x - 1.0,
        Unary::Negate => -x,
        Unary::ToNumber | Unary::ToNumeric => x,
        Unary::BitNot => panic!("no number list for ~"),
    }))
}

/// Two results are the same JS value (NaN equals NaN; -0 and 0 differ).
fn same(a: u64, b: u64) -> bool {
    match (number(a), number(b)) {
        (Some(x), Some(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

/// The list form: 1 to `MAX_OPS` ops, guards first, then exactly one action.
fn check_form(ops: &Ops) {
    let l = ops.as_slice();
    assert!(!l.is_empty() && l.len() <= MAX_OPS, "{l:?}");
    let (last, guards) = l.split_last().unwrap();
    assert!(guards.iter().all(|o| o.is_guard()), "{l:?}");
    assert!(!last.is_guard(), "{l:?}");
    assert_eq!(ops.action(), *last);
}

/// The FunctionInfo of `f<i>` (function `i + 1` of the last unit).
fn info<'a>(t: &'a T, i: usize) -> std::cell::Ref<'a, FunctionInfo> {
    std::cell::Ref::map(t.rt().units.borrow(), |u| &u.last().unwrap().infos[i + 1])
}

// ---- the operation of every opcode -------------------------------------------------------------

#[test]
fn every_opcode_with_an_ic_slot_has_an_operation() {
    use FeedbackSlotKind as K;
    for o in OPCODES.iter() {
        let kinds: Vec<K> = o
            .operands
            .iter()
            .filter_map(|x| match x.param {
                OperandParam::Slot(k) => Some(k),
                _ => None,
            })
            .collect();
        let ic = kinds.iter().any(|k| {
            matches!(
                k,
                K::LoadNamed
                    | K::StoreNamed
                    | K::DefineNamedOwn
                    | K::LoadGlobal
                    | K::LoadGlobalInsideTypeof
                    | K::StoreGlobal
                    | K::LoadKeyed
                    | K::StoreKeyed
                    | K::DefineKeyedOwn
                    | K::StoreInArrayLiteral
                    | K::HasKeyed
                    | K::InstanceOf
                    | K::BinaryOp
                    | K::CompareOp
                    | K::Call
            )
        });
        let none = matches!(o.name, "GetIterator" | "TailCall");
        let got = Operation::of(o.opcode);
        assert_eq!(got.is_some(), ic && !none, "{}: {got:?}", o.name);
    }
}

// ---- the list form, over every int32 handler and hint ----------------------------------------------

#[test]
fn every_list_is_guards_then_one_action() {
    let ops_named = [
        Operation::LoadNamed,
        Operation::StoreNamed,
        Operation::DefineNamed,
        Operation::LoadGlobal,
        Operation::StoreGlobal,
        Operation::InstanceOf,
    ];
    // A fake shape key: the lists only name it.
    let shape_key = 0x0001_0000_0000_1230_u64;
    for op in ops_named {
        for k in 0..16 {
            for key in [shape_key, KEY_STRING] {
                let h = int32_bits(k | (24 << smi::SHIFT));
                check_form(&ops_of(&Case::Handler { key, handler: h }, op));
            }
        }
        check_form(&ops_of(&Case::Empty, op));
        check_form(&ops_of(&Case::Mega, op));
    }
    let binops = [
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Mod,
        BinOp::Exp,
        BinOp::BitOr,
        BinOp::BitXor,
        BinOp::BitAnd,
        BinOp::Shl,
        BinOp::Sar,
        BinOp::Shr,
    ];
    let unaries = [
        Unary::Inc,
        Unary::Dec,
        Unary::Negate,
        Unary::BitNot,
        Unary::ToNumber,
        Unary::ToNumeric,
    ];
    let compares = [
        Compare::Eq,
        Compare::StrictEq,
        Compare::Lt,
        Compare::Gt,
        Compare::Le,
        Compare::Ge,
    ];
    for bits in 0..1024 {
        let c = Case::Hint { bits };
        for &op in &binops {
            for imm in [false, true] {
                check_form(&ops_of(&c, Operation::Binary { op, imm }));
            }
        }
        for &u in &unaries {
            check_form(&ops_of(&c, Operation::Unary(u)));
        }
        for &cmp in &compares {
            check_form(&ops_of(&c, Operation::Compare(cmp)));
        }
    }
    for key in [UNDEFINED, HOLE, shape_key] {
        for op in [
            Operation::LoadKeyed,
            Operation::StoreKeyed,
            Operation::KeyedCold,
        ] {
            check_form(&ops_of(&Case::Keyed { key, bits: 0x3FF }, op));
        }
        for op in [Operation::Call, Operation::CallSpread, Operation::Construct] {
            check_form(&ops_of(&Case::Call { key }, op));
        }
    }
}

#[test]
fn the_hint_lists_follow_the_t0_fast_arms() {
    let b = |bits, op, imm| ops_of(&Case::Hint { bits }, Operation::Binary { op, imm });
    let i32_add = [
        Op::GuardType(Val::Lhs, Ty::Int32),
        Op::GuardType(Val::Rhs, Ty::Int32),
        Op::BinaryArith(BinOp::Add, Ty::Int32),
    ];
    // 0 = int32 or never ran (F-R9); INT32_OVERFLOW keeps int32 operands.
    assert_eq!(b(0, BinOp::Add, false).as_slice(), &i32_add);
    assert_eq!(b(0b10, BinOp::Add, false).as_slice(), &i32_add);
    assert_eq!(
        b(0b100, BinOp::Mul, true).as_slice(),
        &[
            Op::GuardType(Val::Lhs, Ty::Number),
            Op::BinaryArith(BinOp::Mul, Ty::Number)
        ]
    );
    // No T0 arm: `**`; `%` and the bitwise operators on doubles; strings.
    assert_eq!(b(0, BinOp::Exp, false).as_slice(), &[Op::Generic]);
    assert_eq!(b(0b100, BinOp::Mod, false).as_slice(), &[Op::Generic]);
    assert_eq!(b(0b100, BinOp::BitOr, false).as_slice(), &[Op::Generic]);
    assert_eq!(b(0b1_0000, BinOp::Add, false).as_slice(), &[Op::Generic]);
    let u = |bits, u| ops_of(&Case::Hint { bits }, Operation::Unary(u));
    assert_eq!(u(0b100, Unary::BitNot).as_slice(), &[Op::Generic]);
    assert_eq!(
        u(0b100, Unary::Negate).as_slice(),
        &[
            Op::GuardType(Val::Operand, Ty::Number),
            Op::UnaryArith(Unary::Negate, Ty::Number)
        ]
    );
    let c = |bits, c| ops_of(&Case::Hint { bits }, Operation::Compare(c));
    // `==` on doubles goes cold in T0; `===` and `<` have a number arm.
    assert_eq!(c(0b10, Compare::Eq).as_slice(), &[Op::Generic]);
    assert_eq!(
        c(0b10, Compare::StrictEq).action(),
        Op::CompareNumbers(Compare::StrictEq, Ty::Number)
    );
    assert_eq!(
        c(0, Compare::Eq).action(),
        Op::CompareNumbers(Compare::Eq, Ty::Int32)
    );
    // Atoms, strings, null: no list in v0.
    for bits in [1 << 3, 1 << 4, 1 << 5, 1 << 8 | 1 << 3] {
        assert_eq!(c(bits, Compare::StrictEq).as_slice(), &[Op::Generic]);
    }
}

// ---- named sites: the lists against the T0 hit path ----------------------------------------------

/// Compares the lists of every case of `f<i>`'s slot (a 0x21 or 0x22 site, name `name`) with
/// the T0 hit (`ic::load`) on each receiver of `probes`: a hit passes the guards and gives
/// the same value; a list answer where T0 misses is allowed only for the GuardKind widening
/// of `ARRAY_LENGTH` (H16), and must then be the generic answer (`want`).
fn check_loads(t: &mut T, i: usize, name: &str, probes: &[u64], want: &dyn Fn(&mut T, u64) -> u64) {
    let v = t.vector(i);
    let cs = cases(v, SLOT, Operation::LoadNamed);
    for case in &cs {
        let ops = ops_of(case, Operation::LoadNamed);
        check_form(&ops);
        for &o in probes {
            let hit = {
                let fi = info(t, i);
                assert_eq!(fi.constant(0), key(t.rt(), name));
                ic::load(t.rt(), &fi, o, SLOT, 0)
            };
            let e = Env {
                receiver: o,
                ..Env::default()
            };
            let got = eval(t.rt(), case, &ops, &e);
            // T0 tries every case of the entry; this list only its own.
            let mine = match *case {
                Case::Handler { key, .. } => {
                    (is_cell_bits(o) && shape_id(o) == key as u32)
                        || (key == KEY_STRING && kinds::is_string(o))
                }
                _ => false,
            };
            match (hit, got) {
                (Some(h), Out::Value(g)) if mine => assert!(same(h, g), "{ops:?}: {h:#x} {g:#x}"),
                (Some(_), Out::Miss) if mine => {
                    panic!("T0 hit, the list missed: {ops:?} on {o:#x}")
                }
                (None, Out::Value(g)) => {
                    assert_eq!(ops.action(), Op::LoadArrayLength, "{ops:?} on {o:#x}");
                    let w = want(t, o);
                    assert!(same(w, g), "{ops:?}: {w:#x} {g:#x}");
                }
                _ => {}
            }
        }
    }
}

/// `o.name` through the generic path.
fn get(t: &mut T, o: u64, name: &str) -> u64 {
    let k = key(t.rt(), name);
    property::get(&mut t.iso.cx, o, k).unwrap()
}

#[test]
fn named_load_lists_match_the_hit_path() {
    each(|c| {
        let sites = [
            Site::Get("x"),      // f0: own fields, poly, then mega
            Site::Get("x"),      // f1: prototype load, missing
            Site::Get("length"), // f2: array and string length
            Site::Get("charAt"), // f3: a string's method (KEY_STRING prototype case)
            Site::Get("x"),      // f4: a dictionary receiver (SLOW)
            Site::Get("x"),      // f5: out of line
        ];
        let mut t = T::new(c, &sites);
        let rt = t.rt();
        let op = object_prototype(rt);
        let o1 = object_with(rt, op, &[("x", int(1))]);
        let o2 = object_with(rt, op, &[("a", int(0)), ("x", int(2))]);
        let mut many: Vec<(String, u64)> = (0..12).map(|k| (format!("p{k}"), int(k))).collect();
        many.push(("x".into(), int(99)));
        let pairs: Vec<(&str, u64)> = many.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        let ool = object_with(rt, op, &pairs);
        let proto = object_with(rt, op, &[("x", int(7))]);
        let child = object_with(rt, proto, &[("y", int(0))]);
        let lone = object_with(rt, op, &[("z", int(0))]);
        let a = object::new_array(rt, 4);
        object::array_set(rt, a, 0, int(1));
        object::array_set(rt, a, 1, int(2));
        let a2 = object::new_array(rt, 4);
        object::array_set(rt, a2, 0, num(0.5));
        let s = string::new_string(rt, "abc");
        let dict = object_with(rt, op, &[("w", int(0)), ("x", int(3))]);
        let k = key(rt, "w");
        assert!(property::delete(&mut t.iso.cx, dict, k, false).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(dict)));
        let probes = [o1, o2, ool, child, lone, a, a2, s, dict, int(5), UNDEFINED];
        let want = |t: &mut T, o: u64| get(t, o, "length");
        // f0: mono, then poly.
        t.ok(0, o1, UNDEFINED);
        check_loads(&mut t, 0, "x", &probes, &want);
        t.ok(0, o2, UNDEFINED);
        check_loads(&mut t, 0, "x", &probes, &want);
        // f1: a prototype load, then a missing property.
        t.ok(1, child, UNDEFINED);
        t.ok(1, lone, UNDEFINED);
        check_loads(&mut t, 1, "x", &probes, &want);
        // A structural change of the holder: the validity cell goes, the guards miss.
        props::own_define(t.rt(), proto, key(t.rt(), "q"), int(0), attr::DEFAULT);
        check_loads(&mut t, 1, "x", &probes, &want);
        // f2: array and string lengths (and the GuardKind widening on `a2`).
        t.ok(2, a, UNDEFINED);
        t.ok(2, s, UNDEFINED);
        check_loads(&mut t, 2, "length", &probes, &want);
        // f3: a string's method load.
        t.ok(3, s, UNDEFINED);
        check_loads(&mut t, 3, "charAt", &probes, &want);
        // f4: SLOW.
        t.ok(4, dict, UNDEFINED);
        check_loads(&mut t, 4, "x", &probes, &want);
        // f5: out of line.
        t.ok(5, ool, UNDEFINED);
        check_loads(&mut t, 5, "x", &probes, &want);
        let lists: Vec<Op> = (0..6)
            .flat_map(|i| cases(t.vector(i), SLOT, Operation::LoadNamed))
            .map(|c| ops_of(&c, Operation::LoadNamed).action())
            .collect();
        for a in [
            Op::LoadUndefined,
            Op::LoadArrayLength,
            Op::LoadStringLength,
            Op::Generic,
        ] {
            assert!(lists.contains(&a), "{a:?} not in {lists:?}");
        }
        assert!(lists.iter().any(|a| matches!(a, Op::LoadHolderField(..))));
        assert!(
            lists
                .iter()
                .any(|a| matches!(a, Op::LoadField(_, l) if !l.in_object()))
        );
        // Mega: 5 more shapes.
        for n in 0..5 {
            let o = object_with(t.rt(), op, &[(&format!("m{n}"), int(0)), ("x", int(n))]);
            t.ok(0, o, UNDEFINED);
        }
        assert_eq!(
            cases(t.vector(0), SLOT, Operation::LoadNamed),
            vec![Case::Mega]
        );
        assert_eq!(
            ops_of(&Case::Mega, Operation::LoadNamed).as_slice(),
            &[Op::Generic]
        );
    });
}

/// Compares the lists of `f<i>`'s slot (a 0x24 or 0x25 site) with the T0 store hit on fresh
/// receivers from `make` and the values `vals`: the guards pass and the action stores exactly
/// where `ic::store` stores.
fn check_stores(t: &mut T, i: usize, op: Operation, make: &dyn Fn(&T) -> Vec<u64>, vals: &[u64]) {
    let v = t.vector(i);
    let cs = cases(v, SLOT, op);
    for case in &cs {
        let ops = ops_of(case, op);
        check_form(&ops);
        let Case::Handler { key: k, .. } = *case else {
            continue;
        };
        for &x in vals {
            for o in make(t) {
                if !(is_cell_bits(o) && shape_id(o) == k as u32) {
                    continue;
                }
                let e = Env {
                    receiver: o,
                    value: x,
                    ..Env::default()
                };
                let got = eval(t.rt(), case, &ops, &e);
                let hit = {
                    let fi = info(t, i);
                    if op == Operation::StoreNamed {
                        ic::store::<true>(t.rt(), &fi, o, SLOT, 0, x)
                    } else {
                        ic::store::<false>(t.rt(), &fi, o, SLOT, 0, x)
                    }
                };
                assert_eq!(
                    hit,
                    got == Out::Stored,
                    "{ops:?} on {o:#x} = {x:#x}: {got:?}"
                );
            }
        }
    }
}

#[test]
fn named_store_lists_match_the_hit_path() {
    each(|c| {
        let sites = [
            Site::Set("x", false), // f0: an int32 field (checked), then a double one
            Site::Set("x", false), // f1: add in object
            Site::Define("x"),     // f2: add with no chain check
            Site::Set("x", false), // f3: add out of line
            Site::Set("x", false), // f4: a read-only field (SLOW)
        ];
        let mut t = T::new(c, &sites);
        let vals = [int(4), num(2.5), UNDEFINED, NULL];
        let one = |t: &T| -> Vec<u64> {
            let rt = t.rt();
            vec![object_with(rt, object_prototype(rt), &[("x", int(1))])]
        };
        let o = one(&t)[0];
        t.ok(0, o, int(2));
        check_stores(&mut t, 0, Operation::StoreNamed, &one, &vals);
        let o = one(&t)[0];
        t.ok(0, o, num(0.5));
        check_stores(&mut t, 0, Operation::StoreNamed, &one, &vals);
        let empty = |t: &T| -> Vec<u64> { vec![object::new_plain_object(t.rt())] };
        let o = empty(&t)[0];
        t.ok(1, o, int(1));
        check_stores(&mut t, 1, Operation::StoreNamed, &empty, &vals);
        let o = empty(&t)[0];
        t.ok(2, o, int(1));
        check_stores(&mut t, 2, Operation::DefineNamed, &empty, &vals);
        let wide = |t: &T| -> Vec<u64> {
            let rt = t.rt();
            let pairs: Vec<(String, u64)> = (0..12).map(|k| (format!("p{k}"), int(k))).collect();
            let pairs: Vec<(&str, u64)> = pairs.iter().map(|(n, v)| (n.as_str(), *v)).collect();
            vec![object_with(rt, object_prototype(rt), &pairs)]
        };
        let o = wide(&t)[0];
        t.ok(3, o, int(1));
        check_stores(&mut t, 3, Operation::StoreNamed, &wide, &vals);
        let ro = |t: &T| -> Vec<u64> {
            let rt = t.rt();
            let o = object::new_plain_object(rt);
            props::own_define(rt, o, key(rt, "x"), int(1), attr::DEFAULT & !attr::WRITABLE);
            vec![o]
        };
        let o = ro(&t)[0];
        t.ok(4, o, int(1));
        check_stores(&mut t, 4, Operation::StoreNamed, &ro, &vals);
        let actions: Vec<Op> = (0..5)
            .flat_map(|i| {
                let op = if i == 2 {
                    Operation::DefineNamed
                } else {
                    Operation::StoreNamed
                };
                cases(t.vector(i), SLOT, op)
                    .into_iter()
                    .map(move |c| ops_of(&c, op))
            })
            .flat_map(|l| l.as_slice().to_vec())
            .collect();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Op::GuardType(Val::Value, _)))
        );
        for want in [
            Op::GuardProtoValidity(Proto::OfShape(Ref::CaseKey)),
            Op::Generic,
        ] {
            assert!(actions.contains(&want), "{want:?} not in {actions:?}");
        }
        assert!(actions.iter().any(|a| matches!(a, Op::GuardSlotsRoom(..))));
        assert!(actions.iter().any(|a| matches!(a, Op::AddField(..))));
    });
}

#[test]
fn global_lists_match_the_hit_path() {
    each(|c| {
        let sites = [
            Site::Global("icG"),
            Site::StoreGlobal("icG"),
            Site::Global("icL"),
        ];
        let mut t = T::new(c, &sites);
        let g = t.rt().global();
        props::own_define(t.rt(), g, key(t.rt(), "icG"), int(5), attr::DEFAULT);
        let lex = t.rt().root(Root::Lexical);
        props::own_define(t.rt(), lex, key(t.rt(), "icL"), int(6), attr::DEFAULT);
        assert_eq!(t.ok(0, UNDEFINED, UNDEFINED), int(5));
        t.ok(1, int(8), UNDEFINED);
        assert_eq!(t.ok(2, UNDEFINED, UNDEFINED), int(6));
        for (i, op) in [
            (0, Operation::LoadGlobal),
            (1, Operation::StoreGlobal),
            (2, Operation::LoadGlobal),
        ] {
            let cs = cases(t.vector(i), SLOT, op);
            assert_eq!(cs.len(), 1);
            let ops = ops_of(&cs[0], op);
            check_form(&ops);
            let check = |t: &T| {
                let got = eval(t.rt(), &cs[0], &ops, &Env::default());
                let fi = info(t, i);
                if op == Operation::StoreGlobal {
                    let hit = ic::store_global(&t.iso.cx, &fi, SLOT, int(9));
                    assert_eq!(hit, got == Out::Stored, "{ops:?}");
                } else {
                    let hit = ic::load_global(&t.iso.cx, &fi, SLOT);
                    match (hit, got) {
                        (Some(h), Out::Value(v)) => assert!(same(h, v)),
                        (None, Out::Miss) => {}
                        x => panic!("{ops:?}: {x:?}"),
                    }
                }
            };
            check(&t);
            if i == 2 {
                // The TDZ hole: GuardNotHole fails where T0 misses.
                let Op::LoadField(_, loc) = ops.action() else {
                    panic!("{ops:?}")
                };
                let old = field(lex, loc);
                if loc.in_object() {
                    init(cell(lex), loc.offset(), HOLE);
                }
                check(&t);
                if loc.in_object() {
                    init(cell(lex), loc.offset(), old);
                }
            }
        }
        // A new `let` of the same name shadows the global: the lexical shape changes.
        props::own_define(t.rt(), lex, key(t.rt(), "icG"), int(1), attr::DEFAULT);
        let cs = cases(t.vector(0), SLOT, Operation::LoadGlobal);
        let ops = ops_of(&cs[0], Operation::LoadGlobal);
        assert!(matches!(ops.action(), Op::LoadGlobalField(_)), "{ops:?}");
        assert_eq!(eval(t.rt(), &cs[0], &ops, &Env::default()), Out::Miss);
        let fi = info(&t, 0);
        assert_eq!(ic::load_global(&t.iso.cx, &fi, SLOT), None);
    });
}

// ---- a test unit of functions `f<i>` -------------------------------------------------------------

/// The script defines the global `f<i>` for each of `fns` (function `i + 1`), then the
/// functions of `hidden` (inner functions of the last of `fns`, no global), then each of
/// `extra` as the global `g<j>`. Function ids are in pre-order, as the verifier wants.
fn program(
    fns: Vec<FunctionCode>,
    hidden: Vec<FunctionCode>,
    extra: Vec<FunctionCode>,
) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let n = fns.len();
    for i in 0..n {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(&format!("f{i}"));
    }
    for j in 0..extra.len() {
        let k =
            s.b.function_constant(FunctionId((n + hidden.len() + j) as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(&format!("g{j}"));
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns);
    all.extend(hidden);
    all.extend(extra);
    unit("", all)
}

fn new_t(
    c: IsolateConfig,
    fns: Vec<FunctionCode>,
    hidden: Vec<FunctionCode>,
    extra: Vec<FunctionCode>,
) -> T {
    // Type feedback `full`: the BinaryOp, CompareOp and Call records (MODES-D11).
    let mut iso = super::profile::isolate(c);
    iso.with_cx_checkpoint(|cx| {
        cx.run_script(program(fns, hidden, extra), "ic_ops.js")
            .map(|_| ())
    })
    .unwrap();
    T { iso }
}

/// `f<i>(args..)`.
fn call(t: &mut T, name: &str, args: &[u64]) -> u64 {
    let f = t.iso.global_raw(name).unwrap();
    let args: Vec<Value> = args.iter().map(|&a| val(a)).collect();
    t.iso
        .call_raw(f, Value::UNDEFINED, &args)
        .unwrap_or_else(|e| panic!("{name} threw {}", e.message))
        .to_bits()
}

fn global(t: &T, name: &str) -> u64 {
    t.iso.global_raw(name).unwrap().to_bits()
}

/// A function `name(a, b, c)` with one slot of kind `k` and the body `body(b, a, b, c, slot)`.
fn site(
    name: &str,
    k: FeedbackSlotKind,
    body: impl FnOnce(
        &mut FunctionBuilder,
        ep9_bytecode::Register,
        ep9_bytecode::Register,
        ep9_bytecode::Register,
        ep9_bytecode::FeedbackSlot,
    ),
) -> FunctionCode {
    let mut b = FunctionBuilder::new(meta(name, 3, 0, 0, 0));
    let (p0, p1, p2) = (b.parameter(0), b.parameter(1), b.parameter(2));
    let s = b.feedback_slot(k);
    assert_eq!(s.index(), SLOT as usize);
    body(&mut b, p0, p1, p2, s);
    b.return_();
    b.finish().unwrap()
}

fn empty_fn(name: &str, parent: u32) -> FunctionCode {
    let mut b = FunctionBuilder::new(meta(name, 1, parent, 0, 0));
    b.ldar(b.parameter(0)).return_();
    b.finish().unwrap()
}

fn eager(c: IsolateConfig) -> IsolateConfig {
    IsolateConfig {
        feedback_threshold: 0,
        ..c
    }
}

// ---- keyed sites ---------------------------------------------------------------------------------

#[test]
fn keyed_lists_match_the_array_paths() {
    each(|c| {
        let fns = vec![
            site("f0", FeedbackSlotKind::LoadKeyed, |b, a, k, _, s| {
                b.ldar(k).get_keyed_property(a, s);
            }),
            site("f1", FeedbackSlotKind::StoreKeyed, |b, a, k, v, s| {
                b.ldar(v).set_keyed_property(a, k, s).ldar(v);
            }),
            site("f2", FeedbackSlotKind::LoadKeyed, |b, a, k, _, s| {
                b.ldar(k).get_keyed_property(a, s);
            }),
        ];
        let mut t = new_t(eager(c), fns, vec![], vec![]);
        let rt = t.rt();
        let ints = object::new_array(rt, 4);
        for i in 0..3 {
            object::array_set(rt, ints, i, int(i as i32 + 10));
        }
        let dbls = object::new_array(rt, 4);
        object::array_set(rt, dbls, 0, num(0.5));
        object::array_set(rt, dbls, 1, num(1.5));
        let holey = object::new_array(rt, 4);
        object::array_set(rt, holey, 2, int(1));
        let plain = object_with(rt, object_prototype(rt), &[("0", int(1))]);
        call(&mut t, "f0", &[ints, int(0)]);
        call(&mut t, "f0", &[dbls, int(1)]);
        let recvs = [ints, dbls, holey, plain, int(3)];
        let keys = [int(0), int(1), int(2), int(5), int(-1), num(1.5), UNDEFINED];
        let cs = cases(t.vector(0), SLOT, Operation::LoadKeyed);
        let ops = ops_of(&cs[0], Operation::LoadKeyed);
        check_form(&ops);
        assert!(matches!(ops.action(), Op::LoadElement(Kinds(_))), "{ops:?}");
        for &o in &recvs {
            for &k in &keys {
                let e = Env {
                    receiver: o,
                    key: k,
                    ..Env::default()
                };
                match eval(t.rt(), &cs[0], &ops, &e) {
                    Out::Value(v) => {
                        let w = call(&mut t, "f2", &[o, k]);
                        assert!(same(v, w), "{o:#x}[{k:#x}]: {v:#x} {w:#x}");
                    }
                    Out::Miss => {}
                    x => panic!("{x:?}"),
                }
            }
        }
        // What warmed the site passes.
        for (o, k) in [(ints, int(0)), (dbls, int(1))] {
            let e = Env {
                receiver: o,
                key: k,
                ..Env::default()
            };
            assert!(matches!(eval(t.rt(), &cs[0], &ops, &e), Out::Value(_)));
        }
        // StoreKeyed: new (a cold store keeps it new) -> monomorphic (the shape) ->
        // megamorphic (the hole).
        call(&mut t, "f1", &[plain, int(0), int(1)]);
        assert_eq!(
            ops_of(
                &cases(t.vector(1), SLOT, Operation::StoreKeyed)[0],
                Operation::StoreKeyed
            )
            .as_slice(),
            &[Op::Generic]
        );
        call(&mut t, "f1", &[ints, int(0), int(7)]);
        let mono = cases(t.vector(1), SLOT, Operation::StoreKeyed)[0];
        let ops = ops_of(&mono, Operation::StoreKeyed);
        check_form(&ops);
        assert_eq!(
            ops.as_slice()[0],
            Op::GuardShape(Val::Receiver, Ref::CaseKey)
        );
        let store = |t: &T, case: &Case, ops: &Ops, o, k, v| {
            let e = Env {
                receiver: o,
                key: k,
                value: v,
                ..Env::default()
            };
            eval(t.rt(), case, ops, &e)
        };
        assert_eq!(store(&t, &mono, &ops, ints, int(1), int(3)), Out::Stored);
        // A double does not fit an INT32 kind: T0 goes cold (a kind transition).
        assert_eq!(store(&t, &mono, &ops, ints, int(1), num(0.5)), Out::Miss);
        assert_eq!(store(&t, &mono, &ops, dbls, int(0), num(2.5)), Out::Miss);
        assert_eq!(store(&t, &mono, &ops, ints, int(3), int(3)), Out::Miss);
        call(&mut t, "f1", &[dbls, int(0), num(4.5)]);
        let mega = cases(t.vector(1), SLOT, Operation::StoreKeyed)[0];
        assert!(matches!(mega, Case::Keyed { key: HOLE, .. }), "{mega:?}");
        let ops = ops_of(&mega, Operation::StoreKeyed);
        check_form(&ops);
        assert_eq!(store(&t, &mega, &ops, dbls, int(1), num(9.5)), Out::Stored);
        assert_eq!(store(&t, &mega, &ops, dbls, int(1), UNDEFINED), Out::Miss);
        assert_eq!(store(&t, &mega, &ops, plain, int(0), int(1)), Out::Miss);
    });
}

// ---- InstanceOf ----------------------------------------------------------------------------------

#[test]
fn instance_of_lists_match_the_handler() {
    each(|c| {
        let fns = vec![
            site("f0", FeedbackSlotKind::InstanceOf, |b, a, f, _, s| {
                b.ldar(f).test_instance_of(a, s);
            }),
            site("f1", FeedbackSlotKind::InstanceOf, |b, a, f, _, s| {
                b.ldar(f).test_instance_of(a, s);
            }),
        ];
        let extra = vec![empty_fn("g0", 0), empty_fn("g1", 0)];
        let mut t = new_t(eager(c), fns, vec![], extra);
        let (c0, c1) = (global(&t, "g0"), global(&t, "g1"));
        let p0 = {
            let k = t.rt().atom(Atom::Prototype);
            property::get(&mut t.iso.cx, c0, k).unwrap()
        };
        let inst = object::new_object(t.rt(), p0);
        let other = object::new_plain_object(t.rt());
        assert_eq!(call(&mut t, "f0", &[inst, c0]), TRUE);
        let cs = cases(t.vector(0), SLOT, Operation::InstanceOf);
        assert_eq!(cs.len(), 1);
        let ops = ops_of(&cs[0], Operation::InstanceOf);
        check_form(&ops);
        assert!(
            matches!(ops.action(), Op::OrdinaryHasInstance(_)),
            "{ops:?}"
        );
        let check = |t: &mut T, want_pass: &[(u64, u64)]| {
            for o in [inst, other, int(1), UNDEFINED] {
                for f in [c0, c1] {
                    let e = Env {
                        lhs: o,
                        function: f,
                        ..Env::default()
                    };
                    match eval(t.rt(), &cs[0], &ops, &e) {
                        Out::Value(v) => assert_eq!(v, call(t, "f1", &[o, f])),
                        Out::Miss => assert!(!want_pass.contains(&(o, f))),
                        x => panic!("{x:?}"),
                    }
                }
            }
        };
        check(&mut t, &[(inst, c0), (other, c0), (int(1), c0)]);
        // A new `prototype` value is read at each hit (a value store, no shape change).
        let k = t.rt().atom(Atom::Prototype);
        let fresh = object::new_plain_object(t.rt());
        property::set(&mut t.iso.cx, c0, k, fresh, true).unwrap();
        check(&mut t, &[(inst, c0)]);
        assert_eq!(call(&mut t, "f1", &[inst, c0]), FALSE);
    });
}

// ---- Call ----------------------------------------------------------------------------------------

#[test]
fn call_lists_guard_the_target_function() {
    each(|c| {
        // f0(a, b) = a(b); f1() = a new closure of g0's inner function (id 4).
        let f0 = site("f0", FeedbackSlotKind::Call, |b, a, x, _, s| {
            b.call_undefined_receiver1(a, x, s);
        });
        let mut f1 = FunctionBuilder::new(meta("f1", 0, 0, 0, 0));
        let cs = f1.feedback_slot(FeedbackSlotKind::CreateClosure);
        let k = f1.function_constant(FunctionId(3));
        f1.create_closure(k, cs, 0).return_();
        let f1 = f1.finish().unwrap();
        let inner = empty_fn("inner", 2);
        let mut t = new_t(eager(c), vec![f0, f1], vec![inner], vec![empty_fn("g0", 0)]);
        let (a, b) = (call(&mut t, "f1", &[]), call(&mut t, "f1", &[]));
        assert_ne!(a, b);
        let g0 = global(&t, "g0");
        for _ in 0..3 {
            assert_eq!(call(&mut t, "f0", &[a, int(4)]), int(4));
        }
        let cs = cases(t.vector(0), SLOT, Operation::Call);
        assert_eq!(cs.len(), 1);
        let Case::Call { key } = cs[0] else {
            panic!("{cs:?}")
        };
        assert_eq!(key, ExternalRef::new(object::function_info(a)).to_word());
        let ops = ops_of(&cs[0], Operation::Call);
        assert_eq!(
            ops.as_slice(),
            &[
                Op::GuardTarget(Val::Callee, Ref::CaseKey),
                Op::CallTarget(Ref::CaseKey)
            ]
        );
        let host = super::ic::method(t.rt(), "m");
        let probe = |f| {
            let e = Env {
                callee: f,
                ..Env::default()
            };
            eval(t.rt(), &cs[0], &ops, &e)
        };
        // Any closure of the target function passes (a FunctionRef names the function).
        assert_eq!(probe(a), Out::Call);
        assert_eq!(probe(b), Out::Call);
        for f in [
            g0,
            host,
            int(1),
            UNDEFINED,
            object::new_plain_object(t.rt()),
        ] {
            assert_eq!(probe(f), Out::Miss);
        }
        // A second target: megamorphic, no list.
        call(&mut t, "f0", &[g0, int(4)]);
        let cs = cases(t.vector(0), SLOT, Operation::Call);
        assert_eq!(cs, vec![Case::Call { key: HOLE }]);
        assert_eq!(ops_of(&cs[0], Operation::Call).as_slice(), &[Op::Generic]);
        assert_eq!(
            ops_of(&Case::Call { key: UNDEFINED }, Operation::Call).as_slice(),
            &[Op::Generic]
        );
    });
}

// ---- BinaryOp and CompareOp ----------------------------------------------------------------------

/// Operand classes that warm one site each.
const CLASSES: usize = 4;

/// The operands of class `c`: 0 int32s that fit, 1 int32s whose `+`, `-`, `*` overflow, 2
/// doubles (and an int32 with a double), 3 other values.
fn class_pairs(rt: &Runtime, c: usize) -> Vec<(u64, u64)> {
    match c {
        0 => vec![(int(3), int(4)), (int(-7), int(2)), (int(0), int(5))],
        1 => vec![(int(i32::MAX), int(2)), (int(i32::MIN), int(3))],
        2 => vec![(num(0.5), num(1.25)), (int(3), num(-2.5))],
        _ => vec![
            (string::new_string(rt, "a"), int(1)),
            (UNDEFINED, num(0.5)),
            (TRUE, int(1)),
        ],
    }
}

/// Every operand of the soundness grid.
fn grid(rt: &Runtime) -> Vec<u64> {
    vec![
        int(0),
        int(1),
        int(-1),
        int(7),
        int(-7),
        int(31),
        int(32),
        int(i32::MAX),
        int(i32::MIN),
        num(0.5),
        num(-0.0),
        num(-2.5),
        num(1e300),
        num(f64::NAN),
        num(f64::INFINITY),
        UNDEFINED,
        NULL,
        TRUE,
        string::new_string(rt, "12"),
        object::new_plain_object(rt),
    ]
}

/// The opcodes of a binary, unary or compare site and their operations.
fn arith_sites() -> Vec<(&'static str, Operation)> {
    let mut v = Vec::new();
    for o in OPCODES.iter() {
        if let Some(op @ (Operation::Binary { .. } | Operation::Unary(_) | Operation::Compare(_))) =
            Operation::of(o.opcode)
        {
            v.push((o.name, op));
        }
    }
    v
}

/// A site function for `op` (named `name`): `a OP b`, `a OP 3`, `OP a`, `a CMP b`.
fn arith_fn(name: &str, op: Operation, opcode: &str) -> FunctionCode {
    let kind = match op {
        Operation::Compare(_) => FeedbackSlotKind::CompareOp,
        _ => FeedbackSlotKind::BinaryOp,
    };
    let opcode = opcode.to_string();
    site(name, kind, move |b, a, x, _, s| {
        use ep9_bytecode::Opcode as O;
        let code = OPCODES.iter().find(|o| o.name == opcode).unwrap().opcode;
        match op {
            Operation::Binary { imm: true, .. } | Operation::Unary(_) => {
                b.ldar(a);
            }
            _ => {
                b.ldar(x);
            }
        }
        match code {
            O::Add => b.add(a, s),
            O::Sub => b.sub(a, s),
            O::Mul => b.mul(a, s),
            O::Div => b.div(a, s),
            O::Mod => b.r#mod(a, s),
            O::Exp => b.exp(a, s),
            O::BitwiseOr => b.bitwise_or(a, s),
            O::BitwiseXor => b.bitwise_xor(a, s),
            O::BitwiseAnd => b.bitwise_and(a, s),
            O::ShiftLeft => b.shift_left(a, s),
            O::ShiftRight => b.shift_right(a, s),
            O::ShiftRightLogical => b.shift_right_logical(a, s),
            O::AddSmi => b.add_smi(3, s),
            O::SubSmi => b.sub_smi(3, s),
            O::MulSmi => b.mul_smi(3, s),
            O::DivSmi => b.div_smi(3, s),
            O::ModSmi => b.mod_smi(3, s),
            O::ExpSmi => b.exp_smi(3, s),
            O::BitwiseOrSmi => b.bitwise_or_smi(3, s),
            O::BitwiseXorSmi => b.bitwise_xor_smi(3, s),
            O::BitwiseAndSmi => b.bitwise_and_smi(3, s),
            O::ShiftLeftSmi => b.shift_left_smi(3, s),
            O::ShiftRightSmi => b.shift_right_smi(3, s),
            O::ShiftRightLogicalSmi => b.shift_right_logical_smi(3, s),
            O::Inc => b.inc(s),
            O::Dec => b.dec(s),
            O::Negate => b.negate(s),
            O::BitwiseNot => b.bitwise_not(s),
            O::ToNumber => b.to_number(s),
            O::ToNumeric => b.to_numeric(s),
            O::TestEqual => b.test_equal(a, s),
            O::TestEqualStrict => b.test_equal_strict(a, s),
            O::TestLessThan => b.test_less_than(a, s),
            O::TestGreaterThan => b.test_greater_than(a, s),
            O::TestLessThanOrEqual => b.test_less_than_or_equal(a, s),
            O::TestGreaterThanOrEqual => b.test_greater_than_or_equal(a, s),
            o => panic!("{o:?}"),
        };
    })
}

#[test]
fn binary_unary_and_compare_lists_are_sound_and_cover_their_sites() {
    for &d in Dispatch::ALL {
        on_big_stack(move || {
            let sites = arith_sites();
            assert_eq!(sites.len(), 24 + 6 + 6);
            // Per site: one function per class, then the oracle (never read).
            let mut fns = Vec::new();
            for (n, &(name, op)) in sites.iter().enumerate() {
                for c in 0..=CLASSES {
                    fns.push(arith_fn(&format!("f{}", n * (CLASSES + 1) + c), op, name));
                }
            }
            let mut t = new_t(eager(config(d, false)), fns, vec![], vec![]);
            let g = grid(t.rt());
            let mut typed = 0;
            for (n, &(name, op)) in sites.iter().enumerate() {
                let oracle = format!("f{}", n * (CLASSES + 1) + CLASSES);
                let rhs = |r: u64| match op {
                    Operation::Binary { imm: true, .. } => int(3),
                    _ => r,
                };
                for c in 0..CLASSES {
                    let i = n * (CLASSES + 1) + c;
                    let pairs = class_pairs(t.rt(), c);
                    for &(l, r) in &pairs {
                        call(&mut t, &format!("f{i}"), &[l, r]);
                    }
                    let cs = cases(t.vector(i), SLOT, op);
                    let ops = ops_of(&cs[0], op);
                    check_form(&ops);
                    if ops.action() != Op::Generic {
                        typed += 1;
                    }
                    // Soundness on the grid.
                    for &l in &g {
                        for &r in &g {
                            let e = Env {
                                lhs: l,
                                rhs: rhs(r),
                                ..Env::default()
                            };
                            if let Out::Value(v) = eval(t.rt(), &cs[0], &ops, &e) {
                                let w = call(&mut t, &oracle, &[l, r]);
                                assert!(
                                    same(v, w),
                                    "{name} class {c} {ops:?}: {l:#x}, {r:#x}: {v:#x} != {w:#x}"
                                );
                            }
                        }
                    }
                    // Coverage: the numeric classes pass the guards of a typed list.
                    if c < 3 && ops.action() != Op::Generic {
                        for &(l, r) in &pairs {
                            let e = Env {
                                lhs: l,
                                rhs: rhs(r),
                                ..Env::default()
                            };
                            let got = eval(t.rt(), &cs[0], &ops, &e);
                            let mod_miss = matches!(op, Operation::Binary { op: BinOp::Mod, .. })
                                && (int32_of(l) < 0 || int32_of(rhs(r)) == 0);
                            assert!(
                                matches!(got, Out::Value(_)) || mod_miss,
                                "{name} class {c} {ops:?}: {l:#x}, {r:#x}: {got:?}"
                            );
                        }
                    }
                    // The other values never get a typed list.
                    if c == 3 {
                        assert_eq!(ops.as_slice(), &[Op::Generic], "{name}");
                    }
                }
            }
            assert!(typed > 40, "{typed}");
        });
    }
}

// ---- IC_HANDLER cells -----------------------------------------------------------------------------

#[test]
fn handler_parts_reads_each_handler_kind_as_ic_does() {
    each(|c| {
        let sites = [
            Site::Get("x"),
            Site::Get("x"),
            Site::Set("x", false),
            Site::Global("icG"),
        ];
        let mut t = T::new(c, &sites);
        let rt = t.rt();
        let op = object_prototype(rt);
        let proto = object_with(rt, op, &[("x", int(7))]);
        let child = object_with(rt, proto, &[("y", int(0))]);
        let lone = object_with(rt, op, &[("z", int(0))]);
        let empty = object::new_plain_object(rt);
        let g = rt.global();
        props::own_define(rt, g, key(rt, "icG"), int(5), attr::DEFAULT);
        t.ok(0, child, UNDEFINED);
        t.ok(1, lone, UNDEFINED);
        t.ok(2, empty, int(1));
        t.ok(3, UNDEFINED, UNDEFINED);
        let mut kinds_seen = Vec::new();
        for i in 0..4 {
            let op = [
                Operation::LoadNamed,
                Operation::LoadNamed,
                Operation::StoreNamed,
                Operation::LoadGlobal,
            ][i];
            for case in cases(t.vector(i), SLOT, op) {
                let Case::Handler { handler, .. } = case else {
                    panic!("{case:?}")
                };
                assert!(is_cell_bits(handler), "{i}: {case:?}");
                let (k, c16, r, l) = ic::describe_handler(handler);
                assert_eq!(handler_parts(handler), (k & 0xFF, c16, r, l));
                kinds_seen.push(k & 0xFF);
            }
        }
        kinds_seen.sort();
        assert_eq!(
            kinds_seen,
            vec![hk::PROTO_LOAD, hk::MISSING, hk::ADD, hk::GLOBAL]
        );
    });
}

/// A hand-made IC_HANDLER cell of kind `k` (the M6-P0 layout of handler kinds 9-11; ACCIC
/// writes them in M6-P1).
fn fake_handler(rt: &Runtime, k: i32, c16: u64, r: u64, loc: i32) -> u64 {
    let c = rt.alloc(kinds::IC_HANDLER, io::SIZE);
    init(c, io::KIND, int32_bits(k));
    init(c, io::CELL, c16);
    init(c, io::SHAPE, r);
    init(c, io::FIELD, int32_bits(loc));
    c.to_value().to_bits()
}

#[test]
fn accessor_and_dictionary_holder_handlers_have_their_lists() {
    on_big_stack(|| {
        let iso = isolate(config(Dispatch::ALL[0], false));
        let rt = &iso.rt;
        let o = object::new_plain_object(rt);
        let s = shape::shape_of(o);
        let loc = smi::FIELD_IN_OBJECT | (24 << smi::SHIFT);
        let own = fake_handler(rt, hk::GETTER, int32_bits(0), UNDEFINED, loc);
        let inherited = fake_handler(rt, hk::GETTER, s, o, loc);
        let setter = fake_handler(rt, hk::SETTER, int32_bits(0), UNDEFINED, loc);
        let dict = fake_handler(rt, hk::PROTO_DICT_LOAD, s, o, 0);
        let case = |h| Case::Handler { key: s, handler: h };
        let l = |h, op| ops_of(&case(h), op);
        let shape = Op::GuardShape(Val::Receiver, Ref::CaseKey);
        let chain = Op::GuardProtoValidity(Proto::OfShape(Ref::CaseKey));
        assert_eq!(
            l(own, Operation::LoadNamed).as_slice(),
            &[shape, Op::CallGetter(Holder::Receiver, Loc(loc))]
        );
        assert_eq!(
            l(inherited, Operation::LoadNamed).as_slice(),
            &[
                shape,
                chain,
                Op::CallGetter(Holder::Ref(Ref::HandlerRef), Loc(loc))
            ]
        );
        assert_eq!(
            l(setter, Operation::StoreNamed).as_slice(),
            &[shape, Op::CallSetter(Holder::Receiver, Loc(loc))]
        );
        // CreateDataProperty never calls a setter; a getter does not serve a store.
        assert_eq!(l(setter, Operation::DefineNamed).as_slice(), &[Op::Generic]);
        assert_eq!(l(own, Operation::StoreNamed).as_slice(), &[Op::Generic]);
        assert_eq!(
            l(dict, Operation::LoadNamed).as_slice(),
            &[shape, chain, Op::LoadHolderDict(Ref::HandlerRef)]
        );
        // A handler kind no list knows is the generic path. The probe is the top of the int32
        // kind range (K10), never the next free kind 12 (README §3.4 "Append-only parts").
        let unknown = fake_handler(rt, i32::MAX, UNDEFINED, UNDEFINED, 0);
        assert_eq!(l(unknown, Operation::LoadNamed).as_slice(), &[Op::Generic]);
    });
}

// ---- the T0 speed arms (M6-P1 CBC, V11-D20) ---------------------------------------------------------

/// `f0(a, b)` = `a === b`, `f1(a, b)` = `a + b`.
fn arms_t(c: IsolateConfig) -> T {
    let fns = vec![
        site("f0", FeedbackSlotKind::CompareOp, |b, a, x, _, s| {
            b.ldar(x).test_equal_strict(a, s);
        }),
        site("f1", FeedbackSlotKind::BinaryOp, |b, a, x, _, s| {
            b.ldar(x).add(a, s);
        }),
    ];
    new_t(eager(c), fns, vec![], vec![])
}

#[test]
fn strict_equal_of_two_distinct_atoms_is_false_and_records_atom() {
    each(|c| {
        let mut t = arms_t(c);
        let rt = t.rt();
        let (foo, bar) = (key(rt, "foo"), key(rt, "bar"));
        let foo2 = string::new_string(rt, "foo");
        let long = key(rt, "a key text longer than the cons length");
        assert!(string::is_atom(foo) && string::is_atom(bar) && string::is_atom(long));
        assert!(!string::is_atom(foo2) && foo2 != foo);
        // Two atoms: the fast body answers; the text decides.
        assert_eq!(call(&mut t, "f0", &[foo, bar]), FALSE);
        assert_eq!(call(&mut t, "f0", &[foo, long]), FALSE);
        assert_eq!(call(&mut t, "f0", &[foo, foo]), TRUE);
        let bits = |t: &T| int32_of(feedback::value(t.vector(0), SLOT)) as u32;
        let atom = u32::from(ep9_facts::CompareHint::ATOM.bits());
        assert_eq!(bits(&t), atom);
        // An atom and a string of the same text that is not one: the cold body.
        assert_eq!(call(&mut t, "f0", &[foo, foo2]), TRUE);
        assert_eq!(call(&mut t, "f0", &[foo2, bar]), FALSE);
        let s = u32::from(ep9_facts::CompareHint::STRING.bits());
        assert_eq!(bits(&t), atom | s);
        // Other values keep their arms.
        assert_eq!(call(&mut t, "f0", &[foo, int(1)]), FALSE);
        assert_eq!(call(&mut t, "f0", &[num(f64::NAN), num(f64::NAN)]), FALSE);
    });
}

#[test]
fn a_string_add_concats_and_a_too_long_one_still_throws() {
    each(|c| {
        let mut t = arms_t(c);
        let rt = t.rt();
        let (ab, cd) = (string::new_string(rt, "ab"), key(rt, "cd"));
        let empty = string::new_string(rt, "");
        let text = |v| string::to_rust_string(v);
        assert_eq!(text(call(&mut t, "f1", &[ab, cd])), "abcd");
        assert_eq!(call(&mut t, "f1", &[ab, empty]), ab);
        assert_eq!(call(&mut t, "f1", &[empty, cd]), cd);
        let long = string::new_string(t.rt(), "0123456789abcdef");
        let r = call(&mut t, "f1", &[long, long]);
        assert_eq!(text(r), "0123456789abcdef0123456789abcdef");
        // A string and another value: the generic path (ToString of the number).
        assert_eq!(text(call(&mut t, "f1", &[ab, int(1)])), "ab1");
        assert_eq!(text(call(&mut t, "f1", &[int(1), cd])), "1cd");
        let bits = int32_of(feedback::value(t.vector(1), SLOT)) as u32;
        let hint =
            u32::from(ep9_facts::BinaryHint::STRING.bits() | ep9_facts::BinaryHint::NUMBER.bits());
        assert_eq!(bits, hint);
        // Past the length limit: the RangeError of the generic path.
        let mut big = long;
        while string::str_len(big) * 2 <= string::MAX_STRING_LENGTH {
            big = string::concat(t.rt(), big, big).unwrap();
        }
        let f = t.iso.global_raw("f1").unwrap();
        let e = t
            .iso
            .call_raw(f, Value::UNDEFINED, &[val(big), val(big)])
            .unwrap_err();
        assert_eq!(
            e.constructor.as_deref(),
            Some("RangeError"),
            "{}",
            e.message
        );
    });
}

// ---- keyed view sites (TypedArray TA-2 TA-KEYED, design §6.3) ------------------------------------

/// The view lists (`ic_ops::view_keyed`): a keyed case whose kinds set has the bit of
/// TYPED_FIXED or TYPED_VARIABLE and whose key is a view's shape. T0 makes neither part: no T0
/// code records those bits before TA-3 (the kinds-set writer comes with the FactSnapshot
/// names 7 and 8, TA-ORCH-1), and a view store hit leaves the key half alone (`keyed_ic.rs`
/// module `view`; checked first: the T0 case of a site that a view warmed has an undefined
/// key). So each case here is made by hand, as a writer of the element type will make it.
/// Soundness: over views of each type and both bound forms (also after a shrink and a
/// detach), a view of another type, an Array, a plain object and a primitive, every key and
/// value probe whose guards pass gives what the site gives: a load its value, a store the
/// element that NumericToRawBytes of the value makes. Coverage: the probes that warmed the
/// site pass. Without a view bit, with an Array's shape or the hole as the key, the array
/// lists stay; a BigInt view gives `[Generic]`.
#[test]
fn keyed_view_lists_match_the_view_paths() {
    use crate::array_buffer as ab;
    use crate::elements::{kind_of, kinds_set_bit};
    use crate::profile::keyed;
    use crate::typed_array as ta;
    each(|c| {
        let fns = vec![
            site("f0", FeedbackSlotKind::LoadKeyed, |b, a, k, _, s| {
                b.ldar(k).get_keyed_property(a, s);
            }),
            site("f1", FeedbackSlotKind::StoreKeyed, |b, a, k, v, s| {
                b.ldar(v).set_keyed_property(a, k, s).ldar(v);
            }),
        ];
        let mut t = new_t(eager(c), fns, vec![], vec![]);
        let rt = t.rt();
        let arr = object::new_array(rt, 2);
        object::array_set(rt, arr, 0, int(1));
        let plain = object_with(rt, object_prototype(rt), &[("0", int(1))]);
        // Each type: a fixed view on a fixed-length buffer (TYPED_FIXED) and an auto view on a
        // resizable one (TYPED_VARIABLE). The handles root them (H10).
        let mut views = Vec::new();
        let mut held = t.keep_all(&[arr, plain]);
        for ty in 0..12u8 {
            let fb = ab::allocate(rt, NULL, 32, None).unwrap();
            let fixed = ta::new_view(rt, NULL, ty, fb, 0, Some(4)).unwrap();
            let rb = ab::allocate(rt, NULL, 32, Some(64)).unwrap();
            let auto = ta::new_view(rt, NULL, ty, rb, 0, None).unwrap();
            held.extend(t.keep_all(&[fb, fixed, rb, auto]));
            views.push((ty, fixed, fb));
            views.push((ty, auto, rb));
        }
        // The T0 case of a store site that a view warmed: no key (the hit reads no IC state).
        let v0 = views[0].1;
        call(&mut t, "f1", &[v0, int(0), int(1)]);
        assert_eq!(
            cases(t.vector(1), SLOT, Operation::StoreKeyed)[0],
            Case::Keyed {
                key: UNDEFINED,
                bits: keyed::INDEX | keyed::OBJECT,
            }
        );
        let c0 = Case::Keyed {
            key: shape::shape_of(v0),
            bits: keyed::INDEX | keyed::OBJECT,
        };
        // With no view bit the list is the array list (every view probe misses).
        let ops = ops_of(&c0, Operation::StoreKeyed);
        assert_eq!(
            ops.as_slice()[1],
            Op::GuardKind(Val::Receiver, Class::Array)
        );
        let keys = [
            int(0),
            int(1),
            int(3),
            int(4),
            int(9),
            int(-1),
            num(1.5),
            UNDEFINED,
        ];
        let vals = [
            int(7),
            int(300),
            int(-1),
            num(2.5),
            num(f64::NAN),
            num(-0.0),
            UNDEFINED,
        ];
        let (mut loads, mut stores) = (0, 0);
        for (n, &(ty, v, _)) in views.iter().enumerate() {
            let bits = keyed::INDEX | keyed::OBJECT | kinds_set_bit(kind_of(v));
            let case = Case::Keyed {
                key: shape::shape_of(v),
                bits,
            };
            let load = ops_of(&case, Operation::LoadKeyed);
            let store = ops_of(&case, Operation::StoreKeyed);
            check_form(&load);
            check_form(&store);
            if matches!(ty, 7 | 8) {
                assert_eq!(load.as_slice(), &[Op::Generic], "BigInt type {ty}");
                assert_eq!(store.as_slice(), &[Op::Generic], "BigInt type {ty}");
                continue;
            }
            let guard = Op::GuardKind(Val::Receiver, Class::View(ty));
            let not_shared = Op::GuardViewNotShared(Val::Receiver);
            let int_key = Op::GuardType(Val::Key, Ty::Int32);
            assert_eq!(
                load.as_slice(),
                &[guard, not_shared, int_key, Op::LoadViewOrUndefined(ty)]
            );
            assert_eq!(
                store.as_slice(),
                &[
                    guard,
                    not_shared,
                    int_key,
                    Op::GuardViewIndex(Val::Receiver, Val::Key),
                    Op::GuardType(Val::Value, Ty::Number),
                    Op::StoreView(ty),
                ]
            );
            // The warmer passes (coverage).
            let warm = Env {
                receiver: v,
                key: int(0),
                value: int(1),
                ..Env::default()
            };
            assert!(matches!(eval(t.rt(), &case, &load, &warm), Out::Value(_)));
            assert_eq!(eval(t.rt(), &case, &store, &warm), Out::Stored);
            // Soundness over the probes: the views of this type, one of the next type, an
            // Array, a plain object and an int32.
            let other = views[(n + 2) % views.len()].1;
            let recvs = [views[n & !1].1, views[n | 1].1, other, arr, plain, int(3)];
            for &o in &recvs {
                for &k in &keys {
                    let e = Env {
                        receiver: o,
                        key: k,
                        ..Env::default()
                    };
                    match eval(t.rt(), &case, &load, &e) {
                        Out::Value(x) => {
                            let w = call(&mut t, "f0", &[o, k]);
                            assert!(same(x, w), "type {ty} {o:#x}[{k:#x}]: {x:#x} {w:#x}");
                            loads += 1;
                        }
                        Out::Miss => {}
                        x => panic!("{x:?}"),
                    }
                    for &x in &vals {
                        let e = Env { value: x, ..e };
                        match eval(t.rt(), &case, &store, &e) {
                            Out::Stored => {
                                call(&mut t, "f1", &[o, k, x]);
                                let got = ta::get_element(t.rt(), o, f64::from(int32_of(k)));
                                let want = ab::raw_to_value(
                                    t.rt(),
                                    ty,
                                    ab::number_to_raw(ty, number(x).unwrap()),
                                );
                                assert!(same(got, want), "type {ty} {o:#x}[{k:#x}] = {x:#x}");
                                stores += 1;
                            }
                            Out::Miss => {}
                            r => panic!("{r:?}"),
                        }
                    }
                }
            }
        }
        assert!(loads > 0 && stores > 0);
        // The bound at the probe: a shrink, then a detach, of the Int16 auto view's buffer.
        let (_, auto, rb) = views[7];
        let case = Case::Keyed {
            key: shape::shape_of(auto),
            bits: kinds_set_bit(kind_of(auto)),
        };
        let (load, store) = (
            ops_of(&case, Operation::LoadKeyed),
            ops_of(&case, Operation::StoreKeyed),
        );
        let probe = |t: &mut T, i: i32| {
            let e = Env {
                receiver: auto,
                key: int(i),
                value: int(5),
                ..Env::default()
            };
            let w = call(t, "f0", &[auto, int(i)]);
            assert_eq!(eval(t.rt(), &case, &load, &e), Out::Value(w), "[{i}]");
            eval(t.rt(), &case, &store, &e)
        };
        assert_eq!(probe(&mut t, 3), Out::Stored);
        ab::resize(t.rt(), rb, 4).unwrap();
        assert_eq!(probe(&mut t, 1), Out::Stored);
        assert_eq!(probe(&mut t, 2), Out::Miss);
        ab::detach(t.rt(), rb, UNDEFINED).unwrap();
        assert_eq!(probe(&mut t, 0), Out::Miss);
        // No view list without a view's shape: an Array's shape, the hole, undefined.
        let bit = kinds_set_bit(crate::elements::kind::TYPED_FIXED);
        for key in [shape::shape_of(arr), HOLE, UNDEFINED] {
            let case = Case::Keyed { key, bits: bit };
            for op in [Operation::LoadKeyed, Operation::StoreKeyed] {
                let ops = ops_of(&case, op);
                assert!(
                    !ops.as_slice().iter().any(|o| matches!(
                        o,
                        Op::GuardKind(_, Class::View(_))
                            | Op::GuardViewIndex(..)
                            | Op::LoadViewOrUndefined(_)
                            | Op::StoreView(_)
                    )),
                    "{key:#x} {op:?}: {ops:?}"
                );
            }
        }
        t.release_all(held);
    });
}

/// The SAB guard of the view lists (TypedArray TA-3, V13-D8, TA-ORCH-17; route
/// TA-SHARED-MIXED-WIDTH, its SAB part): T0 takes no view case on a SharedArrayBuffer (its
/// bytes are read and written with atomics on the spec path, TA-ORCH-12, -13; `keyed_ic.rs`
/// module `view`), so both view lists hold `GuardViewNotShared(receiver)` right after the
/// GuardKind. For each Number type: views on a fixed-length SharedArrayBuffer and on a
/// growable one (a fixed and a length-tracking view) fail the guard and both lists miss, and
/// T0 does not hit there either (`keyed_ic::VIEW_HITS`), in bounds; views of the same type on a
/// fixed-length and on a resizable ArrayBuffer pass the guard, and T0 hits. A fixed view on a
/// fixed-length SharedArrayBuffer and one on a fixed-length ArrayBuffer have the same shape,
/// so the case key alone does not tell them apart.
#[test]
fn a_view_on_a_shared_buffer_fails_the_not_shared_guard() {
    use crate::array_buffer as ab;
    use crate::elements::{kind_of, kinds_set_bit};
    use crate::keyed_ic::VIEW_HITS;
    use crate::profile::keyed;
    use crate::typed_array as ta;
    each(|c| {
        // The T0 view hits since the last call (loads, stores); each runs on its own thread.
        let hits = || VIEW_HITS.with(|c| c.replace([0, 0]));
        let fns = vec![
            site("f0", FeedbackSlotKind::LoadKeyed, |b, a, k, _, s| {
                b.ldar(k).get_keyed_property(a, s);
            }),
            site("f1", FeedbackSlotKind::StoreKeyed, |b, a, k, v, s| {
                b.ldar(v).set_keyed_property(a, k, s).ldar(v);
            }),
        ];
        let mut t = new_t(eager(c), fns, vec![], vec![]);
        let mut held = Vec::new();
        for ty in [0u8, 1, 2, 3, 4, 5, 6, 9, 10, 11] {
            // (view, shared). The handles root the buffers and views (H10).
            let mut views = Vec::new();
            for (shared, max) in [
                (false, None),
                (false, Some(64)),
                (true, None),
                (true, Some(64)),
            ] {
                let b = if shared {
                    ab::allocate_shared(t.rt(), NULL, 32, max).unwrap()
                } else {
                    ab::allocate(t.rt(), NULL, 32, max).unwrap()
                };
                let fixed = ta::new_view(t.rt(), NULL, ty, b, 0, Some(2)).unwrap();
                let auto = ta::new_view(t.rt(), NULL, ty, b, 0, None).unwrap();
                held.extend(t.keep_all(&[b, fixed, auto]));
                views.push((fixed, shared));
                views.push((auto, shared));
            }
            // views[0]: fixed on a fixed ArrayBuffer; views[4]: fixed on a fixed SAB.
            assert_eq!(kind_of(views[0].0), kind_of(views[4].0));
            assert_eq!(shape::shape_of(views[0].0), shape::shape_of(views[4].0));
            for &(case_view, _) in &views {
                let case = Case::Keyed {
                    key: shape::shape_of(case_view),
                    bits: keyed::INDEX | keyed::OBJECT | kinds_set_bit(kind_of(case_view)),
                };
                let load = ops_of(&case, Operation::LoadKeyed);
                let store = ops_of(&case, Operation::StoreKeyed);
                check_form(&load);
                check_form(&store);
                assert_eq!(store.as_slice().len(), MAX_OPS, "type {ty}: {store:?}");
                for ops in [&load, &store] {
                    assert_eq!(
                        ops.as_slice()[..2],
                        [
                            Op::GuardKind(Val::Receiver, Class::View(ty)),
                            Op::GuardViewNotShared(Val::Receiver),
                        ]
                    );
                }
                for &(v, shared) in &views {
                    let e = Env {
                        receiver: v,
                        key: int(1),
                        value: int(1),
                        ..Env::default()
                    };
                    // The other guards pass for each probe here: the kind is the case's and
                    // the key is a valid integer index, so GuardViewNotShared decides.
                    assert_eq!(kinds::cell_kind(v), kinds::cell_kind(case_view));
                    assert!(ta::valid_index(t.rt(), v, 1.0).is_some());
                    let l = eval(t.rt(), &case, &load, &e);
                    let st = eval(t.rt(), &case, &store, &e);
                    hits();
                    let w = call(&mut t, "f0", &[v, int(1)]);
                    call(&mut t, "f1", &[v, int(1), int(1)]);
                    let h = hits();
                    if shared {
                        assert_eq!((l, st), (Out::Miss, Out::Miss), "type {ty}: shared");
                        assert_eq!(h, [0, 0], "type {ty}: T0 takes no shared view case");
                    } else {
                        assert!(matches!(l, Out::Value(x) if same(x, w)), "type {ty}: {l:?}");
                        assert_eq!(st, Out::Stored, "type {ty}");
                        // T0 hits on these probes, except a Float16 view (T0 takes none).
                        let want = u32::from(ty != 9);
                        assert_eq!(h, [want, want], "type {ty}: T0 hits");
                    }
                }
            }
        }
        t.release_all(held);
    });
}
