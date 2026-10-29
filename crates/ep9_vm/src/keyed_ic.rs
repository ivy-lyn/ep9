//! Keyed access (owner M4-P3 KEYED; OI-04): the fast bodies of the keyed opcodes and the key
//! half of their feedback slots (LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed,
//! StoreInArrayLiteral: 2, 4, 6, 10, 17; IC_LAYOUT 2, the `profile.rs` table).
//!
//! ```text
//! interp/ops.rs (M4-P0 hooks; each after profile::record_keyed, which records nothing)
//!   GetKeyedProperty          array-only inline path (implicit), then -> fast_get   (cold)
//!   SetKeyedProperty          -> fast_set: a present element of an Array            (hit)
//!   DefineKeyedOwnProperty    -> fast_define_own                                    (cold)
//!   StaInArrayLiteral         -> fast_sta_in_array_literal                          (cold)
//!   TestIn                    -> fast_has                                           (cold)
//! cold body: profile::record_keyed_cold (the value half), then the generic operation
//!   (`property::get_keyed`, `set_keyed`, ...: an int32 index makes no string key, M4-P3)
//! ```
//!
//! **The keyed IC state is the key half** (weak), and only this module writes it:
//!
//! | Key half | State | A fast case hits |
//! |---|---|---|
//! | undefined | new (or the shape died at a GC) | no: it records the receiver's shape and goes cold |
//! | a shape | monomorphic: the fast case ran cold once with a receiver of this shape | for a receiver of this shape |
//! | the hole | megamorphic: the fast case ran cold with 2 or more shapes | for any receiver |
//!
//! The **fast case** of `SetKeyedProperty`: the receiver is an Array, the key an int32
//! `i >= 0`, and element `i` is present (below `length`, inside the elements capacity, not a
//! hole), and the value fits the array's elements kind as it is (H18, M5-P1 ELEM1: an int32
//! fits every kind; a non-int32 Value does not fit an INT32 kind; a DOUBLE kind takes only
//! numbers, written as raw f64 with a canonical NaN). Its store replaces the value of an own
//! writable data property (an element is always {w, e, c}), so it needs no `[[Extensible]]`
//! test, no prototype walk and no `proto_index` fuse, and it changes no shape, no kind and
//! no length: a store that needs a kind transition goes cold (transitions happen on the
//! cold store path only). Every other keyed case goes cold and leaves the key half alone, so
//! the key half names only receivers of the fast case.
//!
//! **The cold-miss rule (profile.rs "Keyed sites", MC-w14-6).** A hit records nothing, so a
//! hit is allowed only where the cold path already recorded everything the hit would have
//! recorded: a state change happens only on the fast case and returns `Flow::Cold`, so the
//! cold body records that execution ([`profile::keyed`](crate::profile) `INDEX | ARRAY`; a
//! present element adds no element bit). A hit needs a state that a cold fast-case execution
//! made, so its bits `INDEX | ARRAY` are in the value half (under `type feedback: full`). A
//! function without a vector hits (the record hooks record nothing there either); its first
//! fast-case execution with a vector is cold. The hole and out-of-bounds cases stay cold:
//! their bits (`HOLE`, `OUT_OF_BOUNDS`) differ per execution, and the key half has no room
//! to remember them (no per-site element-case state before the month-5 elements kinds,
//! V9-D39). A LoadKeyed site keeps the implicit INDEX of the ops.rs array path
//! (`facts_export.rs`); this module does not change what an INDEX bit means there
//! (MODES-D17).
//!
//! **IC data only (MC-w14-8, MODES-D11).** The hit reads the key half and the receiver, never
//! the value half that the `profile.rs` record hooks write, so a T0 table that runs no record
//! hook (`type feedback: ics_only`) keeps every hit.
//!
//! **Two tails (M5-P1-ORCH-3, MC-w19-10; M5-P2 ELEM2).** The StoreKeyed hit has two tails
//! with no flag between them: the monomorphic one (store, `Flow::Next`) and the megamorphic
//! one (store, then the sampled kinds-set bit, `profile::keyed_hit_kind`). The monomorphic
//! path pays no budget test. The kinds-set record is the only `profile.rs` hook on either
//! keyed hit path (here, and after the array path of `GetKeyedProperty`, where the bit goes
//! out through `record_next`); it is what the `ics_only` table strips (PROFILE6).
//!
//! **Fuses (MC-w14-18).** The fast case assumes no fuse: it writes a present own element.
//! The generic paths of `property.rs` keep using the `proto_index` flag
//! (`shape::proto_index_keys`, V8's "no elements" protector) for a new element and a hole
//! read.
//!
//! Rules (H16:103, plan rule 8): a hit never allocates and never calls (the element store's
//! barrier and the state change are out of line and cold); the key half holds a shape weakly
//! (a dead shape reads undefined after the GC, and the next cold fast case records the new
//! one), and its store takes the barrier.

