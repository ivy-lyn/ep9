//! The CacheIR-like op lists of the T0 inline caches (owner M6-P1 CBC; plan v11 V11-D20, the
//! M1 scope "CacheIR-like ICs for ... call/binary/compare", doc 09:86): [`ops_of`], one total
//! function from every T0 IC case (named, keyed, global, InstanceOf, Call, BinaryOp,
//! CompareOp) to the H16 op list (`docs/adr/H16-ics-and-feedback.md`, "CacheIR op lists"),
//! with a test per handler kind that the ops match the handler's checks (`tests/ic_ops.rs`).
//!
//! ```text
//!  site opcode --Operation::of--> Operation
//!  feedback slot --cases(v, i, op)--> [Case]          (one per case of the entry)
//!  (Case, Operation) --ops_of--> [guard.., action]    (a fixed list per handler kind and operation)
//!   e.g. FIELD_IN_OBJECT, load  -> GuardShape(receiver, key), LoadField(receiver, loc)
//!        BinaryOp `+`, int32    -> GuardType(lhs, int32), GuardType(rhs, int32), BinaryArith(+, int32)
//!        Call, one target       -> GuardTarget(callee, key), CallTarget(key)
//!        SLOW, MEGA, no case    -> Generic
//! ```
//!
//! Rules (H16 "CacheIR op lists"):
//! - T0 keeps its handlers and runs no list; a list is a reading of T0's handler and slot
//!   forms, so it adds no RW state and changes no IC_LAYOUT (V11-D20). The first readers
//!   outside the tests are later tiers (Spike C, T1, AOT); until then only the tests call it.
//! - Guards first, then exactly one action. Every guard reads its value; nothing is stored
//!   until the action. An action may still miss where T0's handler misses (the `%` of a
//!   negative int32, a value that does not fit an add's field); a miss is the generic path.
//! - Operands are names, not addresses: [`Val`] (a value of the site), [`Ref`] (the case key
//!   or the handler's reference at +24) and [`Loc`] (a smi field handler). The ref node or
//!   H34 relocation each name lowers to comes with the first compiled reader (H16).
//! - Sound: when every guard passes, the action gives what the generic operation gives (or
//!   misses). A list is never wider than this, though it may be wider than T0's own test
//!   (H16 keeps `ARRAY_LENGTH` as `GuardKind`, any array).
//! - Total: every (case, operation) pair has a list; a pair with no typed list (a slot kind
//!   that does not fit the operation, a handler kind a reader does not know, `SLOW`, `MEGA`,
//!   an uninitialized entry) gets `[Generic]` (H16 "Append-only": an unknown op or kind is
//!   a miss).
//! - Ops and their operand values are append-only lists (H16, doc 13 C-24). The v0 ops are
//!   H16's table; [`Op::LoadHolderDict`] is H16's append of M6-P1 ACCIC (holder ref, key: the
//!   key is the site's name); [`Op::BinaryArith`], [`Op::UnaryArith`],
//!   [`Op::CompareNumbers`], [`Op::OrdinaryHasInstance`] and the `Heap` type of [`Ty`] are
//!   appended here (M6-P1 CBC); [`Op::GuardViewIndex`], [`Op::LoadViewOrUndefined`],
//!   [`Op::StoreView`] and the class `View` of [`Class`] for the TypedArray views (TypedArray
//!   TA-2 TA-KEYED, design §6.3; `view_keyed`); [`Op::GuardViewNotShared`] (TypedArray TA-3,
//!   V13-D8: the view lists exclude a view on a SharedArrayBuffer).
//!
//! The IC_HANDLER cell is read through the `instance_ic::io` field offsets: every handler
//! kind shares that layout (+8 kind, +16, +24, +32; the `ic.rs` IC_LAYOUT 4 table). A test
//! checks the reading against `ic::describe_handler` for every handler kind it makes.
#![cfg_attr(not(test), allow(dead_code))]

use ep9_bytecode::Opcode;
use ep9_value::encoding::NUMBER_TAG;

use crate::conv::BinOp;
use crate::elements::{KINDS_SET_MASK, KINDS_SET_SHIFT, kind as ek};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_of};
use crate::ic::{self, KEY_STRING, hk, smi};
use crate::instance_ic::io;
use crate::kinds::is_cell_bits;
use crate::object::{cell, rd, rd_i32};

// ---- the operation of a site ------------------------------------------------------------------

/// A unary operator or numeric conversion on a BinaryOp slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unary {
    Inc,
    Dec,
    Negate,
    BitNot,
    ToNumber,
    ToNumeric,
}

/// A comparison on a CompareOp slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Compare {
    /// `==` (0x4b).
    Eq,
    /// `===` (0x4c).
    StrictEq,
    Lt,
    Gt,
    Le,
    Ge,
}

