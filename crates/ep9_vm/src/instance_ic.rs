//! The instanceof cache (owner M4-P3 IC2; OI-07, H16-Q3, V9-A8): the InstanceOf slot (14;
//! IC_LAYOUT 2, `profile.rs` table) of a `TestInstanceOf` site keeps, per rhs function, an
//! instanceof handler, and the hit runs OrdinaryHasInstance (§7.3.21) with no lookup of
//! `@@hasInstance` or `prototype`.
//!
//! ```text
//!  slot (key weak, value strong)                     INSTANCEOF handler (IC_HANDLER, kind 70)
//!   new            undefined      int32 0             +8  int32 hk::INSTANCEOF
//!   monomorphic    the function   a handler or SLOW   +16 validity cell of the function's
//!   polymorphic    the hole       POLY_IC (2-4 cases:     [[Prototype]] (strong; ic.rs, validity.rs)
//!                                 function, handler)  +24 the function's shape (weak)
//!   megamorphic    the hole       int32 MEGA          +32 int32 field index of its `prototype`
//!
//!  interp/ops.rs TestInstanceOf: fast -> [`fast`] (M4-P0 hook)
//!                                cold -> slow::compare -> property::instance_of
//!                                        (generic; then [`site`] + [`record`] on success)
//! ```
//!
//! A hit (no allocation, no call): `c` is the slot's key or one of its POLY_IC keys, the
//! handler is a cell, `c`'s shape is the one at +24, the validity cell at +16 is valid. Then:
//! `o` not an object gives false (step 3); else `P` = the field at +32 of `c`, which must be
//! an object (else the generic path throws); then the chain walk of `o` (step 6), which
//! misses on a Proxy (its `[[GetPrototypeOf]]` is a trap; M4-P4 PROXY1).
//!
//! Rules:
//! - What the handler guards. InstanceofOperator (§13.10.2) is OrdinaryHasInstance when
//!   `GetMethod(c, @@hasInstance)` is `%Function.prototype[@@hasInstance]%` or undefined, and
//!   `c` is callable and not bound. The handler holds while: `c` keeps its shape (a FUNCTION
//!   cell with a fast shape and no own `@@hasInstance`, with an own data property `prototype`
//!   at the field index; any own property change makes a new shape); and no object on `c`'s
//!   prototype chain changes (the validity cell, H16 K5), where the chain had no
//!   `@@hasInstance`, or the first one was a data property that is neither writable nor
//!   configurable whose value is the intrinsic (so its value cannot change without a
//!   structural change). The `prototype` value is read at every hit, so a store to it needs no
//!   guard.
//! - Every other rhs that the generic path answers (a bound function, a function with its own
//!   or another `@@hasInstance`, an accessor `prototype`, a dictionary-mode function, a
//!   non-function object) gets the `SLOW` handler under its key: the site keeps its place and
//!   its facts (`facts_export.rs` reads the keys), and the generic path runs. A rhs that
//!   throws (not an object, not callable, a non-object `prototype`) records nothing; neither
//!   does a function whose lazy `prototype` is not made yet (a later miss records it).
//! - Writes only when new (V9-D6): a miss whose function already has a current handler or
//!   `SLOW` writes nothing; a megamorphic slot is sticky.
//! - Weak (plan rule 8, H16 K7): the key (the function) and +24 (its shape) are weak, so a
//!   dead function or shape reads undefined and its case misses; +16 keeps a validity cell,
//!   which keeps no object alive.
//! - The record runs in IC code, not in a `profile::record_*` hook, so the jitless
//!   `ics_only` preset keeps it (MODES-D11; MC-w14 IC2 line). Compiled code guards the
//!   chain through `GuardProtoValidity(ProtoRef)`, a Dynamic FuseRef, never the cell (H16
//!   CacheIR); only T0 reads the cell.

use ep9_bytecode::{Opcode, OperandKind};

