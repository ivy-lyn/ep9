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
//! today. The kind is registered ([`register_kinds`], OBJECT_LAYOUT 6) with the size and trace
//! functions of the layout above; no cell of it is made before DICT1.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::key::Probe;
use crate::kinds as kind;
use crate::object::{rd_i32, trace_from_8};

/// The PROPS_INDEXED 84 fields (OBJECT_LAYOUT 6; exported as `layout::PROPS_INDEXED_*`).
pub(crate) mod pif {
    /// The entry capacity (an int32 Value).
    pub const CAPACITY: usize = 8;
    /// The entries in use (an int32 Value).
    pub const COUNT: usize = 16;
    /// The hash index: a bucket array of int32 entry numbers, or undefined below 9 entries.
    pub const INDEX: usize = 24;
    /// The first entry: key, value, attributes (int32), as in PROPS 64.
    pub const ENTRIES: usize = 32;
    /// Bytes per entry.
    pub const ENTRY: usize = 24;
}

/// The size of a PROPS_INDEXED cell: the header words and `capacity` entries.
fn props_indexed_size(c: CellPtr) -> usize {
    pif::ENTRIES + pif::ENTRY * rd_i32(c, pif::CAPACITY) as usize
}

/// Every word after word0 is a Value (int32s, the index cell or undefined, the entries).
fn trace_props_indexed(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, props_indexed_size(c));
}

/// Registers the PROPS_INDEXED kind (OBJECT_LAYOUT 6; `props::register_kinds` calls it).
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::PROPS_INDEXED,
        CellKind {
            name: "property-table-indexed",
            size: CellSize::Dynamic(props_indexed_size),
            trace: trace_props_indexed,
        },
    )
    .expect("register a VM cell kind");
}

/// The hook of a dictionary lookup (`props::dict_find`), before the linear scan of the table
/// `p`: `Some(Some(i))` if the index finds `key` at entry `i`, `Some(None)` if the index proves
/// it absent, `None` if `p` has no index (the caller scans). M6-P0 stub: `None`. Not a GC
/// point.
#[inline(always)]
pub(crate) fn find(_p: CellPtr, _key: u64, _how: Probe) -> Option<Option<usize>> {
    None
}
