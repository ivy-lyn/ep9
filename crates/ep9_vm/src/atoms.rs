//! Atoms (H19): the per-isolate table of interned strings, and the `atoms!` list of names the
//! VM and the builtin tables use.
//!
//! ```text
//! intern(rt, s)        s is a string cell: an atom -> s; a THIN cell -> its atom; a
//!                      flattened CONS -> intern its flat part, then the CONS becomes THIN;
//!                      else hash it (cached in its hash field), probe the table; found -> that
//!                      atom (a CONS or SLICED s becomes THIN, forwarding to it); else s itself
//!                      becomes the atom (ATOM flag, insert). No allocation, so no GC point.
//! atomize(rt, s)       intern, but a CONS is flattened in place first, and a SLICED s whose
//!                      text has no atom yet gets a SEQ copy as the atom; both become THIN.
//!                      GC point. ToPropertyKey uses it (H19-Q1, H19-Q4).
//! intern_str(rt, text) hash the text, probe; found -> that atom; else allocate the string
//!                      FIRST (GC point, no borrow held), set its hash field and ATOM flag,
//!                      THEN borrow_mut and insert.
//! GC                   the heap's weak source (it holds an Rc clone of the table) drops the
//!                      entries whose cells died, after marking; debug builds then check the
//!                      rest (`AtomTable::check`: live, a string, ATOM set, hash, unique).
//! ```
//!
//! - One table per isolate (`Runtime::atoms`): open addressing, linear probing, power-of-two
//!   capacity, at most half full. An entry is (Value bits, 30-bit hash); the hash comes from
//!   the string's content (string.rs), never from its address (H12).
//! - The table is weak. The `atoms!` list and the single-character table are isolate roots
//!   (runtime.rs), and unit strings are rooted by their unit, so those atoms never die.
//! - GC rule \[I41\] (W4-GC-RULES): never hold a borrow of the table across an allocation.
//! - Debug counter (W4-KEY-EQ): every property-key compare that has to look at content (a key
//!   that is not an atom) bumps [`note_non_atom_compare`]. `EP9_ASSERT_ATOM_KEYS=1` makes a
//!   debug isolate panic at drop if its count is not 0.
//! - Number strings (H19-Q6): a 1,024-entry direct-mapped cache (number bits -> string) next
//!   to the table serves ToString of a number ([`crate::conv::number_string`]) and the key of
//!   a number ([`number_atom`]). The GC weak source empties it at every collection, before the
//!   sweep, in O(1): it bumps the cache epoch, and an entry of an older epoch is empty (its
//!   string bits are never read again). So no entry outlives a GC point that could free its
//!   string (rule 8: no live Value is kept in Rust memory across a collection).

#[cfg(debug_assertions)]
use core::cell::Cell;
use std::rc::Rc;

use crate::frame::UNDEFINED;
use crate::kinds::{self, is_string};
use crate::runtime::{Runtime, val};
use crate::string::{self, JsStr};

macro_rules! atoms {
    ($($name:ident = $text:literal,)*) => {
        /// Names the VM and the builtin tables use. Created once per isolate and kept as
        /// roots. The list is append-only: an atom's id (`Atom as u16`) never changes, so
        /// builtin tables and, later, snapshots and units can name an atom by id (H13).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum Atom { $($name,)* }

        impl Atom {
            /// Every atom, in id order.
            pub const ALL: &'static [Atom] = &[$(Atom::$name,)*];

            pub const fn text(self) -> &'static str {
                match self { $(Atom::$name => $text,)* }
            }

            /// The stable id.
            pub const fn id(self) -> u16 {
                self as u16
            }
        }
    };
}

atoms! {
    Empty = "",
    Length = "length",
    Prototype = "prototype",
    Constructor = "constructor",
    Name = "name",
    Message = "message",
    Stack = "stack",
    ToStringKey = "toString",
    ValueOf = "valueOf",
    Raw = "raw",
    ValueKey = "value",
    Done = "done",
    Undefined = "undefined",
    Null = "null",
    True = "true",
    False = "false",
    Object = "object",
    Boolean = "boolean",
    Number = "number",
    String = "string",
    Function = "function",
    Symbol = "symbol",
    Bigint = "bigint",
    NaN = "NaN",
    Infinity = "Infinity",
    Comma = ",",
    GlobalThis = "globalThis",
    // ---- week 4 (property descriptors, iterator protocol) ----
    Get = "get",
    Set = "set",
    Writable = "writable",
    Enumerable = "enumerable",
    Configurable = "configurable",
    Next = "next",
    Return = "return",
    Throw = "throw",
}

