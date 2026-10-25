//! Inline caches for named get and set in T0 (H16 v0; OM-5 to OM-8): the IC entry states,
//! the handlers, the POLY_IC and IC_HANDLER cells, the hit paths that T0's fast handlers
//! inline, and the miss path that computes and records a handler.
//!
//! ```text
//!  feedback slot i (16 B, feedback.rs)       key (weak)                 value (strong)
//!   uninitialized                            undefined                  int32 0
//!   monomorphic                              a shape | KEY_STRING       a handler
//!   polymorphic (2-4 cases)                  hole                       POLY_IC cell
//!   megamorphic (sticky, generic path)       hole                       int32 MEGA
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
//! | IC_HANDLER | 70 | +8 kind, +16 epoch (int32s), +24 holder or target shape (weak), +32 location (an int32 field handler) | 40 |
//!
//! | Handler | Sites | Hit |
//! |---|---|---|
//! | `FIELD_IN_OBJECT`, `FIELD_OUT_OF_LINE` (smi) | 0x21, 0x22: own data property; 0x24: own writable data property; 0x25: own data property {w, e, c} | one load or one barriered store at the offset (in the SLOTS cell for out of line) |
//! | `ARRAY_LENGTH` (smi) | 0x21, 0x22 on an array's shape | the length field (+24) |
//! | `STRING_LENGTH` (smi) | 0x21 on a string, under [`KEY_STRING`] | the string's length, after a kind-byte check |
//! | `SLOW` (smi) | any: accessor, dictionary, exotic or read-only cases | none: the generic path runs, the case only keeps its place |
//! | `PROTO_LOAD` (cell) | 0x21, 0x22 | epoch unchanged and holder alive: the holder's field |
//! | `MISSING` (cell) | 0x21, 0x22 | epoch unchanged: undefined |
//! | `ADD` (cell) | 0x24 (epoch checked), 0x25 | target alive and room in the slots: store, then install the target shape |
//! | smi field, keyed on the lexical record's shape | 0x10, 0x11; 0x12 if writable | a `let`/`const`/`class` binding; the hole (TDZ) misses |
//! | `GLOBAL` (cell), keyed on the global object's shape | 0x10, 0x11; 0x12 if writable | the lexical record still has the shape in the handler: the global object's field |
//!
//! Rules:
//! - Every heap reference in IC data is weak (plan rule 8, I42): entry and case keys, a
//!   handler's holder and target. Only the value half of an entry (a handler or a POLY_IC
//!   cell) and a POLY_IC case's handler are strong. A dead key reads undefined after the GC,
//!   so its case is a miss, and its place is free for the next case.
//! - Handlers that depend on the prototype chain (prototype load, missing property, the
//!   add-property transition of a store) keep the `proto_epoch` they were made under and
//!   miss when it differs (OM-8). The epoch is read through `Cx::proto_epoch` (I43). A
//!   value store to a prototype does not bump the epoch, so a prototype load reads the
//!   holder's field at every hit.
//! - The epoch saturates at [`shape::EPOCH_SATURATED`] (W4-P3FIX-EPOCH-SAT). There, such a
//!   case is `SLOW` instead ([`load_case`], [`store_pre`]), so no handler that compares the
//!   epoch ever holds that value, and every one made before misses from then on. A 0x25 add
//!   case may hold it: its hit does not compare the epoch, and the verifier gives 0x24 and
//!   0x25 different slot kinds, so a 0x24 site never reads it.
//! - A fast shape is always extensible and a non-extensible object has a dictionary shape
//!   (OM-10), so an add-property case, which is made only for a fast shape, can never match a
//!   non-extensible receiver. [`store`] also asserts it.
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

use core::cell::Cell;

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
use crate::unit::FunctionInfo;

/// The entry and case key that stands for "a string receiver" (strings have no shape; their
/// word0 ShapeId is 0, which is the low half of int32 0). Only [`smi::STRING_LENGTH`] uses it,
/// and that handler checks the kind byte, because symbols have ShapeId 0 too.
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
}

/// IC_HANDLER fields.
mod hc {
    pub const KIND: usize = 8;
    pub const EPOCH: usize = 16;
    /// The holder (prototype load) or the target shape (add); undefined for a missing case.
    pub const REF: usize = 24;
    /// The field as a smi field handler.
    pub const LOCATION: usize = 32;
    pub const SIZE: usize = 40;
}

