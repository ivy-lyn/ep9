//! T0 profile recording (owner M4-P1 PROFILE; OI-06, JA-3, H16-Q2) and the IC_LAYOUT 2 feedback
//! slot table (H16; `versions::IC_LAYOUT_VERSION` 2, M4-P0).
//!
//! **IC_LAYOUT 2 slot table.** Each slot of a FEEDBACK cell (`feedback.rs`, 16 B) is a key half
//! (weak) and a value half (strong). One module writes each slot kind:
//!
//! | Slot kind (ABI byte) | Key half (weak) | Value half (strong) | Writer module (role, phase) |
//! |---|---|---|---|
//! | Call (0) at `Call*` sites | **(M4-P1)** the native word of the target's FunctionInfo (not a cell; the FUNCTION cell's +24), or the hole = megamorphic | int32 count of the sampled calls (saturating) | `profile.rs` (PROFILE, M4-P1) |
//! | Call (0) at `Construct*` sites | the target function, or the hole | a construct-site handler (an IC_HANDLER cell, handler kind `CONSTRUCT_SITE`: +16 int32 count, +24 initial shape (weak), +32 int32 `prototype` field index; V9-A8), or an int32 count | `construct_ic.rs` (OBJ-ALLOC, M4-P1) |
//! | LoadNamed, StoreNamed, DefineNamedOwn, LoadGlobal*, StoreGlobal (1, 3, 5, 7-9) | as IC_LAYOUT 1 | as IC_LAYOUT 1; the prototype cases keep a validity cell, not the epoch | `ic.rs` (VALIDITY M4-P1, IC2 M4-P3) |
//! | LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed (2, 4, 6, 10) | the receiver shape (monomorphic) or the hole (`keyed_ic.rs` writes it, KEYED, M4-P3) | int32 bits: element form seen, key types (int32, string, symbol), out of bounds, hole read | `profile.rs` writes the bits (M4-P1); `keyed_ic.rs` writes the key half and never reads the bits (KEYED, M4-P3; MC-w14-8); only the StoreKeyed fast case changes the key half, so the exported state is live only for StoreKeyed (MC-w15-1) |
//! | BinaryOp, CompareOp (11, 12) | undefined | int32 hint bits (int32, number, string, bigint, other) | `profile.rs` |
//! | TypeOf (13) | undefined | int32 type bits | `profile.rs` |
//! | InstanceOf (14) | the rhs function, or the hole (2-4 functions, or megamorphic) | an instanceof handler (an IC_HANDLER cell, handler kind `INSTANCEOF`: +16 validity cell, +24 the rhs function's shape (weak; the hit compares it, so the field index holds), +32 int32 `prototype` field index; V9-A8), a POLY_IC cell (2-4 cases: the function (weak), its handler), or an int32 state | `instance_ic.rs` (IC2, M4-P3) |
//! | Literal (15) | the result shape | int32 count | `literal_site.rs` (OBJ-ALLOC, M4-P1) |
//! | CloneObject (16) | the source shape, or the hole (2-4 source shapes, or megamorphic) | a clone handler (an IC_HANDLER cell, handler kind `CLONE`: +16 int32 count, +24 the result shape (weak), +32 int32 0), a POLY_IC cell (2-4 cases: the source shape (weak), its handler), or an int32 state | `literal_site.rs` (OBJ-ALLOC, M4-P1) |
//! | StoreInArrayLiteral, ForIn, JumpLoop, CreateClosure (17-20) | ForIn: the receiver shape; others undefined | int32 bits or count (JumpLoop: the sampled back edges, **M4-P1**) | `profile.rs` |
//!
//! The keys of InstanceOf and CloneObject are the IC_LAYOUT 1 keys (H16 per-kind table), so the
//! facts exporter fills `TargetFacts` (the rhs functions) and `CloneFacts` (source and result
//! shapes) from them with no `FACT_SCHEMA_VERSION` change; CreateClosure (20) keeps a count,
//! which gives `ClosureFacts` (none, one or many).
//!
//! Rules: a slot is written only when its bit or state is new (load, test, rare store;
//! V9-D6). A hit path never allocates and never calls (H16:103). The IC_HANDLER layout under
//! IC_LAYOUT 2 is in the `ic.rs` module docs.
//!
//! **Record points** (`interp/ops.rs` calls them; ops.rs has no owner after M4-P0, V9-D5).
//! Each hook is `#[inline(always)]` and calls nothing on its fast-body path. Not out of line
//! in debug builds either: an out-of-line hook gets the address of the handler's `St`, and an
//! optimized build with debug assertions (the dev profile of `spikes/spike-c`, opt-level 2 for
//! its dependencies) then loses the sibling tail calls, so a T0 loop grows the native stack
//! until it overflows (M4-P1 fixer). Each execution records once: in its fast body, or in
//! its cold body (which runs only after the fast body returned `Flow::Cold`, `interp::dsl`).
//!
//! **No record code in a fast handler (M4-P1 PROFILE-X64).** A record inlined into a fast
//! handler made x86-64 LLVM save 1-4 callee-saved registers on entry to 22 handlers (and one
//! more on 2 call handlers per x64 target), on every execution, also on the implicit int32
//! paths that record nothing: up to +9% instructions on the T0 kernels, while aarch64 (18
//! scratch registers) did not change. A plain `#[cold]` out-of-line call made it worse (a
//! call forces the frame). So a fast body that must record finishes its result and returns
//! one of three `interp::Flow` values; the sibling handler then tail-calls an out-of-line
//! function with the handler's state that records and dispatches (`interp::dsl`):
//!
//! | Flow | Fast bodies | Record (sibling: tail-called function) |
//! |---|---|---|
//! | `Record(Rec)` (constant bits) | the binary operators on a number pair that is not two int32s ([`NUMBER_PAIR`]) and an int32 `*`, `/` or `>>>` that leaves int32 ([`int32_pair_bits`]); `Inc`, `Dec`, `Negate`, `ToNumber`, `ToNumeric` on a double ([`NUMBER_OPERAND`]); the 4 relational compares on such a pair ([`NUMBER_COMPARE`]); `TestEqualStrict` on such a pair; `ForInPrepare` ([`for_in_bits`]) | [`record`] (`record_next`: it reads the slot operand again from the bytecode) |
//! | `RecordPair(Pair)` | `TestEqual` with null or undefined on a side, `TestEqualStrict` on other pairs it finishes | [`record_pair`] = [`record_compare`] (`record_pair_next`: it gets the pair) |
//! | `JumpRecordCall(t, slot)` | `call_fast` after its frame push, for a call in the window ([`call_window_after_unit`]) | [`record_call_pushed`] (`record_call_next`: in the callee's frame, from its header) |
//!
//! The match loop, `become` and the prefixed step record inline (`Flow::recorded`).
//! An int32 operand or pair reaches none of them (implicit).
//!
//! | Hook | Called from | Arguments |
//! |---|---|---|
//! | [`binary_pair_bits`], [`int32_result_bits`], [`int32_pair_bits`], [`NUMBER_PAIR`] | the fast body of the 12 binary and 12 `*Smi` operators (`binop_fast`, `binop_fast_boxed` for `*` and `/`): the pair's record is chosen from the tags before the operation, the int32 overflow record from the result | operator, operand tags, result |
//! | [`record_binary_cold`] | their cold body (`binop_cold`): an int32 `%` with a negative dividend or a 0 divisor, an int32 `**`, `%`, `**` and the bitwise operators on a double, or other types | slot, operator, left, right |
//! | [`NUMBER_OPERAND`], [`record_unary`] | `Inc`, `Dec`, `Negate`, `ToNumber`, `ToNumeric` (BinaryOp slots): a double in the fast body; other operands (and every `BitwiseNot` that is not int32) in the cold body | slot, operand |
//! | [`NUMBER_COMPARE`], [`Pair`], [`record_compare`] | `TestEqual`, `TestEqualStrict`, `TestLessThan`, `TestGreaterThan`, `TestLessThanOrEqual`, `TestGreaterThanOrEqual`: fast body as above, cold body otherwise | slot, left, right |
//! | [`record_typeof`] | the `TypeOf` cold body (its fast body is only `Flow::Cold`) | slot, operand |
//! | [`record_call`], [`record_call_pushed`] | every `Call*` site except `CallRuntime`: after the frame push of `call_fast`, or in the cold body (`call_record_cold`: every cold exit, `CallWithSpread`, a `CallProperty`/`CallAnyReceiver` list without a receiver) | slot, callee |
//! | [`record_keyed`] | `GetKeyedProperty` (before the array path), `SetKeyedProperty`, `DefineKeyedOwnProperty`, `DefineKeyedOwnPropertyInLiteral` (its fast body is only this hook and `Flow::Cold`), `StaInArrayLiteral`, `TestIn` (records nothing, see below) | slot, receiver, key |
//! | [`record_keyed_cold`] | their cold bodies (out of bounds, a hole, a non-array receiver; every execution of `DefineKeyedOwnPropertyInLiteral`) | slot, receiver, key |
//! | [`record_jump_loop`] | `JumpLoop` | slot |
//! | [`record_closure`] | the `CreateClosure` cold body, after the closure exists (0 = an exception is pending) | slot, the closure bits |
//! | [`for_in_bits`], [`record_for_in`] | `ForInPrepare`: its fast body after the cache registers are set, or its cold body | slot, the key array |
//! | [`record_for_in_next`] | the `ForInNext` cold body (its fast body is only `Flow::Cold`) | slot, receiver |
//!
//! **What T0 records (M4-P1 PROFILE, H16-Q2).** Every value half that this module writes is
//! an int32. A function records nothing until it has its feedback vector (`feedback.rs`,
//! OM-13). The bit numbers of the hint and type sets are those of `ep9_facts` (`bits.rs`), so
//! the exporter copies them.
//!
//! | Slot kind | Value half | Key half | When |
//! |---|---|---|---|
//! | BinaryOp (11; also `Inc`, `Dec`, `Negate`, `BitwiseNot`, `ToNumber`, `ToNumeric`) | `BinaryHint` bits | untouched (undefined) | every execution that is not implicit |
//! | CompareOp (12) | `CompareHint` bits | untouched | every execution that is not implicit |
//! | TypeOf (13) | `TypeSet` bits of the operand | untouched | every execution |
//! | LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed, StoreInArrayLiteral (2, 4, 6, 10, 17) | [`keyed`] bits: the `KeySet` bits 0-3, the receiver (bits 4-7), out of bounds (8), hole (9) | untouched (KEYED, M4-P3, owns it) | every cold execution |
//! | Call (0) at `Call*` sites | the count of sampled calls, saturating at `i32::MAX` | the native word of the one target FunctionInfo sampled, or the hole (megamorphic) | sampled: a call in the first [`CALL_WINDOW`] budget units after a budget reset |
//! | JumpLoop (19) | the count of sampled back edges, saturating at `i32::MAX` | untouched | sampled: a back edge that polls (the budget is used up) |
//! | ForIn (18) | `ForInHint` bits | the one receiver shape seen, or the hole | every execution |
//! | CreateClosure (20) | a count that saturates at [`CLOSURE_MANY`]: 0 none, 1 one, 2 many | untouched | every execution |
//!
//! - **Cost rule (V9-D6: ≤ +1.0% instructions on ci-v0 and each t0-* kernel).** A hook that
//!   writes costs a vector reach (three dependent loads), a load and a test even when no bit
//!   is new, so the hottest cases read no slot at all:
//!   - **Implicit int32.** An int32 pair with an int32 result at a BinaryOp site
//!     (`BinaryHint::INT32`), an int32 operand at a unary site (`INT32`), an int32 pair at a
//!     CompareOp site (`CompareHint::INT32`), and an int32 index of a present element of an
//!     Array at `GetKeyedProperty` ([`keyed::INDEX`] | [`keyed::ARRAY`]) write nothing. The
//!     reader adds these bits to every slot of its kind in a function that has a vector: they
//!     are a prediction that a consumer guards (H16 Consequences).
//!   - **Sampled calls and back edges** (plan v9 R2: "move a kind to the cold path only").
//!     `record_call` records only in the first [`CALL_WINDOW`] units of each
//!     `interp::BUDGET` (the budget is reset at each poll), and `record_jump_loop` only at a
//!     back edge that polls. A hot site is sampled in proportion to its share of the units;
//!     a site that never runs in a window stays uninitialized. The windows come at a fixed
//!     period, so a pattern with that period can hide a target (an open point for the
//!     reader).
//!   - **An int32 `+` or `-` that overflows** takes the fast path's double arm and stays
//!     implicit (its test costs every int32 `+` and `-`); `*`, `/` and `>>>` record
//!     `INT32_OVERFLOW` (their test reuses the boxed result).
//!   - **Keyed sites** record on their cold path only. In v0 every keyed case except the
//!     implicit one is cold. A fast keyed path that a later role adds must miss on a case
//!     that is new for its site (KEYED, M4-P3), so the cold hook still sees it.
//! - **Unary sites** see only the operand: an int32 operand stays implicit even when `Inc`,
//!   `Dec` or `Negate` leaves int32 (the hook has no operator).
//! - **Call keys.** The key is the native word of the callee's FunctionInfo (a FUNCTION
//!   cell's +24, `frame::native`), not the function cell: it is not a cell, so the store needs
//!   no barrier (E1) and no call (a call in a T0 handler makes LLVM save registers on every
//!   execution), it never dies before the isolate (unit and host infos live in the Runtime),
//!   and two closures of one function are one target, as a `FunctionRef` (H16 FactSnapshot)
//!   names them. A callee that is not a FUNCTION cell (a bound function, a proxy, a value
//!   that is not callable) makes the site megamorphic.
//! - **Construct sites** are not recorded here (`construct_ic.rs` writes their Call slots). If
//!   a Call slot's value half is not an int32 (a construct-site handler of a shared slot), the
//!   call count leaves it alone.
//! - No hook allocates, runs JS or is a GC point. Every key store is the hole, a native word
//!   or (ForIn, out of line) a shape with the barrier.