use crate::elements;
use crate::feedback::{self, fv};
use crate::frame::{HOLE, UNDEFINED, int32_of};
use crate::interp::dsl::FSlot;
use crate::interp::{Flow, St};
use crate::kinds;
use crate::object::{self, delems, elems, obj, rd, rd_i32};
use crate::profile;
use crate::runtime::{Runtime, val};
use crate::shape;
use ep9_heap::CellPtr;
use ep9_value::encoding::NUMBER_TAG;

/// The byte offset of the key half of slot `i` in a FEEDBACK cell.
#[inline(always)]
const fn key_off(i: u32) -> usize {
    fv::SLOTS + fv::SLOT_SIZE * i as usize + fv::KEY
}

/// The ShapeId of the object `o` (word0), as the key half's low 32 bits hold it.
#[inline(always)]
fn shape_id(o: u64) -> u32 {
    // SAFETY: a cell value the VM holds is live (crate invariant, object.rs).
    unsafe { object::cell(o).header() }.shape_id().get()
}

/// The elements cell and byte offset of element `i` of the receiver `o` for the fast case:
/// `o` an Array, `k` an int32 `>= 0` below `length` and the capacity, the element not a hole.
/// The cell is an ELEMENTS cell (Values) or an ELEMENTS_DOUBLE cell (raw f64, hole
/// [`delems::HOLE`]); the bool is true for the latter. Not a GC point; reads only.
#[inline(always)]
fn present_element(o: u64, k: u64) -> Option<(CellPtr, usize, bool)> {
    if k & NUMBER_TAG != NUMBER_TAG || !kinds::is_array(o) {
        return None;
    }
    let i = u32::try_from(int32_of(k)).ok()?;
    // The dense bound (one load): a DICTIONARY array (M5-P2 ELEM2) has a capacity of 0, so it
    // misses below whatever its `length` word says.
    if i >= object::array_len_dense(o) {
        return None;
    }
    let e = rd(object::cell(o), obj::ELEMENTS);
    if e == UNDEFINED {
        return None;
    }
    let e = object::cell(e);
    if i >= rd_i32(e, elems::CAPACITY) as u32 {
        return None;
    }
    let off = elems::DATA + 8 * i as usize;
    // SAFETY: `e` is live (crate invariant).
    if unsafe { e.header() }.kind() == kinds::ELEMENTS.get() {
        return (rd(e, off) != HOLE).then_some((e, off, false));
    }
    (object::raw_get(e, off) != delems::HOLE).then_some((e, off, true))
}

/// The keyed IC state change of slot `i` of the vector `v` for a fast-case receiver `o`
/// whose shape the key half `key` does not name: new -> monomorphic (the shape),
/// monomorphic -> megamorphic (the hole). Out of line: once per state change.
#[cold]
#[inline(never)]
fn note_receiver(rt: &Runtime, v: u64, i: u32, o: u64, key: u64) {
    let new = if key == UNDEFINED {
        shape::shape_of(o)
    } else {
        HOLE
    };
    // A shape is a cell: the store takes the barrier (the slot is weak for the GC).
    object::wr(&rt.heap, object::cell(v), key_off(i), new);
}

/// `o[k]` (LoadKeyed slot `s`), after the array-only path of `GetKeyedProperty` missed (a
/// hole, out of bounds, another receiver or key). Always cold: the generic `get_keyed` reads
/// an int32 index without a string key.
#[inline(always)]
pub(crate) fn fast_get(_st: &mut St<'_>, _o: u64, _k: u64, _s: FSlot) -> Flow {
    Flow::Cold
}

