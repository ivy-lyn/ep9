//! Inline caches for named get and set in T0 (H16 v0; OM-5 to OM-8): the IC entry states,
//! the handlers, the POLY_IC and IC_HANDLER cells, the hit paths that T0's fast handlers
//! inline, and the miss path that computes and records a handler.
//!
//! ```text
//!  feedback slot i (16 B, feedback.rs)       key (weak)                 value (strong)
//!   uninitialized                            undefined                  int32 0
//!   monomorphic                              a shape | KEY_STRING       a handler
//!   polymorphic (2-4 cases)                  hole                       POLY_IC cell
//!   megamorphic (sticky)                     hole                       int32 MEGA
//!     named sites: a miss; the miss path probes the stub cache (stub_cache.rs, M4-P3 IC2),
//!     then takes the generic path
//!
//!  hit: (key as u32) == word0.shape_id of the receiver (the cage is aligned, so a shape's
//!       low 32 bits are its ShapeId), then the handler:
//!   int32 (smi) handler   kind in bits 0-3, byte offset in bits 4-31
//!   IC_HANDLER cell       prototype load, missing property, add-property transition
//! ```
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | POLY_IC | 69 | 4 cases from +8: key (weak), handler (strong) | 72 |
//! | IC_HANDLER | 70 | IC_LAYOUT 2 (below): +8 int32 handler kind, +16 the validity cell (strong) or an int32, +24 a weak ref, +32 an int32 | 40 |
//!
//! **IC_LAYOUT 2** (`versions::IC_LAYOUT_VERSION`, M4-P0; plan v9 §2.3, V9-A8), **3** since M5-P0
//! (the `ALLOC_SITE` handler kind and the keyed kinds-set bits, `profile.rs`). The feedback
//! slot table (which module writes which slot kind) is in the `profile.rs` module docs. The
//! IC_HANDLER cell:
//!
//! | Handler kind ([`hk`]) | +16 | +24 (weak) | +32 (int32) | Writer |
//! |---|---|---|---|---|
//! | `PROTO_LOAD`, `MISSING`, `ADD` (0x24) | the validity cell of the receiver shape's prototype (strong; `validity.rs`, M4-P1) | holder or target shape (undefined for `MISSING`) | the location (a smi field handler) | `ic.rs` |
//! | `ADD` (0x25) | undefined (CreateDataProperty does not look at the chain) | target shape | the location | `ic.rs` |
//! | `GLOBAL` | int32 0 (no chain: the case is an own property of the global object, keyed on two shapes) | lexical shape | the location | `ic.rs` |
//! | `CONSTRUCT_SITE` | int32 count | the initial shape | the `prototype` field index | `construct_ic.rs` (M4-P1 OBJ-ALLOC) |
//! | `INSTANCEOF` | the validity cell | the rhs function's shape (the slot key is the function) | the `prototype` field index | `instance_ic.rs` (M4-P3 IC2) |
//! | `CLONE` | int32 count | the result shape (the slot key is the source shape) | int32 0 | `literal_site.rs` (M4-P1 OBJ-ALLOC) |
//! | `ALLOC_SITE` (IC_LAYOUT 3) | the int32 elements kind the site makes | the result shape | int32 count | `literal_site.rs` (M5-P2 ELEM2) |
//!
//! The size stays 40 B and no kind id is added (construct sites, instanceof and clone caches
//! are IC_HANDLER cells; a Literal site keeps only its slot halves). The trace function visits +24
//! weakly and +16 strongly (a validity cell; an int32 or undefined there needs no visit).
//!
//! | Handler | Sites | Hit |
//! |---|---|---|
//! | `FIELD_IN_OBJECT`, `FIELD_OUT_OF_LINE` (smi) | 0x21, 0x22: own data property; 0x24: own writable data property; 0x25: own data property {w, e, c} | one load or one barriered store at the offset (in the SLOTS cell for out of line) |
//! | `ARRAY_LENGTH` (smi) | 0x21, 0x22 on an array's shape | the length field (+24) |
//! | `STRING_LENGTH` (smi) | 0x21 on a string, under [`KEY_STRING`] | the string's length, after a kind-byte check |
//! | `SLOW` (smi) | any: accessor, dictionary, exotic or read-only cases | none: the generic path runs, the case only keeps its place |
//! | `PROTO_LOAD` (cell) | 0x21, 0x22 | validity cell valid and holder alive: the holder's field |
//! | `MISSING` (cell) | 0x21, 0x22 | validity cell valid: undefined |
//! | `PROTO_LOAD`, `MISSING` (cell) under [`KEY_STRING`] (M4-P3 IC2) | 0x21 on a string: a method (`s.charCodeAt`), from %String.prototype% | as above, after a kind-byte check (a symbol has ShapeId 0 too) |
//! | `ADD` (cell) | 0x24 (validity cell checked), 0x25 | target alive and room in the slots: store, then install the target shape |
//! | smi field, keyed on the lexical record's shape | 0x10, 0x11; 0x12 if writable | a `let`/`const`/`class` binding; the hole (TDZ) misses |
//! | `GLOBAL` (cell), keyed on the global object's shape | 0x10, 0x11; 0x12 if writable | the lexical record still has the shape in the handler: the global object's field |
//!
//! Rules:
//! - Every heap reference in IC data is weak (plan rule 8, I42): entry and case keys, a
//!   handler's holder and target. Only the value half of an entry (a handler or a POLY_IC
//!   cell) and a POLY_IC case's handler are strong. A dead key reads undefined after the GC,
//!   so its case is a miss, and its place is free for the next case.
//! - Handlers that depend on the prototype chain (prototype load, missing property, the
//!   add-property transition of a 0x24 store) keep the validity cell of the receiver shape's
//!   prototype (`validity.rs`, H16 K5) and miss once it is invalid: a structural change of
//!   any prototype on that chain (OM-8, `shape::note_change`) clears it. Changes to other
//!   prototypes do not. A value store to a prototype is not a change, so a prototype load
//!   reads the holder's field at every hit. Nothing counts, so nothing saturates.
//! - The case computation reads the cell it sees ([`load_case`], [`store_pre`]; no GC point).
//!   If that cell is not valid when [`record`] runs, `record` gets the current one
//!   (`validity::cell_for`, a GC point) and walks the chain again: the case is recorded only
//!   if the walk gives the same answer under the valid cell. A chain deeper than
//!   `validity::MAX_DEPTH` gets no cell, and its case is not recorded.
//! - A 0x25 add case keeps no cell (its hit does not look at the chain); the verifier gives
//!   0x24 and 0x25 different slot kinds, so a 0x24 site never reads one.
//! - The debug cross-check (plan v9 R1, V9-A25): in debug builds a hit through a valid cell
//!   walks the chain again and asserts the answer, unless no prototype changed since the
//!   cell was made (the runtime's `ShapeState::proto_epoch` equals the cell's). It reads the
//!   epoch from the runtime itself (M5-P0: `Cx` has no epoch field and the hit paths take no
//!   epoch argument). Release builds do not read the epoch.
//! - A fast shape is always extensible and a non-extensible object has a dictionary shape
//!   (OM-10), so an add-property case, which is made only for a fast shape, can never match a
//!   non-extensible receiver. [`store`] also asserts it.
//! - The megamorphic stub cache (M4-P3 IC2, `stub_cache.rs`, H16-Q3): a megamorphic named
//!   site misses in the hit path ([`load`], [`store`] are unchanged, so a monomorphic hit
//!   costs no more); its miss path first probes a per-isolate table by (operation, receiver
//!   ShapeId, name) and runs the handler it finds with the code above ([`mega_load`],
//!   [`mega_store`]); on a table miss it computes the case for an object receiver
//!   ([`skip_miss`], if [`MEGA_FILL`]) and [`record`] puts it in the table (not `SLOW`, not a
//!   string). The table is emptied at every GC.
//! - A string's method load (M4-P3 IC2) is a prototype case keyed on [`KEY_STRING`]: its chain
//!   starts at %String.prototype% (which gets its PROTO_INFO, prototype mode, when the first
//!   such case is recorded). Global and InstanceOf cases (`instance_ic.rs`) never reach the
//!   stub cache.
//! - Objects whose shape cannot tell their own properties are not cached: dictionary shapes
//!   are shared (`SLOW`), prototype-mode (unique) shapes change at every add (no add case),
//!   and the exotic keys of `exotic.rs` (array and String wrapper `length`, a constructor's
//!   lazy `prototype`) are `SLOW` unless stored. Array-index keys are never cached.
//! - A slot that two sites with different names share would apply one name's handler to the
//!   other. BytecodeGen gives each named site its own slot, and globals one slot per name;
//!   [`init_vector`] checks this for every vector and makes a shared slot megamorphic.
//! - The miss path computes the case before the generic operation (no GC point, no JS), runs
//!   the operation, and records the case only if the operation succeeded. An add-property
//!   case is checked against the shape the operation produced. A global case gives the
//!   operation itself when it is a field: a load reads through it ([`global_case_value`]), a
//!   store writes through it ([`store_global_miss`]), so the miss scans each table once.