use core::ptr::NonNull;

use ep9_facts::{BinaryHint, CompareHint, ForInHint, KeySet, TypeSet};
use ep9_heap::{CellPtr, Slot};
use ep9_value::encoding::NUMBER_TAG;
use ep9_value::{Value, Word0};

use crate::conv::BinOp;
use crate::feedback::{self, fv};
use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_of};
use crate::interp::dsl::FSlot;
use crate::interp::{BUDGET, St};
use crate::kinds;
use crate::object;
use crate::runtime::Runtime;
use crate::shape;
use crate::string;

/// Value bits of the keyed slots (2, 4, 6, 10, 17). Bits 0-3 are the `ep9_facts::KeySet`
/// bits; bits 4-7 the receiver ("element form" in v0: no elements kinds before H18);
/// bits 8-9 what the element access found.
pub(crate) mod keyed {
    /// An array index key: an int32 >= 0, a number that is an index, or an index string.
    pub const INDEX: u32 = 1 << 0;
    /// A string key that is not an array index.
    pub const NAME: u32 = 1 << 1;
    /// A symbol key.
    pub const SYMBOL: u32 = 1 << 2;
    /// Any other key (ToPropertyKey converts it): a number that is not an index, an oddball,
    /// an object.
    pub const OTHER_KEY: u32 = 1 << 3;
    /// The receiver is an Array.
    pub const ARRAY: u32 = 1 << 4;
    /// The receiver is another object.
    pub const OBJECT: u32 = 1 << 5;
    /// The receiver is a string.
    pub const STRING: u32 = 1 << 6;
    /// The receiver is another primitive (number, boolean, symbol, undefined, null).
    pub const PRIMITIVE: u32 = 1 << 7;
    /// An index key at or past an Array's length.
    pub const OUT_OF_BOUNDS: u32 = 1 << 8;
    /// An index key below an Array's length whose element is a hole.
    pub const HOLE: u32 = 1 << 9;
}

