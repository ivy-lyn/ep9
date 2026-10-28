#![allow(dead_code)] // m5-stub
//! Elements kinds (owner M5-P1 ELEM1, then M5-P2 ELEM2; H18, V10-D7, V10-D8, MC-w15-21): the
//! kind of an object's elements store, its storage form, and the keyed kinds-set bits of
//! IC_LAYOUT 3. M5-P0 fixes the values (OBJECT_LAYOUT 5, IC_LAYOUT 3; V10-A21: `layout.rs`,
//! `kinds.rs` and `versions.rs` are frozen after M5-P0); the owners build the lattice.
//!
//! ```text
//!  value  kind              storage form (one storage form counts as one IC case)
//!  0      PACKED_INT32      ┐
//!  1      HOLEY_INT32       │ Values in an ELEMENTS cell (kind 65)
//!  4      PACKED_ELEMENTS   │
//!  5      HOLEY_ELEMENTS    ┘
//!  2      PACKED_DOUBLE     ┐ raw f64 in an ELEMENTS_DOUBLE cell (kind 81; hole = the NaN
//!  3      HOLEY_DOUBLE      ┘ 0xFFF7_FFFF_FFF7_FFFF, `object::delems`)
//!  6      DICTIONARY          an ELEMENTS_DICT cell (kind 82)
//!  7-15   reserved, append-only (NONEXTENSIBLE, SEALED, FROZEN if built later)
//!  bit 0 = holey (for 0-5)
//! ```
//!
//! | Where | What |
//! |---|---|
//! | shape `counts` bits 28-31 (`shape::ELEMENTS_KIND_SHIFT`) | the kind of the shape's objects (H18-Q3) |
//! | keyed slot value half, bits 10-16 (IC_LAYOUT 3, [`KINDS_SET_SHIFT`]) | the set of kinds seen at the site, one bit per value 0-6; bits 0-9 keep their meaning (`profile::keyed`) |
//! | an ALLOC_SITE handler +16 (IC_LAYOUT 3, `ic::hk::ALLOC_SITE`) | the kind an array-literal site makes (MODES-D19, V10-D6) |
//!
//! Rules (H18; the owners' tests): transitions are one-way and happen on the store side only;
//! a load never changes a kind; every DOUBLE store makes a NaN canonical. M5-P0 stubs: no
//! object has a kind yet (the shape bits are 0 and nothing reads them); [`kind_of_array`]
//! gives HOLEY_ELEMENTS, which is what every array is today (Values in an ELEMENTS cell, holes
//! allowed).

/// The elements-kind values (OBJECT_LAYOUT 5; V10-A21). Append-only.
pub(crate) mod kind {
    pub const PACKED_INT32: u8 = 0;
    pub const HOLEY_INT32: u8 = 1;
    pub const PACKED_DOUBLE: u8 = 2;
    pub const HOLEY_DOUBLE: u8 = 3;
    pub const PACKED_ELEMENTS: u8 = 4;
    pub const HOLEY_ELEMENTS: u8 = 5;
    pub const DICTIONARY: u8 = 6;
    /// The values in use (0-6); 7-15 are reserved.
    pub const COUNT: u8 = 7;
    /// The largest value the 4 shape bits hold.
    pub const MAX: u8 = 15;
    /// Bit 0 of a value 0-5: the kind allows holes.
    pub const HOLEY_BIT: u8 = 1;
}

/// The storage form of an elements kind (MC-w15-21: one storage form is one IC case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Storage {
    /// Values in an ELEMENTS cell: PACKED/HOLEY INT32 and ELEMENTS.
    Values,
    /// Raw f64 in an ELEMENTS_DOUBLE cell: PACKED/HOLEY DOUBLE.
    Doubles,
    /// An ELEMENTS_DICT cell.
    Dictionary,
}

/// The storage form of kind `k` (0-6), or `None` for a reserved value.
pub(crate) const fn storage(k: u8) -> Option<Storage> {
    match k {
        kind::PACKED_INT32 | kind::HOLEY_INT32 | kind::PACKED_ELEMENTS | kind::HOLEY_ELEMENTS => {
            Some(Storage::Values)
        }
        kind::PACKED_DOUBLE | kind::HOLEY_DOUBLE => Some(Storage::Doubles),
        kind::DICTIONARY => Some(Storage::Dictionary),
        _ => None,
    }
}

/// True for the holey kinds (1, 3, 5).
pub(crate) const fn is_holey(k: u8) -> bool {
    k < kind::DICTIONARY && k & kind::HOLEY_BIT != 0
}

/// IC_LAYOUT 3: the first bit of the kinds set in a keyed slot's value half (bits 10-16, one
/// bit per kind value 0-6; bits 0-9 keep their IC_LAYOUT 2 meaning, `profile::keyed`). M5-P1
/// ELEM1 writes it.
pub(crate) const KINDS_SET_SHIFT: u32 = 10;

/// The kinds-set bit of kind `k` (0-6).
pub(crate) const fn kinds_set_bit(k: u8) -> u32 {
    1 << (KINDS_SET_SHIFT + k as u32)
}

/// The elements kind of the array `a` (M5-P0 stub: every array keeps Values in an ELEMENTS
/// cell and may have holes, so HOLEY_ELEMENTS). ELEM1 reads the shape bits.
#[inline(always)]
pub(crate) fn kind_of_array(_a: u64) -> u8 {
    kind::HOLEY_ELEMENTS
}

/// The hook of an element store `a[i] = v` before it is done (M5-P0 stub: no kind changes).
/// ELEM1 makes the one-way transition here (store side only; a load never calls it).
#[inline(always)]
pub(crate) fn before_store(_rt: &crate::runtime::Runtime, _a: u64, _i: u32, _v: u64) {}

const _: () = {
    assert!(kind::COUNT as u32 + KINDS_SET_SHIFT <= 31);
    assert!(kind::DICTIONARY < kind::COUNT && kind::COUNT <= kind::MAX);
};