use ep9_bytecode::{Instructions, Opcode, OperandKind};
use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::error::FrameRef;
use crate::feedback;
use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of};
use crate::interp::{Cx, slow};
use crate::key;
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, arr, attr, cell, init, obj, rd, rd_i32};
use crate::runtime::{Atom, Root, Runtime};
use crate::shape::{self, slf};
use crate::string;
use crate::stub_cache;
use crate::unit::FunctionInfo;
use crate::validity;

/// The entry and case key that stands for "a string receiver" (strings have no shape; their
/// word0 ShapeId is 0, which is the low half of int32 0). Symbols have ShapeId 0 too, so each
/// case under this key checks the kind byte: [`smi::STRING_LENGTH`], and (M4-P3 IC2) the
/// `PROTO_LOAD` and `MISSING` cases of a string's method load, whose chain starts at
/// %String.prototype% (the `id == 0 && !kind::is_string(o)` miss in the load hit path).
pub(crate) const KEY_STRING: u64 = int32_bits(0);

/// Cases a POLY_IC cell holds (V8's polymorphism limit, OM-6).
pub(crate) const POLY_CASES: usize = 4;

/// Smi handlers: an int32 in an entry's value word, a POLY_IC case, or an IC_HANDLER's
/// location word.
pub(crate) mod smi {
    /// Bits 0-3: the handler kind.
    pub const KIND_MASK: i32 = 0xF;
    /// Bits 4-31: a byte offset.
    pub const SHIFT: u32 = 4;
    /// The field at the byte offset of the object cell.
    pub const FIELD_IN_OBJECT: i32 = 1;
    /// The field at the byte offset of the object's SLOTS cell (+8).
    pub const FIELD_OUT_OF_LINE: i32 = 2;
    /// An array's `length` (the int32 at +24).
    pub const ARRAY_LENGTH: i32 = 3;
    /// A string's `length` (the receiver must be a string).
    pub const STRING_LENGTH: i32 = 4;
    /// The case is known; the generic path runs (accessor, dictionary, exotic, read-only).
    pub const SLOW: i32 = 5;
    /// The value of a megamorphic entry.
    pub const MEGA: i32 = 6;
}

/// IC_HANDLER kinds (the int32 at +8).
pub(crate) mod hk {
    /// A data property of a prototype on the chain: the holder (weak) and its field.
    pub const PROTO_LOAD: i32 = 1;
    /// No object on the chain has the key: undefined.
    pub const MISSING: i32 = 2;
    /// A new data property {w, e, c}: the target shape (weak) and its field.
    pub const ADD: i32 = 3;
    /// A global site (0x10-0x12): an own data property of the global object, while the
    /// lexical record has the shape in `REF` (weak; no `let`/`const`/`class` shadows it).
    pub const GLOBAL: i32 = 4;
    /// IC_LAYOUT 2: a construct site (`construct_ic.rs`, M4-P1 OBJ-ALLOC): +16 int32 count, +24
    /// the initial shape (weak), +32 the int32 `prototype` field index.
    #[allow(dead_code)]
    pub const CONSTRUCT_SITE: i32 = 5;
    /// IC_LAYOUT 2: an instanceof case (`instance_ic.rs`, M4-P3 IC2), keyed by the rhs function:
    /// +16 the validity cell, +24 the function's shape (weak), +32 the int32 `prototype` field
    /// index.
    #[allow(dead_code)]
    pub const INSTANCEOF: i32 = 6;
    /// IC_LAYOUT 2: a CloneObject case (`literal_site.rs`, M4-P1 OBJ-ALLOC), keyed by the source
    /// shape: +16 int32 count, +24 the result shape (weak), +32 int32 0.
    #[allow(dead_code)]
    pub const CLONE: i32 = 7;
    /// IC_LAYOUT 3 (M5-P0; V10-D6, MODES-D19): an array-literal allocation site: +16 the int32
    /// elements kind the site makes (`elements::kind`), +24 the result shape (weak), +32 an
    /// int32 count. The Literal slot of the site and every array made there (ARRAY +32,
    /// `object::arr::ALLOC_SITE`) hold it; one-way generalization; only IC code writes it.
    /// M5-P2 ELEM2 builds it.
    #[allow(dead_code)] // M5-P2 ELEM2
    pub const ALLOC_SITE: i32 = 8;
}

/// IC_HANDLER fields.
mod hc {
    pub const KIND: usize = 8;
    /// The validity cell (PROTO_LOAD, MISSING, 0x24 ADD), undefined (0x25 ADD) or an int32.
    pub const CELL: usize = 16;
    /// The holder (prototype load) or the target shape (add); undefined for a missing case.
    pub const REF: usize = 24;
    /// The field as a smi field handler.
    pub const LOCATION: usize = 32;
    pub const SIZE: usize = 40;
}

/// POLY_IC fields (`crate::layout` exports `CASES`, `CASE` and `HANDLER` for Spike C, C-18).
pub(crate) mod pc {
    pub const CASES: usize = 8;
    pub const CASE: usize = 16;
    /// A case's handler half, after its key (`crate::layout::POLY_HANDLER`, MC-w16-4).
    pub const HANDLER: usize = 8;
    pub const SIZE: usize = CASES + CASE * super::POLY_CASES;
}

#[inline(always)]
const fn smi_handler(kind: i32, offset: usize) -> u64 {
    int32_bits(kind | ((offset as i32) << smi::SHIFT))
}

/// The megamorphic entry value.
pub(crate) const MEGA: u64 = smi_handler(smi::MEGA, 0);
/// The handler of a case whose operation always takes the generic path.
pub(crate) const SLOW: u64 = smi_handler(smi::SLOW, 0);

#[inline(always)]
fn is_i32(b: u64) -> bool {
    b & ep9_value::encoding::NUMBER_TAG == ep9_value::encoding::NUMBER_TAG
}

#[inline(always)]
const fn case_key_off(i: usize) -> usize {
    pc::CASES + pc::CASE * i
}

// ---- kinds -----------------------------------------------------------------------------------

fn trace_poly(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: the heap passes a live POLY_IC cell; its words [8, 72) are 4 (key, handler)
    // Value pairs.
    let s = unsafe { c.slots(pc::CASES, 2 * POLY_CASES) };
    for [key, handler] in s.as_chunks::<2>().0 {
        t.visit_weak(key);
        t.visit(handler);
    }
}

