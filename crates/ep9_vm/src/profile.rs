#![allow(dead_code)] // m4-stub
//! T0 profile recording (owner M4-P1 PROFILE; OI-06, JA-3, H16-Q2) and the IC_LAYOUT 2 feedback
//! slot table (H16; `versions::IC_LAYOUT_VERSION` 2, M4-P0).
//!
//! **IC_LAYOUT 2 slot table.** Each slot of a FEEDBACK cell (`feedback.rs`, 16 B) is a key half
//! (weak) and a value half (strong). One module writes each slot kind:
//!
//! | Slot kind (ABI byte) | Key half (weak) | Value half (strong) | Writer module (role, phase) |
//! |---|---|---|---|
//! | Call (0) at `Call*` sites | the target function, or the hole = megamorphic | int32 call count (saturating) | `profile.rs` (PROFILE, M4-P1) |
//! | Call (0) at `Construct*` sites | the target function, or the hole | a construct-site handler (an IC_HANDLER cell, handler kind `CONSTRUCT_SITE`: +16 int32 count, +24 initial shape (weak), +32 int32 `prototype` field index; V9-A8), or an int32 count | `construct_ic.rs` (OBJ-ALLOC, M4-P1) |
//! | LoadNamed, StoreNamed, DefineNamedOwn, LoadGlobal*, StoreGlobal (1, 3, 5, 7-9) | as IC_LAYOUT 1 | as IC_LAYOUT 1; the prototype cases keep a validity cell, not the epoch | `ic.rs` (VALIDITY M4-P1, IC2 M4-P3) |
//! | LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed (2, 4, 6, 10) | the receiver shape (monomorphic) or the hole | int32 bits: element form seen, key types (int32, string, symbol), out of bounds, hole read | `profile.rs` writes the bits (M4-P1); `keyed_ic.rs` reads them (KEYED, M4-P3) |
//! | BinaryOp, CompareOp (11, 12) | undefined | int32 hint bits (int32, number, string, bigint, other) | `profile.rs` |
//! | TypeOf (13) | undefined | int32 type bits | `profile.rs` |
//! | InstanceOf (14) | the rhs shape | an instanceof handler (an IC_HANDLER cell, handler kind `INSTANCEOF`: +16 validity cell, +24 undefined, +32 int32 `prototype` field index; V9-A8), or an int32 state | `instance_ic.rs` (IC2, M4-P3) |
//! | Literal, CloneObject (15, 16) | the result shape | int32 count | `literal_site.rs` (OBJ-ALLOC, M4-P1) |
//! | StoreInArrayLiteral, ForIn, JumpLoop, CreateClosure (17-20) | ForIn: the receiver shape; others undefined | int32 bits or count | `profile.rs` |
//!
//! Rules: a slot is written only when its bit or state is new (load, test, rare store;
//! V9-D6). A hit path never allocates and never calls (H16:103). The IC_HANDLER layout under
//! IC_LAYOUT 2 is in the `ic.rs` module docs.
//!
//! **Record points** (`interp/ops.rs` calls them; ops.rs has no owner after M4-P0, V9-D5).
//! Each hook is `#[inline(always)]` and empty in M4-P0, so T0 does not change. A fast-body hook
//! runs once per execution of its instruction (a cold body runs only after its fast body
//! returned `Flow::Cold`, `interp::dsl`); a `*_cold` hook runs on the cold path only, after
//! the fast-body hook of the same execution.
//!
//! | Hook | Called from | Arguments |
//! |---|---|---|
//! | [`record_binary`] | the fast body of the 12 binary and 12 `*Smi` operators (`binop_fast`) and the prefixed `*Smi` forms (`scaled_fast`) | slot, operator, left, right |
//! | [`record_binary_cold`] | their cold body (`binop_cold`): an int32 overflow, a `%` that is not exact, or other types | slot, operator, left, right |
//! | [`record_unary`] | `Inc`, `Dec`, `Negate`, `BitwiseNot`, `ToNumber`, `ToNumeric` (BinaryOp slots) | slot, operand |
//! | [`record_compare`] | `TestEqual`, `TestEqualStrict`, `TestLessThan`, `TestGreaterThan`, `TestLessThanOrEqual`, `TestGreaterThanOrEqual` | slot, left, right |
//! | [`record_typeof`] | `TypeOf` | slot, operand |
//! | [`record_call`] | every `Call*` site except `CallRuntime`: at the start of `call_fast`, or before the cold exit of a site that does not reach it (`CallWithSpread`, a `CallProperty`/`CallAnyReceiver` list without a receiver) | slot, callee |
//! | [`record_keyed`] | `GetKeyedProperty` (before the array path), `SetKeyedProperty`, `DefineKeyedOwnProperty`, `DefineKeyedOwnPropertyInLiteral` (its fast body is only this hook and `Flow::Cold`), `StaInArrayLiteral`, `TestIn` | slot, receiver, key |
//! | [`record_keyed_cold`] | their cold bodies (out of bounds, a hole, a non-array receiver; every execution of `DefineKeyedOwnPropertyInLiteral`) | slot, receiver, key |
//! | [`record_jump_loop`] | `JumpLoop` | slot |
//! | [`record_closure`] | the `CreateClosure` cold body, after the closure exists (0 = an exception is pending) | slot, the closure bits |
//! | [`record_for_in`] | `ForInPrepare` (the key array in the accumulator) | slot, the key array |
//! | [`record_for_in_next`] | `ForInNext` | slot, receiver |

