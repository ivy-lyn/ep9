//! Validity cells (M4-P1 VALIDITY; OI-02, V9-D7, H16 K5): the V8-style guard of the prototype
//! IC cases, in place of the one global prototype epoch (`shape::ShapeState::proto_epoch`,
//! which saturates, H16 K18).
//!
//! ```text
//!  prototype P ──word0──> unique shape ──+40──> PROTO_INFO(P) ──+32──> VALIDITY_CELL(P)
//!                                                                       +8  int32 1 = valid
//!  PROTO_INFO(P.[[Prototype]]) ──+32──> VALIDITY_CELL(Q)                +16 users ──> weak list
//!                                         +16 users ──> [ VALIDITY_CELL(P) (weak), ... ]
//!
//!  IC_HANDLER (PROTO_LOAD, MISSING, 0x24 ADD) +16 ──strong──> VALIDITY_CELL of the receiver
//!  shape's prototype: the hit path loads it and compares one word with VALID.
//! ```
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | VALIDITY_CELL | 79 | +8 int32 state (1 valid, 0 invalid), +16 the users list (a weak list, below) or undefined, +24 int32 users in the list, +32 int32 depth, +40 int32 the prototype epoch when it was made | 48 |
//! | weak list (kind TRANSITIONS, 74) | 74 | +8 capacity, +16 entries: VALIDITY_CELLs (weak) or undefined | 16 + 8 cap |
//!
//! Rules:
//! - One current cell per prototype, in its PROTO_INFO +32 (OBJECT_LAYOUT 4); the null
//!   prototype's PROTO_INFO has one too (made in `shape::init`), which never becomes invalid.
//!   A cell guards the whole chain from its prototype to null: it is valid only while no
//!   object on that chain had a structural change (`shape::note_change`: a named property or
//!   an element that appears or goes away, new attributes, `[[SetPrototypeOf]]`,
//!   PreventExtensions, dictionary mode). A value store does not change it.
//! - [`cell_for`] makes a cell only when the cell of the next prototype is valid, and puts
//!   the new cell in that cell's users list. So a valid cell's whole chain has valid cells
//!   (the invariant). [`invalidate`] (from `shape::note_change`) clears a cell, then every
//!   user, recursively, and drops the users list. A cell that is already invalid has no valid
//!   user (the invariant), so the walk stops there: a change costs one load and one compare
//!   unless it clears a valid cell, and each cell is cleared once.
//! - An invalid cell stays invalid: [`cell_for`] makes a new one at the next IC miss. A
//!   handler that keeps an invalid cell misses from then on (H16 K5).
//! - The users list holds its cells weakly (plan rule 8, H16 K7): a user cell lives while its
//!   PROTO_INFO (it is current) or an IC handler keeps it. A cell refers to no prototype, so
//!   a handler that keeps a cell keeps no object alive. The list reuses the TRANSITIONS kind (a
//!   weak array, `shape.rs`); no new kind id (plan v9 rule 18).
//! - Depth: the null prototype's cell has depth 0, a new cell its next cell's depth + 1. A
//!   chain deeper than [`MAX_DEPTH`] gets no cell (its IC cases are not recorded), so
//!   [`invalidate`] and [`cell_for`] recurse at most [`MAX_DEPTH`] times.
//! - Nothing here counts: 2^32 changes saturate nothing (the epoch at +40 is only the debug
//!   cross-check of plan v9 R1, `ic.rs`).
//!
//! | Function | GC point | Job |
//! |---|---|---|
//! | [`register_kind`] | no | VALIDITY_CELL (from `shape::register_kinds`) |
//! | [`new_root_cell`] | yes | the null prototype's cell (`shape::init`) |
//! | [`is_valid`] | no | the hit test: one load, one compare |
//! | [`cell_for`] | yes | the current valid cell of a prototype (made on demand) |
//! | [`invalidate`] | no | a structural change of a prototype (`shape::note_change`) |

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::frame::{UNDEFINED, int32_bits};
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{cell, init, rd, rd_i32, wr};
use crate::runtime::Runtime;
use crate::shape;

/// VALIDITY_CELL fields.
mod vf {
    pub const STATE: usize = 8;
    pub const USERS: usize = 16;
    pub const COUNT: usize = 24;
    pub const DEPTH: usize = 32;
    pub const EPOCH: usize = 40;
    pub const SIZE: usize = 48;
}

/// Weak list fields (the TRANSITIONS layout of `shape.rs`).
mod wl {
    pub const CAPACITY: usize = 8;
    pub const ENTRIES: usize = 16;
}