/// The operation of an IC site, from its opcode (H16: "the operation is the site's").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    /// 0x21 `GetNamedProperty`; 0x22 `GetNamedPropertyFromSuper`, whose receiver here is the
    /// lookup start (H16 K17).
    LoadNamed,
    /// 0x24 `SetNamedProperty`.
    StoreNamed,
    /// 0x25 `DefineNamedOwnProperty` (CreateDataProperty: no chain, no setter).
    DefineNamed,
    /// 0x10 `LdaGlobal`, 0x11 `LdaGlobalInsideTypeof`.
    LoadGlobal,
    /// 0x12 `StaGlobal`.
    StoreGlobal,
    /// 0x23 `GetKeyedProperty`.
    LoadKeyed,
    /// 0x26 `SetKeyedProperty`.
    StoreKeyed,
    /// 0x27 `DefineKeyedOwnProperty`, 0x29 `DefineKeyedOwnPropertyInLiteral`, 0x28
    /// `StaInArrayLiteral`, 0x53 `TestIn`: keyed sites whose T0 body is always cold.
    KeyedCold,
    /// 0x52 `TestInstanceOf`.
    InstanceOf,
    /// 0x5e-0x66: `CallAnyReceiver`, `CallProperty*`, `CallUndefinedReceiver*`.
    Call,
    /// 0x67 `CallWithSpread` (T0: always cold).
    CallSpread,
    /// 0x69 `Construct`, 0x6a `ConstructWithSpread` (the slot holds a construct-site case).
    Construct,
    /// The 12 binary operators (`imm`: false) and their 12 `*Smi` forms (`imm`: true, the
    /// right operand is the immediate).
    Binary { op: BinOp, imm: bool },
    /// `Inc`, `Dec`, `Negate`, `BitwiseNot`, `ToNumber`, `ToNumeric`.
    Unary(Unary),
    /// `TestEqual`, `TestEqualStrict`, `TestLessThan`, `TestGreaterThan`,
    /// `TestLessThanOrEqual`, `TestGreaterThanOrEqual`.
    Compare(Compare),
}

impl Operation {
    /// The operation of a site with opcode `op`, or `None` for an opcode with no IC case: no
    /// feedback slot, or a slot that holds facts only (TypeOf, Literal, CloneObject, ForIn,
    /// JumpLoop, CreateClosure), the empty load and call slots of `GetIterator` (H16 I34), and
    /// the reserved `TailCall`.
    pub(crate) fn of(op: Opcode) -> Option<Operation> {
        use Opcode as O;
        use Operation as P;
        let bin = |op, imm| Some(P::Binary { op, imm });
        match op {
            O::GetNamedProperty | O::GetNamedPropertyFromSuper => Some(P::LoadNamed),
            O::SetNamedProperty => Some(P::StoreNamed),
            O::DefineNamedOwnProperty => Some(P::DefineNamed),
            O::LdaGlobal | O::LdaGlobalInsideTypeof => Some(P::LoadGlobal),
            O::StaGlobal => Some(P::StoreGlobal),
            O::GetKeyedProperty => Some(P::LoadKeyed),
            O::SetKeyedProperty => Some(P::StoreKeyed),
            O::DefineKeyedOwnProperty
            | O::DefineKeyedOwnPropertyInLiteral
            | O::StaInArrayLiteral
            | O::TestIn => Some(P::KeyedCold),
            O::TestInstanceOf => Some(P::InstanceOf),
            O::CallAnyReceiver
            | O::CallProperty
            | O::CallProperty0
            | O::CallProperty1
            | O::CallProperty2
            | O::CallUndefinedReceiver
            | O::CallUndefinedReceiver0
            | O::CallUndefinedReceiver1
            | O::CallUndefinedReceiver2 => Some(P::Call),
            O::CallWithSpread => Some(P::CallSpread),
            O::Construct | O::ConstructWithSpread => Some(P::Construct),
            O::Add => bin(BinOp::Add, false),
            O::Sub => bin(BinOp::Sub, false),
            O::Mul => bin(BinOp::Mul, false),
            O::Div => bin(BinOp::Div, false),
            O::Mod => bin(BinOp::Mod, false),
            O::Exp => bin(BinOp::Exp, false),
            O::BitwiseOr => bin(BinOp::BitOr, false),
            O::BitwiseXor => bin(BinOp::BitXor, false),
            O::BitwiseAnd => bin(BinOp::BitAnd, false),
            O::ShiftLeft => bin(BinOp::Shl, false),
            O::ShiftRight => bin(BinOp::Sar, false),
            O::ShiftRightLogical => bin(BinOp::Shr, false),
            O::AddSmi => bin(BinOp::Add, true),
            O::SubSmi => bin(BinOp::Sub, true),
            O::MulSmi => bin(BinOp::Mul, true),
            O::DivSmi => bin(BinOp::Div, true),
            O::ModSmi => bin(BinOp::Mod, true),
            O::ExpSmi => bin(BinOp::Exp, true),
            O::BitwiseOrSmi => bin(BinOp::BitOr, true),
            O::BitwiseXorSmi => bin(BinOp::BitXor, true),
            O::BitwiseAndSmi => bin(BinOp::BitAnd, true),
            O::ShiftLeftSmi => bin(BinOp::Shl, true),
            O::ShiftRightSmi => bin(BinOp::Sar, true),
            O::ShiftRightLogicalSmi => bin(BinOp::Shr, true),
            O::Inc => Some(P::Unary(Unary::Inc)),
            O::Dec => Some(P::Unary(Unary::Dec)),
            O::Negate => Some(P::Unary(Unary::Negate)),
            O::BitwiseNot => Some(P::Unary(Unary::BitNot)),
            O::ToNumber => Some(P::Unary(Unary::ToNumber)),
            O::ToNumeric => Some(P::Unary(Unary::ToNumeric)),
            O::TestEqual => Some(P::Compare(Compare::Eq)),
            O::TestEqualStrict => Some(P::Compare(Compare::StrictEq)),
            O::TestLessThan => Some(P::Compare(Compare::Lt)),
            O::TestGreaterThan => Some(P::Compare(Compare::Gt)),
            O::TestLessThanOrEqual => Some(P::Compare(Compare::Le)),
            O::TestGreaterThanOrEqual => Some(P::Compare(Compare::Ge)),
            _ => None,
        }
    }
}