// ---- the table -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct Entry {
    /// Value bits of the atom; 0 = empty slot.
    bits: u64,
    /// `string::hash` of the atom.
    hash: u32,
}

const MIN_CAPACITY: usize = 64;

/// Entries of the number-string cache (a power of two; 24 KiB). V8 starts at 256 and grows
/// to 16 Ki; 1,024 holds a loop over a few hundred keys (strings-m1 `number-keys` thrashes a
/// 64-entry cache: +2.7% instructions; M4-P2 STR2 measure).
const NUMBER_CACHE_SIZE: usize = 1024;

/// One number-string cache entry: the f64 bits of a number and a string of its ToString
/// (an atom once [`number_atom`] stored it), valid only in the cache epoch it was stored in.
#[derive(Clone, Copy, Debug, Default)]
struct NumberEntry {
    number: u64,
    string: u64,
    /// The cache epoch of the store; 0 (never a live epoch): empty.
    epoch: u64,
}

/// The atom table of one isolate (`Runtime::atoms`).
#[derive(Debug)]
pub(crate) struct AtomTable {
    entries: Vec<Entry>,
    len: usize,
    /// The thread's non-atom compare count when the table was made.
    compares_base: u64,
    /// The number-string cache (H19-Q6); emptied at every GC by an epoch bump.
    numbers: Box<[NumberEntry; NUMBER_CACHE_SIZE]>,
    /// The live cache epoch (starts at 1; +1 at every GC; a u64 never wraps).
    numbers_epoch: u64,
}

impl Default for AtomTable {
    fn default() -> Self {
        AtomTable {
            entries: Vec::new(),
            len: 0,
            compares_base: thread_non_atom_compares(),
            numbers: Box::new([NumberEntry::default(); NUMBER_CACHE_SIZE]),
            numbers_epoch: 1,
        }
    }
}

impl AtomTable {
    /// Number of atoms.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Property-key compares on this thread that looked at content since this table was made
    /// (debug builds; 0 in release builds). W4-KEY-EQ wants 0.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub(crate) fn non_atom_compares(&self) -> u64 {
        thread_non_atom_compares() - self.compares_base
    }

    /// Restarts the count (tests that compare non-atom keys on purpose).
    #[cfg(test)]
    pub(crate) fn reset_non_atom_compares(&mut self) {
        self.compares_base = thread_non_atom_compares();
    }

    /// The atom with `hash` for which `eq` holds.
    fn find(&self, hash: u32, mut eq: impl FnMut(u64) -> bool) -> Option<u64> {
        if self.entries.is_empty() {
            return None;
        }
        let mask = self.entries.len() - 1;
        let mut i = hash as usize & mask;
        loop {
            let e = self.entries[i];
            if e.bits == 0 {
                return None;
            }
            if e.hash == hash && eq(e.bits) {
                return Some(e.bits);
            }
            i = (i + 1) & mask;
        }
    }

    /// Adds an atom that is not in the table. Not a GC point (Rust memory only).
    fn insert(&mut self, hash: u32, bits: u64) {
        if (self.len + 1) * 2 > self.entries.len() {
            let cap = (self.entries.len() * 2).max(MIN_CAPACITY);
            self.rebuild(cap);
        }
        place(&mut self.entries, Entry { bits, hash });
        self.len += 1;
    }

    /// Re-places the live entries into a table of `cap` slots.
    fn rebuild(&mut self, cap: usize) {
        let old = core::mem::replace(&mut self.entries, vec![Entry::default(); cap]);
        for e in old.into_iter().filter(|e| e.bits != 0) {
            place(&mut self.entries, e);
        }
    }

    /// Drops the entries whose cells are dead. Returns how many it dropped.
    fn sweep(&mut self, mut is_live: impl FnMut(u64) -> bool) -> usize {
        let mut dead = 0;
        for e in self.entries.iter_mut() {
            if e.bits != 0 && !is_live(e.bits) {
                e.bits = 0;
                dead += 1;
            }
        }
        if dead > 0 {
            self.len -= dead;
            // Removing entries breaks probe chains, so place the rest again (and shrink a
            // mostly empty table).
            let mut cap = self.entries.len();
            while cap > MIN_CAPACITY && self.len * 8 < cap {
                cap /= 2;
            }
            self.rebuild(cap);
        }
        dead
    }

    /// The table invariant: every entry is live, a string with the ATOM flag, stored under
    /// its own hash, found by a probe for its text, and the only atom of its text.
    fn check(&self, mut is_live: impl FnMut(u64) -> bool) -> Result<(), String> {
        let mut n = 0;
        for (i, e) in self.entries.iter().enumerate() {
            if e.bits == 0 {
                continue;
            }
            n += 1;
            let b = e.bits;
            if !is_live(b) {
                return Err(format!("atom table slot {i}: dead cell {b:#x}"));
            }
            if !is_string(b) {
                return Err(format!("atom table slot {i}: {b:#x} is not a string"));
            }
            if !string::is_atom(b) {
                return Err(format!("atom table slot {i}: {b:#x} has no ATOM flag"));
            }
            if string::hash(b) != e.hash {
                return Err(format!(
                    "atom table slot {i}: stored hash is not the string's"
                ));
            }
            match self.find(e.hash, |x| string::equals(x, b)) {
                Some(x) if x == b => {}
                Some(x) => {
                    return Err(format!(
                        "atom table: {b:#x} and {x:#x} have the same text {:?}",
                        string::to_rust_string(b)
                    ));
                }
                None => return Err(format!("atom table slot {i}: not found by its hash")),
            }
        }
        if n != self.len {
            return Err(format!("atom table: {n} entries, len says {}", self.len));
        }
        Ok(())
    }
}

