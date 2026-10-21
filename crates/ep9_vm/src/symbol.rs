//! Symbols (§6.1.5.1): the SYMBOL cell, the well-known symbols, `%Symbol.prototype%` and the
//! `Symbol.for` registry (H19 "Symbols").
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | SYMBOL | 8 | +8 `[[Description]]` (a string or undefined), +16 identity hash (int32, 30 bits), +24 flags (int32) | 32 |
//!
//! ```text
//! new_symbol(rt, desc)     allocate (GC point), hash = the next value of the isolate's PRNG
//! symbol_for(cx, key)      flatten + intern the key, probe the registry by the key's content
//!                          hash; found -> that symbol; else allocate FIRST (no borrow held),
//!                          THEN borrow_mut and insert (W4-GC-RULES)
//! key_for(sym)             the [[Description]] of a registered symbol (it is the key), else
//!                          undefined
//! GC                       one precise root source (it holds an Rc clone of SymbolRoots):
//!                          the well-known symbols in WellKnownSymbol order, %Symbol.prototype%,
//!                          then every registry entry (key and symbol)
//! ```
//!
//! - The identity hash never comes from the address (H12). It is the next value of a
//!   per-isolate PRNG (H12-Q3: splitmix64), whose seed comes from the per-process string hash
//!   key (`EP9_HASH_SEED` fixes it). So two isolates of one process, or two runs with one
//!   `EP9_HASH_SEED`, give the same hashes in the same order. The hash is 30 bits, the range of
//!   a string's table hash, so `key::key_hash` serves both in one shape index.
//! - Flags: bit 0 registered (`Symbol.for`), bit 1 well-known, bit 2 private (reserved for
//!   private names, M1).
//! - The registry is strong: a registered symbol lives as long as the isolate, because
//!   `Symbol.for` must give the same symbol again (§20.4.2.2). Its keys are atoms.

use core::cell::{Cell, RefCell};
use std::rc::Rc;

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::atoms;
use crate::conv::Res;
use crate::error::throw_error;
use crate::frame::{UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::kinds::{self as kind, is_string, is_symbol};
use crate::object::{self, cell, init, rd, rd_i32, trace_from_8};
use crate::runtime::{ErrorKind, Root, Runtime};
use crate::string::{self, JsStr, MAX_STRING_LENGTH};

mod symf {
    pub const DESCRIPTION: usize = 8;
    pub const HASH: usize = 16;
    pub const FLAGS: usize = 24;
    pub const SIZE: usize = 32;
}

/// Flag bits of a symbol (int32 at +24).
pub(crate) mod flag {
    /// Made by `Symbol.for` (in the registry).
    pub const REGISTERED: i32 = 1;
    /// One of the well-known symbols.
    pub const WELL_KNOWN: i32 = 2;
    /// Reserved for private names (M1).
    #[allow(dead_code)] // reserved bit (private names, M1)
    pub const PRIVATE: i32 = 4;
}

/// Mask of an identity hash (30 bits, like a string's table hash).
const HASH_MASK: u32 = (1 << 30) - 1;

/// The well-known symbols (§6.1.5.1 table 1), plus `dispose` and `asyncDispose` (explicit
/// resource management). Stable order: the isolate roots follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum WellKnownSymbol {
    AsyncIterator,
    HasInstance,
    IsConcatSpreadable,
    Iterator,
    Match,
    MatchAll,
    Replace,
    Search,
    Species,
    Split,
    ToPrimitive,
    ToStringTag,
    Unscopables,
    Dispose,
    AsyncDispose,
}

impl WellKnownSymbol {
    pub(crate) const ALL: [WellKnownSymbol; 15] = [
        WellKnownSymbol::AsyncIterator,
        WellKnownSymbol::HasInstance,
        WellKnownSymbol::IsConcatSpreadable,
        WellKnownSymbol::Iterator,
        WellKnownSymbol::Match,
        WellKnownSymbol::MatchAll,
        WellKnownSymbol::Replace,
        WellKnownSymbol::Search,
        WellKnownSymbol::Species,
        WellKnownSymbol::Split,
        WellKnownSymbol::ToPrimitive,
        WellKnownSymbol::ToStringTag,
        WellKnownSymbol::Unscopables,
        WellKnownSymbol::Dispose,
        WellKnownSymbol::AsyncDispose,
    ];

