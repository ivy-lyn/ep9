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
//! | IC_HANDLER | 70 | IC_LAYOUT 4 (below): +8 int32 handler kind, +16 the validity cell (strong) or an int32, +24 a weak ref, +32 an int32 | 40 |
//!
//! **IC_LAYOUT 2** (`versions::IC_LAYOUT_VERSION`, M4-P0; plan v9 §2.3, V9-A8), **3** since M5-P0
//! (the `ALLOC_SITE` handler kind and the keyed kinds-set bits, `profile.rs`), **4** since M6-P0
//! (handler kinds 9-11 and the 16-bit keyed kinds set, `elements::KINDS_SET_SHIFT`). The
//! feedback slot table (which module writes which slot kind) is in the `profile.rs` module docs.
//! The IC_HANDLER cell:
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
//! | `GETTER`, `SETTER` (IC_LAYOUT 4) | own accessor: int32 0; inherited: the chain's validity cell | own: undefined; inherited: the holder | the ACCESSOR_PAIR location (a smi field handler) | `ic.rs` (M6-P1 ACCIC; the cold hits, [`cold_load`], [`cold_setter`]) |
//! | `PROTO_DICT_LOAD` (IC_LAYOUT 4) | the receiver chain's validity cell | the dictionary-mode holder | int32 0 | `ic.rs` (M6-P1 ACCIC; the cold hit, [`cold_load`]) |
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
//! | `SLOW` (smi) | any: a dictionary-mode receiver, an accessor of a dictionary-mode holder, a define over an accessor, exotic or read-only cases | none: the generic path runs, the case only keeps its place |
//! | `PROTO_LOAD` (cell) | 0x21, 0x22 | validity cell valid and holder alive: the holder's field |
//! | `MISSING` (cell) | 0x21, 0x22 | validity cell valid: undefined |
//! | `PROTO_LOAD`, `MISSING` (cell) under [`KEY_STRING`] (M4-P3 IC2) | 0x21 on a string: a method (`s.charCodeAt`), from %String.prototype% | as above, after a kind-byte check (a symbol has ShapeId 0 too) |
//! | `ADD` (cell) | 0x24 (validity cell checked), 0x25 | target alive and room in the slots: store, then install the target shape (no room: a miss; the miss path grows the SLOTS cell, then stores: [`grow_add`], M8-P0 TS-STORE) |
//! | smi field, keyed on the lexical record's shape | 0x10, 0x11; 0x12 if writable | a `let`/`const`/`class` binding; the hole (TDZ) misses |
//! | `GLOBAL` (cell), keyed on the global object's shape | 0x10, 0x11; 0x12 if writable | the lexical record still has the shape in the handler: the global object's field; the hole (a lazy namespace key not installed, INTL-I1-D8) misses (M7-P2) |
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
//! - The megamorphic stub cache (M4-P3 IC2, `stub_cache.rs`, H16-Q3): a megamorphic named site
//!   misses in the hit path ([`load`], [`store`] are unchanged, so a monomorphic hit costs no
//!   more); its miss path first probes a per-isolate table by (operation, receiver ShapeId,
//!   name) and runs the handler it finds with the code above ([`mega_load`], [`mega_store`];
//!   0x21, 0x24, 0x25 in the cold body, [`get_named_cold`], [`handler_mega_store`]); on a table
//!   miss it computes the case for an object receiver ([`skip_miss`], if [`MEGA_FILL`]) and
//!   [`record`] puts it in the table (not `SLOW`, not a string). It is emptied at every GC.
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
use crate::object::{self, arr, attr, cell, init, obj, rd, rd_i32, realmf};
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
    /// An array's `length`: the Value at +24 (an int32 below 2^31, a double from 2^31 on).
    pub const ARRAY_LENGTH: i32 = 3;
    /// A string's `length` (the receiver must be a string).
    pub const STRING_LENGTH: i32 = 4;
    /// The case is known; the generic path runs (accessor, dictionary, exotic, read-only).
    pub const SLOW: i32 = 5;
    /// The value of a megamorphic entry.
    pub const MEGA: i32 = 6;
    /// M5-P2 CONSTREP (an append to the handler kinds, H16 "Append-only"): a field store case
    /// (0x24, 0x25) whose field has a representation that is not ANY (`field_rep.rs`). The
    /// hit stores only a value whose tag fits it, else it misses (the miss path widens the
    /// field). Odd kinds are in-object, even ones out of line, as [`FIELD_IN_OBJECT`] and
    /// [`FIELD_OUT_OF_LINE`]. Only store sites hold them; [`plain`] gives the field handler
    /// that readers outside the hit path see (`ic::find`).
    pub const FIELD_INT32_IN_OBJECT: i32 = 9;
    pub const FIELD_INT32_OUT_OF_LINE: i32 = 10;
    /// A number (an int32 or a double).
    pub const FIELD_NUMBER_IN_OBJECT: i32 = 11;
    pub const FIELD_NUMBER_OUT_OF_LINE: i32 = 12;
    /// No number (a cell or another immediate).
    pub const FIELD_HEAP_IN_OBJECT: i32 = 13;
    pub const FIELD_HEAP_OUT_OF_LINE: i32 = 14;

    // Each checked kind: in-object odd, out of line the next even one; `rep_of_kind` gives
    // its representation.
    const _: () = assert!(
        FIELD_INT32_OUT_OF_LINE == FIELD_INT32_IN_OBJECT + 1
            && FIELD_NUMBER_OUT_OF_LINE == FIELD_NUMBER_IN_OBJECT + 1
            && FIELD_HEAP_OUT_OF_LINE == FIELD_HEAP_IN_OBJECT + 1
            && FIELD_INT32_IN_OBJECT & 1 == FIELD_IN_OBJECT & 1
            && rep_of_kind(FIELD_INT32_OUT_OF_LINE) == crate::shape::rep::INT32
            && rep_of_kind(FIELD_NUMBER_IN_OBJECT) == crate::shape::rep::DOUBLE
            && rep_of_kind(FIELD_HEAP_OUT_OF_LINE) == crate::shape::rep::HEAP
            && FIELD_HEAP_OUT_OF_LINE <= KIND_MASK
    );

    /// True for a representation-checked field store kind (9-14).
    #[inline(always)]
    pub(crate) const fn is_checked(k: i32) -> bool {
        k >= FIELD_INT32_IN_OBJECT && k <= FIELD_HEAP_OUT_OF_LINE
    }

    /// The representation (`shape::rep`) a checked kind `k` names.
    #[inline(always)]
    pub(crate) const fn rep_of_kind(k: i32) -> i32 {
        (k - 7) >> 1
    }

    /// The smi field handler `x` with its representation check dropped: a checked kind
    /// becomes [`FIELD_IN_OBJECT`] or [`FIELD_OUT_OF_LINE`] at the same offset; any other
    /// handler stays.
    #[inline]
    pub(crate) const fn plain(x: i32) -> i32 {
        let k = x & KIND_MASK;
        if is_checked(k) {
            (x & !KIND_MASK)
                | if k & 1 == 1 {
                    FIELD_IN_OBJECT
                } else {
                    FIELD_OUT_OF_LINE
                }
        } else {
            x
        }
    }

    /// The checked form of the field handler `x` ([`FIELD_IN_OBJECT`] or
    /// [`FIELD_OUT_OF_LINE`]) for a field of representation `r`: `x` itself for ANY, `None`
    /// for NONE (no value stored yet).
    pub(crate) fn checked(x: i32, r: i32) -> Option<i32> {
        use crate::shape::rep;
        let k = x & KIND_MASK;
        debug_assert!(k == FIELD_IN_OBJECT || k == FIELD_OUT_OF_LINE);
        let base = match r {
            rep::ANY => return Some(x),
            rep::INT32 => FIELD_INT32_IN_OBJECT,
            rep::DOUBLE => FIELD_NUMBER_IN_OBJECT,
            rep::HEAP => FIELD_HEAP_IN_OBJECT,
            _ => return None,
        };
        Some((x & !KIND_MASK) | (base + k - FIELD_IN_OBJECT))
    }
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
    /// `object::arr::ALLOC_SITE`) hold it; one-way generalization. The literal-site code
    /// writes it (`literal_site.rs`; an elements transition generalizes it through ARRAY +32),
    /// never a record hook, so `ics_only` keeps it. Built in M5-P2 (ELEM2).
    pub const ALLOC_SITE: i32 = 8;
    /// IC_LAYOUT 4 (M6-P0, plan v11 §2.3; H17 "Accessor ICs"): a getter case (M6-P1 ACCIC).
    /// Own accessor: +16 int32 0, +24 undefined; inherited: +16 the chain's validity cell,
    /// +24 the holder (weak); both: +32 the field location (a smi field handler) of the
    /// ACCESSOR_PAIR. The fast handler misses; the cold body calls the getter ([`super::cold_load`]).
    /// No stub cache entry (v1).
    pub const GETTER: i32 = 9;
    /// IC_LAYOUT 4: a setter case of a 0x24 store, the fields of [`GETTER`] (M6-P1 ACCIC);
    /// the cold body calls the setter ([`super::cold_setter`]).
    /// A 0x25 define over an accessor stays `SLOW` (it redefines the property).
    pub const SETTER: i32 = 10;
    /// IC_LAYOUT 4 (H17 "Dictionary v1"): a data property of a dictionary-mode holder on the
    /// prototype chain: +16 the receiver chain's validity cell, +24 the holder (weak), +32 int32
    /// 0; a hit looks the key up in the holder's PROPS or PROPS_INDEXED table (M6-P1 ACCIC).
    /// The fast handler misses; the cold body reads the entry ([`super::cold_load`]). A receiver in
    /// dictionary mode stays `SLOW` (H16 K12).
    pub const PROTO_DICT_LOAD: i32 = 11;
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
///
/// M7-P4 TSPASS2: the hit paths that T0's fast handlers inline ([`load`], [`store`]) test
/// each word they turn into a cell (`object::cell`) with `is_cell_bits` first and miss if it
/// is not one. The VM never holds such a word there (a vector is a FEEDBACK_VECTOR cell, an
/// out-of-line field's SLOTS word a SLOTS cell, a handler that is not an int32 an IC_HANDLER
/// cell), so no hit changes; but then `object::cell` has no panic path left in the handler,
/// so the `GetNamedProperty` handler and the `Wide` handler make no call and keep no frame
/// record (aarch64: no `stp x29, x30` on entry and no `ldp` at each exit). A miss runs the
/// generic path, which panics there as before if the word is not a cell.
#[inline(always)]
fn entry(fi: &FunctionInfo, i: u32) -> Option<(u64, u64)> {
    let v = feedback::vector(fi)?;
    if !is_cell_bits(v) {
        return None;
    }
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
        smi::FIELD_OUT_OF_LINE => {
            // The test of [`entry`]: no panic path in the inlined hit path.
            let s = rd(cell(o), obj::SLOTS);
            is_cell_bits(s).then(|| (cell(s), off))
        }
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
        (k, chain) => cold_case_matches(k, chain, c),
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
/// A megamorphic entry misses here: its miss path probes the stub cache ([`mega_load`]; 0x21
/// in its cold body, [`get_named_cold`]). A probe here, even after the compares (M7-P1), costs
/// each monomorphic hit two loads (the site's name operand and the runtime pointer, loaded
/// before any branch; +1.4% instructions on t0-prop, mac a64), so the hit path stays as it was.
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
    load_hit(rt, o, id, move || fi.constant(name), h)
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
    // The tests of [`entry`]: no panic path in the inlined hit path.
    if !is_cell_bits(h) {
        return None;
    }
    let c = cell(h);
    let k = rd_i32(c, hc::KIND);
    if k != hk::PROTO_LOAD && k != hk::MISSING {
        return None;
    }
    let vc = rd(c, hc::CELL);
    if !is_cell_bits(vc) || !validity::valid_cell(vc) {
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
    // A dead holder reads undefined, which is not a cell.
    let holder = rd(c, hc::REF);
    if !is_cell_bits(holder) {
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
        // M5-P2 CONSTREP: a checked kind stores only a value whose tag fits the field's
        // representation; the miss path widens it. Odd kinds are in-object.
        // M6-P2 T0OBJ: one table read for every kind, with no branch per kind
        // (`field_rep::handler_fits`): a plain field kind holds every Value, a checked kind
        // its representation's range, any other kind (SLOW, a length case) none: a miss.
        // The add hit below reads the same ranges (`field_rep::table_fits`).
        if !crate::field_rep::handler_fits(x, v) {
            return false;
        }
        let k = x & smi::KIND_MASK;
        // (Before, a `match` on `k`: 6 aarch64 instructions for kinds 1-2, 9-12 for 9-14.)
        let in_object = k & 1 != 0;
        if in_object {
            object::wr(&rt.heap, c, off, v);
        } else {
            object::wr(&rt.heap, cell(rd(c, obj::SLOTS)), off, v);
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
    // M5-P2 CONSTREP: the new field's representation must hold the value (the first store
    // keeps the const bit); the miss path widens it.
    if !crate::field_rep::table_fits(shape::last_field_rep(target), v) {
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
/// if slot `slot` of `v` is megamorphic and there is one; an add with no room grows the SLOTS
/// cell first ([`grow_add`]). True if it stored; false: the generic path. No JS. GC point.
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
        Some(h) if define => store_or_grow::<false>(rt, o, key, val, h),
        Some(h) => store_or_grow::<true>(rt, o, key, val, h),
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
/// no ShapeId equals. M7-P2: a value that is not a cell misses too (no panic, no call).
#[inline(always)]
fn global_handler(h: u64, lid: u32) -> Option<CellPtr> {
    if is_i32(h) {
        return None;
    }
    let c = as_cell(h)?;
    (rd_i32(c, hc::KIND) == hk::GLOBAL && rd(c, hc::REF) as u32 == lid).then_some(c)
}

/// The object and smi field handler that a global site's entry (`key`, `value`) names, or `None`
/// (a miss): the mono global case first (a `var`, a function, a builtin), then a lexical binding,
/// then a poly entry. As lexical first: a `GLOBAL` case's record shape lacks the name. Each
/// header is read once (a ShapeId load is not merged with another).
#[inline(always)]
fn global_site(key: u64, value: u64, g: CellPtr, lex: CellPtr) -> Option<(CellPtr, i32)> {
    let (gid, lid) = (id_of(g), id_of(lex));
    if key as u32 == gid {
        return Some((g, rd_i32(global_handler(value, lid)?, hc::LOCATION)));
    }
    if key as u32 == lid {
        return is_i32(value).then(|| (lex, int32_of(value)));
    }
    let p = as_cell(value).filter(|_| key == HOLE)?;
    for i in 0..POLY_CASES {
        let (k, h) = (rd(p, case_key_off(i)) as u32, rd(p, case_key_off(i) + 8));
        if k == lid {
            return is_i32(h).then(|| (lex, int32_of(h)));
        }
        if let Some(c) = (k == gid).then(|| global_handler(h, lid)).flatten() {
            return Some((g, rd_i32(c, hc::LOCATION)));
        }
    }
    None
}

/// The field (cell, offset) of a global site's case ([`global_site`]) under `Cx::global_this` and
/// `Cx::realm`'s `GLOBAL_LEXICAL` (= `Root::Lexical`, `realm::switch` writes both; 1 load, not
/// 5). `None` is a miss, also where `object::cell` would panic: no call (M7-P2 GLOBALS).
#[inline(always)]
fn global_field(cx: &Cx, fi: &FunctionInfo, slot: u32) -> Option<(CellPtr, usize)> {
    let (v, i) = (as_cell(feedback::vector(fi)?)?, slot as usize);
    debug_assert!(slot < feedback::slot_count(v.to_value().to_bits()));
    let at = |w| rd(v, feedback::fv::SLOTS + feedback::fv::SLOT_SIZE * i + w);
    let (key, value) = (at(feedback::fv::KEY), at(feedback::fv::VALUE));
    let lex = rd(as_cell(cx.realm)?, realmf::GLOBAL_LEXICAL);
    debug_assert!(lex == cx.rt().root(Root::Lexical) && cx.global_this == cx.rt().global());
    let (o, loc) = global_site(key, value, as_cell(cx.global_this)?, as_cell(lex)?)?;
    let off = (loc >> smi::SHIFT) as usize;
    match loc & smi::KIND_MASK {
        smi::FIELD_IN_OBJECT => Some((o, off)),
        smi::FIELD_OUT_OF_LINE => Some((as_cell(rd(o, obj::SLOTS))?, off)),
        _ => None,
    }
}

/// The hit path of a global load (0x10 `LdaGlobal`, 0x11 `LdaGlobalInsideTypeof`). The hole is
/// never a hit: the TDZ, or a lazy namespace key that is not installed (INTL-I1-D8).
#[inline(always)]
pub(crate) fn load_global(cx: &Cx, fi: &FunctionInfo, slot: u32) -> Option<u64> {
    let (c, off) = global_field(cx, fi, slot)?;
    Some(rd(c, off)).filter(|&v| v != HOLE)
}

/// The hit path of a global store (0x12 `StaGlobal`): as [`load_global`], and the field does not
/// hold the hole (the generic store gives the same result). False on a miss: nothing stored.
#[inline(always)]
pub(crate) fn store_global(cx: &Cx, fi: &FunctionInfo, slot: u32, v: u64) -> bool {
    let Some((c, off)) = global_field(cx, fi, slot).filter(|&(c, off)| rd(c, off) != HOLE) else {
        return false;
    };
    object::wr(&cx.rt().heap, c, off, v);
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

/// The handler slot `i` of the vector `v` keeps for the case key `k`, if any. A
/// representation-checked field store handler is given as its plain field handler
/// ([`smi::plain`], M5-P2 CONSTREP): readers outside the hit paths (the facts exporter,
/// tests) see the field case; [`find_raw`] gives the stored word.
pub(crate) fn find(v: u64, i: u32, k: u64) -> Option<u64> {
    find_raw(v, i, k).map(|h| {
        if is_i32(h) {
            int32_bits(smi::plain(int32_of(h)))
        } else {
            h
        }
    })
}

/// [`find`] with the stored handler word as it is.
pub(crate) fn find_raw(v: u64, i: u32, k: u64) -> Option<u64> {
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
    /// M6-P1 ACCIC: a getter or setter case (`kind` [`hk::GETTER`] or [`hk::SETTER`]): the
    /// location of the ACCESSOR_PAIR, in the receiver (`chain` `None`, `holder` undefined:
    /// an own accessor) or in `holder` (`chain`: the validity cell seen, as for `ProtoLoad`),
    /// and the name.
    Accessor {
        kind: i32,
        holder: u64,
        location: i32,
        chain: Option<u64>,
        name: u64,
    },
    /// M6-P1 ACCIC: a data property of the dictionary-mode `holder` on the chain
    /// ([`hk::PROTO_DICT_LOAD`]), with the validity cell seen and the name.
    ProtoDict { holder: u64, cell: u64, name: u64 },
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
#[derive(Clone, Copy)]
enum Chain {
    /// A data property of the fast-mode holder: (holder, location, writable).
    Data(u64, u64, bool),
    /// An accessor of the fast-mode holder: (holder, the location of its ACCESSOR_PAIR)
    /// (M6-P1 ACCIC: a `GETTER` or `SETTER` case).
    Accessor(u64, u64),
    /// A data property of the dictionary-mode holder (M6-P1 ACCIC: a `PROTO_DICT_LOAD` case).
    DictData(u64),
    /// No object has it.
    Missing,
    /// An exotic property, or an accessor of a dictionary-mode object that has it.
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
            if let Some((_, a)) = object::own_get(p, name) {
                return if a & attr::ACCESSOR == 0 {
                    Chain::DictData(p)
                } else {
                    Chain::Other
                };
            }
        } else if let Some(f) = shape::lookup(rt, s, name) {
            let a = f.details.attrs();
            let loc = field_location(s, usize::from(f.details.field()));
            if a & attr::ACCESSOR != 0 {
                return Chain::Accessor(p, loc);
            }
            return Chain::Data(p, loc, a & attr::WRITABLE != 0);
        }
        p = shape::proto(s);
    }
    Chain::Missing
}

/// The load handler of a case whose receiver lacks `name` and whose chain walk gave `chain`
/// under the validity cell `cell` (seen, [`chain_cell_seen`]). Not a GC point.
fn chain_load(chain: Chain, cell: u64, name: u64) -> Handler {
    match chain {
        Chain::Data(holder, loc, _) => Handler::ProtoLoad {
            holder,
            location: int32_of(loc),
            cell,
            name,
        },
        Chain::Accessor(holder, loc) => Handler::Accessor {
            kind: hk::GETTER,
            holder,
            location: int32_of(loc),
            chain: Some(cell),
            name,
        },
        Chain::DictData(holder) => Handler::ProtoDict { holder, cell, name },
        Chain::Missing => Handler::Missing { cell, name },
        Chain::Other => Handler::Smi(SLOW),
    }
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
        // M6-P1 ACCIC: an inherited getter or a dictionary-mode holder too (the cold hits).
        let p = rt.root(Root::StringPrototype);
        let cell = chain_cell_seen(rt, p);
        return Some(Case {
            key: KEY_STRING,
            handler: chain_load(walk(rt, p, name), cell, name),
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
        // M6-P1 ACCIC: an own getter case; its pair is read from the receiver at each hit.
        Some(f) if f.details.attrs() & attr::ACCESSOR != 0 => Some(Case {
            key: s,
            handler: Handler::Accessor {
                kind: hk::GETTER,
                holder: UNDEFINED,
                location: int32_of(field_location(s, usize::from(f.details.field()))),
                chain: None,
                name,
            },
            stub,
        }),
        Some(f) => smi_case(field_location(s, usize::from(f.details.field()))),
        // An exotic property that is not in the storage (a lazy `prototype`, a String
        // wrapper's `length`).
        None if exotic_name(rt, k, name) => smi_case(SLOW),
        None => {
            let p = shape::proto(s);
            let cell = chain_cell_seen(rt, p);
            Some(Case {
                key: s,
                handler: chain_load(walk(rt, p, name), cell, name),
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
        if !define && a & attr::ACCESSOR != 0 {
            // M6-P1 ACCIC: an own setter case; its pair is read from the receiver at each hit.
            return Some(StorePre::Done(Case {
                key: s,
                handler: Handler::Accessor {
                    kind: hk::SETTER,
                    holder: UNDEFINED,
                    location: int32_of(field_location(s, usize::from(f.details.field()))),
                    chain: None,
                    name,
                },
                stub,
            }));
        }
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
        let seen = chain_cell_seen(rt, p);
        chain = Some(seen);
        match walk(rt, p, name) {
            // M6-P1 ACCIC: an inherited setter case (the holder, under the chain's cell).
            Chain::Accessor(holder, loc) => {
                return Some(StorePre::Done(Case {
                    key: s,
                    handler: Handler::Accessor {
                        kind: hk::SETTER,
                        holder,
                        location: int32_of(loc),
                        chain: Some(seen),
                        name,
                    },
                    stub,
                }));
            }
            w if !adds_past(w) => return slow,
            _ => {}
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
pub(crate) fn store_case_after(rt: &Runtime, pre: StorePre, o: u64, name: u64) -> Option<Case> {
    let (from, chain) = match pre {
        StorePre::Done(c) => return checked_case(rt, c, name),
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

/// A decided store case after its store ran (M5-P2 CONSTREP): a field case gets the
/// representation-checked kind of the field as the store left it ([`smi::checked`]). A field
/// whose const bit is still set (no second store yet) or that holds no representation gets
/// no case: the hit would store without the second-store hook. Other cases stay. Not a GC
/// point.
fn checked_case(rt: &Runtime, c: Case, name: u64) -> Option<Case> {
    let Handler::Smi(h) = c.handler else {
        return Some(c);
    };
    let x = int32_of(h);
    if !matches!(
        x & smi::KIND_MASK,
        smi::FIELD_IN_OBJECT | smi::FIELD_OUT_OF_LINE
    ) {
        return Some(c);
    }
    let f = shape::lookup(rt, c.key, name)?;
    let d = f.details;
    if d.is_const() {
        return None;
    }
    let x = smi::checked(x, d.rep())?;
    Some(Case {
        handler: Handler::Smi(int32_bits(x)),
        ..c
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
        if store {
            // M5-P2 CONSTREP: a global store hit does not check the value (the slot is
            // shared with the loads of the name), so the field holds any value from now on.
            crate::field_rep::widen(rt, ls, f.index);
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
    if store {
        // As for a lexical binding above (M5-P2 CONSTREP).
        crate::field_rep::widen(rt, gs, f.index);
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
    if cold_case(&case.handler) && state(v, i) == State::Megamorphic {
        // [`stub_insert`] drops this case: make no cell (and no validity cell) for it.
        return;
    }
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
        Handler::Accessor {
            kind,
            location,
            chain: None,
            ..
        } => new_handler(rt, kind, int32_bits(0), UNDEFINED, location),
        Handler::Accessor {
            kind,
            holder,
            location,
            chain: Some(seen),
            name,
        } => {
            let same = |c: Chain| matches!(c, Chain::Accessor(h, l) if h == holder && int32_of(l) == location);
            let Some(c) = chain_cell(rt, key, seen, name, same) else {
                return;
            };
            new_handler(rt, kind, c, holder, location)
        }
        Handler::ProtoDict { holder, cell, name } => {
            let same = |c: Chain| matches!(c, Chain::DictData(h) if h == holder);
            let Some(c) = chain_cell(rt, key, cell, name, same) else {
                return;
            };
            new_handler(rt, hk::PROTO_DICT_LOAD, c, holder, 0)
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
/// case keyed on a fast shape whose handler is not `SLOW`. Accessor and dictionary-holder
/// cases (M6-P1 ACCIC) stay out in v1: their hits are cold-body hits, which a stub cache
/// probe does not run. Not a GC point (after the last allocation of [`record`]).
fn stub_insert(rt: &Runtime, case: &Case, handler: u64) {
    let Some((op, name)) = case.stub else {
        return;
    };
    if case.key == KEY_STRING || handler == SLOW || cold_case(&case.handler) {
        return;
    }
    debug_assert!(shape::is_shape(case.key));
    stub_cache::insert(rt, op, case.key as u32, name, handler);
}

/// True for an accessor or a dictionary-holder case (M6-P1 ACCIC): a cold-body hit, which
/// the stub cache does not keep ([`stub_insert`]). So [`record`] makes no handler cell for it
/// at a megamorphic entry.
fn cold_case(h: &Handler) -> bool {
    matches!(h, Handler::Accessor { .. } | Handler::ProtoDict { .. })
}

// ---- accessor and dictionary-holder cases (M6-P1 ACCIC): the cold hits ------------------------
//
// H17 "Accessor ICs" and the H17 Dictionary v1 holder handler (plan v11 V11-D17; H16 K12):
//
//   case (miss path)                               handler (IC_HANDLER)              cold hit
//   own accessor of the receiver's fast shape       GETTER / SETTER, +16 int32 0      pair = receiver's field
//   accessor of a fast-mode holder on the chain     GETTER / SETTER, +16 cell, +24    pair = holder's field
//                                                   holder (weak)                     (cell valid, holder alive)
//   data property of a dictionary-mode holder       PROTO_DICT_LOAD, +16 cell, +24    the holder's own entry
//                                                   holder (weak)                     (cell valid, holder alive)
//
// - The fast handlers (`load`, `store`) miss on these kinds: a hit path never calls JS and never
//   allocates (`profile.rs` module docs). The cold bodies (`slow::get_named`, `get_super`,
//   `set_named`) try the slot's case first ([`cold_load`], [`cold_setter`]): no lookup on the
//   receiver and no chain walk, then the getter or setter call (`accessor::call_getter`,
//   `call_setter`) with the receiver as `this`. Anything else goes on to the generic path.
// - An own case never caches a pair or a holder: two objects of one shape hold their own pairs
//   (H17 Accessor ICs row). A pair is immutable; a redefine stores a new pair in the field
//   (and, on a prototype, bumps the epoch), so the next hit reads and calls the new pair.
// - Data <-> accessor and an attribute change are dictionary moves (OM-10), so a case keyed on
//   a fast shape keeps its kind of property. A holder that turns dictionary, a property added
//   or deleted on the chain, or a new prototype pops the chain's validity cell (`shape::note_change`).
// - A load site and a store site have their own slots (BytecodeGen; [`init_vector`]), so a warm
//   GETTER case never answers a store: a store to an accessor calls the setter every time
//   (MODES-CHECK-w24 MC-w24-8; the V8 bug class of 64b57c27d1). The hit reads no cached value.
// - A setter case with no setter in its pair (a getter-only accessor) runs the generic store,
//   which fails (a TypeError in strict code). A dictionary entry that is an accessor misses.
// - No stub cache entry ([`stub_insert`]) and no facts description yet (`facts_export.rs`).

/// What the cold body of a named load gets from a cached case ([`cold_load`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColdLoad {
    /// The value: a data property of the dictionary-mode holder (`PROTO_DICT_LOAD`).
    Value(u64),
    /// The ACCESSOR_PAIR whose getter the cold body calls (`GETTER`).
    Getter(u64),
}

/// The IC_HANDLER cell and its kind that slot `i` of the vector `v` keeps for the case key
/// of `o` (its shape, or [`KEY_STRING`] for a string), if the case has a cell. Not a GC point.
#[inline]
fn cold_handler(v: u64, i: u32, o: u64) -> Option<(CellPtr, i32)> {
    if !is_cell_bits(o) {
        return None;
    }
    let id = shape_id(o);
    // ShapeId 0 is a string's or a symbol's: only a string has cases under KEY_STRING.
    if id == 0 && !kind::is_string(o) {
        return None;
    }
    let h = handler_for(feedback::key(v, i), feedback::value(v, i), id)?;
    if is_i32(h) {
        return None;
    }
    let c = cell(h);
    Some((c, rd_i32(c, hc::KIND)))
}

/// The ACCESSOR_PAIR that the GETTER or SETTER handler `c` gives for `o` (the receiver, or the
/// lookup start of 0x22): the field of `o` itself (an own case, +16 int32 0), or the field of
/// the cached holder while the chain's validity cell is valid and the holder is alive. `name`
/// (the site's name) feeds only the debug cross-check. Not a GC point.
fn cached_pair(rt: &Runtime, o: u64, c: CellPtr, name: u64) -> Option<u64> {
    let loc = rd_i32(c, hc::LOCATION);
    let chain = rd(c, hc::CELL);
    let pair = if chain == int32_bits(0) {
        debug_assert!(kind::is_object(o), "an own accessor case of a non-object");
        read_field(o, loc)?
    } else {
        if !validity::valid_cell(chain) {
            return None;
        }
        let holder = rd(c, hc::REF);
        if holder == UNDEFINED {
            return None;
        }
        #[cfg(debug_assertions)]
        cross_check(rt, o, name, c);
        read_field(holder, loc)?
    };
    debug_assert!(
        crate::accessor::is_pair(pair),
        "an accessor case whose field holds no ACCESSOR_PAIR"
    );
    #[cfg(not(debug_assertions))]
    let _ = (rt, name);
    Some(pair)
}

/// The cold hit of a named load (0x21 with the receiver `o`, 0x22 with the lookup start `o`)
/// before the generic load: the `GETTER` or `PROTO_DICT_LOAD` case of slot `i` of the vector
/// `v` for `o`, if it applies. `name` is the site's name (an atom). `None`: take the generic
/// path. No allocation, no JS, not a GC point.
pub(crate) fn cold_load(rt: &Runtime, v: u64, i: u32, o: u64, name: u64) -> Option<ColdLoad> {
    let (c, k) = cold_handler(v, i, o)?;
    let r = match k {
        hk::GETTER => cached_pair(rt, o, c, name).map(ColdLoad::Getter),
        hk::PROTO_DICT_LOAD => {
            if !validity::valid_cell(rd(c, hc::CELL)) {
                return None;
            }
            let holder = rd(c, hc::REF);
            if holder == UNDEFINED {
                return None;
            }
            #[cfg(debug_assertions)]
            cross_check(rt, o, name, c);
            debug_assert!(shape::is_dictionary(shape::shape_of(holder)));
            let (x, a) = crate::props::own_get_with(holder, name, key::probe(name))?;
            (a & attr::ACCESSOR == 0).then_some(ColdLoad::Value(x))
        }
        _ => None,
    };
    cold_hit(r)
}

/// The cold hit of a named store `o.name = v` (0x24 only) before the generic store: the pair
/// of the `SETTER` case of slot `i` of the vector `v` for the object `o`, if it applies (the
/// caller calls its setter, or takes the generic path for a pair with no setter). `None`:
/// take the generic path. No allocation, no JS, not a GC point.
pub(crate) fn cold_setter(rt: &Runtime, v: u64, i: u32, o: u64, name: u64) -> Option<u64> {
    if !kind::is_object(o) {
        return None;
    }
    cold_hit(match cold_handler(v, i, o)? {
        (c, hk::SETTER) => cached_pair(rt, o, c, name),
        _ => None,
    })
}

#[cfg(test)]
thread_local! {
    /// Tests (M6-P1 ACCIC): how many times [`cold_load`] or [`cold_setter`] answered.
    pub(crate) static COLD_HITS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// `r`, counted in `COLD_HITS` (tests) if it is a hit. Nothing else.
#[inline(always)]
fn cold_hit<T>(r: Option<T>) -> Option<T> {
    #[cfg(test)]
    if r.is_some() {
        COLD_HITS.with(|c| c.set(c.get() + 1));
    }
    r
}

/// The debug cross-check ([`cross_check`]) of the cold-hit kinds: a `GETTER` or `SETTER` case
/// needs an accessor of its holder at its location, a `PROTO_DICT_LOAD` case a data property
/// of its dictionary-mode holder. False for any other pair of kind and answer.
#[cfg(debug_assertions)]
fn cold_case_matches(k: i32, chain: Chain, c: CellPtr) -> bool {
    match (k, chain) {
        (hk::GETTER | hk::SETTER, Chain::Accessor(holder, loc)) => {
            holder == rd(c, hc::REF) && int32_of(loc) == rd_i32(c, hc::LOCATION)
        }
        (hk::PROTO_DICT_LOAD, Chain::DictData(holder)) => holder == rd(c, hc::REF),
        _ => false,
    }
}

// ---- the stub cache probe of the GetNamedProperty handler (M7-P1 NAMEDLOAD) ------------------

/// The cold body of `GetNamedProperty` (0x21; M7-P1 NAMEDLOAD): `o.key` after the hit path
/// ([`load`]) missed, with the site's name `key` (an atom) and slot `slot`. First the stub
/// cache probe ([`handler_mega_load`]), then the runtime call (`slow::get_named`) if it gives
/// nothing. The value, or 0 if an exception is pending (the runtime call's result). GC point
/// (the runtime call); the probe is not one.
#[inline(always)]
pub(crate) fn get_named_cold(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    o: u64,
    key: u64,
    slot: u32,
) -> u64 {
    match handler_mega_load(cx.rt(), FrameRef(fp).info(), o, slot, key) {
        Some(v) => v,
        None => slow::get_named(cx, fp, pc, o, key, slot),
    }
}

/// The stub cache probe of the `GetNamedProperty` handler: in its cold body
/// ([`get_named_cold`]), after the monomorphic and polymorphic compares of [`load`] missed, and
/// before the runtime call. V8's load handler probes its stub cache there too. If slot `slot`
/// of the vector of `fi` is megamorphic and the stub cache has an entry for `o`'s shape and
/// the name `key` (an atom), the value its handler gives ([`load_with`]). `None`: the runtime
/// call runs as before (it probes again, [`mega_load`], then takes the generic path, which
/// fills the entry, [`record`]).
///
/// The hit path ([`load`]) does not change: the cold body is a handler of its own (H15), so a
/// monomorphic hit loads neither the name operand nor the runtime pointer (the +1.4% on
/// t0-prop that the note of [`load`] names). A hit here skips the runtime call: the frame
/// record (`Runtime::enter`), the call, and its vector and slot reads. No allocation, no JS,
/// not a GC point.
#[inline(always)]
fn handler_mega_load(rt: &Runtime, fi: &FunctionInfo, o: u64, slot: u32, key: u64) -> Option<u64> {
    if !kind::is_object(o) {
        return None;
    }
    let v = feedback::vector(fi)?;
    if !is_mega(v, slot) {
        return None;
    }
    let id = shape_id(o);
    let r = stub_cache::probe(rt, stub_cache::Op::Load, id, key)
        .and_then(|h| load_with(rt, o, id, || key, h));
    // `ic-stats`: a hit counts here; a miss counts in the probe of the runtime call
    // ([`mega_load`]), so each probe counts once.
    if r.is_some() {
        stub_stats!(Load, true);
        #[cfg(test)]
        HANDLER_PROBE_HITS.with(|c| c.set(c.get() + 1));
    }
    r
}

#[cfg(test)]
thread_local! {
    /// Tests (M7-P1 NAMEDLOAD): how many loads the probe of the `GetNamedProperty` handler
    /// ([`handler_mega_load`]) answered.
    pub(crate) static HANDLER_PROBE_HITS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
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
        // A representation-checked store kind reads as its plain field handler (M5-P2
        // CONSTREP; `tests::field_rep` reads the stored word with `find_raw`).
        let x = smi::plain(int32_of(h));
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
        /// Adds that grew the SLOTS cell on the store miss path (`grow_add`, M8-P0 TS-STORE).
        grow: Cell<u64>,
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
            if self.grow.get() > 0 {
                eprintln!("ep9 ic-stats: store grow={}", self.grow.get());
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
            grow: Cell::new(0),
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

    /// An add that grew the SLOTS cell on the store miss path (`grow_add`).
    #[inline(never)]
    pub(crate) fn grow() {
        C.with(|c| c.grow.set(c.grow.get() + 1));
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

// ---- the cell reads of the global hit paths (M7-P2 GLOBALS) -------------------------------------

/// The cell of `b`, or `None` where `object::cell` would panic (a hit path then misses).
#[inline(always)]
fn as_cell(b: u64) -> Option<CellPtr> {
    CellPtr::from_value(crate::runtime::val(b))
}

/// The word0 ShapeId of the live cell `c`.
#[inline(always)]
fn id_of(c: CellPtr) -> u32 {
    // SAFETY: a cell value the VM holds is live (crate invariant, object.rs).
    unsafe { c.header() }.shape_id().get()
}

// ---- the stub cache probe of the store handlers and the add with growth (M8-P0 TS-STORE) -------

/// The stub cache probe of the `SetNamedProperty` (0x24) and `DefineNamedOwnProperty` (0x25,
/// `define`) handlers: in their cold bodies (the front of `slow::set_named`), after the
/// monomorphic and polymorphic compares of [`store`] missed, and before the runtime call. The
/// store form of [`handler_mega_load`]: if slot `slot` of the vector of `fi` is megamorphic
/// and the stub cache has an entry for `o`'s shape and the name `key` (an atom), stores `val`
/// through its handler ([`store_with`]). False: the runtime call runs as before (it probes
/// again, [`mega_store`], which can also grow the SLOTS cell, then takes the generic path,
/// which fills the entry, [`record`]).
///
/// The hit path ([`store`]) does not change. A hit here skips the runtime call: the frame
/// record (`Runtime::enter`), the call, and its vector and slot reads. An add with no room
/// misses here (the growth allocates). No allocation, no JS, not a GC point.
#[inline(always)]
pub(crate) fn handler_mega_store(
    rt: &Runtime,
    fi: &FunctionInfo,
    o: u64,
    slot: u32,
    key: u64,
    val: u64,
    define: bool,
) -> bool {
    if !kind::is_object(o) {
        return false;
    }
    let Some(v) = feedback::vector(fi) else {
        return false;
    };
    if !is_mega(v, slot) {
        return false;
    }
    let done = match stub_cache::probe(rt, store_op(define), shape_id(o), key) {
        Some(h) if define => store_with::<false>(rt, o, || key, val, h),
        Some(h) => store_with::<true>(rt, o, || key, val, h),
        None => false,
    };
    // `ic-stats`: a hit counts here; a miss counts in the probe of the runtime call
    // ([`mega_store`]), so each probe counts once.
    if done {
        stub_stats!(Store, true);
        #[cfg(test)]
        HANDLER_STORE_HITS.with(|c| c.set(c.get() + 1));
    }
    done
}

/// The add with growth of a monomorphic or polymorphic store site, on the miss path
/// (`slow::set_named`, after its frame record): the `ADD` case of slot `slot` of the vector
/// `v` for `o`'s shape, when only the room in the SLOTS cell is missing ([`grow_add`]). A
/// megamorphic slot gives false here ([`mega_store`] grows through its stub cache entry). True
/// if it stored; false: the generic path. No JS. GC point (the growth).
pub(crate) fn grow_store(
    rt: &Runtime,
    v: u64,
    slot: u32,
    o: u64,
    key: u64,
    val: u64,
    define: bool,
) -> bool {
    if !kind::is_object(o) || is_mega(v, slot) {
        return false;
    }
    let Some(h) = handler_for(
        feedback::key(v, slot),
        feedback::value(v, slot),
        shape_id(o),
    ) else {
        return false;
    };
    if define {
        grow_add::<false>(rt, o, key, val, h)
    } else {
        grow_add::<true>(rt, o, key, val, h)
    }
}

/// [`store_with`], then [`grow_add`] if it missed: the stub cache entry of [`mega_store`].
/// GC point (the growth).
#[inline(always)]
fn store_or_grow<const CHECK_EPOCH: bool>(
    rt: &Runtime,
    o: u64,
    key: u64,
    val: u64,
    h: u64,
) -> bool {
    store_with::<CHECK_EPOCH>(rt, o, || key, val, h) || grow_add::<CHECK_EPOCH>(rt, o, key, val, h)
}

/// An add case `h` (0x24 with `CHECK_EPOCH`, or 0x25) for `o` that applies except that the
/// SLOTS cell of `o` has no room for the new out-of-line field: grows the SLOTS cell as the
/// generic add does (`props::reserve_field`, the same capacity rule), then stores through `h`
/// ([`store_with`]). The checks of the hit come first (the validity cell, the target shape
/// alive, the representation of the new field), so a case that misses for any other reason
/// grows nothing. The add keeps the order of the generic `[[Set]]` (H16 K13: the case was
/// checked after a generic store; the hit itself still needs room, so this runs only on the
/// miss path). False: nothing stored, take the generic path (it finds the room made here).
///
/// GC point (the new SLOTS cell): `o`, `val`, `h` and the target shape live in Rust locals
/// (CSS). A GC runs no JS and changes no prototype, so the validity cell checked here stays
/// valid, and [`store_with`] checks it again.
#[inline(never)]
fn grow_add<const CHECK_EPOCH: bool>(rt: &Runtime, o: u64, key: u64, val: u64, h: u64) -> bool {
    if is_i32(h) {
        return false;
    }
    let (hcell, add) = (cell(h), hk::ADD);
    // `add`: the kind test line of [`store_with`] is an adr-lint pin, which must match 1 line.
    if rd_i32(hcell, hc::KIND) != add || (CHECK_EPOCH && !validity::valid_cell(rd(hcell, hc::CELL)))
    {
        return false;
    }
    let target = rd(hcell, hc::REF);
    if target == UNDEFINED
        || !crate::field_rep::table_fits(shape::last_field_rep(target), val)
        || rd_i32(hcell, hc::LOCATION) & smi::KIND_MASK == smi::FIELD_IN_OBJECT
    {
        return false;
    }
    // The new field's index under the target shape; the hit's location is its SLOTS offset.
    let f = shape::count(target) - 1;
    let s = rd(cell(o), obj::SLOTS);
    let have = if s == UNDEFINED {
        0
    } else {
        rd_i32(cell(s), slf::CAPACITY) as usize
    };
    if f - usize::from(shape::capacity(target)) < have {
        // There is room: the hit missed for another reason.
        return false;
    }
    crate::props::reserve_field(rt, o, target, f);
    let done = store_with::<CHECK_EPOCH>(rt, o, || key, val, h);
    if done {
        #[cfg(feature = "ic-stats")]
        stats::grow();
        #[cfg(test)]
        GROW_ADDS.with(|c| c.set(c.get() + 1));
    }
    done
}

#[cfg(test)]
thread_local! {
    /// Tests (M8-P0 TS-STORE): how many stores the probe of the store handlers
    /// ([`handler_mega_store`]) answered.
    pub(crate) static HANDLER_STORE_HITS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
    /// Tests (M8-P0 TS-STORE): how many adds grew the SLOTS cell on the miss path
    /// ([`grow_add`]).
    pub(crate) static GROW_ADDS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

// ---- x86-64 forms of the hit paths (M8-P1 X86-SHAPE2) ------------------------------------------
//
// x86-64 has 5 free caller-saved registers beside the 4 pinned handler arguments (H15; the
// handler type in `interp/mod.rs`), aarch64 has 18. A hit path that needs more live values on
// x86-64 makes LLVM save a callee-saved register on entry for every run of the handler. These
// forms give the same answers as the shared forms in fewer live values and with no jump table;
// aarch64 keeps the shared forms (its handler bytes do not change).

/// The hit path of [`load`] after the case test: [`load_with`] on aarch64 and other targets.
#[cfg(not(target_arch = "x86_64"))]
use self::load_with as load_hit;

/// The hit path of [`load`] after the case test: [`load_with_x86`] on x86-64.
#[cfg(target_arch = "x86_64")]
use self::load_with_x86 as load_hit;

/// [`load_with`] in the x86-64 form (M8-P1 X86-SHAPE2): the same answer for every handler `h`,
/// receiver `o` (a cell) and ShapeId `id`.
///
/// - The smi kinds take compares, not a `match`: LLVM makes the 4-way `match` of
///   [`load_with`] a jump table, and its indirect jump is about 11% of the indirect
///   mispredicts of octane-typescript on x86-64 (S0, cachegrind). The two field kinds share
///   one branch: the field's cell is a select of the receiver and its SLOTS word (an object
///   cell always has that word; the in-object kind does not use the value it reads). So
///   the kinds form at most 3 compare clusters, below the x86-64 jump table minimum of 4.
/// - The prototype path tests the ShapeId first and reads the handler kind again for the
///   `MISSING` test, so `id` and the kind are not live across the validity cell test.
///
/// Compiled on every target (a test compares it with [`load_with`]); called only on x86-64.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
#[inline(always)]
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn load_with_x86(rt: &Runtime, o: u64, id: u32, key: impl Fn() -> u64, h: u64) -> Option<u64> {
    if is_i32(h) {
        let x = int32_of(h);
        let k = x & smi::KIND_MASK;
        if (k as u32).wrapping_sub(smi::FIELD_IN_OBJECT as u32) < 2 {
            // FIELD_IN_OBJECT or FIELD_OUT_OF_LINE ([`field_ref`]).
            let s = rd(cell(o), obj::SLOTS);
            let c = core::hint::select_unpredictable(k == smi::FIELD_IN_OBJECT, o, s);
            // The test of [`entry`]: no panic path in the inlined hit path.
            return is_cell_bits(c).then(|| rd(cell(c), (x >> smi::SHIFT) as usize));
        }
        if k == smi::ARRAY_LENGTH {
            return Some(rd(cell(o), arr::LENGTH));
        }
        if k == smi::STRING_LENGTH && kind::is_string(o) {
            return Some(int32_bits(string::str_len(o) as i32));
        }
        return None;
    }
    if !is_cell_bits(h) {
        return None;
    }
    let c = cell(h);
    let k = rd_i32(c, hc::KIND);
    if k != hk::PROTO_LOAD && k != hk::MISSING {
        return None;
    }
    // ShapeId 0 is a string's or a symbol's: a KEY_STRING prototype case serves strings only.
    if id == 0 && !kind::is_string(o) {
        return None;
    }
    let vc = rd(c, hc::CELL);
    if !is_cell_bits(vc) || !validity::valid_cell(vc) {
        return None;
    }
    if rd_i32(c, hc::KIND) == hk::MISSING {
        #[cfg(debug_assertions)]
        cross_check(rt, o, key(), c);
        return Some(UNDEFINED);
    }
    // A dead holder reads undefined, which is not a cell.
    let holder = rd(c, hc::REF);
    if !is_cell_bits(holder) {
        return None;
    }
    #[cfg(debug_assertions)]
    cross_check(rt, o, key(), c);
    read_field(holder, rd_i32(c, hc::LOCATION))
}

/// The hit path of the `LdaGlobal` and `LdaGlobalInsideTypeof` fast bodies: [`load_global`]
/// on aarch64 and other targets, and on x86-64 with `ic-stats`.
#[cfg(any(not(target_arch = "x86_64"), feature = "ic-stats"))]
pub(crate) use self::load_global as load_global_fast;

/// The hit path of the `LdaGlobal` fast body on x86-64 (M8-P1 X86-SHAPE2; `LdaGlobalInsideTypeof`
/// since M8-P3 LEVER-6): [`load_global`] for
/// a monomorphic entry (a global object case or a lexical binding); `None` for every other
/// entry. The polymorphic loop of [`global_site`] keeps 4 more values live, so x86-64
/// `LdaGlobal` saved 4 registers on entry for every run. A polymorphic entry goes to the cold
/// body, which runs [`load_global`] before the miss path (`interp::tails::load_global`), so a
/// hit stays a hit. With `ic-stats` the fast body keeps [`load_global`]: there it counts a miss
/// for each `None`.
#[cfg(all(target_arch = "x86_64", not(feature = "ic-stats")))]
#[inline(always)]
pub(crate) fn load_global_fast(cx: &Cx, fi: &FunctionInfo, slot: u32) -> Option<u64> {
    let (v, i) = (as_cell(feedback::vector(fi)?)?, slot as usize);
    debug_assert!(slot < feedback::slot_count(v.to_value().to_bits()));
    let at = |w| rd(v, feedback::fv::SLOTS + feedback::fv::SLOT_SIZE * i + w);
    let (key, value) = (at(feedback::fv::KEY), at(feedback::fv::VALUE));
    let lex = || Some(rd(as_cell(cx.realm)?, realmf::GLOBAL_LEXICAL));
    debug_assert!(lex() == Some(cx.rt().root(Root::Lexical)) && cx.global_this == cx.rt().global());
    // [`global_site`] without its polymorphic loop. The global object case first; the record
    // is read in each arm, so its ShapeId is not live beside the global object's.
    let g = as_cell(cx.global_this)?;
    let (o, loc) = if key as u32 == id_of(g) {
        let c = global_handler(value, id_of(as_cell(lex()?)?))?;
        (g, rd_i32(c, hc::LOCATION))
    } else {
        let lex = as_cell(lex()?)?;
        if key as u32 != id_of(lex) || !is_i32(value) {
            return None;
        }
        (lex, int32_of(value))
    };
    let off = (loc >> smi::SHIFT) as usize;
    let c = match loc & smi::KIND_MASK {
        smi::FIELD_IN_OBJECT => o,
        smi::FIELD_OUT_OF_LINE => as_cell(rd(o, obj::SLOTS))?,
        _ => return None,
    };
    Some(rd(c, off)).filter(|&v| v != HOLE)
}

#[cfg(test)]
mod x86_form_tests {
    //! [`load_with_x86`] answers as [`load_with`] (M8-P1 X86-SHAPE2). The form runs only on
    //! x86-64, but this test runs on every target. The prototype cases (cell handlers) run
    //! through the handlers in `tests::ic` and `tests::ic_ops` on x86-64 CI.

    use super::*;
    use crate::tests::{config, isolate};
    use crate::{Dispatch, symbol};

    #[test]
    fn the_x86_load_form_answers_as_the_shared_form_for_every_smi_kind() {
        let iso = isolate(config(Dispatch::Sibling, false));
        let rt = &iso.rt;
        let p = object::proto_of(object::new_plain_object(rt));
        let named = |n: usize| -> u64 {
            let o = object::new_object(rt, p);
            for i in 0..n {
                let k = crate::atoms::intern_str(rt, &format!("p{i}"));
                crate::props::own_define(rt, o, k, int32_bits(i as i32 + 100), attr::DEFAULT);
            }
            o
        };
        let small = named(2);
        let big = named(14);
        assert_ne!(
            rd(cell(big), obj::SLOTS),
            UNDEFINED,
            "the big object has a SLOTS cell"
        );
        let a = object::new_array(rt, 0);
        assert!(object::array_set(rt, a, 5, int32_bits(1)));
        let s = string::new_string(rt, "abcd");
        let y = symbol::new_symbol(rt, UNDEFINED);
        let mut fields = 0;
        // Objects: every kind at offsets that hold Values in an object cell (SLOTS, ELEMENTS,
        // the fixed and in-object fields: `obj`) and in a SLOTS cell (CAPACITY, DATA: `slf`).
        // A string or a symbol: only the kinds that read no field (the IC makes no field case
        // for a ShapeId 0 receiver).
        for (o, offsets, kinds) in [
            (small, &[8, 16, 24, 32][..], 0..16),
            (big, &[8, 16, 24, 32][..], 0..16),
            (a, &[8, 16, 24, 32][..], 0..16),
            (s, &[0][..], 4..16),
            (y, &[0][..], 4..16),
        ] {
            let id = shape_id(o);
            for k in kinds {
                for &off in offsets {
                    let h = smi_handler(k, off);
                    let want = load_with(rt, o, id, || UNDEFINED, h);
                    let got = load_with_x86(rt, o, id, || UNDEFINED, h);
                    assert_eq!(got, want, "kind {k} offset {off} receiver {o:#x}");
                    fields += usize::from(want.is_some());
                }
            }
            assert_eq!(
                load_with_x86(rt, o, id, || UNDEFINED, smi_handler(0, 0)),
                None
            );
            for h in [UNDEFINED, NULL, HOLE, MEGA, SLOW] {
                assert_eq!(load_with_x86(rt, o, id, || UNDEFINED, h), None);
                assert_eq!(load_with(rt, o, id, || UNDEFINED, h), None);
            }
        }
        // Each field kind hit somewhere (big's SLOTS cell; every in-object read), so the
        // compare read values, not only misses.
        assert!(fields > 15, "{fields}");
        assert_eq!(
            load_with_x86(rt, s, 0, || UNDEFINED, smi_handler(4, 0)),
            Some(int32_bits(4))
        );
        assert_eq!(
            load_with_x86(rt, y, 0, || UNDEFINED, smi_handler(4, 0)),
            None
        );
        assert_eq!(
            load_with_x86(rt, a, shape_id(a), || UNDEFINED, smi_handler(3, 0)),
            Some(int32_bits(6))
        );
    }
}
