//! Literal sites (owner M4-P1 OBJ-ALLOC; OI-03, the V9-D8 pattern, V9-A8): the Literal
//! feedback slot (15; IC_LAYOUT 2, `profile.rs` table) keeps the result shape (weak) and an
//! int32 count; no cell. The CloneObject slot (16) keeps the source shape (weak) and a clone
//! handler (an IC_HANDLER cell of handler kind `CLONE`, `ic::hk`: +16 int32 count, +24 the
//! result shape (weak), +32 int32 0), or the hole and a POLY_IC cell (2-4 cases: source shape
//! (weak), clone handler) or the int32 [`ic::MEGA`]. A hit allocates with the result shape and
//! copies the constants (Literal) or the source's fields (CloneObject); computed or nested
//! values stay off the T0 hit. An object literal whose nested values are all shallow arrays
//! (splay's leaf nodes) is built on its cold path from the recorded shape ([`cold_object`]),
//! not by the generic per-key define.
//!
//! ```text
//! interp/ops.rs fast bodies (M4-P0 hooks)          cold path
//!   CreateObjectLiteral       -> fast_object       slow::literal  -> cold_object, miss (slow/literal.rs)
//!   CreateArrayLiteral        -> fast_array        slow::literal  -> miss
//!   CreateEmptyArrayLiteral   -> fast_empty_array  slow::literal  -> miss
//!   CreateEmptyObjectLiteral  -> fast_empty_object slow::literal  (no slot: always cold)
//!   CloneObject               -> fast_clone        slow::clone_object -> miss_clone
//! ```
//!
//! Slot states:
//!
//! | Slot | Key (weak) | Value (strong) | Hit |
//! |---|---|---|---|
//! | Literal, new | undefined | int32 0 | none |
//! | Literal, seen | the shape of the last result the cold path made | int32 count of cold-path results (saturating) | object: a shallow boilerplate whose result shape is the key; array: a shallow boilerplate (or `[]`) and the realm's array shape |
//! | CloneObject, new | undefined | int32 0 | none |
//! | CloneObject, monomorphic | the source shape | a `CLONE` handler | a source with that shape |
//! | CloneObject, polymorphic | the hole | a POLY_IC cell, 2-4 cases | a source with a case's shape |
//! | CloneObject, megamorphic | the hole | int32 [`ic::MEGA`] (sticky) | none |
//!
//! Rules:
//! - The key half and a handler's result shape are weak (plan rule 8, H16 K7): a dead shape
//!   reads undefined after the GC, and the next cold run records the new one.
//! - The count counts the results of the cold path only. A hit writes nothing (V9-D6: a slot
//!   is written only when its state is new), so a count is a lower bound of the allocations.
//! - A hit is a GC point (the allocation): it publishes its frame first (`Runtime::enter`), as
//!   a cold body does. Every Value it holds is in a Rust local (the conservative scan) or in a
//!   frame register, and the shape it allocates with lives in a Rust local, so it stays alive.
//! - A Literal hit checks what keeps it memory safe and right on its own: the shallow flag
//!   (bit 0 of the instruction's flags), the key's prototype (the current realm's
//!   %Object.prototype% or %Array.prototype%, as the cold path uses), and for an object the
//!   key's property count against the boilerplate length and its in-object capacity. The key
//!   is recorded only from the cold result of a site whose slot no other instruction uses
//!   ([`exclusive_slot`]), so the key's properties are the boilerplate's keys in order.
//! - A BigInt constant (the hole marker, `unit.rs`) is not a value: a hit that meets one goes
//!   cold, and the cold path throws its "not implemented".
//! - A CloneObject case is recorded only for a source that a hit can copy field by field: a
//!   fast plain object whose properties are all enumerable data properties, whose result
//!   has the same keys in the same order, all in-object.

use ep9_bytecode::{Constant, Instructions, Oddball, OperandKind};

