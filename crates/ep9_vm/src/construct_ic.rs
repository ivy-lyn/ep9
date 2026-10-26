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
//! - The hit is a GC point (the allocation of `this`): it publishes its frame first
//!   (`Runtime::enter`). The callee and the arguments are in caller registers, the initial
//!   shape in a Rust local. It counts the call (`count_call`, as `call_fast`) and takes one
//!   budget unit before the allocation, and gives both back if the frames do not fit; the
//!   cold path then redoes the whole construct (the new object is garbage).

use crate::call::{self, CallSite};
use crate::dispatch::DispatchKind;
use crate::error::FrameRef;
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::ic::hk;
use crate::interp::dsl::{Count, FSlot, R, RList};
use crate::interp::{CONSTRUCT_RETURN_STUB, Cx, Flow, St};
use crate::kinds::{self as kind, is_cell_bits};
use crate::literal_site::alloc_with_shape;
use crate::object::{self, cell, init, obj, rd, rd_i32};
use crate::runtime::{Atom, frame_top, val};
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
    let Some(v) = feedback::vector(st.info()) else {
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
    rt.enter(st.fp, st.pc);
    // GC point: `f` and the arguments are in this frame's registers, `shape` in a Rust local.
    let this = alloc_with_shape(rt, kind::OBJECT, shape)
        .to_value()
        .to_bits();
    // SAFETY: `fp` is the live top frame (bytecode runs with no scratch area) and `this` is a
    // new object.
    let stub = unsafe { call::push_construct_stub(st.cx, st.fp, this, f, st.next) };
    let nfp = stub.and_then(|stub| {
        let (limit, context) = (st.cx.stack_limit, object::function_context(f));
        let s: &St<'_> = st;
        // SAFETY: `stub` is the new top frame; the callee frame goes right above it, and the
        // reader reads registers of the current verified instruction.
        unsafe {
            call::push_frame(
                frame_top(stub),
                limit,
                fi,
                f,
                this,
                argc.0,
                |i| s.list(args, i),
                stub,
                CONSTRUCT_RETURN_STUB.as_ptr(),
                context,
                f,
            )
        }
    });
    match nfp {
        Some(n) => {
            st.fp = n;
            st.acc = UNDEFINED;
            Flow::Jump(fi.code)
        }
        None => {
            // The cold path throws the RangeError (and takes the unit and the count again).
            st.cx.budget += 1;
            uncount_call(fi);
            Flow::Cold
        }
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