/// `o[k] = v` (StoreKeyed slot `s`): the fast case stores a present Array element in place,
/// in the storage form of its kind, when `v` needs no kind transition.
///
/// The monomorphic hit (the key half names the receiver's shape) and a function without a
/// vector store and return `Flow::Next`: one ShapeId compare, no record, no budget test (the
/// shape holds the kind; the miss that installed the shape recorded it). The megamorphic hit
/// is its own tail (M5-P1-ORCH-3, MC-w19-10; V8 keeps its megamorphic keyed store apart): its
/// own store, then the sampled kinds-set record ([`set_megamorphic`]); no flag joins the two
/// tails, so the monomorphic path pays nothing for it. A state change goes cold
/// ([`note_receiver`]).
#[inline(always)]
pub(crate) fn fast_set(st: &mut St<'_>, o: u64, k: u64, v: u64, s: FSlot) -> Flow {
    let Some((e, off, doubles)) = present_element(o, k) else {
        return Flow::Cold;
    };
    let fits = if doubles {
        val(v).is_number()
    } else {
        // An int32 fits every Values kind; anything else does not fit an INT32 kind.
        v & NUMBER_TAG == NUMBER_TAG || elements::kind_of(o) > elements::kind::HOLEY_INT32
    };
    if !fits {
        return Flow::Cold;
    }
    if let Some(vec) = feedback::vector(st.info()) {
        let key = feedback::key(vec, s.0);
        // An undefined key and the hole never equal a ShapeId (as in `ic::handler_for`).
        if key as u32 != shape_id(o) {
            if key == HOLE {
                return set_megamorphic(st, o, v, s, e, off, doubles);
            }
            note_receiver(st.rt(), vec, s.0, o, key);
            return Flow::Cold;
        }
    }
    store_present(st.rt(), e, off, doubles, v);
    Flow::Next
}

/// The store of a fast-case hit: `v` in place at `off` of the elements cell `e`, as raw f64
/// with a canonical NaN in a DOUBLE cell, else as a Value with the barrier. Not a GC point.
#[inline(always)]
fn store_present(rt: &Runtime, e: CellPtr, off: usize, doubles: bool, v: u64) {
    if doubles {
        object::raw_set(e, off, object::double_bits(v));
    } else {
        object::wr(&rt.heap, e, off, v);
    }
}

/// The megamorphic tail of [`fast_set`]: the store, then the kinds-set bit of a sampled kind
/// for its dispatch (`profile::keyed_hit_kind`; the hook PROFILE6 strips for `ics_only`). Not
/// a GC point.
#[inline(always)]
fn set_megamorphic(
    st: &mut St<'_>,
    o: u64,
    v: u64,
    s: FSlot,
    e: CellPtr,
    off: usize,
    doubles: bool,
) -> Flow {
    core::hint::cold_path();
    store_present(st.rt(), e, off, doubles, v);
    match profile::keyed_hit_kind(st, o) {
        0 => Flow::Next,
        bits => Flow::Record(profile::Rec::new(s, bits)),
    }
}

/// CreateDataProperty `o[k] = v` with the opcode's `flags` (DefineKeyedOwn slot `s`). Always
/// cold.
#[inline(always)]
pub(crate) fn fast_define_own(
    _st: &mut St<'_>,
    _o: u64,
    _k: u64,
    _v: u64,
    _flags: u8,
    _s: FSlot,
) -> Flow {
    Flow::Cold
}

/// An array literal element `a[i] = v` (StoreInArrayLiteral slot `s`). Always cold: the
/// element is a hole of the boilerplate (a `HOLE` record per execution).
#[inline(always)]
pub(crate) fn fast_sta_in_array_literal(
    _st: &mut St<'_>,
    _a: u64,
    _i: u64,
    _v: u64,
    _s: FSlot,
) -> Flow {
    Flow::Cold
}

/// `k in o` (HasKeyed slot `s`): the result goes to the accumulator. Always cold: the generic
/// `property::has` answers an int32 index without a string key.
#[inline(always)]
pub(crate) fn fast_has(_st: &mut St<'_>, _k: u64, _o: u64, _s: FSlot) -> Flow {
    Flow::Cold
}