use crate::conv::BinOp;
use crate::interp::St;
use crate::interp::dsl::FSlot;

/// A binary operator site (BinaryOp slot), before the fast path.
#[inline(always)]
pub(crate) fn record_binary(_st: &mut St<'_>, _s: FSlot, _op: BinOp, _l: u64, _r: u64) {}

/// A binary operator site whose fast path failed (BinaryOp slot), before the generic operation.
#[inline(always)]
pub(crate) fn record_binary_cold(_st: &mut St<'_>, _s: FSlot, _op: BinOp, _l: u64, _r: u64) {}

/// A unary operator or a numeric conversion (BinaryOp slot), before the fast path.
#[inline(always)]
pub(crate) fn record_unary(_st: &mut St<'_>, _s: FSlot, _v: u64) {}

/// A comparison (CompareOp slot), before the fast path.
#[inline(always)]
pub(crate) fn record_compare(_st: &mut St<'_>, _s: FSlot, _l: u64, _r: u64) {}

/// `typeof` (TypeOf slot).
#[inline(always)]
pub(crate) fn record_typeof(_st: &mut St<'_>, _s: FSlot, _v: u64) {}

/// A `Call*` site (Call slot), once per execution, before any test of the callee.
#[inline(always)]
pub(crate) fn record_call(_st: &mut St<'_>, _s: FSlot, _callee: u64) {}

/// A keyed access or `in` (LoadKeyed, StoreKeyed, DefineKeyedOwn, StoreInArrayLiteral or
/// HasKeyed slot), before the fast path.
#[inline(always)]
pub(crate) fn record_keyed(_st: &mut St<'_>, _s: FSlot, _o: u64, _k: u64) {}

/// A keyed access whose fast path failed, before the generic operation.
#[inline(always)]
pub(crate) fn record_keyed_cold(_st: &mut St<'_>, _s: FSlot, _o: u64, _k: u64) {}

/// A loop back edge (JumpLoop slot), before the budget decrement.
#[inline(always)]
pub(crate) fn record_jump_loop(_st: &mut St<'_>, _s: FSlot) {}

/// A closure that `CreateClosure` made (CreateClosure slot); `closure` is 0 if the creation
/// threw.
#[inline(always)]
pub(crate) fn record_closure(_st: &mut St<'_>, _s: FSlot, _closure: u64) {}

/// `ForInPrepare` (ForIn slot): `keys` is the key array of the accumulator.
#[inline(always)]
pub(crate) fn record_for_in(_st: &mut St<'_>, _s: FSlot, _keys: u64) {}

/// `ForInNext` (ForIn slot): `receiver` is the object being enumerated.
#[inline(always)]
pub(crate) fn record_for_in_next(_st: &mut St<'_>, _s: FSlot, _receiver: u64) {}
