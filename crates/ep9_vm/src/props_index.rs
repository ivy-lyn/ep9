#![allow(dead_code)] // m6-stub
//! Dictionary v1 (owner M6-P1 DICT1; the H17 "Dictionary v1" part, plan v11 V11-D18): the
//! hash index of a dictionary-mode object with more than 8 own properties, so a lookup no
//! longer scans every entry (`props::dict_find`).
//!
//! ```text
//!  PROPS 64 (Frozen layout core, H17:176; unchanged)    up to 8 entries, a linear scan
//!  PROPS_INDEXED 84 (OBJECT_LAYOUT 6, VM6 step 4)        above 8 entries:
//!    +8 capacity, +16 count, +24 index (a bucket array of int32 entry numbers, or undefined
//!    below 9 entries), +32 entries (key, value, attrs) as PROPS 64
//!  hook: props::dict_find -> find (before the linear scan)
//! ```
//!
//! An object moves from PROPS to PROPS_INDEXED above 8 entries and never moves back.
//!
//! M6-P0 stub: no table is indexed; [`find`] answers "not indexed" and the caller scans, as
//! today.

use ep9_heap::CellPtr;

use crate::key::Probe;

/// The hook of a dictionary lookup (`props::dict_find`), before the linear scan of the table
/// `p`: `Some(Some(i))` if the index finds `key` at entry `i`, `Some(None)` if the index proves
/// it absent, `None` if `p` has no index (the caller scans). M6-P0 stub: `None`. Not a GC
/// point.
#[inline(always)]
pub(crate) fn find(_p: CellPtr, _key: u64, _how: Probe) -> Option<Option<usize>> {
    None
}