fn trace_handler(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: the heap passes a live IC_HANDLER cell; +16 is a Value slot (a validity cell,
    // undefined or an int32).
    t.visit(unsafe { c.slot(hc::CELL) });
    // SAFETY: as above; +24 is a Value slot (the other words are int32s).
    t.visit_weak(unsafe { c.slot(hc::REF) });
}

/// Registers the POLY_IC and IC_HANDLER cell kinds.
pub(crate) fn register_kinds(heap: &Heap) {
    for (id, k) in [
        (
            kind::POLY_IC,
            CellKind {
                name: "poly-ic",
                size: CellSize::Fixed(pc::SIZE),
                trace: trace_poly,
            },
        ),
        (
            kind::IC_HANDLER,
            CellKind {
                name: "ic-handler",
                size: CellSize::Fixed(hc::SIZE),
                trace: trace_handler,
            },
        ),
    ] {
        heap.register_kind(id, k).expect("register an IC cell kind");
    }
}

// ---- hit paths (inlined into T0's fast handlers: no allocation, no runtime call) ---------------

/// The word0 ShapeId of the cell `o` (0 for a string or a symbol).
#[inline(always)]
fn shape_id(o: u64) -> u32 {
    // SAFETY: a cell value the VM holds is live (crate invariant, object.rs).
    unsafe { cell(o).header() }.shape_id().get()
}

/// The (key, value) entry of slot `i` of the vector of `fi`, if `fi` has a vector.
#[inline(always)]
fn entry(fi: &FunctionInfo, i: u32) -> Option<(u64, u64)> {
    let v = feedback::vector(fi)?;
    Some((feedback::key(v, i), feedback::value(v, i)))
}

/// The handler of the case for a receiver with ShapeId `id`: the monomorphic case or one of
/// the polymorphic ones. `None` is a miss. An empty or dead key (undefined), the hole and a
/// KEY_STRING never equal a ShapeId of an object (those are 8-byte aligned and above the
/// page header), so no other test is needed.
#[inline(always)]
fn handler_for(key: u64, value: u64, id: u32) -> Option<u64> {
    if key as u32 == id {
        return Some(value);
    }
    if key == HOLE && is_cell_bits(value) {
        let p = cell(value);
        for i in 0..POLY_CASES {
            if rd(p, case_key_off(i)) as u32 == id {
                return Some(rd(p, case_key_off(i) + 8));
            }
        }
    }
    None
}

/// The (cell, byte offset) of the field that the smi field handler `loc` names in the object
/// `o`, or `None` if `loc` is another handler.
#[inline(always)]
fn field_ref(o: u64, loc: i32) -> Option<(CellPtr, usize)> {
    let off = (loc >> smi::SHIFT) as usize;
    match loc & smi::KIND_MASK {
        smi::FIELD_IN_OBJECT => Some((cell(o), off)),
        smi::FIELD_OUT_OF_LINE => Some((cell(rd(cell(o), obj::SLOTS)), off)),
        _ => None,
    }
}

/// Reads the field that the smi field handler `loc` names in the object `o`.
#[inline(always)]
fn read_field(o: u64, loc: i32) -> Option<u64> {
    field_ref(o, loc).map(|(c, off)| rd(c, off))
}

/// The debug cross-check of a hit through the valid validity cell of the handler `c` (plan v9
/// R1, V9-A25): unless no prototype changed since the cell was made (the runtime's
/// `ShapeState::proto_epoch` equals the epoch the cell keeps, and it is not saturated), walks the chain of `o` (the receiver or
/// lookup start whose shape is the case key; a string's starts at %String.prototype%) for
/// the site's name `key` and asserts the handler's answer. Not a GC point.
#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn cross_check(rt: &Runtime, o: u64, key: u64, c: CellPtr) {
    let now = rt.shapes.proto_epoch.get();
    if now != shape::EPOCH_SATURATED && now == validity::made_at(rd(c, hc::CELL)) {
        return;
    }
    let start = if kind::is_string(o) {
        rt.root(Root::StringPrototype)
    } else {
        shape::proto(shape::shape_of(o))
    };
    let chain = walk(rt, start, key);
    let ok = match (rd_i32(c, hc::KIND), chain) {
        (hk::PROTO_LOAD, Chain::Data(holder, loc, _)) => {
            holder == rd(c, hc::REF) && int32_of(loc) == rd_i32(c, hc::LOCATION)
        }
        (hk::MISSING, Chain::Missing) => true,
        (hk::ADD, Chain::Missing | Chain::Data(_, _, true)) => true,
        _ => false,
    };
    assert!(
        ok,
        "ic: a valid validity cell gave a stale answer (handler kind {})",
        rd_i32(c, hc::KIND)
    );
}

/// The hit path of a named load (opcodes 0x21 and 0x22): the value of `o.key` from slot
/// `slot` of the vector of `fi`, or `None` on a miss (no vector, no case for `o`'s shape, a
/// `SLOW` case, an invalid validity cell, a dead holder, a megamorphic entry). `o` is the
/// receiver (0x21) or the lookup start (0x22); any value. `name` is the site's name constant
/// (an index into the constants of `fi`): it feeds only the debug cross-check
/// ([`cross_check`]); release builds do not read it.
///
/// A megamorphic entry misses here: its miss path probes the stub cache ([`mega_load`], M4-P3
/// IC2). A probe here would cost every monomorphic hit two loads (the site's name operand and
/// the runtime pointer are arguments that the handler loads before any branch; measured
/// +1.4% instructions on t0-prop, mac a64), so the hit path stays as it was.
#[inline(always)]
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
pub(crate) fn load(rt: &Runtime, fi: &FunctionInfo, o: u64, slot: u32, name: u32) -> Option<u64> {
    if !is_cell_bits(o) {
        return None;
    }
    let (key, value) = entry(fi, slot)?;
    let id = shape_id(o);
    let h = handler_for(key, value, id)?;
    // `move`: a closure that borrowed `name` would keep it in memory, and the T0 handler then
    // loses its sibling tail call in a debug-assertions build at opt-level 2 (Spike C's test
    // profile), so each GetNamedProperty grew the native stack until the RangeError.
    load_with(rt, o, id, move || fi.constant(name), h)
}

/// The value the handler `h` gives for `o` (ShapeId `id`), as for [`load`]. `key` gives the
/// site's name for the debug cross-check only. Not a GC point.
#[inline(always)]
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn load_with(rt: &Runtime, o: u64, id: u32, key: impl Fn() -> u64, h: u64) -> Option<u64> {
    if is_i32(h) {
        let x = int32_of(h);
        return match x & smi::KIND_MASK {
            smi::FIELD_IN_OBJECT | smi::FIELD_OUT_OF_LINE => read_field(o, x),
            smi::ARRAY_LENGTH => Some(rd(cell(o), arr::LENGTH)),
            smi::STRING_LENGTH if kind::is_string(o) => Some(int32_bits(string::str_len(o) as i32)),
            _ => None,
        };
    }
    let c = cell(h);
    let k = rd_i32(c, hc::KIND);
    if (k != hk::PROTO_LOAD && k != hk::MISSING) || !validity::valid_cell(rd(c, hc::CELL)) {
        return None;
    }
    // ShapeId 0 is a string's or a symbol's: a KEY_STRING prototype case serves strings only.
    if id == 0 && !kind::is_string(o) {
        return None;
    }
    if k == hk::MISSING {
        #[cfg(debug_assertions)]
        cross_check(rt, o, key(), c);
        return Some(UNDEFINED);
    }
    let holder = rd(c, hc::REF);
    if holder == UNDEFINED {
        return None;
    }
    #[cfg(debug_assertions)]
    cross_check(rt, o, key(), c);
    read_field(holder, rd_i32(c, hc::LOCATION))
}

/// The hit path of a named store: `o.key = v` (0x24, `CHECK_EPOCH`: an add case checks its
/// validity cell) or CreateDataProperty (0x25) from slot `slot` of the vector of `fi`. False
/// on a miss (also a megamorphic entry: its miss path probes the stub cache, [`mega_store`]);
/// nothing is stored then. `name` (the site's name constant) feeds only the debug cross-check
/// ([`cross_check`]).
#[inline(always)]
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
pub(crate) fn store<const CHECK_EPOCH: bool>(
    rt: &Runtime,
    fi: &FunctionInfo,
    o: u64,
    slot: u32,
    name: u32,
    v: u64,
) -> bool {
    if !is_cell_bits(o) {
        return false;
    }
    let Some((key, value)) = entry(fi, slot) else {
        return false;
    };
    let Some(h) = handler_for(key, value, shape_id(o)) else {
        return false;
    };
    // `move`: as in [`load`] (the sibling tail call of the handler).
    store_with::<CHECK_EPOCH>(rt, o, move || fi.constant(name), v, h)
}

/// Stores `v` through the handler `h` for `o`, as for [`store`]. False if `h` does not apply;
/// nothing is stored then. `key` gives the site's name for the debug cross-check only. Not a
/// GC point.
#[inline(always)]
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn store_with<const CHECK_EPOCH: bool>(
    rt: &Runtime,
    o: u64,
    key: impl Fn() -> u64,
    v: u64,
    h: u64,
) -> bool {
    let c = cell(o);
    if is_i32(h) {
        let x = int32_of(h);
        let off = (x >> smi::SHIFT) as usize;
        match x & smi::KIND_MASK {
            smi::FIELD_IN_OBJECT => object::wr(&rt.heap, c, off, v),
            smi::FIELD_OUT_OF_LINE => object::wr(&rt.heap, cell(rd(c, obj::SLOTS)), off, v),
            _ => return false,
        }
        return true;
    }
    let hcell = cell(h);
    if rd_i32(hcell, hc::KIND) != hk::ADD
        || (CHECK_EPOCH && !validity::valid_cell(rd(hcell, hc::CELL)))
    {
        return false;
    }
    let target = rd(hcell, hc::REF);
    if target == UNDEFINED {
        return false;
    }
    #[cfg(debug_assertions)]
    if CHECK_EPOCH {
        cross_check(rt, o, key(), hcell);
    }
    let loc = rd_i32(hcell, hc::LOCATION);
    let off = (loc >> smi::SHIFT) as usize;
    if loc & smi::KIND_MASK == smi::FIELD_IN_OBJECT {
        object::wr(&rt.heap, c, off, v);
    } else {
        // Out of line: only into room the SLOTS cell has (growth takes the slow path).
        let s = rd(c, obj::SLOTS);
        if s == UNDEFINED || off >= slf::DATA + 8 * rd_i32(cell(s), slf::CAPACITY) as usize {
            return false;
        }
        object::wr(&rt.heap, cell(s), off, v);
    }
    // An add case is made for a fast, extensible shape that is not in prototype mode
    // ([`store_pre`]), so `o` is extensible and its change invalidates no validity cell.
    debug_assert!(shape::is_extensible(shape::shape_of(o)) && !shape::is_prototype_object(o));
    shape::install(rt, o, target);
    true
}