/// The state word of a valid cell (int32 1).
pub(crate) const VALID: u64 = int32_bits(1);
/// The state word of an invalid cell (int32 0).
const INVALID: u64 = int32_bits(0);

/// The deepest chain that gets a cell (the null prototype's cell has depth 0). A deeper
/// chain's IC cases are not recorded; the generic path serves them.
pub(crate) const MAX_DEPTH: i32 = 64;

const MIN_USERS: usize = 4;

fn trace_cell(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: the heap passes a live VALIDITY_CELL; +16 is a Value slot (the users list,
    // strong; its entries are weak). The other words are int32s.
    t.visit(unsafe { c.slot(vf::USERS) });
}

/// Registers the VALIDITY_CELL kind (79). `shape::register_kinds` calls it.
pub(crate) fn register_kind(heap: &Heap) {
    heap.register_kind(
        kind::VALIDITY_CELL,
        CellKind {
            name: "validity-cell",
            size: CellSize::Fixed(vf::SIZE),
            trace: trace_cell,
        },
    )
    .expect("register VALIDITY_CELL");
}

/// A new valid cell with no users at `depth`. GC point.
fn new_cell(rt: &Runtime, depth: i32) -> u64 {
    let c = rt.alloc(kind::VALIDITY_CELL, vf::SIZE);
    init(c, vf::STATE, VALID);
    init(c, vf::USERS, UNDEFINED);
    init(c, vf::COUNT, int32_bits(0));
    init(c, vf::DEPTH, int32_bits(depth));
    init(c, vf::EPOCH, int32_bits(rt.shapes.proto_epoch.get() as i32));
    c.to_value().to_bits()
}

/// The cell of the null prototype's PROTO_INFO (depth 0): no object is on its chain, so it
/// stays valid. `shape::init` calls it once. GC point.
pub(crate) fn new_root_cell(rt: &Runtime) -> u64 {
    new_cell(rt, 0)
}

/// True if `c` is a valid VALIDITY_CELL: the hit test of the prototype IC cases. Any other
/// value (undefined, an int32) is not valid. Not a GC point.
#[inline(always)]
pub(crate) fn is_valid(c: u64) -> bool {
    is_cell_bits(c) && rd(cell(c), vf::STATE) == VALID
}

/// The hit test of a handler that has a cell (`ic.rs`): `c` is a VALIDITY_CELL. One load and
/// one compare. Not a GC point.
#[inline(always)]
pub(crate) fn valid_cell(c: u64) -> bool {
    debug_assert!(kind::cell_kind(c) == kind::VALIDITY_CELL.get());
    rd(cell(c), vf::STATE) == VALID
}

/// The prototype epoch when the cell `c` was made (the R1 debug cross-check, `ic.rs`).
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) fn made_at(c: u64) -> u32 {
    rd_i32(cell(c), vf::EPOCH) as u32
}

/// The depth of the cell `c` (tests).
#[cfg(test)]
pub(crate) fn depth(c: u64) -> i32 {
    rd_i32(cell(c), vf::DEPTH)
}

/// The entries of the users list of the cell `c` that are not cleared (tests).
#[cfg(test)]
fn live_users(c: u64) -> impl Iterator<Item = u64> {
    let cc = cell(c);
    let list = rd(cc, vf::USERS);
    let n = if list == UNDEFINED {
        0
    } else {
        rd_i32(cc, vf::COUNT) as usize
    };
    (0..n)
        .map(move |i| rd(cell(list), wl::ENTRIES + 8 * i))
        .filter(|&u| u != UNDEFINED)
}

/// The number of users of the cell `c` that are not cleared (tests).
#[cfg(test)]
pub(crate) fn user_count(c: u64) -> usize {
    live_users(c).count()
}

/// True if `u` is in the users list of the cell `c` (tests).
#[cfg(test)]
pub(crate) fn has_user(c: u64, u: u64) -> bool {
    live_users(c).any(|x| x == u)
}

/// The current valid cell of the prototype `p` (an object in prototype mode, or null): the
/// one in its PROTO_INFO, or a new one made now (and the cells of the prototypes above it
/// first, so the invariant holds). `None` if the chain is deeper than [`MAX_DEPTH`]. GC
/// point.
pub(crate) fn cell_for(rt: &Runtime, p: u64) -> Option<u64> {
    ensure(rt, p, MAX_DEPTH)
}