use crate::error::FrameRef;
use crate::feedback;
use crate::frame::{FALSE, HOLE, NULL, TRUE, int32_bits};
use crate::ic::{self, Case, Handler, State, hk};
use crate::interp::dsl::FSlot;
use crate::interp::{Cx, Flow, St};
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, attr, cell, init, obj, rd, rd_i32};
use crate::runtime::{Atom, Runtime};
use crate::shape::{self, slf};
use crate::symbol;
use crate::unit::FunctionInfo;
use crate::validity;

/// INSTANCEOF handler fields (an IC_HANDLER cell, kind 70; the `ic.rs` IC_LAYOUT 2 table).
pub(crate) mod io {
    pub const KIND: usize = 8;
    /// The validity cell of the function's `[[Prototype]]` (strong).
    pub const CELL: usize = 16;
    /// The function's shape (weak).
    pub const SHAPE: usize = 24;
    /// The int32 field index of the function's own `prototype`.
    pub const FIELD: usize = 32;
    pub const SIZE: usize = 40;
}

/// POLY_IC case offsets: the `ic.rs` POLY_IC layout (`ic::pc`), not a copy.
const POLY_KEY: usize = ic::pc::CASES;
const POLY_CASE: usize = ic::pc::CASE;

// ---- the hit path -------------------------------------------------------------------------------

/// `o instanceof c` (InstanceOf slot `s`): on a hit the boolean goes to the accumulator.
#[inline(always)]
pub(crate) fn fast(st: &mut St<'_>, o: u64, c: u64, s: FSlot) -> Flow {
    match hit(st.rt(), st.info(), o, c, s.0) {
        Some(b) => {
            ic::ic_count!(hit, InstanceOf);
            st.acc = b;
            Flow::Next
        }
        None => {
            ic::ic_count!(miss, InstanceOf, st.info(), s.0);
            Flow::Cold
        }
    }
}

/// The handler slot `slot` of `fi`'s vector keeps for the function `c` (the monomorphic case
/// or a polymorphic one), if any. A dead key reads undefined, which no cell equals.
#[inline(always)]
fn handler_for(fi: &FunctionInfo, c: u64, slot: u32) -> Option<u64> {
    let v = feedback::vector(fi)?;
    let (key, value) = (feedback::key(v, slot), feedback::value(v, slot));
    if key == c {
        return Some(value);
    }
    if key == HOLE && is_cell_bits(value) {
        let p = cell(value);
        for i in 0..ic::POLY_CASES {
            if rd(p, POLY_KEY + POLY_CASE * i) == c {
                return Some(rd(p, POLY_KEY + POLY_CASE * i + 8));
            }
        }
    }
    None
}

/// The field `field` (an index into the fast shape `s`'s fields) of the object `c`.
#[inline(always)]
fn field(c: u64, s: u64, field: usize) -> u64 {
    let (cap, k) = shape::field_layout(s);
    let cc = cell(c);
    if field < cap {
        rd(cc, obj::FIXED + 8 * (object::fixed_count(k) + field))
    } else {
        rd(cell(rd(cc, obj::SLOTS)), slf::DATA + 8 * (field - cap))
    }
}

/// The hit: the boolean (TRUE or FALSE bits), or `None` for the cold path.
#[inline(always)]
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn hit(rt: &Runtime, fi: &FunctionInfo, o: u64, c: u64, slot: u32) -> Option<u64> {
    if !is_cell_bits(c) {
        return None;
    }
    let h = handler_for(fi, c, slot)?;
    if !is_cell_bits(h) {
        return None; // SLOW
    }
    let hc = cell(h);
    debug_assert_eq!(rd_i32(hc, io::KIND), hk::INSTANCEOF);
    let s = rd(hc, io::SHAPE);
    if shape::shape_of(c) != s || !validity::valid_cell(rd(hc, io::CELL)) {
        return None;
    }
    #[cfg(debug_assertions)]
    cross_check(rt, c, hc);
    // OrdinaryHasInstance step 3: a primitive is no instance.
    if !kind::is_object(o) {
        return Some(FALSE);
    }
    // Steps 4-5: `prototype` must be an object (else the generic path throws).
    let p = field(c, s, rd_i32(hc, io::FIELD) as usize);
    if !kind::is_object(p) {
        return None;
    }
    // Step 6: the chain of `o`. A Proxy's [[GetPrototypeOf]] is a trap: the generic path.
    let mut q = o;
    loop {
        if is_proxy(q) {
            return None;
        }
        q = object::proto_of(q);
        if q == NULL {
            return Some(FALSE);
        }
        if q == p {
            return Some(TRUE);
        }
    }
}