// ---- the megamorphic stub cache (M4-P3 IC2): probes of the miss path ----------------------

/// The miss path of a named load (`slow::get_named`, `slow::get_super`) before the generic
/// load: if slot `slot` of the vector `v` is megamorphic and the stub cache (`stub_cache.rs`)
/// has an entry for `o`'s shape and the name `key` (an atom), the value its handler gives.
/// `None`: take the generic path (which then records the case in the stub cache, [`record`]).
/// No allocation, no JS, not a GC point.
pub(crate) fn mega_load(rt: &Runtime, v: u64, slot: u32, o: u64, key: u64) -> Option<u64> {
    if !kind::is_object(o) || !is_mega(v, slot) {
        return None;
    }
    let id = shape_id(o);
    let r = stub_cache::probe(rt, stub_cache::Op::Load, id, key)
        .and_then(|h| load_with(rt, o, id, || key, h));
    stub_stats!(Load, r.is_some());
    r
}

/// The miss path of a named store (`slow::set_named`) before the generic store: `o.key = val`
/// (0x24) or CreateDataProperty (0x25, `define`) through the stub cache entry for `o`'s shape,
/// if slot `slot` of `v` is megamorphic and there is one. True if it stored; false: take the
/// generic path. No allocation, no JS, not a GC point.
pub(crate) fn mega_store(
    rt: &Runtime,
    v: u64,
    slot: u32,
    o: u64,
    key: u64,
    val: u64,
    define: bool,
) -> bool {
    if !kind::is_object(o) || !is_mega(v, slot) {
        return false;
    }
    let id = shape_id(o);
    let done = match stub_cache::probe(rt, store_op(define), id, key) {
        Some(h) if define => store_with::<false>(rt, o, || key, val, h),
        Some(h) => store_with::<true>(rt, o, || key, val, h),
        None => false,
    };
    stub_stats!(Store, done);
    done
}

/// True if slot `i` of the vector `v` is megamorphic (the hole and int32 MEGA). Not a GC point.
#[inline]
fn is_mega(v: u64, i: u32) -> bool {
    feedback::key(v, i) == HOLE && feedback::value(v, i) == MEGA
}

/// The global-object handler `h` of a global site, if it is one and the lexical record still
/// has the shape it was made under (ShapeId `lid`). A dead lexical shape reads undefined, which
/// no ShapeId equals.
#[inline(always)]
fn global_handler(h: u64, lid: u32) -> Option<CellPtr> {
    if is_i32(h) {
        return None;
    }
    let c = cell(h);
    (rd_i32(c, hc::KIND) == hk::GLOBAL && rd(c, hc::REF) as u32 == lid).then_some(c)
}

/// The hit path of a global load (0x10 `LdaGlobal`, 0x11 `LdaGlobalInsideTypeof`) from slot
/// `slot` of the vector of `fi`: a binding of the lexical record (keyed on its shape; the
/// hole of a TDZ misses), or an own data property of the global object `global` (keyed on
/// its shape, while the lexical record keeps the shape the case was made under). `None` is a
/// miss.
#[inline(always)]
pub(crate) fn load_global(rt: &Runtime, fi: &FunctionInfo, global: u64, slot: u32) -> Option<u64> {
    let (key, value) = entry(fi, slot)?;
    let lexical = rt.root(Root::Lexical);
    let lid = shape_id(lexical);
    if let Some(h) = handler_for(key, value, lid) {
        let v = if is_i32(h) {
            read_field(lexical, int32_of(h))?
        } else {
            return None;
        };
        return (v != HOLE).then_some(v);
    }
    let h = handler_for(key, value, shape_id(global))?;
    let c = global_handler(h, lid)?;
    read_field(global, rd_i32(c, hc::LOCATION))
}

/// The hit path of a global store (0x12 `StaGlobal`): a writable lexical binding outside its
/// TDZ, or a writable own data property of the global object (as for [`load_global`]). False
/// on a miss; nothing is stored then.
#[inline(always)]
pub(crate) fn store_global(
    rt: &Runtime,
    fi: &FunctionInfo,
    global: u64,
    slot: u32,
    v: u64,
) -> bool {
    let Some((key, value)) = entry(fi, slot) else {
        return false;
    };
    let lexical = rt.root(Root::Lexical);
    let lid = shape_id(lexical);
    if let Some(h) = handler_for(key, value, lid) {
        if !is_i32(h) {
            return false;
        }
        let Some((c, off)) = field_ref(lexical, int32_of(h)) else {
            return false;
        };
        if rd(c, off) == HOLE {
            return false;
        }
        object::wr(&rt.heap, c, off, v);
        return true;
    }
    let Some(h) = handler_for(key, value, shape_id(global)) else {
        return false;
    };
    let Some(hcell) = global_handler(h, lid) else {
        return false;
    };
    let Some((c, off)) = field_ref(global, rd_i32(hcell, hc::LOCATION)) else {
        return false;
    };
    object::wr(&rt.heap, c, off, v);
    true
}

// ---- states ----------------------------------------------------------------------------------

/// The state of one IC entry (K3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Uninitialized,
    Monomorphic,
    /// With this many live cases.
    Polymorphic(usize),
    Megamorphic,
}

/// The state of slot `i` of the vector `v`.
pub(crate) fn state(v: u64, i: u32) -> State {
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    if key == UNDEFINED {
        State::Uninitialized
    } else if key != HOLE {
        State::Monomorphic
    } else if is_i32(value) {
        State::Megamorphic
    } else {
        let p = cell(value);
        let n = (0..POLY_CASES)
            .filter(|&c| rd(p, case_key_off(c)) != UNDEFINED)
            .count();
        State::Polymorphic(n)
    }
}

/// The handler slot `i` of the vector `v` keeps for the case key `k`, if any.
pub(crate) fn find(v: u64, i: u32, k: u64) -> Option<u64> {
    let (key, value) = (feedback::key(v, i), feedback::value(v, i));
    if key == k {
        return Some(value);
    }
    if key == HOLE && !is_i32(value) {
        let p = cell(value);
        return (0..POLY_CASES)
            .find(|&c| rd(p, case_key_off(c)) == k)
            .map(|c| rd(p, case_key_off(c) + 8));
    }
    None
}

/// The case key of a receiver or lookup start: its shape, or [`KEY_STRING`] for a string.
fn case_key(o: u64) -> Option<u64> {
    if kind::is_object(o) {
        Some(shape::shape_of(o))
    } else if kind::is_string(o) {
        Some(KEY_STRING)
    } else {
        None
    }
}

/// True if the miss path of a megamorphic entry fills the stub cache (computes the case of an
/// object receiver for [`record`]), which the miss paths probe (`slow::get_named`,
/// `get_super`, `set_named` calling [`mega_load`] and [`mega_store`]).
const MEGA_FILL: bool = true;

/// True if the miss path of slot `i` of `v` for the receiver `o` need not compute a case: `o`
/// has no case key, or its case is `SLOW` (the generic path runs every time). A megamorphic
/// entry computes a case for an object receiver (M4-P3 IC2, if [`MEGA_FILL`]): [`record`]
/// puts it in the stub cache. Not a GC point.
pub(crate) fn skip_miss(v: u64, i: u32, o: u64) -> bool {
    if state(v, i) == State::Megamorphic {
        return !MEGA_FILL || !kind::is_object(o);
    }
    match case_key(o) {
        None => true,
        Some(k) => find(v, i, k) == Some(SLOW),
    }
}

// ---- cases (the miss path) -------------------------------------------------------------------

/// A handler before it is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handler {
    /// An int32 handler.
    Smi(u64),
    /// A prototype load: holder, its field (a smi field handler), the validity cell of the
    /// key's prototype that the case computation saw (valid, invalid or undefined; [`record`]
    /// checks it) and the name (for the walk again in [`record`]).
    ProtoLoad {
        holder: u64,
        location: i32,
        cell: u64,
        name: u64,
    },
    /// A missing property: the validity cell seen (as for `ProtoLoad`) and the name.
    Missing { cell: u64, name: u64 },
    /// An add-property transition: the target shape, its new field, and for 0x24 the
    /// validity cell seen (as for `ProtoLoad`); `None` for 0x25 (no chain check).
    Add {
        target: u64,
        location: i32,
        chain: Option<u64>,
    },
    /// An own data property of the global object (its field) under the lexical record's
    /// shape `lexical`.
    Global { lexical: u64, location: i32 },
    /// A handler that its writer module made (an `INSTANCEOF` IC_HANDLER cell or `SLOW`,
    /// `instance_ic.rs`); recorded as it is.
    Ready(u64),
}