// ---- cases: what a reader sees in a feedback slot ---------------------------------------------

/// One T0 IC case of a feedback slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    /// No case: an uninitialized entry (a reader takes the generic path).
    Empty,
    /// A megamorphic named, global or InstanceOf entry (K3; the stub cache is the generic
    /// path's, H16-Q3).
    Mega,
    /// One case of a named, global or InstanceOf entry: its key (a shape, [`KEY_STRING`] or
    /// the rhs function) and its handler (an int32 or an IC_HANDLER cell).
    Handler { key: u64, handler: u64 },
    /// A keyed slot: its key half (undefined, a receiver shape or the hole; `keyed_ic.rs`) and
    /// the int32 bits of its value half (`profile::keyed`).
    Keyed { key: u64, bits: u32 },
    /// A Call slot: its key half (undefined, the native word of the one target's FunctionInfo,
    /// or the hole; `profile.rs`). A `Construct*` site's slot is read the same way.
    Call { key: u64 },
    /// A BinaryOp or CompareOp slot: its hint bits (`BinaryHint`, `CompareHint`). 0 means
    /// "int32 or never ran" (F-R9): T0 records an int32 pair nowhere (H16-Q2 (1)).
    Hint { bits: u32 },
}

#[inline]
fn is_i32(b: u64) -> bool {
    b & NUMBER_TAG == NUMBER_TAG
}

/// The cases of slot `i` of the feedback vector `v` at a site of operation `op`: one per case
/// of a named, global or InstanceOf entry (a POLY_IC cell gives each live case), else one.
/// Not a GC point; reads only.
pub(crate) fn cases(v: u64, i: u32, op: Operation) -> Vec<Case> {
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    match op {
        Operation::LoadNamed
        | Operation::StoreNamed
        | Operation::DefineNamed
        | Operation::LoadGlobal
        | Operation::StoreGlobal
        | Operation::InstanceOf => {
            if key == UNDEFINED {
                return vec![Case::Empty];
            }
            if key != HOLE {
                return vec![Case::Handler {
                    key,
                    handler: value,
                }];
            }
            if is_i32(value) {
                return vec![Case::Mega];
            }
            let p = cell(value);
            let live: Vec<Case> = (0..ic::POLY_CASES)
                .map(|c| ic::pc::CASES + ic::pc::CASE * c)
                .filter(|&off| rd(p, off) != UNDEFINED)
                .map(|off| Case::Handler {
                    key: rd(p, off),
                    handler: rd(p, off + ic::pc::HANDLER),
                })
                .collect();
            if live.is_empty() {
                vec![Case::Empty]
            } else {
                live
            }
        }
        Operation::LoadKeyed | Operation::StoreKeyed | Operation::KeyedCold => {
            let bits = if is_i32(value) {
                int32_of(value) as u32
            } else {
                0
            };
            vec![Case::Keyed { key, bits }]
        }
        Operation::Call | Operation::CallSpread | Operation::Construct => vec![Case::Call { key }],
        Operation::Binary { .. } | Operation::Unary(_) | Operation::Compare(_) => {
            let bits = if is_i32(value) {
                int32_of(value) as u32
            } else {
                0
            };
            vec![Case::Hint { bits }]
        }
    }
}

// ---- the op list ------------------------------------------------------------------------------

/// A value of the site that an op reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Val {
    /// The receiver (0x21, 0x24, 0x25, keyed sites), or the lookup start (0x22).
    Receiver,
    /// The value a store writes (the accumulator of 0x24, 0x25, 0x26).
    Value,
    /// The key of a keyed site.
    Key,
    /// The left operand of a binary operator or comparison.
    Lhs,
    /// The right operand (the accumulator, or the immediate of a `*Smi` form).
    Rhs,
    /// The operand of a unary operator.
    Operand,
    /// The callee of a call.
    Callee,
    /// The rhs of `instanceof` (a function).
    Function,
    /// The global object (global sites).
    Global,
    /// The script's lexical record (`Root::Lexical`; global sites).
    Lexical,
}

/// A heap thing that an op names (H16 "Refs, not pointers"): a reader lowers it to a ref node
/// (H30) or an H34 relocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ref {
    /// The case key: a ShapeRef (named, global, keyed), the FunctionRef of a call target, or
    /// the rhs function of an InstanceOf case.
    CaseKey,
    /// The IC_HANDLER cell's +24: a holder (PROTO_LOAD, GETTER, SETTER, PROTO_DICT_LOAD), a
    /// target shape (ADD), the lexical record's shape (GLOBAL) or the rhs function's shape
    /// (INSTANCEOF).
    HandlerRef,
}

/// The ProtoRef of [`Op::GuardProtoValidity`]: whose prototype chain the case depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proto {
    /// The prototype of the shape that the ref names (the receiver's or the rhs function's).
    OfShape(Ref),
    /// %String.prototype% (a string receiver's chain, H16 K11).
    StringPrototype,
}