#[inline(always)]
fn is_proxy(o: u64) -> bool {
    let k = kind::cell_kind(o);
    k == kind::PROXY.get() || k == kind::PROXY_CALLABLE.get()
}

/// The debug check of a hit (as `ic.rs`'s R1 cross-check): the handler's case is still the
/// one [`case_of`] computes for `c`. Not a GC point.
#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn cross_check(rt: &Runtime, c: u64, hc: ep9_heap::CellPtr) {
    let ok = match case_of(rt, c, false) {
        Computed::Cached { field, .. } => field == rd_i32(hc, io::FIELD),
        _ => false,
    };
    assert!(ok, "instance_ic: a valid handler gave a stale answer");
}

// ---- the miss path ------------------------------------------------------------------------------

/// The InstanceOf site the top VM frame is at, if it has a feedback vector: (vector, slot).
/// The generic `instanceof` (`property::instance_of`) asks this first, before it runs any JS:
/// the cold body of `TestInstanceOf` published the frame and its bytecode offset
/// (`Runtime::enter`). Any other caller (a host call of `Function.prototype[@@hasInstance]`)
/// is at another instruction and gets `None`. Not a GC point.
pub(crate) fn site(cx: &Cx) -> Option<(u64, u32)> {
    let fp = cx.rt().stack().fp();
    if fp.is_null() {
        return None;
    }
    let f = FrameRef(fp);
    let fi = f.info();
    if !fi.is_bytecode() || fi.code.is_null() {
        return None;
    }
    let v = feedback::vector(fi)?;
    // SAFETY: the code of a bytecode FunctionInfo is `code_len` bytes of verified bytecode.
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let insn = ep9_bytecode::decode_at(code, f.saved_offset()).ok()?;
    if insn.opcode != Opcode::TestInstanceOf {
        return None;
    }
    let j = (0..insn.operand_count()).find(|&j| insn.operand_kind(j) == OperandKind::Slot)?;
    let slot = insn.unsigned(j);
    (slot < feedback::slot_count(v)).then_some((v, slot))
}

/// What [`case_of`] finds for a rhs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Computed {
    /// A handler can serve it: the validity cell the chain had (valid, invalid or undefined)
    /// and the field index of `prototype`.
    Cached { cell: u64, field: i32 },
    /// The generic path serves it every time.
    Slow,
    /// Nothing to record now (the lazy `prototype` is not made yet).
    Later,
}

/// The case of the rhs `c` (an object) now. `intrinsic`: check that a `@@hasInstance` on the
/// chain holds the intrinsic of the current realm (the record does; the debug cross-check of
/// a hit only checks the structure, which is what the handler guards, so a hit in another
/// realm's code passes). Not a GC point.
fn case_of(rt: &Runtime, c: u64, intrinsic: bool) -> Computed {
    if !kind::is_function(c) {
        return Computed::Slow;
    }
    let s = shape::shape_of(c);
    if shape::is_dictionary(s) {
        return Computed::Slow;
    }
    let has = symbol::well_known(rt, symbol::WellKnownSymbol::HasInstance);
    if shape::lookup(rt, s, has).is_some() {
        return Computed::Slow;
    }
    let field = match shape::lookup(rt, s, rt.atom(Atom::Prototype)) {
        None => return Computed::Later,
        Some(f) if f.details.attrs() & attr::ACCESSOR != 0 => return Computed::Slow,
        Some(f) => i32::from(f.details.field()),
    };
    let p = shape::proto(s);
    if !chain_is_ordinary(rt, p, has, intrinsic) {
        return Computed::Slow;
    }
    Computed::Cached {
        cell: shape::validity_cell_of(shape::proto_info_of(rt, p)),
        field,
    }
}