/// A case for one entry: its key (a shape, [`KEY_STRING`], or the rhs function of an
/// InstanceOf case) and its handler. The shape and the holder live in Rust locals until
/// [`record`] stores them (the conservative stack scan keeps them, H10). `stub`: the
/// operation and the name of a named-property case, for the stub cache when its entry is
/// megamorphic (`stub_cache.rs`); `None` for global and InstanceOf cases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Case {
    pub key: u64,
    pub handler: Handler,
    pub stub: Option<(stub_cache::Op, u64)>,
}

/// True if `k` is a key the IC may cache: an atom that is not an array index. Every IC case
/// asks this first. A private name (`#x`, symbol.rs) never reaches IC code: the keyed paths
/// send it to `slow/private.rs`, and T0 records no keyed IC (C7). The assert keeps that true
/// when M1 adds keyed ICs.
fn cacheable_key(k: u64) -> bool {
    debug_assert!(
        !crate::symbol::is_private_key(k),
        "IC code saw a private name (C7)"
    );
    string::is_atom(k) && key::array_index_of(k).is_none()
}

/// True if objects of kind `k` may have the exotic own property `name` that their storage
/// does not show (`exotic.rs`): an array's or a String wrapper's `length`, a constructor's
/// lazy `prototype`, any export of a module namespace (OBJECT_LAYOUT 3: its exports are not
/// in its shape, `module.rs`), any name of a proxy (its cell's shape is empty with a null
/// prototype; its internal methods answer, `proxy.rs`; M4-P4 fixer B, R-VM-P4-1), so a
/// proxy receiver, holder or lookup start, or a proxy on the chain, takes the generic path.
/// Array-index keys never get here, so an arguments object (whose exotic keys are mapped
/// indices) needs no entry.
fn exotic_name(rt: &Runtime, k: u8, name: u64) -> bool {
    if k == kind::ARRAY.get() || k == kind::PRIMITIVE_WRAPPER.get() {
        name == rt.atom(Atom::Length)
    } else if k == kind::FUNCTION.get() {
        name == rt.atom(Atom::Prototype)
    } else {
        k == kind::MODULE_NAMESPACE.get() || crate::proxy::is_proxy_kind(k)
    }
}

/// The smi field handler of field `f` of objects with the fast shape `s`.
fn field_location(s: u64, f: usize) -> u64 {
    let (cap, k) = shape::field_layout(s);
    if f < cap {
        let off = obj::FIXED + 8 * (object::fixed_count(k) + f);
        smi_handler(smi::FIELD_IN_OBJECT, off)
    } else {
        smi_handler(smi::FIELD_OUT_OF_LINE, slf::DATA + 8 * (f - cap))
    }
}

/// What a walk from the prototype `p` finds for `name` (no property on the receiver).
enum Chain {
    /// A data property of the fast-mode holder: (holder, location, writable).
    Data(u64, u64, bool),
    /// No object has it.
    Missing,
    /// An accessor, an exotic property, or a dictionary-mode object that has it.
    Other,
}

/// Walks the prototype chain from `p` (an object or null) for `name`. Not a GC point.
fn walk(rt: &Runtime, mut p: u64, name: u64) -> Chain {
    while p != NULL {
        let s = shape::shape_of(p);
        if exotic_name(rt, shape::object_kind(s), name) {
            return Chain::Other;
        }
        if shape::is_dictionary(s) {
            if object::own_get(p, name).is_some() {
                return Chain::Other;
            }
        } else if let Some(f) = shape::lookup(rt, s, name) {
            let a = f.details.attrs();
            if a & attr::ACCESSOR != 0 {
                return Chain::Other;
            }
            let loc = field_location(s, usize::from(f.details.field()));
            return Chain::Data(p, loc, a & attr::WRITABLE != 0);
        }
        p = shape::proto(s);
    }
    Chain::Missing
}

/// The case for a named load of `name` from `o` (the receiver of 0x21, the lookup start of
/// 0x22), computed before the load runs. `None`: nothing to record (a primitive other than a
/// string, a key the IC does not cache). Not a GC point.
pub(crate) fn load_case(rt: &Runtime, o: u64, name: u64) -> Option<Case> {
    if !cacheable_key(name) {
        return None;
    }
    if kind::is_string(o) {
        if name == rt.atom(Atom::Length) {
            return Some(Case {
                key: KEY_STRING,
                handler: Handler::Smi(smi_handler(smi::STRING_LENGTH, 0)),
                stub: None,
            });
        }
        // M4-P3 IC2: a method load (`s.charCodeAt`, crypto's misses, plan v9 §12.1) is a
        // prototype case from %String.prototype%, keyed on KEY_STRING. A string's other own
        // properties are its indices, which are never cached.
        let p = rt.root(Root::StringPrototype);
        let cell = chain_cell_seen(rt, p);
        let handler = match walk(rt, p, name) {
            Chain::Data(holder, loc, _) => Handler::ProtoLoad {
                holder,
                location: int32_of(loc),
                cell,
                name,
            },
            Chain::Missing => Handler::Missing { cell, name },
            Chain::Other => Handler::Smi(SLOW),
        };
        return Some(Case {
            key: KEY_STRING,
            handler,
            stub: None,
        });
    }
    if !kind::is_object(o) {
        return None;
    }
    let s = shape::shape_of(o);
    let k = shape::object_kind(s);
    let stub = Some((stub_cache::Op::Load, name));
    let smi_case = |h| {
        Some(Case {
            key: s,
            handler: Handler::Smi(h),
            stub,
        })
    };
    if k == kind::ARRAY.get() && name == rt.atom(Atom::Length) {
        return smi_case(smi_handler(smi::ARRAY_LENGTH, 0));
    }
    if shape::is_dictionary(s) {
        return smi_case(SLOW);
    }
    match shape::lookup(rt, s, name) {
        Some(f) if f.details.attrs() & attr::ACCESSOR != 0 => smi_case(SLOW),
        Some(f) => smi_case(field_location(s, usize::from(f.details.field()))),
        // An exotic property that is not in the storage (a lazy `prototype`, a String
        // wrapper's `length`).
        None if exotic_name(rt, k, name) => smi_case(SLOW),
        None => {
            let p = shape::proto(s);
            let cell = chain_cell_seen(rt, p);
            let handler = match walk(rt, p, name) {
                Chain::Data(holder, loc, _) => Handler::ProtoLoad {
                    holder,
                    location: int32_of(loc),
                    cell,
                    name,
                },
                Chain::Missing => Handler::Missing { cell, name },
                Chain::Other => Handler::Smi(SLOW),
            };
            Some(Case {
                key: s,
                handler,
                stub,
            })
        }
    }
}

/// What the miss path of a store knows before the store runs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StorePre {
    /// The case is decided.
    Done(Case),
    /// The store may add `name` to an object with the shape `from`; the shape it produces
    /// decides ([`store_case_after`]). `chain`: the validity cell seen for a 0x24 store,
    /// `None` for 0x25.
    Add { from: u64, chain: Option<u64> },
}

/// The case for `o.name = v` (0x24, `define` false) or CreateDataPropertyOrThrow (0x25,
/// `define` true), before the store runs. Not a GC point.
pub(crate) fn store_pre(rt: &Runtime, o: u64, name: u64, define: bool) -> Option<StorePre> {
    if !cacheable_key(name) || !kind::is_object(o) {
        return None;
    }
    let s = shape::shape_of(o);
    let k = shape::object_kind(s);
    let stub = Some((store_op(define), name));
    let slow = Some(StorePre::Done(Case {
        key: s,
        handler: Handler::Smi(SLOW),
        stub,
    }));
    if shape::is_dictionary(s) || exotic_name(rt, k, name) {
        return slow;
    }
    if let Some(f) = shape::lookup(rt, s, name) {
        let a = f.details.attrs();
        // 0x24: a writable data property takes the value. 0x25: only a {w, e, c} data
        // property keeps its attributes (anything else is redefined, OM-10).
        let ok = if define {
            a == attr::DEFAULT
        } else {
            a & attr::ACCESSOR == 0 && a & attr::WRITABLE != 0
        };
        if !ok {
            return slow;
        }
        return Some(StorePre::Done(Case {
            key: s,
            handler: Handler::Smi(field_location(s, usize::from(f.details.field()))),
            stub,
        }));
    }
    // An add. A prototype-mode object gets a new unique shape at every add, so a case for its
    // shape never hits again: record nothing.
    if shape::flags(s) & shape::flag::UNIQUE != 0 {
        return None;
    }
    // A constructor whose `prototype` is lazy makes it before any other key (props::own_add),
    // and the shape cannot tell constructors from other functions.
    if k == kind::FUNCTION.get() && shape::lookup(rt, s, rt.atom(Atom::Prototype)).is_none() {
        return slow;
    }
    let mut chain = None;
    if !define {
        // [[Set]] adds to the receiver only if the chain has no setter and no read-only
        // property `name` (§10.1.9.2); a writable data property on the chain is fine. The
        // validity cell guards that answer.
        let p = shape::proto(s);
        chain = Some(chain_cell_seen(rt, p));
        if !adds_past(walk(rt, p, name)) {
            return slow;
        }
    }
    Some(StorePre::Add { from: s, chain })
}

