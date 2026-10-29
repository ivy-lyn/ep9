//! Field representation and const bits (M5-P2 CONSTREP; the H17 "Representation and const
//! bits" row, C-02): each fast field has a representation, which only widens, and a const bit
//! that a second store clears. A change of either bit set in place pops the field's Dynamic
//! fuse (descriptor kind 3, `fuse.rs`).
//!
//! ```text
//!  details bits 4-6 (shape::rep): NONE 0 -> INT32 1 -> DOUBLE 2 -> ANY 4
//!                                 NONE 0 -> HEAP 3 -> ANY 4
//!  details bit 7 (shape::DETAILS_CONST): no object of the family stored a second value
//!
//!  the value tag (H6):   int32 -> INT32;  any other number -> DOUBLE;  no number -> HEAP
//!
//!  writers                                   what they do with the bits
//!   props::own_add (the first store)          new descriptor: rep_of(v) + CONST (initial);
//!                                            a transition other objects took: widen to v
//!   props::field_set (any later store)        widen to v, clear CONST (before_field_store)
//!   ic::store hit, 0x24 / 0x25 field case     the handler names the rep: the value tag must
//!                                            fit it, else a miss (the miss path widens)
//!   ic::store hit, ADD case                   the target's last field rep must fit, else a miss
//!   ic::global_case (store)                   the lexical record or global object field
//!                                            goes to ANY, not const: its hits do not check
//!   shape::with_elements_kind                 the new shape's fields widen to the old ones
//!   shape::add (no value known)               ANY, not const (cached function shapes, tests)
//!
//!  a widening: shape::field_owner(s, i) -> every DESCRIPTORS cell of that family in place
//!              (shape::set_field_bits_in_family) -> fuse::on_field_change(i)
//! ```
//!
//! Rules:
//! - One-way. The bits of a field only widen: the representation moves up the order above
//!   (join: equal stays; NONE gives the other; INT32 and DOUBLE give DOUBLE; else ANY), and a
//!   cleared const bit never comes back. A shape's field bits describe every value that an
//!   object of the field's family holds in that field now, and every value it held there.
//! - The first store of a field is the add (`props::own_add`): it does not clear the const bit.
//!   Every later store is a second store: it clears the bit for the whole family (so the bit
//!   says "no object of this family ever replaced the value", V8's const field).
//! - A representation is a fact for compiled field loads (H30, month 6+), not a storage form:
//!   a Value holds a double unboxed (H6), so no field is boxed or moved.
//! - The bits live in the shared DESCRIPTORS entry of the field. A family is the owner shape
//!   that added the field and its descendants; a branch has a copy of the entry. A widening
//!   writes every copy of its family (descendants are live while an object uses them; a dead
//!   branch needs no write). A cell shared with another family (a unique shape, the prefix
//!   rule) is widened too, which is safe.
//! - Writers outside this list must keep the rule themselves: a direct field write stores
//!   only values that the field's bits already allow, or widens first ([`widen`],
//!   [`widen_all`]). The month-5 list of such writers is in the H17 row.
//! - Not GC points: no hook allocates; a widening reads and writes int32s in place and pops
//!   fuses (no JS).

use crate::runtime::Runtime;
pub(crate) use crate::shape::rep;
use crate::shape::{self, DETAILS_CONST, Details, FIELD_BITS};

/// The tag of a number: int32 when every bit is set, a double when some are (H6).
const NUMBER_TAG: u64 = ep9_value::encoding::NUMBER_TAG;

/// The representation the value `v` needs ([`rep`]).
#[inline(always)]
pub(crate) fn rep_of(v: u64) -> i32 {
    let t = v & NUMBER_TAG;
    if t == NUMBER_TAG {
        rep::INT32
    } else if t != 0 {
        rep::DOUBLE
    } else {
        rep::HEAP
    }
}

/// True if a field of representation `r` may hold `v` with no widening. NONE holds nothing
/// (its first value widens it).
#[inline(always)]
pub(crate) fn fits(r: i32, v: u64) -> bool {
    let t = v & NUMBER_TAG;
    match r {
        rep::ANY => true,
        rep::INT32 => t == NUMBER_TAG,
        rep::DOUBLE => t != 0,
        rep::HEAP => t == 0,
        _ => false,
    }
}

/// The least representation above both `a` and `b`.
pub(crate) fn join(a: i32, b: i32) -> i32 {
    match (a, b) {
        _ if a == b => a,
        (rep::NONE, x) | (x, rep::NONE) => x,
        (rep::INT32, rep::DOUBLE) | (rep::DOUBLE, rep::INT32) => rep::DOUBLE,
        _ => rep::ANY,
    }
}