/// POLY_IC fields.
mod pc {
    pub const CASES: usize = 8;
    pub const CASE: usize = 16;
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
    // SAFETY: the heap passes a live IC_HANDLER cell; +24 is a Value slot (the other words
    // are int32s).
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

#[inline(always)]
fn epoch_now(epoch: *const Cell<u32>) -> u32 {
    // SAFETY: `Cx::proto_epoch` points at the runtime's `ShapeState::proto_epoch`, which
    // lives as long as the isolate.
    unsafe { (*epoch).get() }
}

/// The hit path of a named load (opcodes 0x21 and 0x22): the value of `o.key` from slot
/// `slot` of the vector of `fi`, or `None` on a miss (no vector, no case for `o`'s shape, a
/// `SLOW` case, a changed epoch, a dead holder). `o` is the receiver (0x21) or the lookup
/// start (0x22); any value.
#[inline(always)]
pub(crate) fn load(fi: &FunctionInfo, epoch: *const Cell<u32>, o: u64, slot: u32) -> Option<u64> {
    if !is_cell_bits(o) {
        return None;
    }
    let (key, value) = entry(fi, slot)?;
    let h = handler_for(key, value, shape_id(o))?;
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
    if rd_i32(c, hc::EPOCH) as u32 != epoch_now(epoch) {
        return None;
    }
    match rd_i32(c, hc::KIND) {
        hk::PROTO_LOAD => {
            let holder = rd(c, hc::REF);
            if holder == UNDEFINED {
                return None;
            }
            read_field(holder, rd_i32(c, hc::LOCATION))
        }
        hk::MISSING => Some(UNDEFINED),
        _ => None,
    }
}

/// The hit path of a named store: `o.key = v` (0x24, `CHECK_EPOCH`) or CreateDataProperty
/// (0x25) from slot `slot` of the vector of `fi`. False on a miss; nothing is stored then.
#[inline(always)]
pub(crate) fn store<const CHECK_EPOCH: bool>(
    rt: &Runtime,
    fi: &FunctionInfo,
    epoch: *const Cell<u32>,
    o: u64,
    slot: u32,
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
        || (CHECK_EPOCH && rd_i32(hcell, hc::EPOCH) as u32 != epoch_now(epoch))
    {
        return false;
    }
    let target = rd(hcell, hc::REF);
    if target == UNDEFINED {
        return false;
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
    // ([`store_case`]), so `o` is extensible and its change needs no epoch bump.
    debug_assert!(shape::is_extensible(shape::shape_of(o)) && !shape::is_prototype_object(o));
    shape::install(rt, o, target);
    true
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

/// True if the miss path of slot `i` of `v` for the receiver `o` need not compute a case:
/// the entry is megamorphic, `o` has no case key, or its case is `SLOW` (the generic path
/// runs every time). Not a GC point.
pub(crate) fn skip_miss(v: u64, i: u32, o: u64) -> bool {
    if state(v, i) == State::Megamorphic {
        return true;
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
    /// A prototype load: holder, its field (a smi field handler), the epoch.
    ProtoLoad {
        holder: u64,
        location: i32,
        epoch: u32,
    },
    /// A missing property under the epoch.
    Missing { epoch: u32 },
    /// An add-property transition: the target shape, its new field, the epoch.
    Add {
        target: u64,
        location: i32,
        epoch: u32,
    },
    /// An own data property of the global object (its field) under the lexical record's
    /// shape `lexical`.
    Global { lexical: u64, location: i32 },
}

/// A case for one entry: its key (a shape or [`KEY_STRING`]) and its handler. The shape and
/// the holder live in Rust locals until [`record`] stores them (the conservative stack scan
/// keeps them, H10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Case {
    pub key: u64,
    pub handler: Handler,
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
/// in its shape, `module.rs`). Array-index keys never get here, so an arguments object
/// (whose exotic keys are mapped indices) needs no entry.
fn exotic_name(rt: &Runtime, k: u8, name: u64) -> bool {
    if k == kind::ARRAY.get() || k == kind::PRIMITIVE_WRAPPER.get() {
        name == rt.atom(Atom::Length)
    } else if k == kind::FUNCTION.get() {
        name == rt.atom(Atom::Prototype)
    } else {
        k == kind::MODULE_NAMESPACE.get()
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
        // v0: only `length`; a method load on a string takes the generic path.
        return (name == rt.atom(Atom::Length)).then_some(Case {
            key: KEY_STRING,
            handler: Handler::Smi(smi_handler(smi::STRING_LENGTH, 0)),
        });
    }
    if !kind::is_object(o) {
        return None;
    }
    let s = shape::shape_of(o);
    let k = shape::object_kind(s);
    let smi_case = |h| {
        Some(Case {
            key: s,
            handler: Handler::Smi(h),
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
            let epoch = rt.shapes.proto_epoch.get();
            if epoch == shape::EPOCH_SATURATED {
                // The epoch no longer changes, so a case made now would never miss again
                // (W4-P3FIX-EPOCH-SAT).
                return smi_case(SLOW);
            }
            let handler = match walk(rt, shape::proto(s), name) {
                Chain::Data(holder, loc, _) => Handler::ProtoLoad {
                    holder,
                    location: int32_of(loc),
                    epoch,
                },
                Chain::Missing => Handler::Missing { epoch },
                Chain::Other => Handler::Smi(SLOW),
            };
            Some(Case { key: s, handler })
        }
    }
}

/// What the miss path of a store knows before the store runs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StorePre {
    /// The case is decided.
    Done(Case),
    /// The store may add `name` to an object with the shape `from` under `epoch`; the shape
    /// it produces decides ([`store_case_after`]).
    Add { from: u64, epoch: u32 },
}

/// The case for `o.name = v` (0x24, `define` false) or CreateDataPropertyOrThrow (0x25,
/// `define` true), before the store runs. Not a GC point.
pub(crate) fn store_pre(rt: &Runtime, o: u64, name: u64, define: bool) -> Option<StorePre> {
    if !cacheable_key(name) || !kind::is_object(o) {
        return None;
    }
    let s = shape::shape_of(o);
    let k = shape::object_kind(s);
    let slow = Some(StorePre::Done(Case {
        key: s,
        handler: Handler::Smi(SLOW),
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
    let epoch = rt.shapes.proto_epoch.get();
    if !define {
        // A 0x24 add case compares the epoch: none under the saturated one
        // (W4-P3FIX-EPOCH-SAT).
        if epoch == shape::EPOCH_SATURATED {
            return slow;
        }
        // [[Set]] adds to the receiver only if the chain has no setter and no read-only
        // property `name` (§10.1.9.2); a writable data property on the chain is fine.
        match walk(rt, shape::proto(s), name) {
            Chain::Missing | Chain::Data(_, _, true) => {}
            Chain::Data(_, _, false) | Chain::Other => return slow,
        }
    }
    Some(StorePre::Add { from: s, epoch })
}

/// The case of a store that ran: a decided case, or for an add, the transition from `from`
/// to the shape `o` has now, if the store added `name` as the next data property {w, e, c}
/// of a fast shape. Anything else (dictionary mode, another shape) is `SLOW`. Not a GC point.
pub(crate) fn store_case_after(rt: &Runtime, pre: StorePre, o: u64, name: u64) -> Option<Case> {
    let (from, epoch) = match pre {
        StorePre::Done(c) => return Some(c),
        StorePre::Add { from, epoch } => (from, epoch),
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
        debug_assert!(shape::is_extensible(from) && rt.shapes.proto_epoch.get() == epoch);
        Handler::Add {
            target: to,
            location: int32_of(field_location(to, n)),
            epoch,
        }
    } else {
        Handler::Smi(SLOW)
    };
    Some(Case { key: from, handler })
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
/// to `slow::store_global`). When the miss records a field case, the lookup of [`global_case`]
/// finds the binding, `name = v` is stored through that case ([`global_case_store`]) and the
/// case is recorded in slot `slot`: a recording miss scans the lexical record's table and the
/// global object's table once each, not twice. Every other miss runs the generic path,
/// `slow::store_global`, which computes the case itself: an entry that needs no case
/// ([`skip_global_miss`]), or a case that is not a writable field outside a TDZ (`SLOW`: a
/// `const`, an accessor, a read-only or dictionary-mode global; none: an inherited or a
/// missing global; the hole). Returns the stored value, or 0 with a pending exception (as
/// `slow::store_global`). GC point.
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
    if let Some(vector) = feedback::vector(FrameRef(fp).info())
        && !skip_global_miss(rt, vector, slot)
        && let Some(case) = global_case(rt, name, true)
        && global_case_store(rt, &case, v)
    {
        // GC point (the stored value is in the caller's accumulator): the interpreter state
        // first, as `slow::store_global` does.
        rt.enter(fp, pc);
        record(rt, vector, slot, case);
        return v;
    }
    slow::store_global(cx, fp, pc, name, v, slot)
}

/// True if the miss path of the global slot `i` of `v` need not compute a case: the entry is
/// megamorphic, or its case for the lexical record's or the global object's shape is `SLOW`.
/// Not a GC point.
pub(crate) fn skip_global_miss(rt: &Runtime, v: u64, i: u32) -> bool {
    state(v, i) == State::Megamorphic
        || find(v, i, shape::shape_of(rt.root(Root::Lexical))) == Some(SLOW)
        || find(v, i, shape::shape_of(rt.global())) == Some(SLOW)
}

// ---- recording -------------------------------------------------------------------------------

/// A new IC_HANDLER cell. GC point.
fn new_handler(rt: &Runtime, kind: i32, epoch: u32, r: u64, location: i32) -> u64 {
    let c = rt.alloc(kind::IC_HANDLER, hc::SIZE);
    init(c, hc::KIND, int32_bits(kind));
    init(c, hc::EPOCH, int32_bits(epoch as i32));
    init(c, hc::REF, r);
    init(c, hc::LOCATION, int32_bits(location));
    c.to_value().to_bits()
}

/// Records `case` in slot `i` of the vector `v` (OM-6): an empty entry becomes monomorphic; a
/// case for a key the entry has replaces that handler; a new key makes a monomorphic entry
/// polymorphic, fills a free case of a polymorphic one, or makes a full one megamorphic.
/// A megamorphic entry stays. GC point (the handler cell and the POLY_IC cell).
pub(crate) fn record(rt: &Runtime, v: u64, i: u32, case: Case) {
    debug_assert!(
        !matches!(case.handler, Handler::ProtoLoad { epoch, .. } | Handler::Missing { epoch }
            if epoch == shape::EPOCH_SATURATED),
        "a load case under the saturated epoch"
    );
    // GC points below: `v` (also a unit root), the case's key and references live in Rust
    // locals (conservative scan). Each allocation comes before the entry is read, so a key
    // that dies in that GC reads undefined.
    let handler = match case.handler {
        Handler::Smi(h) => h,
        Handler::ProtoLoad {
            holder,
            location,
            epoch,
        } => new_handler(rt, hk::PROTO_LOAD, epoch, holder, location),
        Handler::Missing { epoch } => new_handler(rt, hk::MISSING, epoch, UNDEFINED, 0),
        Handler::Add {
            target,
            location,
            epoch,
        } => new_handler(rt, hk::ADD, epoch, target, location),
        Handler::Global { lexical, location } => new_handler(rt, hk::GLOBAL, 0, lexical, location),
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
        return; // megamorphic: sticky
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
        None => feedback::set_slot(rt, v, i, HOLE, MEGA),
    }
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

/// The parts of a handler, for tests: (kind, epoch, reference, location) of an IC_HANDLER, or
/// the smi kind of an int32 handler as (kind, 0, undefined, the int32).
#[cfg(test)]
pub(crate) fn describe_handler(h: u64) -> (i32, u32, u64, i32) {
    if is_i32(h) {
        let x = int32_of(h);
        return (x & smi::KIND_MASK, 0, UNDEFINED, x);
    }
    let c = cell(h);
    (
        rd_i32(c, hc::KIND) | 0x100,
        rd_i32(c, hc::EPOCH) as u32,
        rd(c, hc::REF),
        rd_i32(c, hc::LOCATION),
    )
}

/// IC hit and miss counters (feature `ic-stats`, H27; never in `ep9`, dep-guards check 11). A
/// report build: `cargo build --release -p ep9_cli --features ep9_vm/ic-stats`. T0's fast
/// handlers count each IC site visit as a hit (the fast path answered) or a miss (it went
/// cold); a miss is sorted by the state of its feedback slot. The counters are per thread
/// (the isolate's thread) and print to stderr when that thread ends, one line per site kind:
///
/// ```text
/// ep9 ic-stats: load hit=N miss=N (no-vector=N uninit=N mono=N poly=N mega=N)
/// ```
///
/// Without the feature the count macro expands to nothing, so the default build is unchanged
/// (NATIVE-INV).
#[cfg(feature = "ic-stats")]
pub(crate) mod stats {
    use core::cell::Cell;

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
    }

    const SITES: [&str; 4] = ["load", "store", "load-global", "store-global"];
    /// Columns: hit, then the misses by slot state.
    const COLS: [&str; 6] = ["hit", "no-vector", "uninit", "mono", "poly", "mega"];

    pub(crate) struct Counters([[Cell<u64>; 6]; 4]);

    impl Drop for Counters {
        fn drop(&mut self) {
            for (s, row) in self.0.iter().enumerate() {
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
        }
    }

    thread_local! {
        static C: Counters = const { Counters([const { [const { Cell::new(0) }; 6] }; 4]) };
    }

    fn bump(site: Site, col: usize) {
        C.with(|c| {
            let x = &c.0[site as usize][col];
            x.set(x.get() + 1);
        });
    }

    /// A hit at a `site` (the fast path answered).
    #[inline(never)]
    pub(crate) fn hit(site: Site) {
        bump(site, 0);
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
    }
}

/// Counts an IC hit or miss with feature `ic-stats` ([`stats`]); nothing without it.
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