const _: () = {
    assert!(keyed::INDEX == KeySet::INDEX.bits() as u32);
    assert!(keyed::NAME == KeySet::NAME.bits() as u32);
    assert!(keyed::SYMBOL == KeySet::SYMBOL.bits() as u32);
    assert!(keyed::OTHER_KEY == KeySet::OTHER.bits() as u32);
    assert!(KeySet::ALL.bits() == 0xF);
};

/// Calls are recorded in the first `CALL_WINDOW` budget units after each budget reset (a
/// poll, or the isolate start).
pub(crate) const CALL_WINDOW: i64 = 64;

/// The CreateClosure count saturates here: "many" (`ClosureCount::Many`).
pub(crate) const CLOSURE_MANY: u32 = 2;

// ---- value tests (no cell access unless named) -------------------------------------------------

#[inline(always)]
fn is_i32(b: u64) -> bool {
    b & NUMBER_TAG == NUMBER_TAG
}

/// An int32 or a double.
#[inline(always)]
fn is_number(b: u64) -> bool {
    b & NUMBER_TAG != 0
}

#[inline(always)]
fn is_bool(b: u64) -> bool {
    b == TRUE || b == FALSE
}

#[inline(always)]
fn is_nullish(b: u64) -> bool {
    b == NULL || b == UNDEFINED
}

