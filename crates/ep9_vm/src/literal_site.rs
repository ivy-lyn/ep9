//! Literal sites (owner M4-P1 OBJ-ALLOC; OI-03, the V9-D8 pattern, V9-A8): the Literal
//! feedback slot (15; IC_LAYOUT 2, `profile.rs` table) keeps the result shape (weak) and an
//! int32 count; no cell. The CloneObject slot (16) keeps the source shape (weak) and a clone
//! handler (an IC_HANDLER cell of handler kind `CLONE`, `ic::hk`: +16 int32 count, +24 the
//! result shape (weak), +32 int32 0), or the hole and a POLY_IC cell (2-4 cases: source shape
//! (weak), clone handler) or the int32 [`ic::MEGA`]. A hit allocates with the result shape and
//! copies the constants (Literal) or the source's fields (CloneObject); computed values stay
//! off the T0 hit. An object literal whose nested values are all shallow arrays (splay's leaf
//! nodes) is built from the recorded shape ([`nested_object`]) by its T0 hit (M5-P2 ELEM2) and
//! by its cold path ([`cold_object`]), not by the generic per-key define. An array-literal site
//! keeps an ALLOC_SITE handler ([`ah`], IC_LAYOUT 3; V10-D6): its arrays are born in the
//! elements kind its earlier arrays reached ([`array_site`], [`generalize`]).
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
//! | Literal, seen | the shape of the last result the cold path made | int32 count of cold-path results (saturating) | object: a shallow boilerplate whose result shape is the key, or one whose nested values are flat array boilerplates; array: a shallow boilerplate (or `[]`) and the realm's array shape |
//! | Literal, array site seen (M5-P2) | as above | an ALLOC_SITE handler ([`ah`]: the site's elements kind, the result shape (weak), the count); every array made there keeps it at ARRAY +32 | as above, with the handler's result shape when it is a fresh array shape of the realm (the site's kind) |
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
//!   ([`shared_slot`]: one walk of the code per function, then O(1) per miss), so the key's
//!   properties are the boilerplate's keys in order.
//! - A BigInt constant (the hole marker, `unit.rs`) is not a value: a hit that meets one goes
//!   cold, and the cold path throws its "not implemented".
//! - A CloneObject case is recorded only for a source that a hit can copy field by field: a
//!   fast plain object whose properties are all enumerable data properties, whose result
//!   has the same keys in the same order, all in-object.

use ep9_bytecode::{Constant, Instructions, Oddball, OperandKind};

use crate::elements;
use crate::error::FrameRef;
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits, int32_of};
use crate::ic::{self, hk};
use crate::interp::dsl::{FSlot, K};
use crate::interp::slow::Lit;
use crate::interp::{Cx, Flow, St};
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, alloc_from_shape, arr, attr, cell, delems, elems, init, obj, rd};
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

/// ALLOC_SITE handler fields (an IC_HANDLER cell, kind 70, handler kind `ALLOC_SITE`; IC_LAYOUT
/// 3, the `ic.rs` table; V10-D6, MODES-D19; M5-P2 ELEM2). The Literal slot of an array-literal
/// site holds it in its value half once the site ran cold, and every array made there holds
/// it at ARRAY +32 (`object::arr::ALLOC_SITE`).
pub(crate) mod ah {
    pub const KIND: usize = 8;
    /// The int32 elements kind the site's arrays are born with (`elements::kind`, 0-5): the
    /// join of every kind its arrays reached (one-way, [`super::generalize`]).
    pub const ELEMENTS_KIND: usize = 16;
    /// The shape (weak) of a fresh array of that kind: the hit allocates with it.
    pub const RESULT: usize = 24;
    /// The int32 count of the site's cold-path results (saturating).
    pub const COUNT: usize = 32;
    pub const SIZE: usize = 40;
}

/// POLY_IC fields (`ic.rs`: 4 cases from +8: key (weak), handler (strong)).
mod pc {
    pub const CASES: usize = 8;
    pub const CASE: usize = 16;
    pub const SIZE: usize = CASES + CASE * crate::ic::POLY_CASES;
}

// ---- allocation ----------------------------------------------------------------------------