/// Puts `e` in the first free slot of its probe chain.
fn place(entries: &mut [Entry], e: Entry) {
    let mask = entries.len() - 1;
    let mut i = e.hash as usize & mask;
    while entries[i].bits != 0 {
        i = (i + 1) & mask;
    }
    entries[i] = e;
}

// ---- interning -------------------------------------------------------------------------------

/// Registers the table's weak source with the heap. Called from `Runtime::new`.
pub(crate) fn init(rt: &Runtime) {
    let table = Rc::clone(&rt.atoms);
    rt.heap.add_weak_source(move |w| {
        let mut t = table
            .try_borrow_mut()
            .expect("the atom table is borrowed across a GC point (W4-GC-RULES)");
        t.sweep(|b| w.is_live(val(b)));
        // The number cache holds string bits in Rust memory: empty it (H19-Q6), O(1).
        t.numbers_epoch += 1;
        #[cfg(debug_assertions)]
        if let Err(e) = t.check(|b| w.is_live(val(b))) {
            panic!("atoms::verify after GC: {e}");
        }
    });
}

/// The atom for the string `s`: the existing atom of its text, or `s` itself, which becomes
/// the atom (see the module docs for THIN cells). Not a GC point. A CONS or SLICED `s` whose
/// text has no atom yet becomes a rope or slice atom; [`atomize`] makes a flat one instead.
pub(crate) fn intern(rt: &Runtime, s: u64) -> u64 {
    debug_assert!(is_string(s));
    if string::is_atom(s) {
        return s;
    }
    if let Some(a) = string::thin_target(s) {
        return a;
    }
    if let Some(f) = string::flat_first(s) {
        // A flattened CONS: its flat part becomes (or finds) the atom, the CONS forwards.
        let a = intern(rt, f);
        forward(rt, s, a);
        return a;
    }
    let hash = string::hash(s);
    let found = rt.atoms.borrow().find(hash, |a| string::equals(a, s));
    if let Some(a) = found {
        forward(rt, s, a);
        return a;
    }
    string::set_atom(s);
    rt.atoms.borrow_mut().insert(hash, s);
    s
}

/// Makes the CONS or SLICED string `s` a THIN cell forwarding to its atom `a` when `a` is a
/// SEQ atom (H19-Q4); any other pair stays as it is. Not a GC point.
fn forward(rt: &Runtime, s: u64, a: u64) {
    if s != a && string::is_rope_or_slice(s) && string::is_seq(a) {
        string::make_thin(rt, s, a);
    }
}