    /// The `Symbol` property name (`iterator` for `Symbol.iterator`).
    pub(crate) const fn name(self) -> &'static str {
        match self {
            WellKnownSymbol::AsyncIterator => "asyncIterator",
            WellKnownSymbol::HasInstance => "hasInstance",
            WellKnownSymbol::IsConcatSpreadable => "isConcatSpreadable",
            WellKnownSymbol::Iterator => "iterator",
            WellKnownSymbol::Match => "match",
            WellKnownSymbol::MatchAll => "matchAll",
            WellKnownSymbol::Replace => "replace",
            WellKnownSymbol::Search => "search",
            WellKnownSymbol::Species => "species",
            WellKnownSymbol::Split => "split",
            WellKnownSymbol::ToPrimitive => "toPrimitive",
            WellKnownSymbol::ToStringTag => "toStringTag",
            WellKnownSymbol::Unscopables => "unscopables",
            WellKnownSymbol::Dispose => "dispose",
            WellKnownSymbol::AsyncDispose => "asyncDispose",
        }
    }
}

// ---- per-isolate state ------------------------------------------------------------------------

/// One `Symbol.for` entry: the key (an atom; 0 = empty slot), its symbol and the key's
/// content hash.
#[derive(Clone, Copy, Debug, Default)]
struct RegEntry {
    key: u64,
    sym: u64,
    hash: u32,
}

const REGISTRY_MIN_CAPACITY: usize = 16;

/// The `Symbol.for` registry (§20.4.2.2 GlobalSymbolRegistry): open addressing, linear
/// probing, power-of-two capacity, at most half full, keyed by the key's content hash (never
/// an address, H12). Entries are never removed.
#[derive(Debug, Default)]
struct Registry {
    entries: Vec<RegEntry>,
    len: usize,
}

impl Registry {
    /// The symbol of the atom `key` with content hash `hash`.
    fn find(&self, hash: u32, key: u64) -> Option<u64> {
        if self.entries.is_empty() {
            return None;
        }
        let mask = self.entries.len() - 1;
        let mut i = hash as usize & mask;
        loop {
            let e = self.entries[i];
            if e.key == 0 {
                return None;
            }
            // Keys are atoms: one cell per text, so equal text is equal bits.
            if e.hash == hash && e.key == key {
                return Some(e.sym);
            }
            i = (i + 1) & mask;
        }
    }

    /// Adds a key that is not in the registry. Not a GC point (Rust memory only).
    fn insert(&mut self, hash: u32, key: u64, sym: u64) {
        if (self.len + 1) * 2 > self.entries.len() {
            let cap = (self.entries.len() * 2).max(REGISTRY_MIN_CAPACITY);
            let old = core::mem::replace(&mut self.entries, vec![RegEntry::default(); cap]);
            for e in old.into_iter().filter(|e| e.key != 0) {
                place(&mut self.entries, e);
            }
        }
        place(&mut self.entries, RegEntry { key, sym, hash });
        self.len += 1;
    }

    fn trace(&self, t: &mut Tracer<'_>) {
        for e in self.entries.iter().filter(|e| e.key != 0) {
            t.visit_bits(e.key);
            t.visit_bits(e.sym);
        }
    }
}

/// Puts `e` in the first free slot of its probe chain.
fn place(entries: &mut [RegEntry], e: RegEntry) {
    let mask = entries.len() - 1;
    let mut i = e.hash as usize & mask;
    while entries[i].key != 0 {
        i = (i + 1) & mask;
    }
    entries[i] = e;
}

/// What the symbol root source visits. Shared (`Rc`) by `SymbolState` and the root closure.
#[derive(Debug)]
struct SymbolRoots {
    /// The well-known symbols in [`WellKnownSymbol`] order (undefined before [`install`]).
    well_known: [Cell<u64>; WellKnownSymbol::ALL.len()],
    /// `%Symbol.prototype%` (undefined before [`install`]).
    prototype: Cell<u64>,
    /// Never borrowed across an allocation (W4-GC-RULES).
    registry: RefCell<Registry>,
}

impl SymbolRoots {
    fn trace(&self, t: &mut Tracer<'_>) {
        for w in &self.well_known {
            t.visit_bits(w.get());
        }
        t.visit_bits(self.prototype.get());
        self.registry
            .try_borrow()
            .expect("the symbol registry is borrowed across a GC point (W4-GC-RULES)")
            .trace(t);
    }
}

/// Per-isolate symbol state (`Runtime::symbols`): the roots (well-known symbols,
/// `%Symbol.prototype%`, the registry) and the identity-hash PRNG.
#[derive(Debug)]
pub(crate) struct SymbolState {
    roots: Rc<SymbolRoots>,
    /// splitmix64 state (H12-Q3).
    rng: Cell<u64>,
}

impl Default for SymbolState {
    fn default() -> Self {
        SymbolState {
            roots: Rc::new(SymbolRoots {
                well_known: core::array::from_fn(|_| Cell::new(UNDEFINED)),
                prototype: Cell::new(UNDEFINED),
                registry: RefCell::new(Registry::default()),
            }),
            rng: Cell::new(prng_seed()),
        }
    }
}