/// The cell class of [`Op::GuardKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Array,
    String,
    /// (TypedArray TA-2 TA-KEYED, design §6.3) the view kind of the element type `t` (a spec
    /// Table 71 row: cell kind `kinds::INT8_VIEW` + `t`).
    View(u8),
}

/// A value type of [`Op::GuardType`] (H16: int32 / number / string / symbol / bigint; only the
/// ones a list uses are here). `Heap` (appended M6-P1) is a value that is not a number (a cell
/// or another immediate): the check of the HEAP field store kinds (13, 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    Int32,
    /// An int32 or a double.
    Number,
    Heap,
}

/// A field: a smi field handler (`ic::smi::FIELD_IN_OBJECT` or `FIELD_OUT_OF_LINE` with the
/// byte offset in bits 4-31), as T0 keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Loc(pub i32);

impl Loc {
    /// True for a field in the object cell, false for one in its SLOTS cell.
    pub(crate) fn in_object(self) -> bool {
        self.0 & smi::KIND_MASK == smi::FIELD_IN_OBJECT
    }

    /// The byte offset in the object cell or the SLOTS cell.
    pub(crate) fn offset(self) -> usize {
        (self.0 >> smi::SHIFT) as usize
    }
}

/// The elements kinds a keyed site saw: one bit per kind value 0-15 (`elements::kind`), the
/// IC_LAYOUT 4 kinds set (bits 10-25 of the slot's value half) shifted down. 0: none recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kinds(pub u16);

/// The object an accessor op calls the accessor pair of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holder {
    /// An own accessor of [`Val::Receiver`] (at 0x22: of the lookup start).
    Receiver,
    /// An inherited one: the holder at the handler's +24 (weak).
    Ref(Ref),
}

/// One op of a list (H16 "Op (v0 list)", plus the ops appended in M6-P1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    // Guards (H16 v0).
    /// The value is a cell whose word0 ShapeId is the shape's (the K9 compare).
    GuardShape(Val, Ref),
    /// The prototype chain of the case is still valid (T0: the handler's validity cell, +16).
    GuardProtoValidity(Proto),
    /// The value is a cell of the class (the kind byte).
    GuardKind(Val, Class),
    /// The value has the type.
    GuardType(Val, Ty),
    /// The value is the case's target: a FUNCTION whose FunctionInfo is the call key (Call),
    /// or the rhs function itself (InstanceOf; T0 compares the cell).
    GuardTarget(Val, Ref),
    /// The key (an int32) is the index of a present element of the receiver (an Array): at
    /// least 0, below its length and its elements capacity, not a hole.
    GuardIndex(Val, Val),
    /// The object's SLOTS cell exists and has room at the field (an out-of-line ADD).
    GuardSlotsRoom(Val, Loc),
    /// The field of the value is not the hole (a lexical binding outside its TDZ).
    GuardNotHole(Val, Loc),
    // Actions (H16 v0).
    LoadField(Val, Loc),
    /// A store through the write barrier.
    StoreField(Val, Loc),
    /// The field of the holder at the handler's +24 (a dead holder misses).
    LoadHolderField(Ref, Loc),
    /// The K13 add: the store, then the target shape. It misses when the value does not fit
    /// the new field's representation (M5-P2 CONSTREP; `ic::store_with`), and a dead target
    /// misses.
    AddField(Val, Ref, Loc),
    LoadArrayLength,
    LoadStringLength,
    LoadUndefined,
    /// The global object's field (GLOBAL at 0x10, 0x11).
    LoadGlobalField(Loc),
    /// One barriered store into the global object's field (GLOBAL at 0x12).
    StoreGlobalField(Loc),
    /// The element at the key (any storage form; a DOUBLE cell gives a number).
    LoadElement(Kinds),
    /// The value, in place, in the storage form of the receiver's elements kind; it misses
    /// when the value does not fit the kind (a non-int32 into an INT32 kind, a non-number
    /// into a DOUBLE kind; `keyed_ic::fast_set`).
    StoreElement(Kinds),
    /// A call of the target (one frame push for a bytecode target). It misses where T0's
    /// `call_fast` goes cold for a reason the guard cannot see: another realm, a sloppy target
    /// with a primitive receiver, a class constructor, interrupt due, the call that makes the
    /// target's vector, a full VM stack.
    CallTarget(Ref),
    /// A call of the getter of the ACCESSOR_PAIR at the field of the holder (H17 "Accessor ICs").
    /// `this` is [`Val::Receiver`] at 0x21; at 0x22 it is the site's `this`, not the lookup start.
    CallGetter(Holder, Loc),
    /// A call of the setter, with the receiver and the value.
    CallSetter(Holder, Loc),
    /// The generic operation (SLOW, MEGA, no case; megamorphic: the stub cache).
    Generic,
    // Actions appended in M6-P1 (CBC).
    /// `lhs OP rhs` on operands of the type the guards checked, as T0's fast body computes
    /// it: an int32 pair gives the int32 result that fits, else the exact double (an overflow,
    /// `*`, `/`, `>>>` past `i32::MAX`); a number pair (Add, Sub, Mul, Div only) gives the
    /// double. Misses on an int32 `%` with a negative dividend or a 0 divisor.
    BinaryArith(BinOp, Ty),
    /// The unary operator on an operand of the type the guards checked: an int32 gives the
    /// int32 result that fits, else the double; a number (not `~`) gives the double.
    UnaryArith(Unary, Ty),
    /// The comparison of two operands of the type the guards checked (IEEE rules for doubles:
    /// NaN compares false, -0 equals 0).
    CompareNumbers(Compare, Ty),
    /// OrdinaryHasInstance (§7.3.21 steps 3-6) with `P` = the rhs function's field at this
    /// index: a primitive gives false; it misses when `P` is not an object, on a Proxy in the
    /// chain, or when the chain walk uses up the interrupt budget (`instance_ic::hit`).
    OrdinaryHasInstance(i32),
    /// H16 `LoadHolderDict(holder ref, key)` (M6-P1 ACCIC), the key being the site's name:
    /// the value of that name in the dictionary-mode holder at the handler's +24 (its PROPS
    /// or PROPS_INDEXED table; H17 "Dictionary v1"); it misses when the holder died or no
    /// longer has the name as a data property.
    LoadHolderDict(Ref),
    // Appended in TypedArray TA-2 TA-KEYED (design §6.3; the view cases of `keyed_ic.rs`).
    /// The key (an int32) is a valid integer index of the receiver, a TypedArray
    /// (IsValidIntegerIndex, §10.4.5.14): at least 0 and below the view's bound at this
    /// access, in the form of its elements kind: TYPED_FIXED its length field (0 once
    /// detached), TYPED_VARIABLE the length from its buffer's byte length now (out of bounds
    /// after a shrink or a detach).
    GuardViewIndex(Val, Val),
    /// The element of type `t` at the key of the receiver, a view of that type (an int32
    /// key): a Number (Uint32 above 2^31 - 1 gives a double; a float NaN boxes as the one
    /// NaN), or undefined when the key is not a valid integer index (no prototype walk,
    /// §10.4.5.4). `t` holds a Number (not BigInt64, BigUint64).
    LoadViewOrUndefined(u8),
    /// TypedArraySetElement (§10.4.5.16) of a Number (checked by `GuardType(Value, Number)`)
    /// at a valid integer index (checked by [`Op::GuardViewIndex`]) of the receiver, a view of
    /// type `t`: NumericToRawBytes of the value (one NaN pattern per float type), in place,
    /// with no barrier (the bytes are not Values). It never calls: the value is a Number.
    StoreView(u8),
    // Appended in TypedArray TA-3 (V13-D8, TA-ORCH-17; route TA-SHARED-MIXED-WIDTH, its SAB
    // part).
    /// The value is a view whose buffer (`[[ViewedArrayBuffer]]`, the view's +24) is not a
    /// SharedArrayBuffer. T0 takes the spec path for a view on a SharedArrayBuffer (its bytes
    /// are read and written only with atomics, TA-ORCH-12, -13; `keyed_ic.rs` module `view`),
    /// so a list that reads or writes the elements with plain loads and stores checks this
    /// first. A view keeps its buffer for life (a detach or a resize keeps the buffer's
    /// class), so the answer for one view never changes.
    GuardViewNotShared(Val),
}

