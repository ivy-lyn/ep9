//! Feedback (H16, OM-12 to OM-14): the lazy FeedbackVector of a function and its unit's slots.
//!
//! ```text
//! frame fp-4 ──> FunctionInfo.feedback ──> unit slot (UnitFeedback: a strong root range,
//!                 (null: no vector yet)     │   one Value per function, like `consts`)
//!                                           └─> FEEDBACK cell (kind 68)
//!                                                 +8   slot count (int32)
//!                                                 +16  FunctionInfo (native word)
//!                                                 +24  slot 0: key (weak) | value (strong)
//!                                                 +40  slot 1: ...       16 B per slot
//! ```
//!
//! Frame slot fp-3 keeps `new.target` (H14-Q1); T0 reaches the vector through the
//! FunctionInfo (fp-4), so a frame that is already running sees a new vector at once.
//!
//! When a function gets its vector (OM-13; `t` = `IsolateConfig::feedback_threshold`):
//!
//! | Where | What |
//! |---|---|
//! | T0 call fast path (`interp::ops::call_fast`) | counts the call; the call that reaches `t` goes cold |
//! | `slow::call` (every `Call*`/`Construct*` cold path) | [`count_call`]: makes the vector before the frame push, while the callee and the arguments are in caller registers |
//! | `interp::run` (an entry from Rust: scripts, host callbacks) | [`count_call`] with the entry frame pushed |
//! | `JumpLoop` budget poll (`slow::interrupt_at`) | [`ensure`]: a running function that has none gets one |
//!
//! `t = 0` makes the vector at the first call; `t = u32::MAX` never makes one (not even at a
//! poll). Every slot starts as (undefined, int32 0), except an IC slot that sites with
//! different names share: it starts megamorphic (`ic::init_vector`). Host functions and VM
//! stubs never get a vector. The key half is weak (cleared to undefined when its cell dies);
//! the value half is strong (OM-14). `ic.rs` reads and writes the slots of the named-property
//! and global sites.

use core::ptr::NonNull;

use ep9_heap::{AllocError, CellKind, CellPtr, CellSize, Heap, RootStrength, Tracer};
use ep9_value::Value;

use crate::frame::{int32_bits, native_of};
use crate::kinds;
use crate::object::{cell, init, rd, rd_i32, wr};
use crate::runtime::Runtime;
use crate::unit::{FunctionInfo, LoadedUnit};

/// One function's feedback slot: a Value (undefined or its FEEDBACK cell) in the unit's root
/// range.
pub(crate) type Slot = ep9_heap::Slot;

/// FEEDBACK cell layout (byte offsets).
pub(crate) mod fv {
    /// Slot count (int32).
    pub const COUNT: usize = 8;
    /// The FunctionInfo of the vector's function (native word).
    pub const INFO: usize = 16;
    /// First slot.
    pub const SLOTS: usize = 24;
    /// Bytes per slot.
    pub const SLOT_SIZE: usize = 16;
    /// Key word of a slot (weak).
    pub const KEY: usize = 0;
    /// Value word of a slot (strong).
    pub const VALUE: usize = 8;
}

/// A function with more feedback slots than this runs without a vector (a 256 MiB cell).
const MAX_SLOTS: usize = 1 << 24;

/// The feedback slots of one unit (`LoadedUnit::feedback`), one per function. Each holds
/// undefined until that function's vector exists.
pub(crate) struct UnitFeedback {
    slots: Box<[Slot]>,
}

impl UnitFeedback {
    /// The feedback of a unit with `functions` functions, made in `LoadedUnit::load` before
    /// the unit's first allocation. Registers the slots as a strong root range.
    pub(crate) fn new(rt: &Runtime, functions: usize) -> UnitFeedback {
        let slots: Box<[Slot]> = (0..functions)
            .map(|_| Slot::new(Value::UNDEFINED))
            .collect();
        // SAFETY: the slots live in a box that moves with this value into its LoadedUnit,
        // which the isolate keeps until the Runtime drops (`Runtime::units`). The heap is the
        // Runtime's first field, so it drops (with this range) before the box. Moving the box
        // does not move its slots. They hold `undefined` now and Value bits later (never 0).
        unsafe {
            rt.heap
                .add_root_range(NonNull::from(&slots[..]), RootStrength::Strong);
        }
        UnitFeedback { slots }
    }
}

/// The feedback slot of function `function_id` of `unit`, if it has one.
pub(crate) fn unit_slot(unit: &LoadedUnit, function_id: u32) -> Option<&Slot> {
    unit.feedback.slots.get(function_id as usize)
}

/// The FEEDBACK cell of `fi`, if it has one.
#[inline(always)]
pub(crate) fn vector(fi: &FunctionInfo) -> Option<u64> {
    let p = fi.feedback.get();
    // SAFETY: a non-null `feedback` points at a slot of the UnitFeedback of `fi`'s unit
    // ([`make`]), which lives as long as the isolate.
    (!p.is_null()).then(|| unsafe { (*p).bits() })
}

/// Counts one call of the bytecode function `fi` that has no vector yet, and makes the vector
/// on the call that reaches `threshold` (OM-13). GC point.
///
/// The caller has recorded its frame (`Runtime::enter`), and every Value it still needs is in
/// a frame, a root or a Rust local.
#[inline]
pub(crate) fn count_call(rt: &Runtime, threshold: u32, fi: &FunctionInfo) {
    if !fi.feedback.get().is_null() {
        return;
    }
    // Without a vector, `invocations` is below `threshold` (so at most u32::MAX - 1).
    let calls = fi.invocations.get() + 1;
    if calls < threshold {
        fi.invocations.set(calls);
    } else if threshold == u32::MAX {
        // Never: start the count again. The T0 fast path goes cold once per 2^32 - 1 calls.
        fi.invocations.set(0);
    } else {
        fi.invocations.set(calls);
        make(rt, fi);
    }
}