use crate::error::FrameRef;
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits, int32_of};
use crate::ic::{self, hk};
use crate::interp::dsl::{FSlot, K};
use crate::interp::slow::Lit;
use crate::interp::{Cx, Flow, St};
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, alloc_from_shape, arr, attr, cell, elems, init, obj, rd};
use crate::runtime::{Root, Runtime, val};
use crate::shape::{self, slf};
use crate::unit::FunctionInfo;

/// Bit 0 of the flags of `CreateObjectLiteral` and `CreateArrayLiteral`: the boilerplate has no
/// nested boilerplate (opcodes.rs).
const SHALLOW: u8 = 1;
/// Bit 0 of the flags of `CloneObject`: the result has a null prototype (opcodes.rs).
const CLONE_NULL_PROTO: u8 = 1;

/// CLONE handler fields (an IC_HANDLER cell, kind 70; the `ic.rs` IC_LAYOUT 2 table).
pub(crate) mod ch {
    pub const KIND: usize = 8;
    pub const COUNT: usize = 16;
    pub const RESULT: usize = 24;
    pub const ZERO: usize = 32;
    pub const SIZE: usize = 40;
}

/// POLY_IC fields (`ic.rs`: 4 cases from +8: key (weak), handler (strong)).
mod pc {
    pub const CASES: usize = 8;
    pub const CASE: usize = 16;
    pub const SIZE: usize = CASES + CASE * crate::ic::POLY_CASES;
}

// ---- allocation ----------------------------------------------------------------------------

/// A new array with `shape` (the realm's array shape), `length` = `n` and an ELEMENTS cell of
/// capacity `n` whose element `i` is `value(i)` (the hole for none), or no ELEMENTS cell for
/// `n` = 0. GC point: the ELEMENTS cell comes first and lives in a Rust local while the array
/// is allocated, so both are filled with initializing stores (E2: each host is the newest
/// cell when it is filled).
#[inline(always)]
fn alloc_array(rt: &Runtime, shape: u64, n: usize, value: impl Fn(usize) -> u64) -> u64 {
    let e = if n > 0 {
        let e = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * n);
        init(e, elems::CAPACITY, int32_bits(n as i32));
        for i in 0..n {
            init(e, elems::DATA + 8 * i, value(i));
        }
        e.to_value().to_bits()
    } else {
        UNDEFINED
    };
    let a = alloc_from_shape(rt, kind::ARRAY, shape);
    init(a, arr::LENGTH, int32_bits(n as i32));
    if n > 0 {
        init(a, obj::ELEMENTS, e);
    }
    a.to_value().to_bits()
}

/// The key of Literal slot `s` of the running function, if it holds a shape.
#[inline(always)]
fn literal_shape(fi: &FunctionInfo, s: FSlot) -> Option<u64> {
    let v = feedback::vector(fi)?;
    let k = feedback::key(v, s.0);
    // The key is undefined (new, or the shape died) or a shape (only `miss` writes it).
    is_cell_bits(k).then_some(k)
}

// ---- fast bodies ------------------------------------------------------------------------------

/// `CreateObjectLiteral` (boilerplate constant `boilerplate`, Literal slot `s`).
#[inline(always)]
pub(crate) fn fast_object(st: &mut St<'_>, boilerplate: K, s: FSlot, flags: u8) -> Flow {
    if flags & SHALLOW == 0 {
        return Flow::Cold;
    }
    let fi = st.info();
    let Some(shape) = literal_shape(fi, s) else {
        return Flow::Cold;
    };
    let rt = st.rt();
    let Constant::ObjectBoilerplate(props) = fi.constant_entry(boilerplate.0) else {
        return Flow::Cold;
    };
    let n = props.len();
    let (cap, k) = shape::field_layout(shape);
    // The realm check (the cold path uses the current realm's %Object.prototype%) and the
    // bounds of the stores below.
    if k != kind::OBJECT.get()
        || shape::count(shape) != n
        || cap < n
        || shape::proto(shape) != rt.root(Root::ObjectPrototype)
    {
        return Flow::Cold;
    }
    rt.enter(st.fp, st.pc);
    // GC point; `shape` lives in a Rust local.
    let o = alloc_from_shape(rt, kind::OBJECT, shape);
    for (i, &(_, v)) in props.iter().enumerate() {
        // A computed value (`None`) stays undefined: code after the creation defines it.
        if let Some(ci) = v {
            let b = fi.constant(ci.0);
            if b == HOLE {
                // A BigInt (or a hole, which BytecodeGen never puts in an object): the cold
                // path decides. The new object is garbage.
                return Flow::Cold;
            }
            init(o, object::OBJECT_FIELD0 + 8 * i, b);
        }
    }
    st.acc = o.to_value().to_bits();
    Flow::Next
}