impl Op {
    /// True for a guard, false for an action.
    pub(crate) fn is_guard(self) -> bool {
        matches!(
            self,
            Op::GuardShape(..)
                | Op::GuardProtoValidity(_)
                | Op::GuardKind(..)
                | Op::GuardType(..)
                | Op::GuardTarget(..)
                | Op::GuardIndex(..)
                | Op::GuardSlotsRoom(..)
                | Op::GuardNotHole(..)
                | Op::GuardViewIndex(..)
                | Op::GuardViewNotShared(_)
        )
    }
}

/// The most ops a list holds (a keyed store on a view: 5 guards and the store).
pub(crate) const MAX_OPS: usize = 6;

/// An op list: guards, then one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ops {
    len: u8,
    ops: [Op; MAX_OPS],
}

impl Ops {
    fn of(list: &[Op]) -> Ops {
        let mut ops = [Op::Generic; MAX_OPS];
        ops[..list.len()].copy_from_slice(list);
        Ops {
            len: list.len() as u8,
            ops,
        }
    }

    /// The ops, guards first.
    pub(crate) fn as_slice(&self) -> &[Op] {
        &self.ops[..usize::from(self.len)]
    }

    /// The last op: the action.
    pub(crate) fn action(&self) -> Op {
        self.ops[usize::from(self.len) - 1]
    }
}

const GENERIC: &[Op] = &[Op::Generic];

/// The IC_HANDLER parts of a cell handler `h`: (kind +8, +16, +24, +32). Every handler kind
/// shares the layout of the `ic.rs` IC_LAYOUT 4 table; `instance_ic::io` names its offsets.
pub(crate) fn handler_parts(h: u64) -> (i32, u64, u64, i32) {
    let c = cell(h);
    (
        rd_i32(c, io::KIND),
        rd(c, io::CELL),
        rd(c, io::SHAPE),
        rd_i32(c, io::FIELD),
    )
}

