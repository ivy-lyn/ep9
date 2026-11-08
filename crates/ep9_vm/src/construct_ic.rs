//! Construct sites (owner M4-P1 OBJ-ALLOC; OI-05, V9-D8, V9-A8): the Call slot (0) of a
//! `Construct*` site keeps the target (weak) and a construct-site handler: an IC_HANDLER cell of
//! handler kind `CONSTRUCT_SITE` (`ic::hk`; +16 int32 count, +24 initial shape (weak), +32
//! int32 `prototype` field index), or an int32 count (IC_LAYOUT 2, `profile.rs` table). A hit
//! is a callee compare, one field load, a shape-proto compare, the allocation and the frame push
//! with new.target; no `Vec` on a hit.
//!
//! ```text
//! interp/ops.rs   Construct           -> fast::<false>   (M4-P0 hook)
//!                 ConstructWithSpread -> fast::<true>    (always cold)
//! interp/slow/call.rs construct branch, after `this` exists -> miss
//! ```
//!
//! | State | Key (weak) | Value (strong) |
//! |---|---|---|
//! | new | undefined | int32 0 |
//! | monomorphic, cached | the constructor | a `CONSTRUCT_SITE` handler: count, the initial shape of its instances, the field index of its `prototype` |
//! | monomorphic, not cached | the callee (a bound function, a host function, a derived class, a constructor whose `prototype` is not an object, `new.target` that is not the callee, ...) | int32 count |
//! | megamorphic (sticky) | the hole | int32 count |
//!
//! Rules:
//! - The count counts the constructs of the cold path (saturating); a hit writes nothing
//!   (V9-D6). The key and the initial shape are weak (plan rule 8, H16 K7): a dead
//!   constructor reads undefined after the GC, and the next cold construct records again.
//! - The handler holds for the constructor in the key alone, so a slot that two sites share
//!   gets one right handler.
//! - A hit needs: the callee in the key, new.target = the callee (`new F`), the handler's
//!   initial shape alive, the constructor's `prototype` field (one load at the handler's field
//!   index, in a fast-mode shape) equal to the initial shape's prototype, a T0 (Interp) callee
//!   of the current realm, a budget unit and room on the VM stack. `prototype` of a
//!   constructor is non-configurable, so in fast mode it keeps its field index: a delete of an
//!   earlier property, a prototype change and `preventExtensions` all go to dictionary mode,
//!   which the hit checks.
//! - The hit is a GC point (the allocation of `this`): it publishes its frame first, with the
//!   words of `Runtime::enter` (the top frame, and its bytecode offset from the caller's
//!   FunctionInfo that the hit already holds; M7-P3 CONSTRUCT). The callee and the arguments
//!   are in caller registers, the initial shape in a Rust local. It counts the call
//!   (`count_call`, as `call_fast`) and takes one budget unit before the allocation.
//! - The hit pushes the same two frames as the cold path (H14: the stub frame that holds
//!   `this`, then the callee frame with return pc `ConstructReturn`), with one size test for
//!   both before it counts anything (M7-P3 CONSTRUCT): the stub has no arguments and no
//!   registers, so the callee frame starts [`STUB_SLOTS`] slots above this frame's top. If they
//!   do not fit, the hit goes cold before its count and its unit, and the cold path throws the
//!   RangeError. After the allocation it writes both frames in place
//!   ([`write_construct_frames`]): the words of `call::push_construct_stub` and
//!   `call::push_frame` (test `tests::construct::the_hit_frames_equal_the_cold_push`).

use crate::call::{self, CallSite};
use crate::dispatch::DispatchKind;
use crate::error::FrameRef;
use crate::feedback;
use crate::frame::{CONSTRUCT_BIT, HEADER, HOLE, UNDEFINED, int32_bits, native_of, slot};
use crate::ic::hk;
use crate::interp::dsl::{Count, FSlot, R, RList};
use crate::interp::{CONSTRUCT_RETURN_STUB, Cx, Flow, St};
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, alloc_from_shape, cell, init, obj, rd, rd_i32};
use crate::runtime::{Atom, val};
use crate::shape::{self, slf};
use crate::unit::FunctionInfo;

/// CONSTRUCT_SITE handler fields (an IC_HANDLER cell, kind 70; the `ic.rs` IC_LAYOUT 2 table).
pub(crate) mod cs {
    pub const KIND: usize = 8;
    pub const COUNT: usize = 16;
    pub const SHAPE: usize = 24;
    pub const FIELD: usize = 32;
    pub const SIZE: usize = 40;
}

/// The slots of a `[[Construct]]` stub frame: the receiver and the header. The stub's
/// FunctionInfo (`Runtime::construct_stub`, `FunctionInfo::stub`) has no parameters and no
/// registers, so its frame is these slots alone, and the callee frame starts right above them.
pub(crate) const STUB_SLOTS: usize = 1 + HEADER;