// ---- slot access ---------------------------------------------------------------------------
//
// The hooks read the vector through raw slot references: `object::cell` checks the tag and
// panics, and a panic path makes LLVM push a frame in every handler that inlines a hook.

#[inline(always)]
const fn key_off(i: u32) -> usize {
    fv::SLOTS + fv::SLOT_SIZE * i as usize + fv::KEY
}

#[inline(always)]
const fn value_off(i: u32) -> usize {
    fv::SLOTS + fv::SLOT_SIZE * i as usize + fv::VALUE
}

/// The cell Value `c` as a cell pointer, with no check.
#[inline(always)]
fn cell_ptr(c: u64) -> CellPtr {
    debug_assert!(kinds::is_cell_bits(c) && c != 0);
    let p = core::ptr::with_exposed_provenance_mut::<Word0>(c as usize);
    // SAFETY: every caller passes the bits of a live cell Value: they are its address, whose
    // provenance `Value::from_cell_ptr` exposed, and they are not 0.
    unsafe { CellPtr::from_raw(NonNull::new_unchecked(p)) }
}

/// The word at byte offset `off` of the live cell `c`.
///
/// Callers: a key or value word of a slot of the running function's vector (live: the unit's
/// root range holds it, `feedback.rs`; the verifier checked the slot operand against the
/// function's slot list, which sized the vector), or +24 of a live FUNCTION cell.
#[inline(always)]
fn word<'a>(c: u64, off: usize) -> &'a Slot {
    // SAFETY: see above.
    unsafe { cell_ptr(c).slot(off) }
}

/// The feedback vector of the running function, if it has one.
#[inline(always)]
fn vector_of(st: &St<'_>) -> Option<u64> {
    let v = feedback::vector(st.info())?;
    debug_assert!(kinds::cell_kind(v) == kinds::FEEDBACK.get());
    Some(v)
}

/// The key word (weak) of slot `i` of `v`.
#[inline(always)]
fn key(v: u64, i: u32) -> u64 {
    debug_assert!(i < feedback::slot_count(v));
    word(v, key_off(i)).bits()
}

/// The value word (strong) of slot `i` of `v`.
#[inline(always)]
fn value(v: u64, i: u32) -> u64 {
    debug_assert!(i < feedback::slot_count(v));
    word(v, value_off(i)).bits()
}

/// Stores the int32 `n` in the value half of slot `i` of `v`. `Slot::set` takes the barrier
/// only for a cell, and an int32 is a constant tag here, so this is one store.
#[inline(always)]
fn put_int(rt: &Runtime, v: u64, i: u32, n: u32) {
    // SAFETY: `v` is live (see `word`).
    let h = unsafe { cell_ptr(v).header() };
    word(v, value_off(i)).set(h, Value::int32(n as i32), rt.heap.barrier());
}

/// Stores the cell or the hole `k` in the key half (weak) of slot `i` of `v`, with the
/// barrier. Out of line: only a state change gets here.
#[cold]
#[inline(never)]
fn put_key(rt: &Runtime, v: u64, i: u32, k: u64) {
    object::wr(&rt.heap, object::cell(v), key_off(i), k);
}

/// Adds `bits` to the int32 value half of slot `s`: a load and a test, and a store only when
/// a bit is new (V9-D6).
#[inline(always)]
fn add_bits(st: &St<'_>, s: FSlot, bits: u32) {
    let Some(v) = vector_of(st) else { return };
    let old = value(v, s.0);
    debug_assert!(is_i32(old), "a profile slot holds an int32: {old:#x}");
    // A test of the old bits and an OR into the whole word (the tag is unchanged), not a
    // compare with a retagged value: no 64-bit constant in a register, so a handler that
    // records on its fast path needs no callee-saved register on x86-64.
    if old as u32 & bits != bits {
        core::hint::cold_path();
        debug_assert!(bits <= i32::MAX as u32);
        // SAFETY: `old` is an int32 and `bits` fits its low half: the result is an int32.
        let new = unsafe { Value::from_bits_unchecked(old | u64::from(bits)) };
        // E1: an int32 is not a cell, so no barrier.
        word(v, value_off(s.0)).init(new);
    }
}

/// A record that a fast body leaves to its dispatch (`interp::Flow::Record`): add `bits` to the
/// int32 value half of one slot. The fast body only computes the bits; [`record`] runs after
/// it, in `interp::dsl::record_next` (the sibling expansion: an out-of-line function that the
/// handler reaches by a tail call, so the handler itself has no record code and, on x86-64,
/// saves no register on entry) or inline (the match loop, `become`, prefixed instructions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rec(u64);

impl Rec {
    /// `bits` (not 0) for slot `s`.
    #[inline(always)]
    pub(crate) fn new(s: FSlot, bits: u32) -> Rec {
        debug_assert!(bits != 0 && bits <= i32::MAX as u32);
        Rec((u64::from(s.0) << 32) | u64::from(bits))
    }

    /// The bits and `place` (the instruction's `interp::dsl::slot_place`: where its slot
    /// operand lies and its length) as one register-wide word: the last argument of
    /// `interp::dsl::record_next`. The slot index is left out, so a handler keeps no register
    /// for it.
    #[inline(always)]
    pub(crate) fn sib_word(self, place: u64) -> u64 {
        (place << 32) | (self.0 & 0xFFFF_FFFF)
    }
}