/// A new array with `shape` (an array shape of the realm), `length` = `n` and an elements
/// cell of capacity `n` whose element `i` is `value(i)` (the hole for none), or no cell for
/// `n` = 0. The cell has the storage form of the shape's elements kind (H18, M5-P1 ELEM1):
/// Values in an ELEMENTS cell, or raw f64 (canonical NaN, the hole pattern for a hole) in an
/// ELEMENTS_DOUBLE cell; the caller checked that the kind fits every value
/// ([`literal_kind`]). GC point: the cell comes first and lives in a Rust local while the
/// array is allocated, so both are filled with initializing stores (E2: each host is the
/// newest cell when it is filled; raw f64 words take no barrier).
#[inline(always)]
fn alloc_array(rt: &Runtime, shape: u64, n: usize, value: impl Fn(usize) -> u64) -> u64 {
    let doubles =
        elements::storage(shape::elements_kind(shape)) == Some(elements::Storage::Doubles);
    let e = if n > 0 && doubles {
        let e = elements::new_double_cell(rt, n);
        for i in 0..n {
            let v = value(i);
            if v != HOLE {
                object::raw_set(e, delems::DATA + 8 * i, object::double_bits(v));
            }
        }
        e.to_value().to_bits()
    } else if n > 0 {
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

/// A new array for a nested array boilerplate of [`cold_object`]: as [`alloc_array`], with
/// the elements kind its values need, found while they are stored (one pass: a hole makes
/// it holey; a DOUBLE kind copies the Values cell into an ELEMENTS_DOUBLE cell). `ashape` is
/// the realm's array root shape. GC point: each cell lives in a Rust local while the next
/// one, the shape and the array are made; each is filled right after its allocation (E2).
fn alloc_nested_array(rt: &Runtime, ashape: u64, n: usize, value: impl Fn(usize) -> u64) -> u64 {
    let mut ek = elements::kind::PACKED_INT32;
    let e = if n > 0 {
        let e = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * n);
        init(e, elems::CAPACITY, int32_bits(n as i32));
        for i in 0..n {
            let v = value(i);
            ek = if v == HOLE {
                elements::holey(ek)
            } else {
                elements::join(ek, elements::value_kind(v))
            };
            init(e, elems::DATA + 8 * i, v);
        }
        if elements::storage(ek) == Some(elements::Storage::Doubles) {
            let d = elements::new_double_cell(rt, n);
            for i in 0..n {
                let v = rd(e, elems::DATA + 8 * i);
                if v != HOLE {
                    object::raw_set(d, delems::DATA + 8 * i, object::double_bits(v));
                }
            }
            d.to_value().to_bits()
        } else {
            e.to_value().to_bits()
        }
    } else {
        UNDEFINED
    };
    // A root array shape of the realm: never `None`.
    let shape = shape::with_elements_kind(rt, ashape, ek).expect("a root array shape");
    let a = alloc_from_shape(rt, kind::ARRAY, shape);
    init(a, arr::LENGTH, int32_bits(n as i32));
    if n > 0 {
        init(a, obj::ELEMENTS, e);
    }
    a.to_value().to_bits()
}

/// The elements kind an array literal needs for its constant elements (`elements`: `None` for
/// a computed element, else the constant index): PACKED_INT32, joined with each constant's
/// kind; an elision (the hole constant) makes it holey. A computed element is a hole until
/// its `StaInArrayLiteral` store, which makes the kind its value needs (no JS can read the
/// array before), so it does not count.
#[inline(always)]
fn literal_kind(fi: &FunctionInfo, elements: &[Option<ep9_bytecode::ConstIndex>]) -> u8 {
    elements
        .iter()
        .fold(elements::kind::PACKED_INT32, |k, e| match e {
            None => k,
            Some(ci) => {
                let c = fi.constant(ci.0);
                if c == HOLE {
                    elements::holey(k)
                } else {
                    elements::join(k, elements::value_kind(c))
                }
            }
        })
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
        return fast_nested_object(st, boilerplate, s);
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
    // The key is the shape of a cold result, which stored these constants (`slow::literal`
    // `array_from_boilerplate`): its elements kind holds them, so a hit makes no kind
    // transition. Checked in debug builds only (a second pass per hit cost splay +4%, M5-P1).
    debug_assert!(
        elements::allows(literal_kind(fi, elements), shape::elements_kind(shape)),
        "a literal key kind below its constants' kind"
    );
    let (shape, site) = site_shape(rt, fi, s, shape);
    rt.enter(st.fp, st.pc);
    // GC points; `shape` and `site` live in Rust locals. A computed element (`None`) stays a
    // hole until `StaInArrayLiteral` stores it, as on the cold path.
    let a = alloc_array(rt, shape, elements.len(), |i| {
        elements[i].map_or(HOLE, |ci| fi.constant(ci.0))
    });
    // The array is the newest cell (E2).
    init(cell(a), arr::ALLOC_SITE, site);
    st.acc = a;
    Flow::Next
}

/// The shape an array-literal hit allocates with and the site handler the array keeps
/// (ARRAY +32; undefined when the slot holds none): the handler's result shape when it is
/// alive and still a fresh array shape of this realm (its kind is the site's, at least the
/// key's: kinds only grow), else the key `shape`. Reads only (a hit writes no feedback,
/// V9-D6). Not a GC point.
#[inline(always)]
fn site_shape(rt: &Runtime, fi: &FunctionInfo, s: FSlot, shape: u64) -> (u64, u64) {
    let Some(v) = feedback::vector(fi) else {
        return (shape, UNDEFINED);
    };
    let h = feedback::value(v, s.0);
    if !is_cell_bits(h) {
        return (shape, UNDEFINED);
    }
    let r = rd(cell(h), ah::RESULT);
    if r != shape && is_cell_bits(r) && is_array_shape(rt, r) {
        (r, h)
    } else {
        (shape, h)
    }
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
    let (shape, site) = site_shape(rt, st.info(), s, shape);
    rt.enter(st.fp, st.pc);
    let a = alloc_array(rt, shape, 0, |_| HOLE);
    // The array is the newest cell (E2).
    init(cell(a), arr::ALLOC_SITE, site);
    st.acc = a;
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

/// The feedback slots of one function that more than one instruction names
/// ([`FunctionInfo::shared_slots`]): bit `s % 32` of word `s / 32` is set for slot `s`. `None`: the
/// code did not decode (verified code always does), so every slot counts as shared.
pub(crate) struct SharedSlots(Option<Box<[u32]>>);

impl SharedSlots {
    /// One walk of the code of `fi`. Not a GC point.
    fn of(fi: &FunctionInfo) -> SharedSlots {
        // SAFETY: as in `slot_at`.
        let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
        let (mut seen, mut shared) = (Vec::<u32>::new(), Vec::<u32>::new());
        for insn in Instructions::new(code) {
            #[cfg(test)]
            SCANNED.with(|c| c.set(c.get() + 1));
            let Ok(insn) = insn else {
                debug_assert!(false, "verified code decodes");
                return SharedSlots(None);
            };
            let slots = (0..insn.operand_count())
                .filter(|&j| insn.operand_kind(j) == OperandKind::Slot)
                .map(|j| insn.unsigned(j) as usize);
            for (n, s) in slots.clone().enumerate() {
                // An instruction that names a slot twice is still one instruction.
                if slots.clone().take(n).any(|t| t == s) {
                    continue;
                }
                let (w, bit) = (s / 32, 1u32 << (s % 32));
                if seen.len() <= w {
                    seen.resize(w + 1, 0);
                    shared.resize(w + 1, 0);
                }
                if seen[w] & bit != 0 {
                    shared[w] |= bit;
                }
                seen[w] |= bit;
            }
        }
        SharedSlots(Some(shared.into_boxed_slice()))
    }

    fn contains(&self, slot: u32) -> bool {
        let Some(words) = &self.0 else { return true };
        let s = slot as usize;
        words
            .get(s / 32)
            .is_some_and(|w| w & (1u32 << (s % 32)) != 0)
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: instructions that [`SharedSlots::of`] decoded.
    pub(crate) static SCANNED: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// True if feedback slot `slot` of `fi` is named by more than one instruction. BytecodeGen
/// gives each literal site its own slot, but the verifier does not require it, and a Literal
/// key is right only for the boilerplate it was made from. The first call walks the code once
/// and keeps the answer for every slot in `fi` (a function's code never changes), so each
/// later call is O(1): a walk per miss would cost O(sites x code length) per function (the
/// M4-P2 nesting_caps regression: a script's vector at entry, 61181b1, made every literal
/// site of top-level code miss into the walk). Not a GC point.
pub(crate) fn shared_slot(fi: &FunctionInfo, slot: u32) -> bool {
    fi.shared_slots
        .get_or_init(|| SharedSlots::of(fi))
        .contains(slot)
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
/// `pc`). Records the result's shape (weak) and counts the result. At an array-literal site
/// the value half is the site's ALLOC_SITE handler ([`array_site`]) instead of the count. GC
/// point at an array-literal site only (the handler, a kind transition): `result` lives in a
/// Rust local of the caller; not called for a throw.
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
    // A site handler only for a slot that this site alone uses (as the key): a shared slot
    // would mix the kinds of two sites.
    let array = matches!(kind, Lit::Array | Lit::EmptyArray)
        && kind::is_array(result)
        && !shared_slot(fi, i);
    if array {
        // GC point: `result` lives in a Rust local of the caller; `v` is rooted by the unit.
        array_site(rt, v, i, result);
    }
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    // The count: in the site handler at an array-literal site, else the value half.
    let value = if array { value } else { bump(value) };
    let shape = shape::shape_of(result);
    if key == shape {
        // Checked when it was recorded (shapes do not change): only the count.
        feedback::set_slot(rt, v, i, key, value);
        return;
    }
    let describable = !shape::is_dictionary(shape) && literal_shape_ok(fi, kind, k, shape);
    // A hit trusts the shallow flag (bit 0); the verifier does not check it against the
    // boilerplate, so a site whose flag is wrong gets no key.
    let flag_ok = flags as u8 & SHALLOW == 0 || shallow_boilerplate(fi, k);
    let new_key = if describable && flag_ok {
        // A new key only for a slot that this site alone uses (the key fits its boilerplate).
        if !shared_slot(fi, i) {
            shape
        } else {
            UNDEFINED
        }
    } else {
        UNDEFINED
    };
    feedback::set_slot(rt, v, i, new_key, value);
}

/// The allocation-site part of an array-literal miss (V10-D6, MODES-D19): the site's
/// ALLOC_SITE handler (made at the first miss, in slot `i`'s value half), the cold `result`
/// moved to the site's kind when the site saw a more general one (a store-side transition,
/// one-way; no load changes a kind), the site kind joined with the result's, the result
/// shape, the count, and the result's back pointer ARRAY +32. Written by this cold path and
/// by store-side transitions ([`generalize`]) only, never by a `profile.rs` record hook, so
/// `ics_only` keeps it. GC point (the handler, the transition): `result` lives in a Rust
/// local of the caller.
fn array_site(rt: &Runtime, v: u64, i: u32, result: u64) {
    let value = feedback::value(v, i);
    let h = if is_cell_bits(value) {
        value
    } else {
        // GC point: `result` lives in a Rust local.
        let h = rt.alloc(kind::IC_HANDLER, ah::SIZE);
        init(h, ah::KIND, int32_bits(hk::ALLOC_SITE));
        init(
            h,
            ah::ELEMENTS_KIND,
            int32_bits(i32::from(elements::kind_of(result))),
        );
        init(h, ah::RESULT, UNDEFINED);
        init(h, ah::COUNT, int32_bits(0));
        let h = h.to_value().to_bits();
        // The value half is strong; the key half is written by `miss` after this.
        feedback::set_slot(rt, v, i, feedback::key(v, i), h);
        h
    };
    let c = cell(h);
    let site = object::rd_i32(c, ah::ELEMENTS_KIND) as u8;
    let rk = elements::kind_of(result);
    let want = elements::join(site, rk);
    if want != rk && want < elements::kind::DICTIONARY {
        // GC point: `result` and `h` live in Rust locals (the handler is also in the slot).
        elements::transition(rt, result, want);
    }
    if want != site && want < elements::kind::DICTIONARY {
        object::wr(&rt.heap, c, ah::ELEMENTS_KIND, int32_bits(i32::from(want)));
    }
    let s = shape::shape_of(result);
    if is_array_shape(rt, s) {
        object::wr(&rt.heap, c, ah::RESULT, s);
    }
    object::wr(&rt.heap, c, ah::COUNT, bump(rd(c, ah::COUNT)));
    object::wr(&rt.heap, cell(result), arr::ALLOC_SITE, h);
}

/// A store-side kind transition of the array `a` (to kind `to`, below DICTIONARY) whose ARRAY
/// +32 holds the ALLOC_SITE handler `h` (`elements::transition`): the site kind becomes the
/// join of its kind and `to`, and the handler's result shape becomes `a`'s new shape if that
/// is a fresh array shape of kind `to`, so the site's next arrays are born in that kind.
/// One-way. Not a GC point.
pub(crate) fn generalize(rt: &Runtime, h: u64, a: u64, to: u8) {
    debug_assert!(to < elements::kind::DICTIONARY);
    let c = cell(h);
    debug_assert_eq!(object::rd_i32(c, ah::KIND), hk::ALLOC_SITE);
    let site = object::rd_i32(c, ah::ELEMENTS_KIND) as u8;
    let want = elements::join(site, to);
    if want != site {
        object::wr(&rt.heap, c, ah::ELEMENTS_KIND, int32_bits(i32::from(want)));
    }
    let s = shape::shape_of(a);
    if shape::elements_kind(s) == want && is_array_shape(rt, s) {
        object::wr(&rt.heap, c, ah::RESULT, s);
    }
}

/// The ALLOC_SITE handler of an array-literal site's Literal slot value `value`, if any:
/// (elements kind, count). For the facts export and the tests.
pub(crate) fn site_of(value: u64) -> Option<(u8, u32)> {
    if !is_cell_bits(value) || kind::cell_kind(value) != kind::IC_HANDLER.get() {
        return None;
    }
    let c = cell(value);
    (object::rd_i32(c, ah::KIND) == hk::ALLOC_SITE).then(|| {
        (
            object::rd_i32(c, ah::ELEMENTS_KIND) as u8,
            object::rd_i32(c, ah::COUNT) as u32,
        )
    })
}

#[cfg(test)]
thread_local! {
    /// Tests: object literals that [`cold_object`] built.
    pub(crate) static COLD_BUILDS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// The cold path of `CreateObjectLiteral` at `pc` (boilerplate constant `k`) in frame `fp`,
/// before the generic build (`slow::literal`): [`nested_object`] from the site's recorded
/// shape. `None` (nothing made) when it does not apply. GC point.
pub(crate) fn cold_object(cx: &mut Cx, fp: *mut u64, pc: *const u8, k: u32) -> Option<u64> {
    let rt = cx.rt();
    let fi = FrameRef(fp).info();
    // No vector, no recorded shape: decode no instruction (most cold runs of a literal are
    // the runs of a function before its vector exists).
    feedback::vector(fi)?;
    let (i, _) = slot_at(fi, pc)?;
    let o = nested_object(rt, fi, i, k)?;
    #[cfg(test)]
    COLD_BUILDS.with(|c| c.set(c.get() + 1));
    Some(o)
}

/// `CreateObjectLiteral` of a boilerplate that is not shallow (Literal slot `s`): the T0 hit
/// of [`nested_object`] (M5-P2 ELEM2, splay's leaf nodes `{array: [0, 1, ...], string: ...}`).
/// As a shallow hit, it writes no feedback (V9-D6); the recorded shape comes from the cold
/// path. A GC point: it publishes its frame first.
#[inline(always)]
fn fast_nested_object(st: &mut St<'_>, boilerplate: K, s: FSlot) -> Flow {
    let fi = st.info();
    let rt = st.rt();
    rt.enter(st.fp, st.pc);
    match nested_object(rt, fi, s.0, boilerplate.0) {
        Some(o) => {
            st.acc = o;
            Flow::Next
        }
        None => Flow::Cold,
    }
}

/// An object literal (boilerplate constant `k`, Literal slot `i`) that is not shallow, whose
/// nested values are all shallow array boilerplates (`{array: [0, 1, 2], s: ...}`: splay's
/// leaf nodes), built from the slot's recorded shape as a hit builds a shallow one, with each
/// nested array made like a shallow array literal. `None` (nothing made) for anything else:
/// no recorded shape, another nested value, a BigInt, more than `call::INLINE_ARGS` nested
/// arrays. Reads the slot only. GC point: the nested arrays live in an inline buffer on the
/// native stack (the conservative scan) until the object holds them; the object is
/// allocated last, so its fields are initializing stores (E2).
fn nested_object(rt: &Runtime, fi: &FunctionInfo, i: u32, k: u32) -> Option<u64> {
    let v = feedback::vector(fi)?;
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
            && let Constant::ArrayBoilerplate(items) = fi.constant_entry(ci.0)
        {
            let a = alloc_nested_array(rt, ashape, items.len(), |j| {
                items[j].map_or(HOLE, |c| fi.constant(c.0))
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
    // A source that has (or had) elements (an index key of a plain object, H18-Q3, M5-P1):
    // the hit copies fields only, so the cold path copies it. Elements kind 0 = no elements.
    if shape::elements_kind(ss) != 0 || shape::elements_kind(rs) != 0 {
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