/// The stub cache operation of a named store: 0x25 (`define`) or 0x24.
fn store_op(define: bool) -> stub_cache::Op {
    if define {
        stub_cache::Op::Define
    } else {
        stub_cache::Op::Store
    }
}

/// True if a `[[Set]]` that finds `chain` on the prototype chain adds an own data property to
/// the receiver: no object has the key, or a writable data property has it.
fn adds_past(chain: Chain) -> bool {
    matches!(chain, Chain::Missing | Chain::Data(_, _, true))
}

/// The validity cell that the PROTO_INFO of the prototype `p` keeps now (valid, invalid or
/// undefined): what a case computation saw. Undefined for an object that is not in prototype
/// mode yet (%String.prototype% until a shape or [`chain_cell`] names it). Not a GC point.
fn chain_cell_seen(rt: &Runtime, p: u64) -> u64 {
    if p != NULL && !shape::is_prototype_object(p) {
        return UNDEFINED;
    }
    shape::validity_cell_of(shape::proto_info_of(rt, p))
}

/// The validity cell a handler for the case key `key` (a shape) keeps: `seen`, if it is still
/// valid (no prototype on the chain of `key` changed since the case was computed); else the
/// current cell of `key`'s prototype (`validity::cell_for`), if a walk for `name` gives the
/// case's answer (`same`) under it. `None`: record nothing (the chain changed the answer, or
/// it is deeper than `validity::MAX_DEPTH`). GC point.
fn chain_cell(
    rt: &Runtime,
    key: u64,
    seen: u64,
    name: u64,
    same: impl Fn(Chain) -> bool,
) -> Option<u64> {
    if validity::is_valid(seen) {
        return Some(seen);
    }
    // GC point: `key`, `name` and the case's references live in the caller's Rust locals
    // (CSS). A GC runs no JS, so the walk below sees the chain the new cell guards.
    let p = key_proto(rt, key);
    if p != NULL && !shape::is_prototype_object(p) {
        // %String.prototype% (KEY_STRING) gets its PROTO_INFO here (prototype mode).
        shape::proto_info_for(rt, p);
    }
    let c = validity::cell_for(rt, p)?;
    same(walk(rt, p, name)).then_some(c)
}

/// The prototype a case for the key `key` starts its chain at: the shape's, or
/// %String.prototype% for [`KEY_STRING`] (a string's prototype in the current realm; a vector
/// belongs to one FunctionInfo, whose functions are all of one realm, `realm.rs`). Not a GC
/// point.
fn key_proto(rt: &Runtime, key: u64) -> u64 {
    if key == KEY_STRING {
        rt.root(Root::StringPrototype)
    } else {
        shape::proto(key)
    }
}

/// The case of a store that ran: a decided case, or for an add, the transition from `from`
/// to the shape `o` has now, if the store added `name` as the next data property {w, e, c}
/// of a fast shape. Anything else (dictionary mode, another shape) is `SLOW`. Not a GC point.
pub(crate) fn store_case_after(_rt: &Runtime, pre: StorePre, o: u64, name: u64) -> Option<Case> {
    let (from, chain) = match pre {
        StorePre::Done(c) => return Some(c),
        StorePre::Add { from, chain } => (from, chain),
    };
    let to = shape::shape_of(o);
    let n = shape::count(from);
    let added = !shape::is_dictionary(to)
        && shape::flags(to) & shape::flag::UNIQUE == 0
        && shape::parent(to) == from
        && shape::count(to) == n + 1
        && shape::key_at(to, n) == name
        && shape::details_at(to, n).attrs() == attr::DEFAULT
        && usize::from(shape::details_at(to, n).field()) == n;
    let handler = if added {
        debug_assert!(shape::is_extensible(from));
        Handler::Add {
            target: to,
            location: int32_of(field_location(to, n)),
            chain,
        }
    } else {
        Handler::Smi(SLOW)
    };
    Some(Case {
        key: from,
        handler,
        stub: Some((store_op(chain.is_none()), name)),
    })
}

/// The case for a global load (`store` false) or store of `name`, before it runs: a binding
/// of the lexical record (keyed on its shape; a `const` is `SLOW` for a store), or an own data
/// property of the global object (keyed on its shape, under the lexical record's shape; a
/// read-only one is `SLOW` for a store). Accessors and dictionary-mode records are `SLOW`. An
/// inherited or missing global gives `None` (a sloppy store makes it, and the next miss
/// records it). Not a GC point.
pub(crate) fn global_case(rt: &Runtime, name: u64, store: bool) -> Option<Case> {
    if !cacheable_key(name) {
        return None;
    }
    let slow = |key| {
        Some(Case {
            key,
            handler: Handler::Smi(SLOW),
            stub: None,
        })
    };
    let usable = |a: i32| a & attr::ACCESSOR == 0 && (!store || a & attr::WRITABLE != 0);
    let ls = shape::shape_of(rt.root(Root::Lexical));
    if shape::is_dictionary(ls) {
        return slow(ls);
    }
    if let Some(f) = shape::lookup(rt, ls, name) {
        if !usable(f.details.attrs()) {
            return slow(ls);
        }
        let loc = field_location(ls, usize::from(f.details.field()));
        return Some(Case {
            key: ls,
            handler: Handler::Smi(loc),
            stub: None,
        });
    }
    let gs = shape::shape_of(rt.global());
    if shape::is_dictionary(gs) {
        return slow(gs);
    }
    let f = shape::lookup(rt, gs, name)?;
    if !usable(f.details.attrs()) {
        return slow(gs);
    }
    Some(Case {
        key: gs,
        handler: Handler::Global {
            lexical: ls,
            location: int32_of(field_location(gs, usize::from(f.details.field()))),
        },
        stub: None,
    })
}

/// The value a global load case reads, from the lookup that made the case (the miss path then
/// needs no second scan, W4-P1-GLOBAL-ONE-SCAN): a lexical binding or a field of the global
/// object. `None` for a `SLOW` case and for the hole of a TDZ (the generic path throws). Not a
/// GC point.
pub(crate) fn global_case_value(rt: &Runtime, case: &Case) -> Option<u64> {
    let v = match case.handler {
        Handler::Smi(h) => read_field(rt.root(Root::Lexical), int32_of(h))?,
        Handler::Global { location, .. } => read_field(rt.global(), location)?,
        _ => return None,
    };
    (v != HOLE).then_some(v)
}

/// The store form of [`global_case_value`] (M2-P5-PLAN-D8): stores `v` through a global store
/// case, at the location that the lookup of the case found, with the barriered store of the
/// hit path ([`store_global`]): a writable lexical binding outside its TDZ, or a writable own
/// data property of the global object. False for a `SLOW` case and for the hole of a TDZ;
/// nothing is stored then. Not a GC point.
fn global_case_store(rt: &Runtime, case: &Case, v: u64) -> bool {
    let (o, location) = match case.handler {
        Handler::Smi(h) => (rt.root(Root::Lexical), int32_of(h)),
        Handler::Global { location, .. } => (rt.global(), location),
        _ => return false,
    };
    // `SLOW` is not a field handler (`field_ref` gives `None`).
    let Some((c, off)) = field_ref(o, location) else {
        return false;
    };
    if rd(c, off) == HOLE {
        return false;
    }
    object::wr(&rt.heap, c, off, v);
    true
}