/// Makes the vector of the running bytecode function `fi` if it has none, unless the
/// threshold is `u32::MAX`. Callers: the budget poll at a `JumpLoop` (OM-13) or at a call
/// (`slow::call`), and the entry of a unit's entry code. GC point; same contract as
/// [`count_call`].
pub(crate) fn ensure(rt: &Runtime, threshold: u32, fi: &FunctionInfo) {
    if threshold != u32::MAX && fi.feedback.get().is_null() {
        make(rt, fi);
    }
}

/// Allocates the vector of the bytecode function `fi` (every slot (undefined, int32 0), a
/// shared IC slot megamorphic), stores it in the unit's slot and links `fi` to that slot. GC
/// point.
///
/// A function with more than [`MAX_SLOTS`] slots, or whose vector is too large for one
/// cell, gets none: its count starts again, so `invocations` stays below the threshold.
fn make(rt: &Runtime, fi: &FunctionInfo) {
    let Some(unit) = fi.loaded_unit() else {
        debug_assert!(false, "a feedback vector for a function without a unit");
        return;
    };
    let id = fi.function_id;
    let n = unit.unit.functions[id as usize].feedback_slots.len();
    if n > MAX_SLOTS {
        fi.invocations.set(0);
        return;
    }
    let c = match rt
        .heap
        .alloc(kinds::FEEDBACK, fv::SLOTS + fv::SLOT_SIZE * n)
    {
        Ok(c) => c,
        Err(AllocError::TooLarge) => {
            fi.invocations.set(0);
            return;
        }
        // v0: out of memory is fatal, as in `Runtime::alloc`.
        Err(e) => panic!("ep9: heap allocation failed: {e:?}"),
    };
    // E2: `c` is new and no GC point follows until it is linked. Keys stay `undefined` (the
    // allocator's pre-fill).
    init(c, fv::COUNT, int32_bits(n as i32));
    init(c, fv::INFO, native_of(fi));
    for i in 0..n {
        init(c, slot_offset(i) + fv::VALUE, int32_bits(0));
    }
    // A slot that sites with different names share gets no IC (still no GC point).
    crate::ic::init_vector(
        fi,
        &unit.unit.functions[id as usize].bytecode,
        c.to_value().to_bits(),
    );
    let slot = unit_slot(unit, id).expect("one feedback slot per unit function");
    // E3: a store into a root takes no barrier.
    slot.init(c.to_value());
    fi.feedback.set(slot);
}

#[inline(always)]
const fn slot_offset(i: usize) -> usize {
    fv::SLOTS + fv::SLOT_SIZE * i
}

/// The number of slots of the vector `v`.
#[inline(always)]
pub(crate) fn slot_count(v: u64) -> u32 {
    rd_i32(cell(v), fv::COUNT) as u32
}

/// The key word (weak) of slot `i` of the vector `v`.
#[inline(always)]
pub(crate) fn key(v: u64, i: u32) -> u64 {
    debug_assert!(i < slot_count(v));
    rd(cell(v), slot_offset(i as usize) + fv::KEY)
}

/// The value word (strong) of slot `i` of the vector `v`.
#[inline(always)]
pub(crate) fn value(v: u64, i: u32) -> u64 {
    debug_assert!(i < slot_count(v));
    rd(cell(v), slot_offset(i as usize) + fv::VALUE)
}

/// Sets slot `i` of the vector `v` to (`key`, `value`), with the barrier.
pub(crate) fn set_slot(rt: &Runtime, v: u64, i: u32, key: u64, value: u64) {
    debug_assert!(i < slot_count(v));
    let c = cell(v);
    let off = slot_offset(i as usize);
    wr(&rt.heap, c, off + fv::KEY, key);
    wr(&rt.heap, c, off + fv::VALUE, value);
}

// ---- kind ----------------------------------------------------------------------------------

fn vector_size(c: CellPtr) -> usize {
    slot_offset(rd_i32(c, fv::COUNT) as usize)
}

fn trace_vector(c: CellPtr, t: &mut Tracer<'_>) {
    let n = rd_i32(c, fv::COUNT) as usize;
    // SAFETY: the heap passes a live FEEDBACK cell; its words [24, 24 + 16 n) are n
    // (key, value) Value pairs.
    let slots = unsafe { c.slots(fv::SLOTS, 2 * n) };
    for [key, value] in slots.as_chunks::<2>().0 {
        t.visit_weak(key);
        t.visit(value);
    }
}

/// Registers the FEEDBACK cell kind.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kinds::FEEDBACK,
        CellKind {
            name: "feedback",
            size: CellSize::Dynamic(vector_size),
            trace: trace_vector,
        },
    )
    .expect("register the FEEDBACK kind");
}

/// The vector's FunctionInfo (the +16 native word).
#[cfg(test)]
pub(crate) fn info_of<'a>(v: u64) -> &'a FunctionInfo {
    // SAFETY: +16 of a FEEDBACK cell holds the native word of a unit FunctionInfo, which lives
    // as long as the isolate.
    unsafe { &*crate::frame::native_ptr::<FunctionInfo>(rd(cell(v), fv::INFO)) }
}

/// True if `v` is a FEEDBACK cell.
#[cfg(test)]
pub(crate) fn is_vector(v: u64) -> bool {
    kinds::cell_kind(v) == kinds::FEEDBACK.get()
}