/// `Construct` (`SPREAD` false) or `ConstructWithSpread` (true): the constructor in `ctor`,
/// the arguments in `args` (`argc` registers), new.target in the accumulator, Call slot `s`.
#[inline(always)]
pub(crate) fn fast<const SPREAD: bool>(
    st: &mut St<'_>,
    ctor: R,
    args: RList,
    argc: Count,
    s: FSlot,
) -> Flow {
    if SPREAD {
        return Flow::Cold;
    }
    let f = st.reg(ctor);
    // `new F(...)`: BytecodeGen puts the callee in the accumulator as new.target.
    if st.acc != f {
        return Flow::Cold;
    }
    let caller = st.info();
    let Some(v) = feedback::vector(caller) else {
        return Flow::Cold;
    };
    if feedback::key(v, s.0) != f {
        return Flow::Cold;
    }
    let h = feedback::value(v, s.0);
    // An int32 count: not cached. A cell: the handler (only `miss` writes a cell here; the
    // key is the constructor, so `f` is a FUNCTION).
    if !is_cell_bits(h) {
        return Flow::Cold;
    }
    let Some(shape) = hit_shape(f, h) else {
        return Flow::Cold;
    };
    let fi = object::function_info(f);
    if fi.slot.kind_byte() != DispatchKind::Interp as u8 || object::function_realm(f) != st.cx.realm
    {
        return Flow::Cold;
    }
    // Both frames, one size test, before the count and the unit (module docs). `frame_top` of
    // this frame from its FunctionInfo, loaded once above.
    let stub = st
        .fp
        .wrapping_add(caller.register_count as usize + STUB_SLOTS);
    let Some(nfp) = call::call_frame_fp(stub, st.cx.stack_limit, fi, argc.0) else {
        return Flow::Cold;
    };
    if !count_call(st, fi) {
        return Flow::Cold;
    }
    st.cx.budget -= 1;
    if st.cx.budget <= 0 {
        st.cx.budget += 1;
        uncount_call(fi);
        return Flow::Cold;
    }
    let rt = st.rt();
    // `Runtime::enter(fp, pc)`, with the caller's info at hand: `pc` is the current
    // instruction of the running bytecode function `caller`, so its offset is `pc - code`.
    rt.stack().set_fp(st.fp);
    let off = st.pc.addr().wrapping_sub(caller.code.addr());
    debug_assert_eq!(caller.offset_of(st.pc), Some(off as u32));
    // SAFETY: header slot 7 of the live top frame.
    unsafe { *st.fp.offset(slot::BYTECODE_OFFSET) = int32_bits(off as i32) };
    // GC point: `f` and the arguments are in this frame's registers, `shape` in a Rust local.
    let this = alloc_from_shape(rt, kind::OBJECT, shape)
        .to_value()
        .to_bits();
    let s: &St<'_> = st;
    // SAFETY: `stub` and `nfp` are the frames that the size test above placed right above the
    // live top frame `fp` (bytecode runs with no scratch area) and below the stack limit; the
    // reader reads registers of the current verified instruction; `this` is a new object.
    unsafe {
        write_construct_frames(
            ConstructFrames {
                caller_fp: s.fp,
                next_pc: s.next,
                stub_info: &rt.construct_stub,
                stub,
                fp: nfp,
                fi,
                ctor: f,
                this,
                context: object::function_context(f),
            },
            argc.0,
            |i| s.list(args, i),
        );
    }
    st.fp = nfp;
    st.acc = UNDEFINED;
    Flow::Jump(fi.code)
}

/// The two frames of a construct hit: the stub frame at `stub` (its base is the caller's
/// frame top, [`STUB_SLOTS`] below it) and the callee frame of `fi` at `fp`, the frame that
/// `call::call_frame_fp` places at the stub's top for the call's argument count.
pub(crate) struct ConstructFrames<'a> {
    pub caller_fp: *mut u64,
    pub next_pc: *const u8,
    pub stub_info: &'a FunctionInfo,
    pub stub: *mut u64,
    pub fp: *mut u64,
    pub fi: &'a FunctionInfo,
    /// The constructor: the callee of both frames and new.target (`new F`).
    pub ctor: u64,
    pub this: u64,
    /// The constructor's context.
    pub context: u64,
}