/// A comparison record that a fast body leaves to its dispatch (`interp::Flow::RecordPair`):
/// the operand pair, whose class ([`compare_bits`]) needs cell reads, so it is computed after
/// the fast body, in `interp::dsl::record_pair_next` (sibling expansion) or inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pair {
    s: FSlot,
    l: u64,
    r: u64,
}

impl Pair {
    #[inline(always)]
    pub(crate) fn new(s: FSlot, l: u64, r: u64) -> Pair {
        Pair { s, l, r }
    }

    /// The pair (the slot is left out: `record_pair_next` reads its operand again).
    #[inline(always)]
    pub(crate) fn operands(self) -> (u64, u64) {
        (self.l, self.r)
    }
}

/// Runs the comparison record `p`: [`record_compare`].
#[inline(always)]
pub(crate) fn record_pair(st: &St<'_>, p: Pair) {
    record_compare(st, p.s, p.l, p.r);
}

/// Runs the record `r` (a load and a test; a store only when a bit is new, V9-D6).
#[inline(always)]
pub(crate) fn record(st: &St<'_>, r: Rec) {
    add_bits(st, FSlot((r.0 >> 32) as u32), r.0 as u32);
}

/// Counts one execution in the int32 value half of slot `i` of `v`, saturating at `limit`.
/// A value half that is not an int32 (a handler of a shared Call slot) is left alone: one
/// unsigned compare tests both.
#[inline(always)]
pub(crate) fn count(rt: &Runtime, v: u64, i: u32, limit: u32) {
    let n = value(v, i).wrapping_sub(NUMBER_TAG);
    if n < u64::from(limit) {
        put_int(rt, v, i, n as u32 + 1);
    }
}

/// [`count`] for a slot whose value half is always an int32 (it starts as int32 0,
/// `feedback.rs`, and only this module writes it: JumpLoop, CreateClosure). It tests only
/// the low half and adds 1 to the whole word, so it needs no 64-bit constant in a register
/// (x86-64 has none to spare in a handler without saving one).
#[inline(always)]
fn count_int32(v: u64, i: u32, limit: u32) {
    let old = value(v, i);
    debug_assert!(is_i32(old), "a count slot holds an int32: {old:#x}");
    if (old as u32) < limit {
        // SAFETY: `old` is an int32 whose low half is below `limit <= i32::MAX`, so `old + 1`
        // is the int32 one more (no carry into the tag).
        let new = unsafe { Value::from_bits_unchecked(old + 1) };
        // E1: an int32 is not a cell, so no barrier.
        word(v, value_off(i)).init(new);
    }
}

// ---- BinaryOp ------------------------------------------------------------------------------

/// True if `a OP b` is not an int32 for an int32 pair: an overflow, -0, a fraction, NaN or
/// Infinity. `*` and `/` box the result with the same `Value::number` call as
/// `interp::ops::fast_binop`, so in a handler LLVM computes it once and adds only the tag test.
#[inline(always)]
pub(crate) fn int32_pair_overflows(op: BinOp, a: i32, b: i32) -> bool {
    let boxed = |d: f64| Value::number(d).to_bits();
    match op {
        BinOp::Add => a.checked_add(b).is_none(),
        BinOp::Sub => a.checked_sub(b).is_none(),
        BinOp::Mul => !is_i32(boxed(f64::from(a) * f64::from(b))),
        BinOp::Div => !is_i32(boxed(f64::from(a) / f64::from(b))),
        // NaN for b = 0; -0 for an exact negative dividend (i32::MIN % -1 too).
        BinOp::Mod => b == 0 || (a < 0 && a.wrapping_rem(b) == 0),
        BinOp::Exp => {
            if b >= 0 {
                a.checked_pow(b as u32).is_none()
            } else {
                // 1 ** -n = 1, (-1) ** -n = ±1; 0 ** -n is Infinity, others are fractions.
                a != 1 && a != -1
            }
        }
        BinOp::Shr => (a as u32) >> (b as u32 & 31) > i32::MAX as u32,
        BinOp::BitOr | BinOp::BitXor | BinOp::BitAnd | BinOp::Shl | BinOp::Sar => false,
    }
}

/// The `BinaryHint` bit of one operand of a pair that is not two int32s.
#[inline(always)]
fn binary_operand(v: u64) -> u32 {
    let h = if is_number(v) {
        BinaryHint::NUMBER
    } else if is_nullish(v) || is_bool(v) {
        BinaryHint::ODDBALL
    } else if kinds::is_string(v) {
        BinaryHint::STRING
    } else {
        // v0 has no BigInt values; objects and symbols take the generic path.
        BinaryHint::OTHER
    };
    u32::from(h.bits())
}

/// The record of a binary operator site (BinaryOp slot) whose fast path made `v` from an
/// int32 pair: 0 (implicit), except a `*`, `/` or `>>>` whose result is not an int32 (an
/// overflow, -0, a fraction, a `>>>` past `i32::MAX`): `INT32_OVERFLOW`. An `+` or `-` that
/// overflows takes the fast path's double arm and stays implicit: its test would add
/// instructions to every int32 `+` and `-` (V9-D6); the double it makes is recorded as
/// `NUMBER` at the sites it reaches. `*` and `/` box their result with the same
/// `Value::number` as [`int32_pair_overflows`], and `>>>` gives an int32 exactly when it is at
/// most `i32::MAX`, so a test of the result's tag is that rule.
#[inline(always)]
pub(crate) fn int32_pair_bits(op: BinOp, v: u64) -> u32 {
    if matches!(op, BinOp::Mul | BinOp::Div | BinOp::Shr) && !is_i32(v) {
        core::hint::cold_path();
        u32::from(BinaryHint::INT32_OVERFLOW.bits())
    } else {
        0
    }
}