/// `StaGlobal` (0x12) after an IC miss in a function that has a feedback vector (M2-P5-PLAN-D8,
/// the store side of W4-P1-GLOBAL-ONE-SCAN; T0's cold arm sends a miss with no vector straight
/// to `slow::store_global`). The miss checks the entry ([`skip_global_miss`]) and computes the
/// case ([`global_case`]) once. When the case is a writable field outside a TDZ, `name = v` is
/// stored through it ([`global_case_store`]) and the case is recorded in slot `slot`: a
/// recording miss scans the lexical record's table and the global object's table once each,
/// not twice. Every other miss runs the generic store with that check and that case
/// (`slow::store_global_with`; P4F-R1: it does not compute them again): an entry that needs no
/// case, or a case that is not a writable field outside a TDZ (`SLOW`: a `const`, an accessor,
/// a read-only or dictionary-mode global; none: an inherited or a missing global; the hole).
/// Returns the stored value, or 0 with a pending exception (as `slow::store_global`). GC point.
#[cold]
#[inline(never)]
pub(crate) fn store_global_miss(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    v: u64,
    slot: u32,
) -> u64 {
    let rt = cx.rt();
    let vector = feedback::vector(FrameRef(fp).info());
    let case = vector
        .filter(|&vec| !skip_global_miss(rt, vec, slot))
        .and_then(|_| global_case(rt, name, true));
    if let (Some(vec), Some(c)) = (vector, case)
        && global_case_store(rt, &c, v)
    {
        // GC point (the stored value is in the caller's accumulator): the interpreter state
        // first, as `slow::store_global` does.
        rt.enter(fp, pc);
        record(rt, vec, slot, c);
        return v;
    }
    slow::store_global_with(cx, fp, pc, name, v, slot, vector, case)
}