/// `CreateArrayLiteral` (boilerplate constant `boilerplate`, Literal slot `s`).
#[inline(always)]
pub(crate) fn fast_array(st: &mut St<'_>, boilerplate: K, s: FSlot, flags: u8) -> Flow {
    if flags & SHALLOW == 0 {
        return Flow::Cold;
    }
    let fi = st.info();
    let Some(shape) = literal_shape(fi, s) else {
        return Flow::Cold;
    };
    let rt = st.rt();
    if !is_array_shape(rt, shape) {
        return Flow::Cold;
    }
    let Constant::ArrayBoilerplate(elements) = fi.constant_entry(boilerplate.0) else {
        return Flow::Cold;
    };
    // The hole marker is an elision or a BigInt; only an elision is a value here.
    let bigint = elements.iter().any(|&e| {
        e.is_some_and(|ci| {
            fi.constant(ci.0) == HOLE
                && !matches!(fi.constant_entry(ci.0), Constant::Oddball(Oddball::Hole))
        })
    });
    if bigint {
        return Flow::Cold;
    }
    rt.enter(st.fp, st.pc);
    // GC points; `shape` lives in a Rust local. A computed element (`None`) stays a hole until
    // `StaInArrayLiteral` stores it, as on the cold path.
    let a = alloc_array(rt, shape, elements.len(), |i| {
        elements[i].map_or(HOLE, |ci| fi.constant(ci.0))
    });
    st.acc = a;
    Flow::Next
}

/// `CreateEmptyArrayLiteral` (Literal slot `s`).
#[inline(always)]
pub(crate) fn fast_empty_array(st: &mut St<'_>, s: FSlot) -> Flow {
    let Some(shape) = literal_shape(st.info(), s) else {
        return Flow::Cold;
    };
    let rt = st.rt();
    if !is_array_shape(rt, shape) {
        return Flow::Cold;
    }
    rt.enter(st.fp, st.pc);
    st.acc = alloc_array(rt, shape, 0, |_| HOLE);
    Flow::Next
}

/// `CreateEmptyObjectLiteral` (no feedback slot): always the cold path (`slow::literal`,
/// which finds the `{}` shape through %Object.prototype%'s PROTO_INFO).
#[inline(always)]
pub(crate) fn fast_empty_object(_st: &mut St<'_>) -> Flow {
    Flow::Cold
}

/// True if `shape` is the shape of a new array of the current realm: no own properties, no
/// in-object slots, kind ARRAY, prototype %Array.prototype%.
#[inline(always)]
fn is_array_shape(rt: &Runtime, shape: u64) -> bool {
    shape::field_layout(shape) == (0, kind::ARRAY.get())
        && shape::count(shape) == 0
        && shape::proto(shape) == rt.root(Root::ArrayPrototype)
}

