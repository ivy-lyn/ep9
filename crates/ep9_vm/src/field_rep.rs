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
//!   branch needs no write). A prototype-mode object (a unique shape) leaves its tree family
//!   with its own copy of the cell (`shape::proto_info_for`), so no cell is shared between
//!   families: a widening never writes a cell that the family's owner, and so its kind-3
//!   fuse, does not read.
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

/// The Values a representation holds, as one range of bit patterns (M6-P2 T0OBJ): the table
/// form of [`fits`] that the store IC hits read ([`table_fits`]). `v` fits
/// when `v - lo < span` (wrapping). The patterns sort heap below double below int32 (H6: a
/// cell is below 2^47, an immediate is small, a double's top 16 bits are 0x0002..=0xFFF2, an
/// int32's are 0xFFFE), so each representation is one range:
///
/// ```text
///  rep      lo            span           Values
///  NONE     0             0              none
///  INT32    NUMBER_TAG    2^49           [NUMBER_TAG, 2^64)
///  DOUBLE   2^49          2^64 - 2^49    [2^49, 2^64): every number
///  HEAP     0             2^49           [0, 2^49): no number
///  ANY      1             2^64 - 1       every pattern but 0
/// ```
///
/// It gives [`fits`] for every pattern but 0 in ANY, which is never a Value (the niche,
/// `ep9_value`). A check is one subtract and one compare, with no branch on the
/// representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fit {
    lo: u64,
    span: u64,
}

/// The lowest double pattern: every number is at or above it (H6, a double is offset by 2^49).
const DOUBLE_LO: u64 = 1 << 49;

// The ranges are the tag tests of [`fits`]: the tag bits are the top bits (2^64 - 2^49), so
// "every bit set" is `v >= NUMBER_TAG` and "one bit set" is `v >= 2^49`.
const _: () =
    assert!(NUMBER_TAG == 0xFFFE_0000_0000_0000 && NUMBER_TAG.wrapping_neg() == DOUBLE_LO);

impl Fit {
    /// Holds no Value (NONE, and a handler kind that is not a field store).
    pub(crate) const NONE: Fit = Fit { lo: 0, span: 0 };

    /// The range of representation `r` ([`rep`]); NONE for a value that names none.
    pub(crate) const fn of(r: i32) -> Fit {
        match r {
            rep::INT32 => Fit {
                lo: NUMBER_TAG,
                span: NUMBER_TAG.wrapping_neg(),
            },
            rep::DOUBLE => Fit {
                lo: DOUBLE_LO,
                span: DOUBLE_LO.wrapping_neg(),
            },
            rep::HEAP => Fit {
                lo: 0,
                span: DOUBLE_LO,
            },
            rep::ANY => Fit {
                lo: 1,
                span: u64::MAX,
            },
            _ => Fit::NONE,
        }
    }

    /// True if the range holds `v`.
    #[inline(always)]
    pub(crate) const fn holds(self, v: u64) -> bool {
        v.wrapping_sub(self.lo) < self.span
    }
}

/// [`Fit::of`] for every 3-bit representation value (5-7 name none).
static REP_FIT: [Fit; 8] = {
    let mut t = [Fit::NONE; 8];
    let mut r = 0;
    while r < 8 {
        t[r] = Fit::of(r as i32);
        r += 1;
    }
    t
};

/// [`fits`] as a table read, for the paths that run on every store: the add hit of the store
/// IC (`ic::store_with`) and the store hooks below. `r` is a 3-bit representation value
/// (`shape::rep`); `v` is a Value.
#[inline(always)]
pub(crate) fn table_fits(r: i32, v: u64) -> bool {
    debug_assert!((0..8).contains(&r), "a representation is 3 bits: {r}");
    let f = REP_FIT[(r & 7) as usize].holds(v);
    debug_assert!(
        f == fits(r, v) || (v == 0 && r == rep::ANY),
        "the table differs from the tag test"
    );
    f
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
    if b == shape::ANY_FIELD
        || (b & DETAILS_CONST == 0 && table_fits((b & rep::MASK) >> rep::SHIFT, v))
    {
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
    if b & rep::MASK != shape::ANY_FIELD && !table_fits((b & rep::MASK) >> rep::SHIFT, v) {
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

/// [`widen`] for every field of the fast shape `s`: for direct writers outside CONSTREP's
/// files (the H17 row lists them; `literal_site::miss_clone`). Not a GC point.
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