/// The op list of the case `case` at a site of operation `op` (H16 `ops_of(case, operation)`).
/// Total: a pair with no typed list gives `[Generic]`. Not a GC point; reads the handler cell
/// of a cell case only.
pub(crate) fn ops_of(case: &Case, op: Operation) -> Ops {
    use Operation as P;
    match (*case, op) {
        (Case::Handler { key, handler }, P::LoadNamed | P::StoreNamed | P::DefineNamed) => {
            named(key, handler, op)
        }
        (Case::Handler { handler, .. }, P::LoadGlobal | P::StoreGlobal) => {
            global(handler, op == P::StoreGlobal)
        }
        (Case::Handler { handler, .. }, P::InstanceOf) => instance_of(handler),
        (Case::Keyed { key, bits }, P::LoadKeyed | P::StoreKeyed) => keyed(key, bits, op),
        (Case::Call { key }, P::Call) if key != UNDEFINED && key != HOLE => Ops::of(&[
            Op::GuardTarget(Val::Callee, Ref::CaseKey),
            Op::CallTarget(Ref::CaseKey),
        ]),
        (Case::Hint { bits }, P::Binary { op, imm }) => binary(bits, op, imm),
        (Case::Hint { bits }, P::Unary(u)) => unary(bits, u),
        (Case::Hint { bits }, P::Compare(c)) => compare(bits, c),
        _ => Ops::of(GENERIC),
    }
}

/// The guards of a case on the receiver's chain: its shape (or, under [`KEY_STRING`], its
/// kind) and, if `chain`, the validity of the chain the case depends on.
fn receiver_guards(key: u64, chain: bool, out: &mut Vec<Op>) {
    if key == KEY_STRING {
        out.push(Op::GuardKind(Val::Receiver, Class::String));
        if chain {
            out.push(Op::GuardProtoValidity(Proto::StringPrototype));
        }
    } else {
        out.push(Op::GuardShape(Val::Receiver, Ref::CaseKey));
        if chain {
            out.push(Op::GuardProtoValidity(Proto::OfShape(Ref::CaseKey)));
        }
    }
}

/// Named sites (0x21, 0x22, 0x24, 0x25): the H16 handler table.
fn named(key: u64, h: u64, op: Operation) -> Ops {
    let load = op == Operation::LoadNamed;
    let mut l = Vec::with_capacity(MAX_OPS);
    if is_i32(h) {
        let x = int32_of(h);
        let k = x & smi::KIND_MASK;
        match k {
            smi::FIELD_IN_OBJECT | smi::FIELD_OUT_OF_LINE if key != KEY_STRING => {
                l.push(Op::GuardShape(Val::Receiver, Ref::CaseKey));
                l.push(if load {
                    Op::LoadField(Val::Receiver, Loc(x))
                } else {
                    Op::StoreField(Val::Receiver, Loc(x))
                });
            }
            k if smi::is_checked(k) && !load && key != KEY_STRING => {
                let ty = match smi::rep_of_kind(k) {
                    crate::shape::rep::INT32 => Ty::Int32,
                    crate::shape::rep::DOUBLE => Ty::Number,
                    _ => Ty::Heap,
                };
                l.push(Op::GuardShape(Val::Receiver, Ref::CaseKey));
                l.push(Op::GuardType(Val::Value, ty));
                l.push(Op::StoreField(Val::Receiver, Loc(smi::plain(x))));
            }
            smi::ARRAY_LENGTH if load && key != KEY_STRING => {
                // H16: GuardKind, a deliberate widening of T0's shape key (any ARRAY has its
                // int32 `length` at +24).
                l.push(Op::GuardKind(Val::Receiver, Class::Array));
                l.push(Op::LoadArrayLength);
            }
            smi::STRING_LENGTH if load && key == KEY_STRING => {
                l.push(Op::GuardKind(Val::Receiver, Class::String));
                l.push(Op::LoadStringLength);
            }
            _ => return Ops::of(GENERIC),
        }
        return Ops::of(&l);
    }
    if !is_cell_bits(h) {
        return Ops::of(GENERIC);
    }
    let (kind, chain_cell, _r, loc) = handler_parts(h);
    match kind {
        hk::PROTO_LOAD | hk::MISSING if load => {
            receiver_guards(key, true, &mut l);
            l.push(if kind == hk::MISSING {
                Op::LoadUndefined
            } else {
                Op::LoadHolderField(Ref::HandlerRef, Loc(loc))
            });
        }
        hk::ADD if !load && key != KEY_STRING => {
            // 0x24 checks the chain (the setter or read-only rule of OrdinarySet); 0x25
            // (CreateDataProperty) does not (K13).
            receiver_guards(key, op == Operation::StoreNamed, &mut l);
            if !Loc(loc).in_object() {
                l.push(Op::GuardSlotsRoom(Val::Receiver, Loc(loc)));
            }
            l.push(Op::AddField(Val::Receiver, Ref::HandlerRef, Loc(loc)));
        }
        hk::GETTER | hk::SETTER
            if (kind == hk::GETTER && load)
                || (kind == hk::SETTER && op == Operation::StoreNamed) =>
        {
            // Own accessor: +16 int32 0; inherited: +16 the chain's validity cell, +24 the
            // holder (the `ic.rs` IC_LAYOUT 4 table).
            let own = is_i32(chain_cell);
            receiver_guards(key, !own, &mut l);
            let holder = if own {
                Holder::Receiver
            } else {
                Holder::Ref(Ref::HandlerRef)
            };
            l.push(if kind == hk::GETTER {
                Op::CallGetter(holder, Loc(loc))
            } else {
                Op::CallSetter(holder, Loc(loc))
            });
        }
        hk::PROTO_DICT_LOAD if load => {
            receiver_guards(key, true, &mut l);
            l.push(Op::LoadHolderDict(Ref::HandlerRef));
        }
        _ => return Ops::of(GENERIC),
    }
    Ops::of(&l)
}