/// Writes the frames of `w` with `argc` arguments that `arg(i)` reads: the words that
/// `call::push_construct_stub(cx, caller_fp, this, ctor, next_pc)` and then
/// `call::push_frame(stub, .., fi, ctor, this, argc, arg, stub, CONSTRUCT_RETURN_STUB,
/// context, ctor)` write (H14 "`[[Construct]]` from JS"). The stub has no arguments and no
/// registers (`stub_info`), so it is 9 stores; the callee frame gets the construct bit and
/// new.target = the constructor.
///
/// # Safety
/// `w.stub` is `w.caller_fp`'s frame top + [`STUB_SLOTS`], `w.fp` is
/// `call::call_frame_fp(w.stub, limit, w.fi, argc)` (not `None`) for a `limit` at most the
/// stack end, every slot of `[frame top of caller, w.fp + register count)` is above every live
/// frame, `w.stub_info` has no parameters and no registers, and `arg(i)` returns valid Value
/// bits for `i < argc`.
#[inline(always)]
pub(crate) unsafe fn write_construct_frames(
    w: ConstructFrames<'_>,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) {
    debug_assert!(w.stub_info.parameter_count == 0 && w.stub_info.register_count == 0);
    let (stub, fp) = (w.stub, w.fp);
    // SAFETY: the slots lie in the two frames that the caller's size test placed above every
    // live frame and below the stack limit (caller contract), so these writes clobber nothing
    // live.
    unsafe {
        *stub.offset(slot::RECEIVER) = w.this;
        *stub.offset(slot::CALLER_FP) = native_of(w.caller_fp);
        *stub.offset(slot::RETURN_PC) = native_of(w.next_pc);
        *stub.offset(slot::CALLEE) = w.ctor;
        *stub.offset(slot::ARGC) = int32_bits(0);
        *stub.offset(slot::FUNCTION_INFO) = native_of(w.stub_info);
        *stub.offset(slot::NEW_TARGET) = UNDEFINED;
        *stub.offset(slot::CONTEXT) = UNDEFINED;
        *stub.offset(slot::BYTECODE_OFFSET) = int32_bits(0);
        *fp.offset(slot::CALLER_FP) = native_of(stub);
        *fp.offset(slot::RETURN_PC) = native_of(CONSTRUCT_RETURN_STUB.as_ptr());
        *fp.offset(slot::CALLEE) = w.ctor;
        *fp.offset(slot::ARGC) = int32_bits((argc | CONSTRUCT_BIT) as i32);
        *fp.offset(slot::FUNCTION_INFO) = native_of(w.fi);
        *fp.offset(slot::NEW_TARGET) = w.ctor;
        *fp.offset(slot::CONTEXT) = w.context;
        *fp.offset(slot::BYTECODE_OFFSET) = int32_bits(0);
        *fp.offset(slot::RECEIVER) = w.this;
        // The arguments (`a<i>` at `PARAM0 - i`), then undefined for the missing ones (from the
        // stub's top, the callee frame's base) and for the registers.
        let mut p = fp.offset(slot::PARAM0);
        let end = p.wrapping_sub(argc as usize);
        let mut i = 0;
        while p > end {
            // Volatile as in `call::write_call_args`: a plain store; LLVM makes no vector loop.
            p.write_volatile(arg(i));
            p = p.wrapping_sub(1);
            i += 1;
        }
        fill_undefined(stub, end.wrapping_add(1));
        fill_undefined(fp, fp.wrapping_add(w.fi.register_count as usize));
    }
}

/// Undefined in every slot of `[from, to)`, one plain store per slot (the loop of
/// `call::push_frame`: these ranges are short, and LLVM's vector loop costs more set-up).
///
/// # Safety
/// `[from, to)` is writable stack memory.
#[inline(always)]
unsafe fn fill_undefined(from: *mut u64, to: *mut u64) {
    let mut p = from;
    while p < to {
        // SAFETY: `p` is in `[from, to)` (caller contract).
        unsafe { p.write_volatile(UNDEFINED) };
        p = p.wrapping_add(1);
    }
}

/// The initial shape of the handler `h` for the constructor `f`, if it is alive and still
/// has `f.prototype` as its prototype.
#[inline(always)]
fn hit_shape(f: u64, h: u64) -> Option<u64> {
    let hc = cell(h);
    let shape = rd(hc, cs::SHAPE);
    if shape == UNDEFINED {
        return None;
    }
    let fs = shape::shape_of(f);
    if shape::is_dictionary(fs) {
        return None;
    }
    let field = rd_i32(hc, cs::FIELD) as usize;
    let (cap, k) = shape::field_layout(fs);
    let fc = cell(f);
    let p = if field < cap {
        rd(fc, obj::FIXED + 8 * (object::fixed_count(k) + field))
    } else {
        rd(cell(rd(fc, obj::SLOTS)), slf::DATA + 8 * (field - cap))
    };
    (shape::proto(shape) == p).then_some(shape)
}