impl SymbolState {
    /// The next identity hash (30 bits): splitmix64 over the isolate's state.
    fn next_hash(&self) -> u32 {
        let s = self.rng.get().wrapping_add(0x9e37_79b9_7f4a_7c15);
        self.rng.set(s);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 34) as u32 & HASH_MASK
    }
}

/// The PRNG seed of a new isolate: 60 bits of the per-process string hash key (random, or
/// fixed by `EP9_HASH_SEED`), read through the keyed hash of two fixed texts. Never an
/// address.
fn prng_seed() -> u64 {
    let (_, a) = string::hash_text(JsStr::Latin1(b"ep9 symbol hash seed 0"));
    let (_, b) = string::hash_text(JsStr::Latin1(b"ep9 symbol hash seed 1"));
    (u64::from(a) << 30) | u64::from(b)
}

// ---- the cell ---------------------------------------------------------------------------------

fn trace_symbol(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, symf::SIZE);
}

/// Registers the SYMBOL cell kind.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::SYMBOL,
        CellKind {
            name: "symbol",
            size: CellSize::Fixed(symf::SIZE),
            trace: trace_symbol,
        },
    )
    .expect("register a VM cell kind");
}

/// A new symbol with `description` (a string or undefined) and `flags`. GC point:
/// `description` lives in a Rust local (conservative scan) across the allocation.
fn alloc_symbol(rt: &Runtime, description: u64, flags: i32) -> u64 {
    debug_assert!(description == UNDEFINED || is_string(description));
    let c = rt.alloc(kind::SYMBOL, symf::SIZE);
    init(c, symf::DESCRIPTION, description);
    init(c, symf::HASH, int32_bits(rt.symbols.next_hash() as i32));
    init(c, symf::FLAGS, int32_bits(flags));
    c.to_value().to_bits()
}

/// A new symbol whose `[[Description]]` is `description` (a string or undefined). This is
/// `Symbol(description)` after its ToString step. GC point.
#[allow(dead_code)] // callers: P2-BTAB (`Cx::new_symbol`), P3-BLTB (the Symbol constructor)
pub(crate) fn new_symbol(rt: &Runtime, description: u64) -> u64 {
    alloc_symbol(rt, description, 0)
}

/// `[[Description]]` of the symbol `sym`: a string or undefined.
pub(crate) fn description(sym: u64) -> u64 {
    debug_assert!(is_symbol(sym));
    rd(cell(sym), symf::DESCRIPTION)
}

/// The identity hash of the symbol `sym` (30 bits). Not a GC point.
pub(crate) fn hash(sym: u64) -> u32 {
    debug_assert!(is_symbol(sym));
    rd_i32(cell(sym), symf::HASH) as u32
}

/// The [`flag`] bits of the symbol `sym`.
pub(crate) fn flags(sym: u64) -> i32 {
    debug_assert!(is_symbol(sym));
    rd_i32(cell(sym), symf::FLAGS)
}

/// True for a symbol made by `Symbol.for`.
pub(crate) fn is_registered(sym: u64) -> bool {
    flags(sym) & flag::REGISTERED != 0
}

/// True for one of the well-known symbols.
#[allow(dead_code)] // callers: tests; P3 (Symbol builtins)
pub(crate) fn is_well_known(sym: u64) -> bool {
    flags(sym) & flag::WELL_KNOWN != 0
}

// ---- isolate setup and roots ----------------------------------------------------------------

/// Registers the symbol root source, then creates the well-known symbols (descriptions
/// "Symbol.iterator" and so on, §6.1.5.1) and `%Symbol.prototype%` (an ordinary object whose
/// prototype is `%Object.prototype%`, §20.4.3). Runs once, in `realm::init`, after the
/// intrinsics.
pub(crate) fn install(rt: &Runtime) {
    let roots = Rc::clone(&rt.symbols.roots);
    debug_assert!(
        roots.prototype.get() == UNDEFINED,
        "symbol::install runs once"
    );
    let source = Rc::clone(&roots);
    rt.heap.add_root_source(move |t| source.trace(t));
    for w in WellKnownSymbol::ALL {
        // GC points: each new symbol is a root before the next allocation.
        let desc = atoms::intern_str(rt, &format!("Symbol.{}", w.name()));
        let s = alloc_symbol(rt, desc, flag::WELL_KNOWN);
        roots.well_known[w as usize].set(s);
    }
    let proto = object::new_object(rt, rt.root(Root::ObjectPrototype));
    roots.prototype.set(proto);
}

/// The well-known symbol `w` of this isolate (undefined before [`install`]).
pub(crate) fn well_known(rt: &Runtime, w: WellKnownSymbol) -> u64 {
    rt.symbols.roots.well_known[w as usize].get()
}

/// `%Symbol.prototype%`: the prototype of Symbol wrapper objects (undefined before
/// [`install`]).
pub(crate) fn prototype(rt: &Runtime) -> u64 {
    rt.symbols.roots.prototype.get()
}