/// The join of two sets of field bits ([`FIELD_BITS`], in place): the joined representation,
/// const only if both are.
pub(crate) fn join_bits(a: i32, b: i32) -> i32 {
    let r = join((a & rep::MASK) >> rep::SHIFT, (b & rep::MASK) >> rep::SHIFT);
    (r << rep::SHIFT) | (a & b & DETAILS_CONST)
}

/// The field bits of a new descriptor whose first value is `v`: its representation, const.
#[inline]
pub(crate) fn initial(v: u64) -> i32 {
    (rep_of(v) << rep::SHIFT) | DETAILS_CONST
}

/// The hook of a second store of `v` into fast field `f` of an object with shape `s`
/// (`props::field_set`), before the store: widens the representation to `v` and clears the
/// const bit. Costs one details load and a compare when the field is ANY and not const. Not a
/// GC point.
#[inline(always)]
pub(crate) fn before_field_store(rt: &Runtime, s: u64, f: usize, v: u64) {
    let b = shape::field_bits_at(s, f);
    if b & DETAILS_CONST == 0 && fits((b & rep::MASK) >> rep::SHIFT, v) {
        return;
    }
    generalize(rt, s, f, rep_of(v), true);
}

/// The hook of the first store of `v` into field `f` of an object with shape `s`, the shape
/// an add gave it (`props::own_add`, after `shape::add_with`): a new descriptor holds `v`
/// already; a transition that other objects took widens to `v`. The const bit stays. Not a
/// GC point.
#[inline]
pub(crate) fn on_init_store(rt: &Runtime, s: u64, f: usize, v: u64) {
    let b = shape::field_bits_at(s, f);
    if !fits((b & rep::MASK) >> rep::SHIFT, v) {
        generalize(rt, s, f, rep_of(v), false);
    }
}

/// Widens field `f` of the fast shape `s` (and its family) to ANY and clears its const bit:
/// for a writer whose hits do not check the value (a global store case, `ic::global_case`).
/// Not a GC point.
pub(crate) fn widen(rt: &Runtime, s: u64, f: usize) {
    if shape::field_bits_at(s, f) != shape::ANY_FIELD {
        generalize(rt, s, f, rep::ANY, true);
    }
}

/// [`widen`] for every field of the fast shape `s`. Not a GC point.
#[allow(dead_code)] // for direct writers outside CONSTREP's files (the H17 row lists them)
pub(crate) fn widen_all(rt: &Runtime, s: u64) {
    if shape::is_dictionary(s) {
        return;
    }
    for f in 0..shape::count(s) {
        widen(rt, s, f);
    }
}

/// Widens each field of the fast shape `to` to hold what field `i` of `from` allows: an
/// object moves from `from` to `to` with its values (`shape::with_elements_kind`). Not a GC
/// point.
pub(crate) fn join_fields(rt: &Runtime, from: u64, to: u64) {
    debug_assert!(shape::count(from) == shape::count(to));
    for i in 0..shape::count(from) {
        let (a, b) = (shape::field_bits_at(from, i), shape::field_bits_at(to, i));
        if join_bits(a, b) != b {
            let r = join((a & rep::MASK) >> rep::SHIFT, (b & rep::MASK) >> rep::SHIFT);
            generalize(rt, to, i, r, a & DETAILS_CONST == 0);
        }
    }
}

/// The widening: joins `r` into the representation of field `f` of the fast shape `s` and,
/// if `clear_const`, clears its const bit, in every DESCRIPTORS cell of the field's family
/// (`shape::set_field_bits_in_family`); then pops the kind-3 fuses of the changed entry. Not a
/// GC point.
#[cold]
#[inline(never)]
pub(crate) fn generalize(rt: &Runtime, s: u64, f: usize, r: i32, clear_const: bool) {
    let want = (r << rep::SHIFT) | if clear_const { 0 } else { DETAILS_CONST };
    let owner = shape::field_owner(s, f);
    let changed = shape::set_field_bits_in_family(owner, f, |d: Details| {
        d.with_field_bits(join_bits(d.field_bits(), want) & FIELD_BITS)
    });
    debug_assert!(
        {
            let b = shape::field_bits_at(s, f);
            join((b & rep::MASK) >> rep::SHIFT, r) == (b & rep::MASK) >> rep::SHIFT
                && !(clear_const && b & DETAILS_CONST != 0)
        },
        "a widening left the field of the store's shape narrower"
    );
    if changed {
        note_widened(rt, f);
    }
}

/// Descriptor `f` of some DESCRIPTORS cell widened: pops every kind-3 fuse whose field bits
/// changed (`fuse::on_field_change`). Not a GC point.
#[inline]
pub(crate) fn note_widened(rt: &Runtime, f: usize) {
    crate::fuse::on_field_change(rt, f);
}