/// The T0 count of a call of `fi` while it has no feedback vector (`interp::ops::count_call`):
/// `false` for the call that reaches the threshold, which goes cold so that `slow::call`
/// makes the vector.
#[inline(always)]
fn count_call(st: &St<'_>, fi: &FunctionInfo) -> bool {
    if fi.feedback.get().is_null() {
        // Without a vector, `invocations` is below the threshold, so `+ 1` never overflows.
        let calls = fi.invocations.get() + 1;
        if calls >= st.cx.feedback_threshold {
            return false;
        }
        fi.invocations.set(calls);
    }
    true
}

/// Takes back the count of [`count_call`] before a cold exit (`interp::ops::uncount_call`).
#[inline(always)]
fn uncount_call(fi: &FunctionInfo) {
    if fi.feedback.get().is_null() {
        fi.invocations.set(fi.invocations.get() - 1);
    }
}

/// `value` + 1 as an int32 count (saturating): the value's own count, or a handler's.
fn next_count(value: u64) -> u64 {
    let n = if is_cell_bits(value) {
        rd_i32(cell(value), cs::COUNT)
    } else if val(value).is_int32() {
        crate::frame::int32_of(value)
    } else {
        0
    };
    int32_bits(n.saturating_add(1).max(1))
}

/// The cold construct path of `site` in frame `fp` (`slow::call`, construct branch) made the
/// receiver `this` (the hole for a derived constructor) for the bytecode constructor `callee`
/// of the current realm with `new_target`. Records the site's state (table above). GC point
/// (a new handler cell): every Value here is in a caller register or a Rust local.
#[inline]
pub(crate) fn miss(
    cx: &mut Cx,
    fp: *mut u64,
    site: &CallSite,
    callee: u64,
    new_target: u64,
    this: u64,
) {
    let rt = cx.rt();
    let Some(v) = feedback::vector(FrameRef(fp).info()) else {
        return;
    };
    let i = site.slot;
    if i >= feedback::slot_count(v) {
        debug_assert!(false, "a construct slot past the vector");
        return;
    }
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    let count = next_count(value);
    let target = site.callee;
    if key == HOLE || (key != UNDEFINED && key != target) {
        // Megamorphic for good.
        feedback::set_slot(rt, v, i, HOLE, count);
        return;
    }
    let Some((shape, field)) = cacheable(cx, site, callee, new_target, this) else {
        feedback::set_slot(rt, v, i, target, count);
        return;
    };
    if is_cell_bits(value) && key == target {
        // The handler of `target` already: the new shape (the old one died, or `prototype`
        // changed) and the count.
        let c = cell(value);
        object::wr(&rt.heap, c, cs::SHAPE, shape);
        object::wr(&rt.heap, c, cs::FIELD, int32_bits(field as i32));
        object::wr(&rt.heap, c, cs::COUNT, count);
        return;
    }
    // GC point: `shape` lives in a Rust local; `target` in the caller's callee register.
    let c = rt.alloc(kind::IC_HANDLER, cs::SIZE);
    init(c, cs::KIND, int32_bits(hk::CONSTRUCT_SITE));
    init(c, cs::COUNT, count);
    init(c, cs::SHAPE, shape);
    init(c, cs::FIELD, int32_bits(field as i32));
    // The GC may have cleared a dead key; the site is still this target's.
    feedback::set_slot(rt, v, i, target, c.to_value().to_bits());
}

/// (initial shape, `prototype` field index) if a hit can make this construct's receiver: the
/// site's callee is `callee` itself (not bound) and new.target too, `this` is a new plain
/// object (not the hole of a derived constructor) whose shape's prototype is the value of
/// `callee`'s own `prototype` data property in its fast-mode shape. Not a GC point.
fn cacheable(
    cx: &Cx,
    site: &CallSite,
    callee: u64,
    new_target: u64,
    this: u64,
) -> Option<(u64, usize)> {
    if site.spread || site.callee != callee || new_target != callee || this == HOLE {
        return None;
    }
    if !kind::is_function(callee) || !kind::is_object(this) {
        return None;
    }
    let shape = shape::shape_of(this);
    let (_, k) = shape::field_layout(shape);
    if k != kind::OBJECT.get() || shape::is_dictionary(shape) || shape::count(shape) != 0 {
        return None;
    }
    let fs = shape::shape_of(callee);
    if shape::is_dictionary(fs) {
        return None;
    }
    let found = shape::lookup(cx.rt(), fs, cx.rt().atom(Atom::Prototype))?;
    if found.details.is_accessor() {
        return None;
    }
    let field = usize::from(found.details.field());
    let p = object::own_get(callee, cx.rt().atom(Atom::Prototype))?.0;
    (kind::is_object(p) && shape::proto(shape) == p).then_some((shape, field))
}

/// The int32 at `off` of the handler `h` (tests).
#[cfg(test)]
pub(crate) fn handler_i32(h: u64, off: usize) -> i32 {
    rd_i32(cell(h), off)
}