/// Global sites (0x10-0x12): a lexical binding (a smi field keyed on the lexical record's
/// shape) or an own data property of the global object (GLOBAL, keyed on its shape).
fn global(h: u64, store: bool) -> Ops {
    if is_i32(h) {
        let x = int32_of(h);
        if !matches!(
            x & smi::KIND_MASK,
            smi::FIELD_IN_OBJECT | smi::FIELD_OUT_OF_LINE
        ) {
            return Ops::of(GENERIC);
        }
        let loc = Loc(x);
        return Ops::of(&[
            Op::GuardShape(Val::Lexical, Ref::CaseKey),
            Op::GuardNotHole(Val::Lexical, loc),
            if store {
                Op::StoreField(Val::Lexical, loc)
            } else {
                Op::LoadField(Val::Lexical, loc)
            },
        ]);
    }
    if !is_cell_bits(h) {
        return Ops::of(GENERIC);
    }
    let (kind, _, _, loc) = handler_parts(h);
    if kind != hk::GLOBAL {
        return Ops::of(GENERIC);
    }
    let loc = Loc(loc);
    Ops::of(&[
        Op::GuardShape(Val::Global, Ref::CaseKey),
        Op::GuardShape(Val::Lexical, Ref::HandlerRef),
        if store {
            Op::StoreGlobalField(loc)
        } else {
            Op::LoadGlobalField(loc)
        },
    ])
}

/// InstanceOf (0x52): an INSTANCEOF case (`instance_ic.rs`).
fn instance_of(h: u64) -> Ops {
    if !is_cell_bits(h) {
        return Ops::of(GENERIC); // SLOW
    }
    let (kind, _, _, field) = handler_parts(h);
    if kind != hk::INSTANCEOF {
        return Ops::of(GENERIC);
    }
    Ops::of(&[
        Op::GuardTarget(Val::Function, Ref::CaseKey),
        Op::GuardShape(Val::Function, Ref::HandlerRef),
        Op::GuardProtoValidity(Proto::OfShape(Ref::HandlerRef)),
        Op::OrdinaryHasInstance(field),
    ])
}

/// Keyed sites: the T0 array paths (`GetKeyedProperty`'s implicit one, `keyed_ic::fast_set`),
/// and the view cases of `keyed_ic.rs` ([`view_keyed`], TA-2 TA-KEYED).
fn keyed(key: u64, bits: u32, op: Operation) -> Ops {
    let kinds = Kinds(((bits & KINDS_SET_MASK) >> KINDS_SET_SHIFT) as u16);
    if let Some(l) = view_keyed(key, kinds, op) {
        return l;
    }
    let index = [
        Op::GuardKind(Val::Receiver, Class::Array),
        Op::GuardType(Val::Key, Ty::Int32),
        Op::GuardIndex(Val::Receiver, Val::Key),
    ];
    let mut l = Vec::with_capacity(MAX_OPS);
    if op == Operation::LoadKeyed {
        // The array path runs before any IC state (the key half is not read).
        l.extend_from_slice(&index);
        l.push(Op::LoadElement(kinds));
        return Ops::of(&l);
    }
    // StoreKeyed: monomorphic (a shape) or megamorphic (the hole); a new state goes cold.
    if key == UNDEFINED {
        return Ops::of(GENERIC);
    }
    if key != HOLE {
        l.push(Op::GuardShape(Val::Receiver, Ref::CaseKey));
    }
    l.extend_from_slice(&index);
    l.push(Op::StoreElement(kinds));
    Ops::of(&l)
}