/// `CloneObject` of `source` (CloneObject slot `s`).
#[inline(always)]
pub(crate) fn fast_clone(st: &mut St<'_>, source: u64, flags: u8, s: FSlot) -> Flow {
    if flags & CLONE_NULL_PROTO != 0 || !kind::is_object(source) {
        return Flow::Cold;
    }
    let Some(v) = feedback::vector(st.info()) else {
        return Flow::Cold;
    };
    let (key, value) = (feedback::key(v, s.0), feedback::value(v, s.0));
    let Some(h) = clone_handler(key, value, source_shape_id(source)) else {
        return Flow::Cold;
    };
    let result = rd(cell(h), ch::RESULT);
    let rt = st.rt();
    if result == UNDEFINED || shape::proto(result) != rt.root(Root::ObjectPrototype) {
        return Flow::Cold;
    }
    let n = shape::count(result);
    let ss = shape::shape_of(source);
    let (scap, _) = shape::field_layout(ss);
    rt.enter(st.fp, st.pc);
    // GC point; `result` lives in a Rust local, `source` in a frame register (its shape, and
    // so where its fields are, does not change in a GC).
    let o = alloc_from_shape(rt, kind::OBJECT, result);
    let sc = cell(source);
    for i in 0..n {
        let f = if i < scap {
            rd(sc, object::OBJECT_FIELD0 + 8 * i)
        } else {
            rd(cell(rd(sc, obj::SLOTS)), slf::DATA + 8 * (i - scap))
        };
        init(o, object::OBJECT_FIELD0 + 8 * i, f);
    }
    st.acc = o.to_value().to_bits();
    Flow::Next
}

/// The word0 ShapeId of the JS object `o`.
#[inline(always)]
fn source_shape_id(o: u64) -> u32 {
    // SAFETY: a cell value the VM holds is live (crate invariant, object.rs).
    unsafe { cell(o).header() }.shape_id().get()
}

/// The CLONE handler of the case for a source with ShapeId `id`: the monomorphic case or one
/// of the polymorphic ones. An undefined key, the hole and int32 0 never equal the ShapeId of
/// an object (8-byte aligned, above the page header), as in `ic.rs`.
#[inline(always)]
fn clone_handler(key: u64, value: u64, id: u32) -> Option<u64> {
    if key as u32 == id && is_cell_bits(value) {
        return Some(value);
    }
    if key == HOLE && is_cell_bits(value) {
        let p = cell(value);
        for i in 0..ic::POLY_CASES {
            if rd(p, pc::CASES + pc::CASE * i) as u32 == id {
                return Some(rd(p, pc::CASES + pc::CASE * i + 8));
            }
        }
    }
    None
}

// ---- cold paths -------------------------------------------------------------------------------

/// The feedback slot operand and the flag operand (0 if none) of the instruction at `pc` of
/// the bytecode function `fi`.
fn slot_at(fi: &FunctionInfo, pc: *const u8) -> Option<(u32, u32)> {
    let off = fi.offset_of(pc)?;
    // SAFETY: the code of a bytecode FunctionInfo is `code_len` bytes of verified bytecode.
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let insn = ep9_bytecode::decode_at(code, off).ok()?;
    let operand = |want: fn(OperandKind) -> bool| {
        (0..insn.operand_count())
            .find(|&j| want(insn.operand_kind(j)))
            .map(|j| insn.unsigned(j))
    };
    let slot = operand(|k| k == OperandKind::Slot)?;
    let flags = operand(|k| matches!(k, OperandKind::Flag8 | OperandKind::Flag16)).unwrap_or(0);
    Some((slot, flags))
}

/// True if no value of the boilerplate constant `k` of `fi` is a nested boilerplate.
fn shallow_boilerplate(fi: &FunctionInfo, k: u32) -> bool {
    let nested = |ci: Option<ep9_bytecode::ConstIndex>| {
        ci.is_some_and(|ci| {
            matches!(
                fi.constant_entry(ci.0),
                Constant::ArrayBoilerplate(_) | Constant::ObjectBoilerplate(_)
            )
        })
    };
    match fi.constant_entry(k) {
        Constant::ObjectBoilerplate(props) => !props.iter().any(|&(_, v)| nested(v)),
        Constant::ArrayBoilerplate(elements) => !elements.iter().any(|&e| nested(e)),
        _ => true,
    }
}

/// True if the instruction at `pc` is the only one of `fi` with feedback slot `slot`.
/// BytecodeGen gives each literal site its own slot, but the verifier does not require it,
/// and a Literal key is right only for the boilerplate it was made from. Not a GC point.
pub(crate) fn exclusive_slot(fi: &FunctionInfo, pc: *const u8, slot: u32) -> bool {
    let Some(off) = fi.offset_of(pc) else {
        return false;
    };
    // SAFETY: as in `slot_at`.
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    Instructions::new(code).all(|insn| {
        let Ok(insn) = insn else { return false };
        insn.offset == off
            || !(0..insn.operand_count())
                .any(|j| insn.operand_kind(j) == OperandKind::Slot && insn.unsigned(j) == slot)
    })
}

