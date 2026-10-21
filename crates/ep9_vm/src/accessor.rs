//! Accessor properties (§6.2.6, H17): the immutable ACCESSOR_PAIR cell, and the getter and
//! setter calls of `[[Get]]` and `[[Set]]`.
//!
//! ```text
//!  own property with attr::ACCESSOR      value slot (shape field or dictionary entry)
//!    attrs = ACCESSOR | e | c  ──────────>  ACCESSOR_PAIR  +8 getter  +16 setter
//! ```
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | ACCESSOR_PAIR | 67 | +8 `[[Get]]`, +16 `[[Set]]`: a callable, or undefined when absent | 24 |
//!
//! Rules (JSC style, plan §2 row 6):
//! - A pair never changes after it is made. `[[DefineOwnProperty]]` builds a new pair from
//!   the old halves and the descriptor, and stores it in place of the old one
//!   (`property::define_own_property`). So a pair that some code has read stays a true
//!   snapshot of the property it came from.
//! - An accessor property has no `WRITABLE` bit. Code that reads raw property storage
//!   (`props::own_get`, `own_entry`, `own_value_or_proto`) gets the pair for an accessor and
//!   must test `attr::ACCESSOR` before it uses the value as a JS value. A pair never reaches
//!   JS: `[[Get]]` calls its getter and `[[GetOwnProperty]]` splits it into `get` and `set`.
//! - Replacing the pair of a prototype bumps the epoch (`shape::note_change`), so an IC that
//!   caches a getter or setter found on a prototype is valid while the epoch holds.

use ep9_heap::{CellKind, CellSize, Heap, Tracer};

use crate::call;
use crate::conv::Res;
use crate::descriptor::Desc;
use crate::frame::UNDEFINED;
use crate::interp::Cx;
use crate::kinds::{self as kind, is_callable};
use crate::object::{attr, cell, init, rd, trace_from_8};
use crate::runtime::Runtime;

mod pairf {
    pub const GET: usize = 8;
    pub const SET: usize = 16;
    pub const SIZE: usize = 24;
}

fn trace_pair(c: ep9_heap::CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, pairf::SIZE);
}

/// Registers the ACCESSOR_PAIR cell kind.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::ACCESSOR_PAIR,
        CellKind {
            name: "accessor-pair",
            size: CellSize::Fixed(pairf::SIZE),
            trace: trace_pair,
        },
    )
    .expect("register a VM cell kind");
}

/// A new pair of `get` and `set` (each a callable or undefined). GC point: both live in Rust
/// locals (conservative scan) across the allocation.
pub(crate) fn new_pair(rt: &Runtime, get: u64, set: u64) -> u64 {
    debug_assert!(get == UNDEFINED || is_callable(get));
    debug_assert!(set == UNDEFINED || is_callable(set));
    let c = rt.alloc(kind::ACCESSOR_PAIR, pairf::SIZE);
    init(c, pairf::GET, get);
    init(c, pairf::SET, set);
    c.to_value().to_bits()
}

/// True if `v` is an ACCESSOR_PAIR cell.
#[inline]
pub(crate) fn is_pair(v: u64) -> bool {
    kind::cell_kind(v) == kind::ACCESSOR_PAIR.get()
}

/// `[[Get]]` of a pair: a callable or undefined.
#[inline]
pub(crate) fn getter(pair: u64) -> u64 {
    debug_assert!(is_pair(pair));
    rd(cell(pair), pairf::GET)
}

/// `[[Set]]` of a pair: a callable or undefined.
#[inline]
pub(crate) fn setter(pair: u64) -> u64 {
    debug_assert!(is_pair(pair));
    rd(cell(pair), pairf::SET)
}

/// The complete descriptor of a stored property: an accessor descriptor (`get`, `set`,
/// `enumerable`, `configurable`) if `attrs` has `attr::ACCESSOR`, else a data descriptor.
pub(crate) fn stored_desc(v: u64, attrs: i32) -> Desc {
    if attrs & attr::ACCESSOR == 0 {
        return Desc::data(v, attrs);
    }
    Desc {
        get: Some(getter(v)),
        set: Some(setter(v)),
        enumerable: Some(attrs & attr::ENUMERABLE != 0),
        configurable: Some(attrs & attr::CONFIGURABLE != 0),
        ..Desc::default()
    }
}

/// The getter step of OrdinaryGet (§10.1.8.1 steps 4-7): undefined without a getter, else
/// `Call(getter, receiver)`. Runs JS (GC point).
#[cold]
#[inline(never)]
pub(crate) fn call_getter(cx: &mut Cx, pair: u64, receiver: u64) -> Res<u64> {
    let g = getter(pair);
    if g == UNDEFINED {
        return Ok(UNDEFINED);
    }
    // GC: `g` and `receiver` live in Rust locals; the call copies them into its frame.
    call::call_function(cx, g, receiver, &[])
}

/// The setter step of OrdinarySetWithOwnDescriptor (§10.1.9.2 steps 4-7): false without a
/// setter, else `Call(setter, receiver, «v»)` and true. Runs JS (GC point).
#[cold]
#[inline(never)]
pub(crate) fn call_setter(cx: &mut Cx, pair: u64, receiver: u64, v: u64) -> Res<bool> {
    let s = setter(pair);
    if s == UNDEFINED {
        return Ok(false);
    }
    call::call_function(cx, s, receiver, &[v])?;
    Ok(true)
}
