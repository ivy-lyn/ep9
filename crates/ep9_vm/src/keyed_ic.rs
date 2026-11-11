//! Keyed access (owner M4-P3 KEYED; OI-04): the fast bodies of the keyed opcodes and the key
//! half of their feedback slots (LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed,
//! StoreInArrayLiteral: 2, 4, 6, 10, 17; IC_LAYOUT 2, the `profile.rs` table).
//!
//! ```text
//! interp/ops.rs (M4-P0 hooks; each after profile::record_keyed, which records nothing)
//!   GetKeyedProperty          array-only inline path (implicit), then -> fast_get   (views)
//!   SetKeyedProperty          -> fast_set: a present Array element (hit), grow case, views
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
/// hole, out of bounds, another receiver or key): the view case ([`view::get`], TA-2); all else
/// is cold (the generic `get_keyed` reads an int32 index without a string key).
#[inline(always)]
pub(crate) fn fast_get(st: &mut St<'_>, o: u64, k: u64, s: FSlot) -> Flow {
    view::get(st, o, k, s)
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
            view::set(st, o, k, v, s)
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

#[cfg(test)]
pub(crate) use view::VIEW_HITS;

/// The view cases of the keyed fast bodies (TypedArray TA-2 TA-KEYED).
mod view {
    //! **The view cases (TypedArray TA-2 TA-KEYED; design `docs/plan/notes/typedarray-design.md`
    //! §6.1, §4.1, §5.2).** [`get`] and [`set`] are the tails of `fast_get` and `fast_set` for
    //! a receiver that is no Array case: a TypedArray (a view kind, `kinds::is_view_kind`)
    //! with an int32 key `i >= 0`; anything else is cold. Each starts with one compare of the
    //! cell kind (a receiver kind past `PROXY_CALLABLE` is a view), then a cold-path hint, so
    //! the view code stays off the block order of the array paths as far as LLVM allows (M7-P5
    //! ICWIDE: a new case there cost +2 instructions per array hit; this one costs about 1 per
    //! Array load hit and none per Array store hit, mac a64). It makes no call and reads no
    //! IC state, so the `GetKeyedProperty` handler keeps no frame and the `SetKeyedProperty`
    //! handler saves no more registers.
    //!
    //! ```text
    //! element type t = cell kind - INT8_VIEW; BigInt64, BigUint64 (a BigInt allocates) and
    //!   Float16 (its conversion is a call, `ep9_math::f16`) are cold
    //! bound (IsValidIntegerIndex, read at each access: a detach or resize between two calls
    //!   changes it, so no IC state remembers it):
    //!   TYPED_FIXED (7)     the +40 length (0 once detached)
    //!   TYPED_VARIABLE (8)  a resizable ArrayBuffer: its byte length (slot 1; detached: out of
    //!                       bounds), the +48 byte offset, the +40 length or "auto" (out of
    //!                       bounds when the view does not fit); a growable SharedArrayBuffer:
    //!                       cold (its length is in the shared block, behind the table)
    //! GetKeyedProperty   i in bounds: the element as a Number (Uint32 above 2^31 - 1 a double,
    //!                    a float NaN the one NaN); else undefined (no prototype walk, §10.4.5.4)
    //! SetKeyedProperty   i in bounds and v a Number that needs no call (an int32 for the integer
    //!                    types, which Uint8Clamped clamps; any Number for Float32 and Float64):
    //!                    the store; else cold (out of bounds too; a double into an integer type
    //!                    needs `trunc`, a libm call on baseline x86-64)
    //! ```
    //!
    //! The bytes of a SharedArrayBuffer are read and written with Relaxed atomics of the element
    //! width (TA-D11). A view element is aligned to its size (a byte offset that is a multiple
    //! of the size, on a store base that is 8-aligned), else the case is cold. The bound and the
    //! data word come from the view and from its buffer, which `typed_array::new_view` took only
    //! after `array_buffer::buffer_class` passed: an engine buffer keeps its table entry while
    //! it lives, and only `array_buffer.rs` writes its slots (BRAND-T, design §5.1 note (1)), so
    //! the raw slot reads here trust them. The view case reads no Value slot it does not check
    //! and writes none (a store writes raw bytes: no barrier).
    //!
    //! **The cold-miss rule** holds for the views as for the Arrays: the cold body records
    //! `INDEX | OBJECT` for an int32 key `>= 0` on a view (no element case, and no kinds-set bit:
    //! the view kinds are no `elements::plain_elements_kind`, and the kinds-set bits 7 and 8
    //! have no writer before TA-3; a negative int32 records `OTHER_KEY`, so it is cold here).
    //! - A LoadKeyed view hit records as cold ([`super::record_as_cold`], the grow case form):
    //!   the LoadKeyed key half keeps no state (H16 "the keyed IC state is live only for
    //!   StoreKeyed"), and a state change there would be a call in the `GetKeyedProperty`
    //!   handler, which gives its array path a frame (M7-P5 ICWIDE).
    //! - A StoreKeyed view hit records as cold too, and it leaves the key half alone (as every
    //!   case that is not the Array fast case, MC-w14-6), so the key half names only Array
    //!   receivers, and an Array hit still needs a cold `ARRAY` record first. A view shape in
    //!   the key half (design §6.1) would make the hit read the vector, and that read gave the
    //!   `SetKeyedProperty` handler two more saved registers on entry (+3 instructions per
    //!   Array store hit; navier-stokes +0.4%, mac a64).

    use core::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering::Relaxed};

    use super::record_as_cold;
    use crate::abi::ObjectClass;
    use crate::array_buffer as ab;
    use crate::conv;
    use crate::elements::kind as ek;
    use crate::external_ref::ExternalRef;
    use crate::frame::{UNDEFINED, int32_bits, int32_of};
    use crate::interp::dsl::FSlot;
    use crate::interp::{Flow, St};
    use crate::kinds;
    use crate::layout;
    use crate::object::{self, elems, rd};
    use crate::profile::keyed;
    use crate::runtime::val;
    use crate::shape;
    use ep9_heap::CellPtr;
    use ep9_value::encoding::NUMBER_TAG;

    #[cfg(test)]
    thread_local! {
        /// Tests (TA-2 TA-KEYED): the view hits, loads and stores ([`get`], [`set`]).
        pub(crate) static VIEW_HITS: core::cell::Cell<[u32; 2]> =
            const { core::cell::Cell::new([0, 0]) };
    }

    /// The bits the cold body records for an int32 key `>= 0` on a view
    /// (`profile::keyed_key`, `profile::keyed_receiver`).
    const VIEW_BITS: u32 = keyed::INDEX | keyed::OBJECT;

    /// False if `o` is no view, by one compare: a JS receiver whose cell kind is past
    /// `PROXY_CALLABLE` is a view (the kinds after the views are engine cells, never a
    /// receiver Value); a non-cell gives kind 0. The first test of [`get`] and [`set`], before
    /// the cold-path hint (module doc). Not a GC point.
    #[inline(always)]
    fn maybe_view(o: u64) -> bool {
        kinds::cell_kind(o) > kinds::PROXY_CALLABLE.get()
    }

    /// True if `o` is a TypedArray: a cell of one of the 12 view kinds. Not a GC point.
    #[inline(always)]
    fn is_view(o: u64) -> bool {
        kinds::is_view_kind(kinds::cell_kind(o))
    }

    /// The int32 of the Value `v`, if it is an int32.
    #[inline(always)]
    fn int32(v: u64) -> Option<i32> {
        (v & NUMBER_TAG == NUMBER_TAG).then(|| int32_of(v))
    }

    /// The size in bytes of the element type `t` (a Table 71 row; `layout::VIEW_ELEMENT_SIZES`
    /// with no bounds check).
    #[inline(always)]
    fn elem_size(t: u8) -> usize {
        match t {
            0..=2 => 1,
            3 | 4 | 9 => 2,
            5 | 6 | 10 => 4,
            _ => 8,
        }
    }

    /// Where the key of a view access lands ([`element`]).
    enum At {
        /// Element `i` of type `t` at `p`, aligned to its size, inside the view's live store;
        /// `shared` for the bytes of a SharedArrayBuffer.
        Elem { t: u8, p: *mut u8, shared: bool },
        /// IsValidIntegerIndex is false (out of bounds, detached): a load gives undefined.
        Out,
        /// A case the view hit does not take: the cold body.
        Cold,
    }

    /// Internal slot `i` of the engine buffer `b` (a view's +24: trusted, module doc), read
    /// raw, with no check that can panic. `None` if `b` or its slots word is not a cell (never
    /// for an engine buffer). Not a GC point.
    #[inline(always)]
    fn buffer_slot(b: u64, i: usize) -> Option<u64> {
        let bc = CellPtr::from_value(val(b))?;
        let s = CellPtr::from_value(val(rd(bc, object::slot_obj::SLOTS)))?;
        Some(rd(s, elems::DATA + 8 * i))
    }

    /// IsValidIntegerIndex (§10.4.5.14) of the int32 key `k` on the view `o` against its
    /// current bound, and the address of the element: the rules of the module doc. Not a GC
    /// point; reads only.
    #[inline(always)]
    fn element(o: u64, k: u64) -> At {
        debug_assert!(is_view(o));
        let Some(i) = int32(k).and_then(|i| usize::try_from(i).ok()) else {
            return At::Cold;
        };
        let t = kinds::cell_kind(o) - kinds::INT8_VIEW.get();
        if matches!(t, 7..=9) {
            return At::Cold;
        }
        let size = elem_size(t);
        let c = object::cell(o);
        let b = rd(c, layout::VIEW_BUFFER as usize);
        let Some(bc) = CellPtr::from_value(val(b)) else {
            return At::Cold;
        };
        const SAB: u64 = int32_bits(ObjectClass::SharedArrayBuffer.id() as i32);
        let shared = rd(bc, object::slot_obj::CLASS) == SAB;
        let len = int32(rd(c, layout::VIEW_LENGTH as usize));
        let n = if shape::elements_kind(shape::shape_of(o)) == ek::TYPED_FIXED {
            // The +40 length (0 once detached); a length of 2^31 or more is a double: cold.
            match len {
                Some(n) => n as usize,
                None => return At::Cold,
            }
        } else {
            // TYPED_VARIABLE: the bound reads the buffer at each access (a resize writes no
            // view).
            if shared {
                return At::Cold;
            }
            let (Some(flags), Some(bytes), Some(off), Some(len)) = (
                buffer_slot(b, ab::slot::FLAGS).and_then(int32),
                buffer_slot(b, ab::slot::BYTE_LENGTH).and_then(int32),
                int32(rd(c, layout::VIEW_BYTE_OFFSET as usize)),
                len,
            ) else {
                return At::Cold;
            };
            if flags & ab::flag::DETACHED != 0 {
                return At::Out;
            }
            let (bytes, off) = (bytes as usize, off as usize);
            if len == layout::VIEW_LENGTH_AUTO {
                if off > bytes {
                    return At::Out;
                }
                (bytes - off) / size
            } else {
                // A fixed length on a resizable buffer: out of bounds when it does not fit.
                let n = len as usize;
                if off + n * size > bytes {
                    return At::Out;
                }
                n
            }
        };
        if i >= n {
            return At::Out;
        }
        let data = ExternalRef::<u8>::from_word(rd(c, layout::VIEW_DATA as usize))
            .get()
            .cast_mut();
        let p = data.wrapping_add(i * size);
        if data.is_null() || !p.addr().is_multiple_of(size) {
            return At::Cold;
        }
        At::Elem { t, p, shared }
    }

    /// The little-endian bytes of element type `t` at `p` (Relaxed atomics for shared bytes).
    ///
    /// # Safety
    /// `[p, p + elem_size(t))` is inside a live store, and `p` is aligned to that size.
    #[inline(always)]
    unsafe fn load_raw(p: *mut u8, t: u8, shared: bool) -> u64 {
        // SAFETY: the caller's contract; shared bytes are only accessed atomically (TA-D11).
        unsafe {
            match (elem_size(t), shared) {
                (1, false) => u64::from(p.read()),
                (2, false) => u64::from(u16::from_le(p.cast::<u16>().read())),
                (4, false) => u64::from(u32::from_le(p.cast::<u32>().read())),
                (_, false) => u64::from_le(p.cast::<u64>().read()),
                (1, true) => u64::from(AtomicU8::from_ptr(p).load(Relaxed)),
                (2, true) => u64::from(u16::from_le(AtomicU16::from_ptr(p.cast()).load(Relaxed))),
                (4, true) => u64::from(u32::from_le(AtomicU32::from_ptr(p.cast()).load(Relaxed))),
                (_, true) => u64::from_le(AtomicU64::from_ptr(p.cast()).load(Relaxed)),
            }
        }
    }

    /// Writes the low bytes of `x` as element type `t` at `p`, little endian ([`load_raw`]'s
    /// rules).
    ///
    /// # Safety
    /// As [`load_raw`].
    #[inline(always)]
    unsafe fn store_raw(p: *mut u8, t: u8, x: u64, shared: bool) {
        // SAFETY: as `load_raw`.
        unsafe {
            match (elem_size(t), shared) {
                (1, false) => p.write(x as u8),
                (2, false) => p.cast::<u16>().write((x as u16).to_le()),
                (4, false) => p.cast::<u32>().write((x as u32).to_le()),
                (_, false) => p.cast::<u64>().write(x.to_le()),
                (1, true) => AtomicU8::from_ptr(p).store(x as u8, Relaxed),
                (2, true) => AtomicU16::from_ptr(p.cast()).store((x as u16).to_le(), Relaxed),
                (4, true) => AtomicU32::from_ptr(p.cast()).store((x as u32).to_le(), Relaxed),
                (_, true) => AtomicU64::from_ptr(p.cast()).store(x.to_le(), Relaxed),
            }
        }
    }

    /// RawBytesToNumeric (§25.1.3.15) of the bytes `x` of the Number element type `t`, as
    /// `array_buffer::raw_to_value` boxes it (the int32 form when it fits; the one NaN).
    #[inline(always)]
    fn raw_value(t: u8, x: u64) -> u64 {
        match t {
            0 => int32_bits(i32::from(x as u8 as i8)),
            1 | 2 => int32_bits(i32::from(x as u8)),
            3 => int32_bits(i32::from(x as u16 as i16)),
            4 => int32_bits(i32::from(x as u16)),
            5 => int32_bits(x as u32 as i32),
            6 => conv::num(f64::from(x as u32)),
            10 => conv::num(f64::from(f32::from_bits(x as u32))),
            _ => conv::num(f64::from_bits(x)),
        }
    }

    /// NumericToRawBytes (§25.1.3.17) of `v` for the element type `t` when it needs no call:
    /// an int32 for the integer types (Uint8Clamped clamps it: ToUint8Clamp of an integer), any
    /// Number for Float32 and Float64 (one NaN pattern each, as `array_buffer::number_to_raw`).
    /// `None` for any other value or type.
    #[inline(always)]
    fn value_raw(t: u8, v: u64) -> Option<u64> {
        match t {
            2 => Some(int32(v)?.clamp(0, 255) as u64),
            0..=6 => Some(u64::from(int32(v)? as u32)),
            10 => {
                let n = val(v).as_number()?;
                let f = if n.is_nan() { f32::NAN } else { n as f32 };
                Some(u64::from(f.to_bits()))
            }
            11 => {
                let n = val(v).as_number()?;
                Some(if n.is_nan() { f64::NAN } else { n }.to_bits())
            }
            _ => None,
        }
    }

    /// The view case of `GetKeyedProperty`: for a view `o`, the element or undefined into the
    /// accumulator, recorded as the cold body records it ([`VIEW_BITS`]); else cold. Never
    /// allocates, never calls; not a GC point.
    #[inline(always)]
    pub(super) fn get(st: &mut St<'_>, o: u64, k: u64, s: FSlot) -> Flow {
        if !maybe_view(o) {
            return Flow::Cold;
        }
        core::hint::cold_path();
        if !is_view(o) {
            return Flow::Cold;
        }
        st.acc = match element(o, k) {
            At::Cold => return Flow::Cold,
            At::Out => UNDEFINED,
            At::Elem { t, p, shared } => {
                // SAFETY: element `i` is below the view's current bound, so it is inside the
                // live store (the view holds its buffer and the buffer its store, design
                // §5.1), and `p` is aligned (checked).
                raw_value(t, unsafe { load_raw(p, t, shared) })
            }
        };
        #[cfg(test)]
        VIEW_HITS.with(|c| c.set([c.get()[0] + 1, c.get()[1]]));
        record_as_cold(s, VIEW_BITS)
    }

    /// The view case of `SetKeyedProperty`: for a view `o` (the receiver), the in-bounds store
    /// of a Number that needs no call, recorded as the cold body records it ([`VIEW_BITS`]);
    /// else cold. The key half stays as it is (module doc). Never allocates, never calls; not
    /// a GC point.
    #[inline(always)]
    pub(super) fn set(_st: &mut St<'_>, o: u64, k: u64, v: u64, s: FSlot) -> Flow {
        if !maybe_view(o) {
            return Flow::Cold;
        }
        core::hint::cold_path();
        if !is_view(o) {
            return Flow::Cold;
        }
        let At::Elem { t, p, shared } = element(o, k) else {
            return Flow::Cold;
        };
        let Some(x) = value_raw(t, v) else {
            return Flow::Cold;
        };
        // SAFETY: as `get`; no JS ran after the bound was read (`v` is a Number already).
        unsafe { store_raw(p, t, x, shared) };
        #[cfg(test)]
        VIEW_HITS.with(|c| c.set([c.get()[0], c.get()[1] + 1]));
        record_as_cold(s, VIEW_BITS)
    }
}