/// The int32 count `value + 1`, saturating; a value that is not an int32 counts as 0.
fn bump(value: u64) -> u64 {
    let n = if val(value).is_int32() {
        int32_of(value)
    } else {
        0
    };
    int32_bits(n.saturating_add(1).max(1))
}

/// The cold path of a literal of `kind` (constant `k`) at `pc` in frame `fp` made `result`
/// (`slow::literal`, after the object exists; the slot is an operand of the instruction at
/// `pc`). Records the result's shape (weak) and counts the result. Not a GC point (one
/// barriered slot write); not called for a throw.
#[inline]
pub(crate) fn miss(cx: &mut Cx, fp: *mut u64, pc: *const u8, kind: Lit, k: u32, result: u64) {
    if matches!(kind, Lit::EmptyObject | Lit::Template) || !kind::is_object(result) {
        return;
    }
    let rt = cx.rt();
    let fi = FrameRef(fp).info();
    let Some(v) = feedback::vector(fi) else {
        return;
    };
    let Some((i, flags)) = slot_at(fi, pc) else {
        return;
    };
    if i >= feedback::slot_count(v) {
        debug_assert!(false, "a literal slot past the vector");
        return;
    }
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    let shape = shape::shape_of(result);
    if key == shape {
        // Checked when it was recorded (shapes do not change): only the count.
        feedback::set_slot(rt, v, i, key, bump(value));
        return;
    }
    let describable = !shape::is_dictionary(shape) && literal_shape_ok(fi, kind, k, shape);
    // A hit trusts the shallow flag (bit 0); the verifier does not check it against the
    // boilerplate, so a site whose flag is wrong gets no key.
    let flag_ok = flags as u8 & SHALLOW == 0 || shallow_boilerplate(fi, k);
    let new_key = if describable && flag_ok {
        // A new key only for a slot that this site alone uses (the key fits its boilerplate).
        if key == shape || exclusive_slot(fi, pc, i) {
            shape
        } else {
            UNDEFINED
        }
    } else {
        UNDEFINED
    };
    feedback::set_slot(rt, v, i, new_key, bump(value));
}