/// The view lists of a keyed site (TypedArray TA-2 TA-KEYED, design §6.3), or `None` (the
/// array lists): only when the kinds set has the bit of TYPED_FIXED (7) or TYPED_VARIABLE (8)
/// and the case key is the shape of a view, which gives the element type `t` (the kinds set
/// does not hold it, so a site with no such key, LoadKeyed and megamorphic ones included,
/// keeps the array lists). The shape is read only then: the key half of a T0 keyed slot is a
/// live shape, undefined or the hole. T0 makes no such case yet: a view store hit leaves the
/// key half alone (`keyed_ic.rs` module `view`), and no T0 code writes the two view bits (TA-3
/// left the kinds-set writer to the first FACT_SCHEMA bump, TA-ORCH-14), so these lists wait
/// for a writer of the element type: route TA-KEYED-VIEW-CARRIER (due M5, V13-D8). The test
/// that a view hit keeps the key half is
/// `tests::ic::the_store_key_half_with_views_keeps_the_cold_miss_rule`.
///
/// ```text
/// LoadKeyed   GuardKind(receiver, View(t)), GuardViewNotShared(receiver), GuardType(key, int32),
///             LoadViewOrUndefined(t)
/// StoreKeyed  GuardKind(receiver, View(t)), GuardViewNotShared(receiver), GuardType(key, int32),
///             GuardViewIndex(receiver, key), GuardType(value, number), StoreView(t)
/// BigInt64, BigUint64: [Generic] (the element is a BigInt; T0 goes cold)
/// ```
///
/// GuardKind, not GuardShape, is a deliberate widening of the shape key (as `ARRAY_LENGTH`):
/// an int32 key on a view reads no named property and no prototype (§10.4.5.4; §10.4.5.5 with
/// the view as the receiver), and the bound form is the receiver's own elements kind, read at
/// run time. Both lists are wider than T0's own test (T0 takes no Float16 and no negative key,
/// and stores only an int32 into an integer type), as H16 allows. They are not wider at a
/// SharedArrayBuffer: T0 misses there, and GuardViewNotShared misses too (the view's shape
/// does not tell: a fixed-length view on a SharedArrayBuffer and one on a fixed-length
/// ArrayBuffer of the same type have the same elements kind 7 and can share a shape).
fn view_keyed(key: u64, kinds: Kinds, op: Operation) -> Option<Ops> {
    const VIEWS: u16 = 1 << ek::TYPED_FIXED | 1 << ek::TYPED_VARIABLE;
    if kinds.0 & VIEWS == 0 || key == UNDEFINED || key == HOLE || !is_cell_bits(key) {
        return None;
    }
    let k = crate::shape::object_kind(key);
    if !crate::kinds::is_view_kind(k) {
        return None;
    }
    if crate::kinds::is_bigint_view_kind(k) {
        return Some(Ops::of(GENERIC));
    }
    let t = k - crate::kinds::INT8_VIEW.get();
    let guards = [
        Op::GuardKind(Val::Receiver, Class::View(t)),
        Op::GuardViewNotShared(Val::Receiver),
        Op::GuardType(Val::Key, Ty::Int32),
    ];
    let mut l = Vec::with_capacity(MAX_OPS);
    l.extend_from_slice(&guards);
    if op == Operation::LoadKeyed {
        l.push(Op::LoadViewOrUndefined(t));
    } else {
        l.push(Op::GuardViewIndex(Val::Receiver, Val::Key));
        l.push(Op::GuardType(Val::Value, Ty::Number));
        l.push(Op::StoreView(t));
    }
    Some(Ops::of(&l))
}

const B_INT32: u32 = ep9_facts::BinaryHint::INT32.bits() as u32;
const B_INT32_OVERFLOW: u32 = ep9_facts::BinaryHint::INT32_OVERFLOW.bits() as u32;
const B_NUMBER: u32 = ep9_facts::BinaryHint::NUMBER.bits() as u32;
const C_INT32: u32 = ep9_facts::CompareHint::INT32.bits() as u32;
const C_NUMBER: u32 = ep9_facts::CompareHint::NUMBER.bits() as u32;

/// The operand type of a numeric hint: `Int32` if the site saw only int32 operands (also with
/// a result outside int32), `Number` if it saw numbers, `None` for anything else.
fn numeric(bits: u32, int32: u32, number: u32) -> Option<Ty> {
    if bits & !int32 == 0 {
        Some(Ty::Int32)
    } else if bits & !(int32 | number) == 0 {
        Some(Ty::Number)
    } else {
        None
    }
}

/// BinaryOp (0x2a-0x41): the arms of `interp::ops::binop_fast`. T0 has no fast arm for `**`,
/// nor for `%`, the bitwise operators and `>>>` on a double.
fn binary(bits: u32, op: BinOp, imm: bool) -> Ops {
    let ty = numeric(bits, B_INT32 | B_INT32_OVERFLOW, B_NUMBER);
    let ty = match (ty, op) {
        (_, BinOp::Exp) | (None, _) => return Ops::of(GENERIC),
        (Some(Ty::Number), BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) => Ty::Number,
        (Some(Ty::Number), _) => return Ops::of(GENERIC),
        (Some(t), _) => t,
    };
    let mut l = vec![Op::GuardType(Val::Lhs, ty)];
    if !imm {
        l.push(Op::GuardType(Val::Rhs, ty));
    }
    l.push(Op::BinaryArith(op, ty));
    Ops::of(&l)
}

/// The unary operators on a BinaryOp slot: their fast arms in `interp/ops.rs` (an int32 for
/// all; a double for all but `~`).
fn unary(bits: u32, u: Unary) -> Ops {
    match numeric(bits, B_INT32 | B_INT32_OVERFLOW, B_NUMBER) {
        Some(Ty::Number) if u == Unary::BitNot => Ops::of(GENERIC),
        Some(ty) => Ops::of(&[Op::GuardType(Val::Operand, ty), Op::UnaryArith(u, ty)]),
        None => Ops::of(GENERIC),
    }
}

/// CompareOp (0x4b-0x50): the int32 arms of all six and the number arms of `===` and the
/// relational four. `==` on doubles, `== null`, the identity arm of `===` and its arm for two
/// atoms (M6-P1) have no list (a list for them is a fill).
fn compare(bits: u32, c: Compare) -> Ops {
    let ty = match numeric(bits, C_INT32, C_NUMBER) {
        Some(Ty::Number) if c == Compare::Eq => return Ops::of(GENERIC),
        Some(ty) => ty,
        None => return Ops::of(GENERIC),
    };
    Ops::of(&[
        Op::GuardType(Val::Lhs, ty),
        Op::GuardType(Val::Rhs, ty),
        Op::CompareNumbers(c, ty),
    ])
}