/// The record a binary operator site chooses from its operand tags before the fast path: 0 for
/// an int32 pair (implicit; see [`int32_result_bits`]), [`NUMBER_PAIR`] otherwise (used only
/// when the fast path finishes, so the pair is two numbers).
#[inline(always)]
pub(crate) fn binary_pair_bits(l: u64, r: u64) -> u32 {
    if is_i32(l) && is_i32(r) {
        0
    } else {
        NUMBER_PAIR
    }
}

/// The record of a fast binary operation on top of `pair` ([`binary_pair_bits`]) with result
/// `v`: [`int32_pair_bits`] for an int32 pair, else 0.
#[inline(always)]
pub(crate) fn int32_result_bits(op: BinOp, pair: u32, v: u64) -> u32 {
    if pair == 0 { int32_pair_bits(op, v) } else { 0 }
}

/// The record of a binary operator site whose fast path made its result from a number pair
/// that is not two int32s. Operands that are not numbers go cold, where
/// [`record_binary_cold`] records them.
pub(crate) const NUMBER_PAIR: u32 = BinaryHint::NUMBER.bits() as u32;

/// The record of a unary operator or numeric conversion site (BinaryOp slot) whose fast path
/// took a double operand ([`unary_bits`] of a double).
pub(crate) const NUMBER_OPERAND: u32 = BinaryHint::NUMBER.bits() as u32;

/// The record of a relational comparison (CompareOp slot) whose fast path took a number pair
/// that is not two int32s ([`compare_pair`] of such a pair).
pub(crate) const NUMBER_COMPARE: u32 = CompareHint::NUMBER.bits() as u32;

/// A binary operator site whose fast path failed (BinaryOp slot), before the generic operation:
/// a number pair the fast path does not finish (`%`, `**` and the bitwise operators on a
/// double, an int32 `%` that is not exact in int32, an int32 `**`) or other operands.
#[inline(always)]
pub(crate) fn record_binary_cold(st: &mut St<'_>, s: FSlot, op: BinOp, l: u64, r: u64) {
    let bits = if is_i32(l) && is_i32(r) {
        if !int32_pair_overflows(op, int32_of(l), int32_of(r)) {
            return;
        }
        BinaryHint::INT32_OVERFLOW.bits().into()
    } else if is_number(l) && is_number(r) {
        BinaryHint::NUMBER.bits().into()
    } else {
        binary_operand(l) | binary_operand(r)
    };
    add_bits(st, s, bits);
}

/// The bits a unary operator or a numeric conversion (BinaryOp slot) records for the operand
/// `v`: 0 for an int32 (implicit), `NUMBER` for a double, else the operand's class.
#[inline(always)]
pub(crate) fn unary_bits(v: u64) -> u32 {
    if is_i32(v) { 0 } else { binary_operand(v) }
}

/// A unary operator or a numeric conversion (BinaryOp slot) whose fast path failed (the
/// operand is not a number, or `~` of a double), before the generic operation:
/// [`unary_bits`].
#[inline(always)]
pub(crate) fn record_unary(st: &mut St<'_>, s: FSlot, v: u64) {
    let bits = unary_bits(v);
    if bits != 0 {
        add_bits(st, s, bits);
    }
}

// ---- CompareOp -----------------------------------------------------------------------------

/// The `CompareHint` class of one operand.
#[inline(always)]
fn compare_operand(v: u64) -> u16 {
    let h = if is_i32(v) {
        CompareHint::INT32
    } else if is_number(v) {
        CompareHint::NUMBER
    } else if is_bool(v) {
        CompareHint::BOOLEAN
    } else if is_nullish(v) {
        CompareHint::NULLISH
    } else {
        match kinds::cell_kind(v) {
            1..=7 if string::is_atom(v) => CompareHint::ATOM,
            1..=7 => CompareHint::STRING,
            8 => CompareHint::SYMBOL,
            16..=63 => CompareHint::RECEIVER,
            _ => CompareHint::OTHER,
        }
    };
    h.bits()
}

/// The `CompareHint` bits of a pair: the class both share; `NUMBER` for an int32 and a
/// double; `STRING` for an atom and another string; `RECEIVER | NULLISH` for an object and
/// null or undefined (V8's ReceiverOrNullOrUndefined, as a set); else `OTHER`.
#[inline(always)]
fn compare_pair(l: u64, r: u64) -> u32 {
    const INT_NUM: u16 = CompareHint::INT32.bits() | CompareHint::NUMBER.bits();
    const STRINGS: u16 = CompareHint::ATOM.bits() | CompareHint::STRING.bits();
    const OBJ_NULL: u16 = CompareHint::RECEIVER.bits() | CompareHint::NULLISH.bits();
    let (a, b) = (compare_operand(l), compare_operand(r));
    let h = if a == b {
        a
    } else {
        match a | b {
            INT_NUM => CompareHint::NUMBER.bits(),
            STRINGS => CompareHint::STRING.bits(),
            OBJ_NULL => OBJ_NULL,
            _ => CompareHint::OTHER.bits(),
        }
    };
    u32::from(h)
}

/// The bits a comparison (CompareOp slot) records for the pair `l`, `r`: 0 for an int32 pair
/// (implicit), else [`compare_pair`].
#[inline(always)]
pub(crate) fn compare_bits(l: u64, r: u64) -> u32 {
    if is_i32(l) && is_i32(r) {
        0
    } else {
        compare_pair(l, r)
    }
}