#[cfg(test)]
thread_local! {
    /// Tests (P4F-R1): how many times [`skip_global_miss`] ran.
    pub(crate) static SKIP_CHECKS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// True if the miss path of the global slot `i` of `v` need not compute a case: the entry is
/// megamorphic, or its case for the lexical record's or the global object's shape is `SLOW`.
/// Not a GC point.
pub(crate) fn skip_global_miss(rt: &Runtime, v: u64, i: u32) -> bool {
    #[cfg(test)]
    SKIP_CHECKS.with(|c| c.set(c.get() + 1));
    state(v, i) == State::Megamorphic
        || find(v, i, shape::shape_of(rt.root(Root::Lexical))) == Some(SLOW)
        || find(v, i, shape::shape_of(rt.global())) == Some(SLOW)
}

// ---- recording -------------------------------------------------------------------------------

/// A new IC_HANDLER cell: `chain` at +16 (a validity cell, undefined or int32 0). GC point.
fn new_handler(rt: &Runtime, kind: i32, chain: u64, r: u64, location: i32) -> u64 {
    let c = rt.alloc(kind::IC_HANDLER, hc::SIZE);
    init(c, hc::KIND, int32_bits(kind));
    init(c, hc::CELL, chain);
    init(c, hc::REF, r);
    init(c, hc::LOCATION, int32_bits(location));
    c.to_value().to_bits()
}

/// Records `case` in slot `i` of the vector `v` (OM-6): an empty entry becomes monomorphic; a
/// case for a key the entry has replaces that handler; a new key makes a monomorphic entry
/// polymorphic, fills a free case of a polymorphic one, or makes a full one megamorphic.
/// A megamorphic entry stays. GC point (the handler cell and the POLY_IC cell).
pub(crate) fn record(rt: &Runtime, v: u64, i: u32, case: Case) {
    // GC points below: `v` (also a unit root), the case's key and references live in Rust
    // locals (conservative scan). Each allocation comes before the entry is read, so a key
    // that dies in that GC reads undefined. A prototype case gets its validity cell first
    // ([`chain_cell`]); `None` records nothing.
    let key = case.key;
    let handler = match case.handler {
        Handler::Smi(h) => h,
        Handler::ProtoLoad {
            holder,
            location,
            cell,
            name,
        } => {
            let same = |c: Chain| matches!(c, Chain::Data(h, l, _) if h == holder && int32_of(l) == location);
            let Some(c) = chain_cell(rt, key, cell, name, same) else {
                return;
            };
            new_handler(rt, hk::PROTO_LOAD, c, holder, location)
        }
        Handler::Missing { cell, name } => {
            let same = |c: Chain| matches!(c, Chain::Missing);
            let Some(c) = chain_cell(rt, key, cell, name, same) else {
                return;
            };
            new_handler(rt, hk::MISSING, c, UNDEFINED, 0)
        }
        Handler::Add {
            target,
            location,
            chain,
        } => {
            let c = match chain {
                None => UNDEFINED,
                Some(seen) => {
                    // The added key: the target's last one (`store_case_after`).
                    let name = shape::key_at(target, shape::count(key));
                    match chain_cell(rt, key, seen, name, adds_past) {
                        Some(c) => c,
                        None => return,
                    }
                }
            };
            new_handler(rt, hk::ADD, c, target, location)
        }
        Handler::Global { lexical, location } => {
            new_handler(rt, hk::GLOBAL, int32_bits(0), lexical, location)
        }
        Handler::Ready(h) => h,
    };
    let key = feedback::key(v, i);
    if key == UNDEFINED || key == case.key {
        feedback::set_slot(rt, v, i, case.key, handler);
        return;
    }
    if key != HOLE {
        // Monomorphic to polymorphic. GC point: the old case's handler lives in the entry.
        let p = rt.alloc(kind::POLY_IC, pc::SIZE);
        let (old_key, old_handler) = (feedback::key(v, i), feedback::value(v, i));
        if old_key == UNDEFINED {
            // The old shape died in that GC.
            feedback::set_slot(rt, v, i, case.key, handler);
            return;
        }
        init(p, case_key_off(0), old_key);
        init(p, case_key_off(0) + 8, old_handler);
        init(p, case_key_off(1), case.key);
        init(p, case_key_off(1) + 8, handler);
        // The other cases stay undefined (the allocator's pre-fill).
        feedback::set_slot(rt, v, i, HOLE, p.to_value().to_bits());
        return;
    }
    let value = feedback::value(v, i);
    if is_i32(value) {
        // Megamorphic: sticky; the case goes to the stub cache.
        stub_insert(rt, &case, handler);
        return;
    }
    let p = cell(value);
    let at = (0..POLY_CASES)
        .find(|&c| rd(p, case_key_off(c)) == case.key)
        .or_else(|| (0..POLY_CASES).find(|&c| rd(p, case_key_off(c)) == UNDEFINED));
    match at {
        Some(c) => {
            object::wr(&rt.heap, p, case_key_off(c), case.key);
            object::wr(&rt.heap, p, case_key_off(c) + 8, handler);
        }
        None => {
            feedback::set_slot(rt, v, i, HOLE, MEGA);
            stub_insert(rt, &case, handler);
        }
    }
}

/// Puts the case of a megamorphic entry in the stub cache (`stub_cache.rs`): a named-property
/// case keyed on a fast shape whose handler is not `SLOW`. Not a GC point (after the last
/// allocation of [`record`]).
fn stub_insert(rt: &Runtime, case: &Case, handler: u64) {
    let Some((op, name)) = case.stub else {
        return;
    };
    if case.key == KEY_STRING || handler == SLOW {
        return;
    }
    debug_assert!(shape::is_shape(case.key));
    stub_cache::insert(rt, op, case.key as u32, name, handler);
}

// ---- vectors -----------------------------------------------------------------------------------

/// The name a named-property or global site gives its slot. GetIterator's load slot
/// (0x8f, no IC in v0) gets a name no string has, so sharing it makes the slot megamorphic.
fn site_name(fi: &FunctionInfo, op: Opcode, insn: &ep9_bytecode::Instruction<'_>) -> u64 {
    if op == Opcode::GetIterator {
        return NULL;
    }
    let k = (0..insn.operand_count())
        .find(|&j| insn.operand_kind(j) == OperandKind::Const)
        .expect("a named site has a name constant");
    fi.constant(insn.unsigned(k))
}

/// Sets every IC slot of the new vector `v` of `fi` that two sites with different names
/// share to megamorphic, so no case of one name serves the other. BytecodeGen never shares
/// such a slot (each named site has its own; a global slot is per name), but the verifier
/// does not require it. Not a GC point: `v` was just allocated and is filled with
/// initializing stores.
pub(crate) fn init_vector(fi: &FunctionInfo, code: &[u8], v: u64) {
    let n = feedback::slot_count(v) as usize;
    // One name per slot; UNDEFINED until a site names it, HOLE once two names clash. Raw
    // Value bits of unit constants (the unit's roots keep them).
    let mut names: Vec<u64> = Vec::new();
    for insn in Instructions::new(code) {
        let Ok(insn) = insn else {
            debug_assert!(false, "verified code decodes");
            return;
        };
        let op = insn.opcode;
        if !matches!(
            op,
            Opcode::GetNamedProperty
                | Opcode::GetNamedPropertyFromSuper
                | Opcode::SetNamedProperty
                | Opcode::DefineNamedOwnProperty
                | Opcode::LdaGlobal
                | Opcode::LdaGlobalInsideTypeof
                | Opcode::StaGlobal
                | Opcode::GetIterator
        ) {
            continue;
        }
        let Some(j) =
            (0..insn.operand_count()).find(|&j| insn.operand_kind(j) == OperandKind::Slot)
        else {
            continue;
        };
        let slot = insn.unsigned(j) as usize;
        if slot >= n {
            debug_assert!(false, "a slot past the vector");
            continue;
        }
        if names.is_empty() {
            names.resize(n, UNDEFINED);
        }
        let name = site_name(fi, op, &insn);
        let seen = names[slot];
        if seen == UNDEFINED {
            names[slot] = name;
        } else if seen != name {
            names[slot] = HOLE;
        }
    }
    let c = cell(v);
    for (i, &name) in names.iter().enumerate() {
        if name == HOLE {
            // E1: neither word is a cell.
            init(c, feedback::fv::SLOTS + feedback::fv::SLOT_SIZE * i, HOLE);
            init(
                c,
                feedback::fv::SLOTS + feedback::fv::SLOT_SIZE * i + feedback::fv::VALUE,
                MEGA,
            );
        }
    }
}

// ---- tests' view -------------------------------------------------------------------------------

/// The parts of a handler, for tests: (kind, +16, reference, location) of an IC_HANDLER (+16:
/// the validity cell, undefined or an int32), or the smi kind of an int32 handler as (kind,
/// undefined, undefined, the int32).
#[cfg(test)]
pub(crate) fn describe_handler(h: u64) -> (i32, u64, u64, i32) {
    if is_i32(h) {
        let x = int32_of(h);
        return (x & smi::KIND_MASK, UNDEFINED, UNDEFINED, x);
    }
    let c = cell(h);
    (
        rd_i32(c, hc::KIND) | 0x100,
        rd(c, hc::CELL),
        rd(c, hc::REF),
        rd_i32(c, hc::LOCATION),
    )
}

/// IC hit and miss counters (feature `ic-stats`, H27; never in `ep9`, dep-guards check 11). A
/// report build: `cargo build --release -p ep9_cli --features ep9_vm/ic-stats`. T0's fast
/// handlers count each IC site visit as a hit (the fast path answered) or a miss (it went
/// cold); a miss is sorted by the state of its feedback slot, and a megamorphic named miss
/// also counts its stub cache probe (`stub-cache load hit=.. miss=..`). The counters are per thread (the isolate's thread) and
/// print to stderr when that thread ends, one line per site kind, then the 10 sites with the
/// most misses in each of the states uninit and mega (M4-P3 IC2: the raytrace and crypto
/// probes):
///
/// ```text
/// ep9 ic-stats: load hit=N miss=N (no-vector=N uninit=N mono=N poly=N mega=N)
/// ep9 ic-stats: mega-site load <function>@<slot> miss=N
/// ep9 ic-stats: uninit-site load <function>@<slot> miss=N
/// ```
///
/// Without the feature the count macro expands to nothing, so the default build is unchanged
/// (NATIVE-INV).
#[cfg(feature = "ic-stats")]
pub(crate) mod stats {
    use core::cell::{Cell, RefCell};
    use std::collections::HashMap;

    use super::{MEGA, entry};
    use crate::frame::{HOLE, UNDEFINED};
    use crate::unit::FunctionInfo;

    /// The IC site kinds of T0.
    #[derive(Clone, Copy)]
    pub(crate) enum Site {
        /// `GetNamedProperty`, `GetNamedPropertyFromSuper` (0x21, 0x22).
        Load = 0,
        /// `SetNamedProperty`, `DefineNamedOwnProperty` (0x24, 0x25).
        Store = 1,
        /// `LdaGlobal`, `LdaGlobalInsideTypeof` (0x10, 0x11).
        LoadGlobal = 2,
        /// `StaGlobal` (0x12).
        StoreGlobal = 3,
        /// `TestInstanceOf` (`instance_ic.rs`, M4-P3 IC2).
        InstanceOf = 4,
    }

    const SITES: [&str; 5] = ["load", "store", "load-global", "store-global", "instanceof"];
    /// Columns: hit, then the misses by slot state.
    const COLS: [&str; 6] = ["hit", "no-vector", "uninit", "mono", "poly", "mega"];

    pub(crate) struct Counters {
        n: [[Cell<u64>; 6]; 5],
        /// Misses per (state column, site kind, function name, slot), for the columns uninit
        /// and mega.
        sites: RefCell<HashMap<(usize, usize, String, u32), u64>>,
        /// Stub cache probes of megamorphic misses (`mega_load`, `mega_store`): [site][hit?].
        stub: [[Cell<u64>; 2]; 2],
    }

    impl Drop for Counters {
        fn drop(&mut self) {
            for (s, row) in self.n.iter().enumerate() {
                let n: Vec<u64> = row.iter().map(Cell::get).collect();
                if n.iter().all(|&x| x == 0) {
                    continue;
                }
                let miss: u64 = n[1..].iter().sum();
                let cols: Vec<String> = (1..6).map(|i| format!("{}={}", COLS[i], n[i])).collect();
                eprintln!(
                    "ep9 ic-stats: {} hit={} miss={miss} ({})",
                    SITES[s],
                    n[0],
                    cols.join(" ")
                );
            }
            for (s, row) in self.stub.iter().enumerate() {
                let (miss, hit) = (row[0].get(), row[1].get());
                if hit + miss > 0 {
                    eprintln!(
                        "ep9 ic-stats: stub-cache {} hit={hit} miss={miss}",
                        SITES[s]
                    );
                }
            }
            for col in [5, 2] {
                let mut sites: Vec<_> = self
                    .sites
                    .borrow()
                    .iter()
                    .filter(|(k, _)| k.0 == col)
                    .map(|(k, &v)| (v, k.clone()))
                    .collect();
                sites.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                for (count, (_, s, name, slot)) in sites.into_iter().take(10) {
                    eprintln!(
                        "ep9 ic-stats: {}-site {} {name}@{slot} miss={count}",
                        COLS[col], SITES[s]
                    );
                }
            }
        }
    }

    thread_local! {
        static C: Counters = Counters {
            n: [const { [const { Cell::new(0) }; 6] }; 5],
            sites: RefCell::new(HashMap::new()),
            stub: [const { [const { Cell::new(0) }; 2] }; 2],
        };
    }

    fn bump(site: Site, col: usize) {
        C.with(|c| {
            let x = &c.n[site as usize][col];
            x.set(x.get() + 1);
        });
    }

    /// A hit at a `site` (the fast path answered).
    #[inline(never)]
    pub(crate) fn hit(site: Site) {
        bump(site, 0);
    }

    /// A stub cache probe at a megamorphic `site` (`Load` or `Store`); `hit` if it answered.
    #[inline(never)]
    pub(crate) fn stub(site: Site, hit: bool) {
        C.with(|c| {
            let x = &c.stub[site as usize][usize::from(hit)];
            x.set(x.get() + 1);
        });
    }

    /// A miss at slot `slot` of `fi`'s vector, sorted by the slot state.
    #[cold]
    #[inline(never)]
    pub(crate) fn miss(site: Site, fi: &FunctionInfo, slot: u32) {
        let col = match entry(fi, slot) {
            None => 1,
            Some((UNDEFINED, _)) => 2,
            Some((HOLE, v)) if v == MEGA => 5,
            Some((HOLE, _)) => 4,
            Some(_) => 3,
        };
        bump(site, col);
        if col == 2 || col == 5 {
            C.with(|c| {
                *c.sites
                    .borrow_mut()
                    .entry((col, site as usize, fi.name.clone(), slot))
                    .or_insert(0) += 1;
            });
        }
    }
}

/// Counts an IC hit or miss with feature `ic-stats` (the `stats` module); nothing without it.
macro_rules! ic_count {
    (hit, $site:ident) => {
        #[cfg(feature = "ic-stats")]
        $crate::ic::stats::hit($crate::ic::stats::Site::$site);
    };
    (miss, $site:ident, $fi:expr, $slot:expr) => {
        #[cfg(feature = "ic-stats")]
        $crate::ic::stats::miss($crate::ic::stats::Site::$site, $fi, $slot);
    };
}
pub(crate) use ic_count;

/// Counts a stub cache probe of a megamorphic miss with feature `ic-stats`; nothing without it.
macro_rules! stub_stats {
    ($site:ident, $hit:expr) => {
        #[cfg(feature = "ic-stats")]
        $crate::ic::stats::stub($crate::ic::stats::Site::$site, $hit);
    };
}
use stub_stats;