/// The atom for the string `s`, always a SEQ atom unless one of another kind already holds
/// the text (H19-Q1, V9-D13: flatten on atomize; H19-Q4: thin forwarders). A CONS is
/// flattened in place first (a GC point; the flat hash and compare are much cheaper than a
/// rope walk, M4-P2 STR2 measure), then [`intern`] makes its flat part the atom or finds the
/// atom, and the CONS becomes THIN. A SLICED string probes first (its units are flat); with
/// no atom it gets a SEQ copy of its text as the atom (the atom never keeps a large parent
/// alive) and becomes THIN. GC point. Every other kind: [`intern`].
pub(crate) fn atomize(rt: &Runtime, s: u64) -> u64 {
    debug_assert!(is_string(s));
    if string::is_atom(s) {
        return s;
    }
    match kinds::cell_kind(s) {
        3 => {
            // GC point: `s` is in this frame (conservative scan).
            let _ = string::flatten_rt(rt, s);
            return intern(rt, s);
        }
        4 => {}
        _ => return intern(rt, s),
    }
    let hash = string::hash(s);
    let found = rt.atoms.borrow().find(hash, |a| string::equals(a, s));
    if let Some(a) = found {
        forward(rt, s, a);
        return a;
    }
    // SLICED: the view reads the parent's units, which `s` keeps alive across the allocation
    // in `intern_text` (cells never move; the conservative scan sees `s`).
    let a = string::with_text(s, |t| {
        intern_text(rt, t, |rt| match t {
            JsStr::Latin1(b) => string::new_string_latin1(rt, b),
            JsStr::Utf16(u) => string::new_string_utf16(rt, u),
        })
    });
    forward(rt, s, a);
    a
}

/// The atom for `text`, allocating it if it is new. GC point. Every property key built from
/// Rust text goes through here.
fn intern_text(rt: &Runtime, text: JsStr<'_>, make: impl FnOnce(&Runtime) -> u64) -> u64 {
    let (field, hash) = string::hash_text(text);
    let found = rt
        .atoms
        .borrow()
        .find(hash, |a| string::equals_text(a, text));
    if let Some(a) = found {
        return a;
    }
    // Allocate first: the GC may run and sweep the table, so no borrow is held here.
    let s = make(rt);
    string::set_hash_field(s, field);
    string::set_atom(s);
    rt.atoms.borrow_mut().insert(hash, s);
    s
}

/// The atom for the Rust text `s`. GC point.
pub(crate) fn intern_str(rt: &Runtime, s: &str) -> u64 {
    if s.is_ascii() {
        return intern_latin1(rt, s.as_bytes());
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    intern_utf16(rt, &units)
}

/// The atom for Latin-1 bytes. GC point.
pub(crate) fn intern_latin1(rt: &Runtime, bytes: &[u8]) -> u64 {
    intern_text(rt, JsStr::Latin1(bytes), |rt| {
        string::new_string_latin1(rt, bytes)
    })
}

/// The atom for UTF-16 code units (Latin-1 storage when every unit fits). GC point.
pub(crate) fn intern_utf16(rt: &Runtime, units: &[u16]) -> u64 {
    intern_text(rt, JsStr::Utf16(units), |rt| {
        string::new_string_utf16(rt, units)
    })
}

/// Creates the `atoms!` list as isolate roots. Runs first in `realm::init`.
pub(crate) fn init_static(rt: &Runtime) {
    for &a in Atom::ALL {
        let s = intern_str(rt, a.text());
        rt.set_atom(a, s);
    }
}

/// Checks the atom table (debug builds: at isolate drop and in tests; the GC's weak source
/// runs the same table check after every collection): each entry is a live string with the
/// ATOM flag under its own hash and is the only atom of its text; each `atoms!` root and
/// single-character root is that text's atom. Returns the atom count.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) fn verify(rt: &Runtime) -> Result<usize, String> {
    let t = rt.atoms.borrow();
    t.check(|b| rt.heap.is_cell(crate::object::cell(b).addr()))?;
    let in_table = |b: u64| t.find(string::hash(b), |x| x == b).is_some();
    for &a in Atom::ALL {
        let b = rt.atom(a);
        if b == UNDEFINED {
            continue; // before init_static
        }
        if !in_table(b) || string::to_rust_string(b) != a.text() {
            return Err(format!("atom root {a:?} is not the atom of {:?}", a.text()));
        }
    }
    for c in 0..=u8::MAX {
        let b = rt.single_char(c);
        if b != UNDEFINED && (!in_table(b) || !string::equals_text(b, JsStr::Latin1(&[c]))) {
            return Err(format!(
                "single-character root {c} is not the atom of its text"
            ));
        }
    }
    Ok(t.len())
}