#[cfg(test)]
thread_local! {
    /// Tests: object literals that [`cold_object`] built.
    pub(crate) static COLD_BUILDS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// The cold path of `CreateObjectLiteral` at `pc` (boilerplate constant `k`) in frame `fp`,
/// before the generic build (`slow::literal`): an object literal that is not shallow, whose
/// nested values are all shallow array boilerplates (`{array: [0, 1, 2], s: ...}`: splay's
/// leaf nodes), built from the site's recorded shape as a hit builds a shallow one, with each
/// nested array made like a shallow array literal. `None` (nothing made) for anything else:
/// no recorded shape, another nested value, a BigInt, more than `call::INLINE_ARGS` nested
/// arrays. GC point: the nested arrays live in an inline buffer on the native stack (the
/// conservative scan) until the object holds them; the object is allocated last, so its
/// fields are initializing stores (E2).
pub(crate) fn cold_object(cx: &mut Cx, fp: *mut u64, pc: *const u8, k: u32) -> Option<u64> {
    let rt = cx.rt();
    let fi = FrameRef(fp).info();
    let v = feedback::vector(fi)?;
    let (i, _) = slot_at(fi, pc)?;
    if i >= feedback::slot_count(v) {
        return None;
    }
    let shape = feedback::key(v, i);
    if !is_cell_bits(shape) {
        return None;
    }
    let Constant::ObjectBoilerplate(props) = fi.constant_entry(k) else {
        return None;
    };
    let n = props.len();
    let (cap, ok) = shape::field_layout(shape);
    if ok != kind::OBJECT.get()
        || shape::count(shape) != n
        || cap < n
        || shape::proto(shape) != rt.root(Root::ObjectPrototype)
    {
        return None;
    }
    // Check every value before the first allocation.
    let mut nested = 0;
    for &(_, val) in props.iter() {
        let Some(ci) = val else { continue };
        match fi.constant_entry(ci.0) {
            Constant::ArrayBoilerplate(elements) => {
                let flat = elements.iter().all(|&e| {
                    e.is_none_or(|c| {
                        matches!(
                            fi.constant_entry(c.0),
                            Constant::Number(_) | Constant::String(_) | Constant::Oddball(_)
                        )
                    })
                });
                if !flat {
                    return None;
                }
                nested += 1;
            }
            Constant::Number(_) | Constant::String(_) => {}
            Constant::Oddball(o) if *o != Oddball::Hole => {}
            _ => return None,
        }
    }
    if nested == 0 || nested > crate::call::INLINE_ARGS {
        return None;
    }
    // GC points. The array shape of the current realm (made once, then found).
    let ashape = shape::initial_shape(rt, rt.root(Root::ArrayPrototype), kind::ARRAY, 0);
    let mut arrays: crate::call::ArgBuf<u64> = crate::call::ArgBuf::new();
    for &(_, val) in props.iter() {
        if let Some(ci) = val
            && let Constant::ArrayBoilerplate(elements) = fi.constant_entry(ci.0)
        {
            let a = alloc_array(rt, ashape, elements.len(), |j| {
                elements[j].map_or(HOLE, |c| fi.constant(c.0))
            });
            arrays.push(a);
        }
    }
    let o = alloc_from_shape(rt, kind::OBJECT, shape);
    let mut next = 0;
    for (j, &(_, val)) in props.iter().enumerate() {
        let Some(ci) = val else { continue };
        let b = if matches!(fi.constant_entry(ci.0), Constant::ArrayBoilerplate(_)) {
            next += 1;
            arrays[next - 1]
        } else {
            fi.constant(ci.0)
        };
        init(o, object::OBJECT_FIELD0 + 8 * j, b);
    }
    #[cfg(test)]
    COLD_BUILDS.with(|c| c.set(c.get() + 1));
    Some(o.to_value().to_bits())
}

/// True if `shape` (the fast shape of a new result of the literal `kind`, constant `k`) is
/// one a hit can make again: an object shape with one in-object field per boilerplate entry
/// in boilerplate order, or the array shape. Not a GC point.
fn literal_shape_ok(fi: &FunctionInfo, kind: Lit, k: u32, shape: u64) -> bool {
    match kind {
        Lit::Object => {
            let Constant::ObjectBoilerplate(props) = fi.constant_entry(k) else {
                return false;
            };
            let n = props.len();
            let (cap, ok) = shape::field_layout(shape);
            ok == kind::OBJECT.get()
                && shape::count(shape) == n
                && cap >= n
                && props.iter().enumerate().all(|(i, &(kc, _))| {
                    let d = shape::details_at(shape, i);
                    usize::from(d.field()) == i
                        && d.attrs() == attr::DEFAULT
                        && crate::key::key_eq(shape::key_at(shape, i), fi.constant(kc.0))
                })
        }
        Lit::Array | Lit::EmptyArray => {
            shape::field_layout(shape) == (0, kind::ARRAY.get()) && shape::count(shape) == 0
        }
        Lit::Template | Lit::EmptyObject => false,
    }
}

/// The cold path of `CloneObject` at `pc` made `result` from `source` (`slow::clone_object`).
/// Records the case (source shape, result shape) if a hit can copy it: monomorphic, then up
/// to [`ic::POLY_CASES`] cases, then megamorphic for good. GC point (a new handler or POLY_IC
/// cell): `source` and `result` are in Rust locals of the caller.
pub(crate) fn miss_clone(cx: &mut Cx, fp: *mut u64, pc: *const u8, source: u64, result: u64) {
    let rt = cx.rt();
    let fi = FrameRef(fp).info();
    let Some(v) = feedback::vector(fi) else {
        return;
    };
    let Some((i, _)) = slot_at(fi, pc) else {
        return;
    };
    if i >= feedback::slot_count(v) {
        debug_assert!(false, "a clone slot past the vector");
        return;
    }
    if !kind::is_object(source) || !kind::is_object(result) {
        return;
    }
    let (ss, rs) = (shape::shape_of(source), shape::shape_of(result));
    if !clonable(ss, rs) {
        return;
    }
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    if key == HOLE && value == ic::MEGA {
        return;
    }
    // The case of `ss` already: give its handler the new result shape and the count.
    if let Some(h) = clone_handler(key, value, source_shape_id(source)) {
        let c = cell(h);
        object::wr(&rt.heap, c, ch::RESULT, rs);
        object::wr(&rt.heap, c, ch::COUNT, bump(rd(c, ch::COUNT)));
        return;
    }
    // GC point: `ss` and `rs` live in Rust locals; `v` is rooted by the unit.
    let h = new_clone_handler(rt, rs);
    // Read again: the GC may have cleared a dead key.
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    if key == UNDEFINED {
        // New, or the monomorphic case's source shape died (its handler goes too).
        feedback::set_slot(rt, v, i, ss, h);
        return;
    }
    let poly = if key == HOLE && is_cell_bits(value) {
        value
    } else {
        // Monomorphic -> polymorphic: the old case (if its key is alive) and the new one.
        let p = rt.alloc(kind::POLY_IC, pc::SIZE);
        if key != UNDEFINED && is_cell_bits(value) {
            init(p, pc::CASES, key);
            init(p, pc::CASES + 8, value);
        }
        let p = p.to_value().to_bits();
        feedback::set_slot(rt, v, i, HOLE, p);
        p
    };
    let p = cell(poly);
    let free = (0..ic::POLY_CASES).find(|&c| rd(p, pc::CASES + pc::CASE * c) == UNDEFINED);
    match free {
        Some(c) => {
            object::wr(&rt.heap, p, pc::CASES + pc::CASE * c, ss);
            object::wr(&rt.heap, p, pc::CASES + pc::CASE * c + 8, h);
        }
        None => feedback::set_slot(rt, v, i, HOLE, ic::MEGA),
    }
}

/// A new CLONE handler for the result shape `rs` (weak at +24), count 1. GC point.
fn new_clone_handler(rt: &Runtime, rs: u64) -> u64 {
    let c = rt.alloc(kind::IC_HANDLER, ch::SIZE);
    init(c, ch::KIND, int32_bits(hk::CLONE));
    init(c, ch::COUNT, int32_bits(1));
    init(c, ch::RESULT, rs);
    init(c, ch::ZERO, int32_bits(0));
    c.to_value().to_bits()
}

/// True if a clone of a source with shape `ss` gives a result with shape `rs` that a hit can
/// make field by field: both fast plain-object shapes, the same keys in the same order, the
/// source's properties all enumerable data properties at fields 0..n, the result's all
/// {w, e, c} in-object at fields 0..n. Not a GC point.
fn clonable(ss: u64, rs: u64) -> bool {
    if shape::is_dictionary(ss) || shape::is_dictionary(rs) {
        return false;
    }
    let n = shape::count(ss);
    let (rcap, rk) = shape::field_layout(rs);
    shape::object_kind(ss) == kind::OBJECT.get()
        && rk == kind::OBJECT.get()
        && shape::count(rs) == n
        && rcap >= n
        && (0..n).all(|i| {
            let (sd, dd) = (shape::details_at(ss, i), shape::details_at(rs, i));
            usize::from(sd.field()) == i
                && usize::from(dd.field()) == i
                && !sd.is_accessor()
                && sd.attrs() & attr::ENUMERABLE != 0
                && dd.attrs() == attr::DEFAULT
                && shape::key_at(ss, i) == shape::key_at(rs, i)
        })
}

/// Reads the int32 at `off` of the IC_HANDLER `h` (tests).
#[cfg(test)]
pub(crate) fn handler_i32(h: u64, off: usize) -> i32 {
    object::rd_i32(cell(h), off)
}