fn ensure(rt: &Runtime, p: u64, budget: i32) -> Option<u64> {
    let info = shape::proto_info_of(rt, p);
    let c = shape::validity_cell_of(info);
    if is_valid(c) {
        return Some(c);
    }
    // The null prototype's cell is always valid, so `p` is an object here.
    debug_assert!(kind::is_object(p));
    if budget == 0 {
        return None;
    }
    // GC points below: `p`, `info`, `parent` and `nc` live in Rust locals (CSS). A GC runs no
    // JS, so no chain changes and `parent` stays valid.
    let parent = ensure(rt, shape::proto(shape::shape_of(p)), budget - 1)?;
    let depth = rd_i32(cell(parent), vf::DEPTH) + 1;
    if depth > MAX_DEPTH {
        return None;
    }
    let nc = new_cell(rt, depth);
    add_user(rt, parent, nc);
    debug_assert!(valid_cell(parent));
    shape::set_validity_cell(rt, info, nc);
    Some(nc)
}

/// Puts `user` in the users list of `c` (weak). A full list first drops its cleared entries;
/// it grows when more than half of it is still in use. GC point.
fn add_user(rt: &Runtime, c: u64, user: u64) {
    let cc = cell(c);
    let n = rd_i32(cc, vf::COUNT) as usize;
    let list = rd(cc, vf::USERS);
    let cap = if list == UNDEFINED {
        0
    } else {
        rd_i32(cell(list), wl::CAPACITY) as usize
    };
    let at = |i: usize| wl::ENTRIES + 8 * i;
    if n < cap {
        wr(&rt.heap, cell(list), at(n), user);
        wr(&rt.heap, cc, vf::COUNT, int32_bits(n as i32 + 1));
        return;
    }
    if cap > 0 {
        // Full: move the entries that are not cleared to the front.
        let l = cell(list);
        let mut live = 0;
        for i in 0..n {
            let u = rd(l, at(i));
            if u != UNDEFINED {
                if i != live {
                    wr(&rt.heap, l, at(live), u);
                }
                live += 1;
            }
        }
        for i in live..n {
            wr(&rt.heap, l, at(i), UNDEFINED);
        }
        if live <= cap / 2 {
            wr(&rt.heap, l, at(live), user);
            wr(&rt.heap, cc, vf::COUNT, int32_bits(live as i32 + 1));
            return;
        }
    }
    let new_cap = (2 * cap).max(MIN_USERS);
    let nl = rt.alloc(kind::TRANSITIONS, at(new_cap));
    init(nl, wl::CAPACITY, int32_bits(new_cap as i32));
    // The allocation may have collected: copy the entries that are still live (the old list
    // lives in `c`).
    let mut m = 0;
    if list != UNDEFINED {
        let l = cell(list);
        for i in 0..rd_i32(cc, vf::COUNT) as usize {
            let u = rd(l, at(i));
            if u != UNDEFINED {
                init(nl, at(m), u);
                m += 1;
            }
        }
    }
    init(nl, at(m), user);
    wr(&rt.heap, cc, vf::USERS, nl.to_value().to_bits());
    wr(&rt.heap, cc, vf::COUNT, int32_bits(m as i32 + 1));
}

/// Clears the cell `c` (a VALIDITY_CELL, or undefined: no cell yet) and every user of it,
/// recursively, and drops the users lists. An invalid cell has no valid user (the
/// invariant), so the walk stops at one. Not a GC point (barriered stores only).
pub(crate) fn invalidate(rt: &Runtime, c: u64) {
    if !is_valid(c) {
        return;
    }
    clear(rt, c);
}

#[cold]
#[inline(never)]
fn clear(rt: &Runtime, c: u64) {
    let cc = cell(c);
    debug_assert!(rd(cc, vf::STATE) == VALID);
    wr(&rt.heap, cc, vf::STATE, INVALID);
    let list = rd(cc, vf::USERS);
    if list == UNDEFINED {
        return;
    }
    let n = rd_i32(cc, vf::COUNT) as usize;
    for i in 0..n {
        let u = rd(cell(list), wl::ENTRIES + 8 * i);
        if is_valid(u) {
            // A user is one deeper: the recursion is at most MAX_DEPTH deep.
            clear(rt, u);
        }
    }
    wr(&rt.heap, cc, vf::USERS, UNDEFINED);
    wr(&rt.heap, cc, vf::COUNT, int32_bits(0));
}

/// Tests: makes the cell `c` valid again (a test that needs a hit with a valid cell after a
/// change, such as one about a dead holder).
#[cfg(test)]
pub(crate) fn force_valid(rt: &Runtime, c: u64) {
    wr(&rt.heap, cell(c), vf::STATE, VALID);
}