/// A comparison (CompareOp slot) whose fast path failed, before the generic operation:
/// [`compare_bits`].
#[inline(always)]
pub(crate) fn record_compare(st: &St<'_>, s: FSlot, l: u64, r: u64) {
    let bits = compare_bits(l, r);
    if bits != 0 {
        add_bits(st, s, bits);
    }
}

// ---- TypeOf --------------------------------------------------------------------------------

/// The `TypeSet` bit of `v`.
#[inline(always)]
fn type_of(v: u64) -> u32 {
    let t = if is_i32(v) {
        TypeSet::INT32
    } else if is_number(v) {
        TypeSet::DOUBLE
    } else if v == UNDEFINED {
        TypeSet::UNDEFINED
    } else if v == NULL {
        TypeSet::NULL
    } else if is_bool(v) {
        TypeSet::BOOLEAN
    } else {
        let k = kinds::cell_kind(v);
        if k == kinds::OBJECT.get() {
            TypeSet::PLAIN_OBJECT
        } else if k == kinds::ARRAY.get() {
            TypeSet::ARRAY
        } else if kinds::is_callable(v) {
            TypeSet::FUNCTION
        } else if (1..=7).contains(&k) {
            TypeSet::STRING
        } else if k == kinds::SYMBOL.get() {
            TypeSet::SYMBOL
        } else {
            TypeSet::OTHER_OBJECT
        }
    };
    u32::from(t.bits())
}

/// `typeof` (TypeOf slot): the operand's `TypeSet` bit.
#[inline(always)]
pub(crate) fn record_typeof(st: &mut St<'_>, s: FSlot, v: u64) {
    add_bits(st, s, type_of(v));
}

// ---- Call ----------------------------------------------------------------------------------

/// The key of a call target: the native word of its FunctionInfo (a FUNCTION cell's +24), or
/// the hole for a callee that has none (a bound function, a proxy, a value that is not
/// callable). Not a cell, so the key store needs no barrier and no call (E1), and two
/// closures of one function are one target.
#[inline(always)]
fn target_key(callee: u64) -> u64 {
    if kinds::is_function(callee) {
        word(callee, object::fun::INFO).bits()
    } else {
        HOLE
    }
}

/// A `Call*` site (Call slot) whose call went cold: called once per execution, from the cold
/// body (`interp::ops::call_record_cold`), after the fast tests of the callee failed; a call
/// that `call_fast` finishes records with [`record_call_pushed`] instead. Sampled:
/// only a call in the first [`CALL_WINDOW`] units of the budget (`cx.budget` above
/// `BUDGET - CALL_WINDOW`; the budget is reset at each poll) counts and keeps the target's
/// FunctionInfo (monomorphic) or the hole (megamorphic). The hot path is one load and one
/// test of the budget.
#[inline(always)]
pub(crate) fn record_call(st: &mut St<'_>, s: FSlot, callee: u64) {
    if st.cx.budget <= BUDGET - CALL_WINDOW {
        return;
    }
    core::hint::cold_path();
    if let Some(v) = vector_of(st) {
        record_call_in(st.rt(), v, s, callee);
    }
}

/// True if a call whose budget unit `call_fast` has just taken (`budget` is the value after
/// the decrement) is in the window of [`record_call`]: the budget before it was above
/// `BUDGET - CALL_WINDOW`.
#[inline(always)]
pub(crate) fn call_window_after_unit(budget: i64) -> bool {
    budget >= BUDGET - CALL_WINDOW
}

/// [`record_call`] of a call in the window whose frame `call_fast` has pushed: `st.fp` is the
/// callee's new frame, whose header holds the caller frame and the callee; `s` is the Call
/// slot of the caller's instruction. Same record as [`record_call`] before the push (no GC
/// point and no write to the caller's vector lies between them). Runs after the handler
/// (`interp::Flow::JumpRecordCall`, `interp::dsl::record_call_next`), so the call handlers
/// keep no register for it.
#[inline(always)]
pub(crate) fn record_call_pushed(st: &St<'_>, s: FSlot) {
    // SAFETY: `st.fp` is a frame that `call::push_frame` just wrote: its caller-fp slot holds
    // the live caller frame, whose FunctionInfo slot holds a FunctionInfo native word.
    let fi = unsafe {
        let caller = crate::frame::native_ptr::<u64>(st.hdr(crate::frame::slot::CALLER_FP));
        &*crate::frame::native_ptr::<crate::unit::FunctionInfo>(
            *caller.offset(crate::frame::slot::FUNCTION_INFO),
        )
    };
    if let Some(v) = feedback::vector(fi) {
        debug_assert!(kinds::cell_kind(v) == kinds::FEEDBACK.get());
        record_call_in(st.rt(), v, s, st.hdr(crate::frame::slot::CALLEE));
    }
}

/// The record of a sampled call: count it, and keep the target or the hole.
#[inline(always)]
fn record_call_in(rt: &Runtime, v: u64, s: FSlot, callee: u64) {
    count(rt, v, s.0, i32::MAX as u32);
    let k = key(v, s.0);
    if k != HOLE {
        let t = target_key(callee);
        if k != t {
            let new = if k == UNDEFINED { t } else { HOLE };
            // SAFETY: `new` is the hole or a native word (a FunctionInfo address plus the
            // double offset, `frame::native`): valid Value bits that are not a cell.
            let new = unsafe { Value::from_bits_unchecked(new) };
            // E1: not a cell, so no barrier.
            word(v, key_off(s.0)).init(new);
        }
    }
}

// ---- keyed ---------------------------------------------------------------------------------