/// The atom of `text` if one exists (no insert). Tests only (lookup without insert for
/// [[Get]] is M1 work).
#[cfg(test)]
pub(crate) fn lookup(rt: &Runtime, text: &str) -> Option<u64> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let t = JsStr::Utf16(&units);
    let (_, hash) = string::hash_text(t);
    rt.atoms.borrow().find(hash, |a| string::equals_text(a, t))
}

// ---- the number-string cache (H19-Q6) ------------------------------------------------------

/// The cache slot of the number `d`: an int32 value picks the slot by its low bits (V8 uses
/// a Smi's value), any other number by its mixed bits. -0 shares the slot of 0, and the bit
/// compare keeps them apart.
#[inline]
fn number_slot(d: f64) -> usize {
    let i = d as i32;
    let h = if f64::from(i) == d {
        i as u32 as u64
    } else {
        let b = d.to_bits();
        b ^ (b >> 32) ^ (b >> 17)
    };
    h as usize & (NUMBER_CACHE_SIZE - 1)
}

/// The cached string of the number `d` (an atom when [`number_atom`] stored it), if any. Not a
/// GC point.
pub(crate) fn cached_number_string(rt: &Runtime, d: f64) -> Option<u64> {
    let t = rt.atoms.borrow();
    let e = t.numbers[number_slot(d)];
    (e.epoch == t.numbers_epoch && e.number == d.to_bits()).then_some(e.string)
}

/// Stores `s`, a string of ToString(`d`), in the number cache (it replaces the slot's entry).
/// Not a GC point; the caller allocated `s` before (no table borrow across an allocation).
pub(crate) fn cache_number_string(rt: &Runtime, d: f64, s: u64) {
    debug_assert!(is_string(s));
    let mut t = rt.atoms.borrow_mut();
    let epoch = t.numbers_epoch;
    t.numbers[number_slot(d)] = NumberEntry {
        number: d.to_bits(),
        string: s,
        epoch,
    };
}

/// The cached atom of ToString(`d`), if the cache has a string for `d`: an atom as it is, any
/// other string interned (a SEQ string becomes the atom in place or finds it), and the slot
/// then holds the atom. Not a GC point.
pub(crate) fn number_atom(rt: &Runtime, d: f64) -> Option<u64> {
    let s = cached_number_string(rt, d)?;
    if string::is_atom(s) {
        return Some(s);
    }
    let a = intern(rt, s);
    cache_number_string(rt, d, a);
    Some(a)
}

// ---- the non-atom compare counter (debug builds) -------------------------------------------

#[cfg(debug_assertions)]
thread_local! {
    static NON_ATOM_COMPARES: Cell<u64> = const { Cell::new(0) };
}

/// Counts one property-key compare that had to look at content (W4-KEY-EQ). An isolate runs
/// on one thread, so a thread counter serves every isolate of that thread.
#[inline]
pub(crate) fn note_non_atom_compare() {
    #[cfg(debug_assertions)]
    NON_ATOM_COMPARES.with(|c| c.set(c.get() + 1));
}

fn thread_non_atom_compares() -> u64 {
    #[cfg(debug_assertions)]
    {
        NON_ATOM_COMPARES.with(Cell::get)
    }
    #[cfg(not(debug_assertions))]
    {
        0
    }
}

/// Debug builds, at isolate drop: [`verify`], then, with `EP9_ASSERT_ATOM_KEYS` set, the
/// W4-KEY-EQ gate (this isolate compared no non-atom key). Skipped while a panic unwinds.
#[cfg(debug_assertions)]
pub(crate) fn check_at_drop(rt: &Runtime) {
    if std::thread::panicking() {
        return;
    }
    if let Err(e) = verify(rt) {
        panic!("atoms::verify at isolate drop: {e}");
    }
    if std::env::var_os("EP9_ASSERT_ATOM_KEYS").is_none() {
        return;
    }
    let n = rt.atoms.borrow().non_atom_compares();
    assert!(
        n == 0,
        "{n} property-key compares looked at content (a key was not an atom; W4-KEY-EQ)"
    );
}