/// True if `GetMethod(_, @@hasInstance)` on the chain from `p` (an object or null) finds
/// nothing, or first a data property that is neither writable nor configurable and (if
/// `intrinsic`) holds `%Function.prototype[@@hasInstance]%` of this realm. Not a GC point.
fn chain_is_ordinary(rt: &Runtime, mut p: u64, has: u64, intrinsic: bool) -> bool {
    while p != NULL {
        let k = kind::cell_kind(p);
        if k == kind::PROXY.get()
            || k == kind::PROXY_CALLABLE.get()
            || k == kind::MODULE_NAMESPACE.get()
        {
            return false;
        }
        if let Some((v, a)) = crate::props::own_get(p, has) {
            return a & (attr::ACCESSOR | attr::WRITABLE | attr::CONFIGURABLE) == 0
                && (!intrinsic || is_has_instance_intrinsic(rt, v));
        }
        p = object::proto_of(p);
    }
    true
}

/// Whether `h` is `%Function.prototype%[@@hasInstance]` of the current realm (VM builtin
/// 0.30). Allocates nothing.
fn is_has_instance_intrinsic(rt: &Runtime, h: u64) -> bool {
    let id = crate::builtins::BuiltinId::new(
        crate::natives::VM_BLOCK,
        crate::natives::vm::FUNCTION_HAS_INSTANCE,
    );
    crate::builtins::existing_function(rt, id) == Some(h)
}

/// True if `h` (a handler that slot `slot` of `v` keeps for `c`) is current: a cell whose
/// shape is `c`'s and whose validity cell is valid. Not a GC point.
fn current(c: u64, h: u64) -> bool {
    if !is_cell_bits(h) {
        return false;
    }
    let hc = cell(h);
    rd(hc, io::SHAPE) == shape::shape_of(c) && validity::valid_cell(rd(hc, io::CELL))
}

/// Records the case of the rhs `c` in slot `slot` of the vector `v`, after the generic
/// `instanceof` of the site ([`site`]) returned. Writes only a new state (V9-D6). GC point
/// (the handler, a validity cell, a POLY_IC cell): `c` lives in a Rust local (CSS).
pub(crate) fn record(rt: &Runtime, v: u64, slot: u32, c: u64) {
    if !kind::is_object(c) || ic::state(v, slot) == State::Megamorphic {
        return;
    }
    let old = ic::find(v, slot, c);
    let handler = match case_of(rt, c, true) {
        Computed::Later => return,
        Computed::Slow => {
            if old == Some(ic::SLOW) {
                return;
            }
            ic::SLOW
        }
        Computed::Cached { cell: seen, field } => {
            if old.is_some_and(|h| current(c, h)) {
                return;
            }
            let Some(vc) = chain_cell(rt, c, seen, field) else {
                return;
            };
            // GC point: `c`, its shape and `vc` live in Rust locals (CSS).
            let h = rt.alloc(kind::IC_HANDLER, io::SIZE);
            init(h, io::KIND, int32_bits(hk::INSTANCEOF));
            init(h, io::CELL, vc);
            init(h, io::SHAPE, shape::shape_of(c));
            init(h, io::FIELD, int32_bits(field));
            h.to_value().to_bits()
        }
    };
    ic::record(
        rt,
        v,
        slot,
        Case {
            key: c,
            handler: Handler::Ready(handler),
            stub: None,
        },
    );
}

/// The validity cell an INSTANCEOF handler for `c` keeps: `seen` if it is still valid; else
/// the current cell of `c`'s prototype (`validity::cell_for`), if the case under it is still
/// the one computed (`field`). `None`: record nothing (the chain changed, or it is deeper than
/// `validity::MAX_DEPTH`). GC point.
fn chain_cell(rt: &Runtime, c: u64, seen: u64, field: i32) -> Option<u64> {
    if validity::is_valid(seen) {
        return Some(seen);
    }
    // GC point: `c` lives in the caller's Rust locals (CSS). A GC runs no JS, so no shape
    // changes, and the case below is the one the new cell guards.
    let vc = validity::cell_for(rt, shape::proto(shape::shape_of(c)))?;
    match case_of(rt, c, true) {
        Computed::Cached { cell, field: f } if cell == vc && f == field => Some(vc),
        _ => None,
    }
}