/// The receiver bits of a keyed access.
#[inline(always)]
fn keyed_receiver(o: u64) -> u32 {
    if kinds::is_array(o) {
        keyed::ARRAY
    } else if kinds::is_object(o) {
        keyed::OBJECT
    } else if kinds::is_string(o) {
        keyed::STRING
    } else {
        keyed::PRIMITIVE
    }
}

/// The `KeySet` bits of a key, and for a canonical array index (an int32 `>= 0`, an index
/// string, or a Number whose `ToString` is one, -0 included) the index itself, so that every
/// form of the same index records the same element case.
#[inline(always)]
fn keyed_key(k: u64) -> (u32, Option<u32>) {
    let index = if is_i32(k) {
        u32::try_from(int32_of(k)).ok()
    } else if is_number(k) {
        // Number::toString of an integral double in [0, 2^32 - 1) is an array index.
        let d = f64::from_bits(k.wrapping_sub(ep9_value::encoding::DOUBLE_ENCODE_OFFSET));
        // `as u32` and back, not `trunc` (a libm call on baseline x86-64).
        ((0.0..4_294_967_295.0).contains(&d) && f64::from(d as u32) == d).then_some(d as u32)
    } else if kinds::is_string(k) {
        // A canonical index has at most 10 digits, so a rope (at least MIN_CONS_LENGTH
        // units) is never one and is not copied (`string::array_index`).
        return match string::array_index(k) {
            Some(i) => (keyed::INDEX, Some(i)),
            None => (keyed::NAME, None),
        };
    } else if kinds::is_symbol(k) {
        return (keyed::SYMBOL, None);
    } else {
        None
    };
    let bits = if index.is_some() {
        keyed::INDEX
    } else {
        keyed::OTHER_KEY
    };
    (bits, index)
}

/// What element `i` of the Array `o` is: past the length, a hole, or present (0).
#[inline(always)]
fn element_case(o: u64, i: u32) -> u32 {
    if i >= object::array_len(o) {
        keyed::OUT_OF_BOUNDS
    } else if object::array_get(o, i).is_none() {
        keyed::HOLE
    } else {
        0
    }
}

/// A keyed access or `in` (LoadKeyed, StoreKeyed, DefineKeyedOwn, StoreInArrayLiteral or
/// HasKeyed slot), before the fast path. Records nothing: the one keyed case that T0 runs
/// fast (an int32 index of a present Array element at `GetKeyedProperty`) is implicit, and
/// every other case goes cold, where [`record_keyed_cold`] records it. A fast keyed path that
/// a later role adds must miss on a case that is new for its site (KEYED, M4-P3).
#[inline(always)]
pub(crate) fn record_keyed(_st: &mut St<'_>, _s: FSlot, _o: u64, _k: u64) {}

/// A keyed access whose fast path failed, before the generic operation: the key and receiver
/// bits, and for an index of an Array what the element access finds (out of bounds, a hole).
#[inline(always)]
pub(crate) fn record_keyed_cold(st: &mut St<'_>, s: FSlot, o: u64, k: u64) {
    let Some(v) = vector_of(st) else { return };
    let (key, index) = keyed_key(k);
    let mut bits = key | keyed_receiver(o);
    if let Some(i) = index
        && kinds::is_array(o)
    {
        bits |= element_case(o, i);
    }
    let old = value(v, s.0);
    if old as u32 | bits != old as u32 {
        put_int(st.rt(), v, s.0, old as u32 | bits);
    }
}

// ---- JumpLoop, CreateClosure, ForIn ----------------------------------------------------------

/// A loop back edge (JumpLoop slot), before the budget decrement. Sampled: counts only a back
/// edge that uses up the interrupt budget (the handler then goes cold and polls), so the hot
/// loop path does not reach the vector.
#[inline(always)]
pub(crate) fn record_jump_loop(st: &mut St<'_>, s: FSlot) {
    if st.cx.budget <= 1
        && let Some(v) = vector_of(st)
    {
        count_int32(v, s.0, i32::MAX as u32);
    }
}

/// A closure that `CreateClosure` made (CreateClosure slot); `closure` is 0 if the creation
/// threw. Counts none, one, many.
#[inline(always)]
pub(crate) fn record_closure(st: &mut St<'_>, s: FSlot, closure: u64) {
    if closure == 0 {
        return;
    }
    if let Some(v) = vector_of(st) {
        count_int32(v, s.0, CLOSURE_MANY);
    }
}

/// The bits of `ForInPrepare` (ForIn slot): `keys` is the key array of the accumulator. v0
/// has no enum cache, so the keys always come from the generic path (`ForInEnumerate`): `ANY`.
#[inline(always)]
pub(crate) fn for_in_bits(_keys: u64) -> u32 {
    u32::from(ForInHint::ANY.bits())
}

/// `ForInPrepare` on its cold path (the accumulator is not a key array): [`for_in_bits`].
#[inline(always)]
pub(crate) fn record_for_in(st: &mut St<'_>, s: FSlot, keys: u64) {
    add_bits(st, s, for_in_bits(keys));
}

/// `ForInNext` (ForIn slot): `receiver` is the object being enumerated. The key half keeps
/// its shape (monomorphic) or the hole (a second shape); a receiver that is not an object
/// records nothing.
#[inline(always)]
pub(crate) fn record_for_in_next(st: &mut St<'_>, s: FSlot, receiver: u64) {
    if !kinds::is_object(receiver) {
        return;
    }
    let Some(v) = vector_of(st) else { return };
    let k = key(v, s.0);
    if k == HOLE {
        return;
    }
    let shape = shape::shape_of(receiver);
    if k != shape {
        put_key(st.rt(), v, s.0, if k == UNDEFINED { shape } else { HOLE });
    }
}