// ---- the registry -----------------------------------------------------------------------------

/// `Symbol.for(key)` steps 2-6 (§20.4.2.2) for the string `key` (the caller did ToString):
/// the registry's symbol for that text, or a new registered symbol whose description is the
/// key. GC point.
#[allow(dead_code)] // callers: P2-BTAB (`abi` symbol_for), P3-BLTB (`Symbol.for`)
pub(crate) fn symbol_for(cx: &mut Cx, key: u64) -> u64 {
    debug_assert!(is_string(key));
    let key = string::flatten(cx, key);
    let rt = cx.rt();
    let key = atoms::intern(rt, key);
    let hash = string::hash(key);
    let found = rt.symbols.roots.registry.borrow().find(hash, key);
    if let Some(s) = found {
        return s;
    }
    // Allocate first (GC point, no borrow held; `key` is a Rust local), then insert.
    let s = alloc_symbol(rt, key, flag::REGISTERED);
    rt.symbols.roots.registry.borrow_mut().insert(hash, key, s);
    s
}

/// KeyForSymbol (§20.4.5.1): the registry key of `sym` (a string), or undefined if `sym` is
/// not registered. A registered symbol's description is its key. Not a GC point.
#[allow(dead_code)] // callers: P2-BTAB (`abi` key_for), P3-BLTB (`Symbol.keyFor`)
pub(crate) fn key_for(sym: u64) -> u64 {
    if is_registered(sym) {
        description(sym)
    } else {
        UNDEFINED
    }
}

/// The number of `Symbol.for` entries (tests).
#[cfg(test)]
pub(crate) fn registry_len(rt: &Runtime) -> usize {
    rt.symbols.roots.registry.borrow().len
}

// ---- text ---------------------------------------------------------------------------------------

/// SymbolDescriptiveString (§20.4.3.3.1) as Rust text, for error messages and `describe`
/// (lone surrogates become U+FFFD). Never runs JS, never allocates a cell.
pub(crate) fn descriptive_text(sym: u64) -> String {
    let d = description(sym);
    if d == UNDEFINED {
        "Symbol()".into()
    } else {
        format!("Symbol({})", string::to_rust_string(d))
    }
}

/// SymbolDescriptiveString (§20.4.3.3.1): the string "Symbol(" + description + ")". Throws
/// RangeError "Invalid string length" past the string limit. GC point.
#[allow(dead_code)] // callers: P2-BTAB (`abi` descriptive_string), P3-BLTB (String(sym))
pub(crate) fn descriptive_string(cx: &mut Cx, sym: u64) -> Res<u64> {
    let d = description(sym);
    let mut units: Vec<u16> = "Symbol(".encode_utf16().collect();
    if d != UNDEFINED {
        let d = string::flatten(cx, d);
        if string::str_len(d) > MAX_STRING_LENGTH - 8 {
            return Err(throw_error(
                cx,
                ErrorKind::RangeError,
                "Invalid string length",
            ));
        }
        units.extend(string::view(cx, d).to_utf16());
    }
    units.push(u16::from(b')'));
    Ok(string::new_string_utf16(cx.rt(), &units))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_order_and_names() {
        for (i, w) in WellKnownSymbol::ALL.iter().enumerate() {
            assert_eq!(*w as usize, i);
        }
        let mut names: Vec<&str> = WellKnownSymbol::ALL.iter().map(|w| w.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 15);
    }

    #[test]
    fn the_registry_probes_by_hash_and_grows() {
        // Fake atom bits: the registry compares bits only, and this test runs no GC.
        let mut r = Registry::default();
        for i in 1..=100u64 {
            // Every key collides on the low bits of its hash: worst case for the probe chain.
            r.insert((i as u32) << 20, i << 3, i << 4);
        }
        assert_eq!(r.len, 100);
        assert!(r.entries.len() >= 200 && r.entries.len().is_power_of_two());
        for i in 1..=100u64 {
            assert_eq!(r.find((i as u32) << 20, i << 3), Some(i << 4));
        }
        assert_eq!(r.find(5 << 20, 6 << 3), None, "same hash, other key");
        assert_eq!(r.find(1000 << 20, 1000 << 3), None);
    }

    #[test]
    fn identity_hashes_are_30_bits_and_follow_the_seed() {
        let a = SymbolState::default();
        let b = SymbolState::default();
        let ha: Vec<u32> = (0..64).map(|_| a.next_hash()).collect();
        let hb: Vec<u32> = (0..64).map(|_| b.next_hash()).collect();
        assert_eq!(ha, hb, "one seed, one sequence");
        assert!(ha.iter().all(|&h| h <= HASH_MASK));
        let mut d = ha.clone();
        d.sort_unstable();
        d.dedup();
        assert!(d.len() > 60, "a PRNG, not a constant: {ha:?}");
    }
}
