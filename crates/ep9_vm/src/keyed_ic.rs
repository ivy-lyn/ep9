//! Keyed access (owner M4-P3 KEYED; OI-04): the fast bodies of the keyed opcodes and the key
//! half of their feedback slots (LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed,
//! StoreInArrayLiteral: 2, 4, 6, 10, 17; IC_LAYOUT 2, the `profile.rs` table).
//!
//! ```text
//! interp/ops.rs (M4-P0 hooks; each after profile::record_keyed, which records nothing)
//!   GetKeyedProperty          array-only inline path (implicit), then -> fast_get   (cold)
//!   SetKeyedProperty          -> fast_set: a present element of an Array (hit), grow case
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
//! cold store path only). Every other keyed case leaves the key half alone (MC-w14-6), so
//! the key half names only receivers of the fast case.
//!
//! **The cold-miss rule (profile.rs "Keyed sites", MC-w14-6).** A hit records nothing, so a
//! hit is allowed only where the cold path already recorded everything the hit would have
//! recorded: a state change happens only on the fast case and returns `Flow::Cold`, so the
//! cold body records that execution ([`profile::keyed`](crate::profile) `INDEX | ARRAY`; a
//! present element adds no element bit). A hit needs a state that a cold fast-case execution
//! made, so its bits `INDEX | ARRAY` are in the value half (under `type feedback: full`). A
//! function without a vector hits (the record hooks record nothing there either); its first
//! fast-case execution with a vector is cold. A hole or out-of-bounds case is never a hit:
//! its bits (`HOLE`, `OUT_OF_BOUNDS`) differ per execution, and the key half has no room to
//! remember them (V9-D39). The store grow case (`grow_in_place`) records them as the cold body
//! does (`Flow::Record`). A LoadKeyed site keeps the implicit INDEX of the ops.rs array path
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
use crate::frame::{HOLE, UNDEFINED, int32_bits, int32_of};
use crate::interp::dsl::FSlot;
use crate::interp::{Flow, St};
use crate::kinds;
use crate::object::{self, delems, elems, obj, rd, rd_i32};
use crate::profile::{self, keyed};
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
/// ([`note_receiver`]). A missing element goes to the grow case ([`grow_in_place`]).
#[inline(always)]
pub(crate) fn fast_set(st: &mut St<'_>, o: u64, k: u64, v: u64, s: FSlot) -> Flow {
    let Some((e, off, doubles)) = present_element(o, k) else {
        // M7-P5 ICWIDE: a missing element of an Array, in place (in line, no call).
        return if k & NUMBER_TAG == NUMBER_TAG
            && kinds::is_array(o)
            && let Ok(i) = u32::try_from(int32_of(k))
            && let Some(bits) = grow_in_place(st.rt(), o, i, v)
        {
            record_as_cold(s, bits)
        } else {
            Flow::Cold
        };
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
    // A FROZEN array (M7-P3 ELEMKINDS) takes no store: it goes cold before the key half can
    // name its shape, so the monomorphic hit (a shape id compare) needs no test of its own.
    if let Some(vec) = feedback::vector(st.info()) {
        let key = feedback::key(vec, s.0);
        // An undefined key and the hole never equal a ShapeId (as in `ic::handler_for`).
        if key as u32 != shape_id(o) {
            if is_frozen(o) {
                return Flow::Cold;
            }
            if key == HOLE {
                return set_megamorphic(st, o, v, s, e, off, doubles);
            }
            note_receiver(st.rt(), vec, s.0, o, key);
            return Flow::Cold;
        }
    } else if is_frozen(o) {
        return Flow::Cold;
    }
    store_present(st.rt(), e, off, doubles, v);
    Flow::Next
}

/// **The grow case** of `SetKeyedProperty` (M7-P5 ICWIDE).
///
/// `a[i] = v` for the Array `a` whose element `i` is missing (a hole, or at or past `length`)
/// with no allocation: `property::array_store` (the array is extensible and no prototype has
/// an index key, so no setter or read-only property on the chain decides, §10.1.9.2), then
/// `object::array_set` in the case its in-line store takes (the kind `v` needs is the kind
/// the array has, the index is inside the capacity of the elements cell, the array is not a
/// prototype), with `length` set to `i + 1` at or past it. The bits the cold body records
/// (INDEX, ARRAY, the element case and the kinds-set bit of the kind before the store), or
/// `None` and nothing stored. Not a GC point.
///
/// It does not allocate and does not call, and it reads and writes neither half of the slot:
/// [`record_as_cold`] gives the bits to the record of the handler, so the value half after a
/// grow case is the one the cold path gives (the cold-miss rule), and the key half still
/// names only receivers of the fast case.
///
/// The load side has no such case. M7-P5 ICWIDE measured the wide load cases of [`fast_get`]
/// (a hole or an index past `length` of an Array, an element of another object, a string key
/// through the named-load case computation) and left them cold: that body is in line in the
/// `GetKeyedProperty` handler, so any call in it (the shape lookup, `object::element_get`)
/// and any new panic path (the fuse read through `Runtime::root`) makes LLVM put the frame
/// record on entry of the whole handler, and even a call-free hole case moved the block order
/// of the array path (+2 instructions per array hit; crypto +0.3% to +0.6%, mac a64). Their
/// place is the cold body (`interp/ops.rs`).
#[inline(always)]
fn grow_in_place(rt: &Runtime, a: u64, i: u32, v: u64) -> Option<u32> {
    use elements::kind as ek;
    if !crate::property::is_extensible(a) || shape::proto_index_keys(rt) {
        return None;
    }
    let (from, refuse) = elements::store_kind(a, v);
    if refuse || from == ek::DICTIONARY || i >= object::arr::MAX_LENGTH {
        return None;
    }
    let len = object::array_len_dense(a);
    debug_assert!(object::array_get(a, i).is_none());
    // The kind of `object::array_set`; only a store that needs no transition goes on.
    let int32 = v & NUMBER_TAG == NUMBER_TAG;
    let values = from & !ek::HOLEY_BIT == ek::PACKED_ELEMENTS;
    let to = if i <= len && (int32 || (values && v != HOLE)) {
        from
    } else {
        let k = elements::join(from, elements::value_kind(v));
        if i > len { elements::holey(k) } else { k }
    };
    if to != from || shape::is_prototype_object(a) {
        return None;
    }
    let e = rd(object::cell(a), obj::ELEMENTS);
    if e == UNDEFINED {
        return None;
    }
    let e = object::cell(e);
    if i >= rd_i32(e, elems::CAPACITY) as u32 {
        return None;
    }
    // `profile::element_case`; not DICTIONARY, so the dense bound is the length.
    let case = if i >= len {
        keyed::OUT_OF_BOUNDS
    } else {
        keyed::HOLE
    };
    let bits = keyed::INDEX | keyed::ARRAY | case | elements::kinds_set_bit(from);
    let doubles = elements::storage(to) == Some(elements::Storage::Doubles);
    store_present(rt, e, elems::DATA + 8 * i as usize, doubles, v);
    if i >= len {
        object::wr(
            &rt.heap,
            object::cell(a),
            object::arr::LENGTH,
            int32_bits(i as i32 + 1),
        );
    }
    #[cfg(test)]
    GROW_HITS.with(|c| c.set(c.get() + 1));
    Some(bits)
}

/// The dispatch of a grow case ([`grow_in_place`]): `bits` (never 0) as a record of slot
/// `s`, which ORs them into the value half as `profile::record_keyed_cold` would have on the
/// cold path (the record of a `full` table; an `ics_only` table runs none, as its cold hook
/// records nothing). So a slot's value half after a grow case is the one the cold path gives,
/// and the case reads neither half (MC-w14-8).
#[inline(always)]
fn record_as_cold(s: FSlot, bits: u32) -> Flow {
    Flow::Record(profile::Rec::new(s, bits))
}

#[cfg(test)]
thread_local! {
    /// Tests (M7-P5 ICWIDE): how many stores the grow case ([`grow_in_place`]) did.
    pub(crate) static GROW_HITS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// True if the Array `o` is FROZEN (`shape::level::FROZEN`, M7-P3 ELEMKINDS): its elements
/// are not writable. Not a GC point.
#[inline(always)]
fn is_frozen(o: u64) -> bool {
    elements::integrity_of(o) & crate::shape::level::FROZEN != 0
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
