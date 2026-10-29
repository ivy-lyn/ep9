//! RegExp objects in the VM (owners M4-P2 RX0A, M4-P3 REGEXP1, then M5-P2 REGEXP2; V9-D15,
//! V9-A7): the RegExp object (a SLOT_OBJECT of `ObjectClass::RegExp`), `CreateRegExpLiteral`,
//! the per-isolate program table and program cache, and the Annex B legacy static values.
//!
//! ```text
//! /a+/g  (CreateRegExpLiteral)  -> create_literal: a RegExp object, MATCHER = undefined
//! Cx::regexp_create(proto, p, f) -> create: flags parsed, pattern compiled now, MATCHER = index
//! Cx::regexp_exec(re, s, i)     -> exec: program_index (compiles at the first exec of a
//!                                  literal), flatten s, Engine::exec from i, poll
//!                                  interrupts; the match spans + the group names
//!
//! Runtime.regexp_programs (ProgramTable, Rust memory)        RegExp object (heap)
//!   entry i: owner (weak word) ─────────────────────────────> SLOT_OBJECT, class RegExp
//!            program (Program: regress code, no Value) <───── slot MATCHER: int32 i
//!            names (the group names, UTF-16, no Value)
//!   GC: the weak source frees entry i when its owner died; i goes to the free list
//!   cache:   the last 64 programs by (source units, flag bits), most recently used first
//!            (Rc<Compiled> shared with the entries; no Value, no GC hook)
//!   widened: the UTF-16 copy of the last Latin-1 subject (key = its flat string's bits);
//!            the same weak source empties it in every GC
//!   legacy:  per realm (key: its %RegExp%, weak): RegExp.input and the last match (subject
//!            + spans); a root source traces the strings, the weak source drops a dead realm
//! ```
//!
//! `Engine` is the one name of the backend here (RX0 = `ep9_regexp::Rx0` on regress 0.12.0
//! in month 4; the month-5 fork and RX1/RX2 change only the alias, MC-w14-19). A program is
//! an int32 table index in the object, never a process address.
//!
//! Internal slots of a RegExp object ([`slot`]):
//!
//! | Slot | Spec | Value |
//! |---|---|---|
//! | 0 `SOURCE` | `[[OriginalSource]]` | the pattern text, a string |
//! | 1 `FLAGS` | `[[OriginalFlags]]` | the flags, a string in the order `dgimsuvy` |
//! | 2 `RECORD` | `[[RegExpRecord]]` | the flag bits (int32; `ep9_regexp::Flags`, bits 0-7), and bit 8 `[[LegacyFeaturesEnabled]]` (Annex B, M5-P2) |
//! | 3 `MATCHER` | `[[RegExpMatcher]]` | the int32 index of the program, or undefined before the first exec of a literal |
//! | 4 `REALM` | `[[Realm]]` (Annex B, M5-P2) | the `%RegExp%` of the realm that made the object |
//!
//! Rules:
//! - A compiled program is Rust memory with no Value in it (plan rule 8, V9-A7). It lives in
//!   `Runtime::regexp_programs` ([`ProgramTable`], made in `Runtime::new`); a RegExp object
//!   holds its int32 index, not a native pointer (so H25-Q1/Q2 do not apply).
//! - The table holds its owner weakly, as the atom table does (`atoms.rs`): the owner word
//!   is not a root and the GC never traces it. The table's weak source frees the entry of a
//!   dead owner after marking; no Rust `Drop` runs from the heap (`ep9_heap` lib.rs, H12).
//! - Never hold a borrow of the table across a GC point: the weak source borrows it.
//! - `runtime.rs` has no owner after M4-P0, so the owners change only this file.
//! - A literal compiles at its first exec (the parser already rejected bad literals; a
//!   pattern that the parser takes and regress does not throws its SyntaxError there).
//!   `regexp_create` (the RegExp constructor, M4-P3) compiles at once.
//! - The group names of a program (M4-P3 REGEXP1) come from [`group_names`], a scan of the
//!   pattern after it compiled: the `[[GroupName]]` of each capture, for the `groups` object
//!   of a match result (§22.2.7.2 steps 29-31). They are Rust memory beside the program.
//! - The backend reads UTF-16, so a Latin-1 subject is copied to UTF-16 before a match. A
//!   global match, replace or split runs one exec per match on one subject, so [`exec`] keeps
//!   the copy of the last Latin-1 subject ([`Widened`]) and the next exec on the same flat
//!   string reuses it: one copy per loop, not one per match (M4-P3 P3-ORCH-13). The key is
//!   the flat string's bits. It is safe because the copy never lives across a GC: only a GC
//!   frees a cell, and the table's weak source empties the copy in every GC (minor ones
//!   too), so while the key is set the cell at those bits is the string that was copied, and
//!   a string's text never changes (a SLICED cell that becomes THIN keeps its text).
//! - The program cache (M5-P2, M4-P3-OPEN (4)): a literal makes a new object per evaluation
//!   and each compiles at its first exec; the cache makes a seen (source, flags) pair cost a
//!   lookup, not a compile. Its key is the pattern's code units (a Rust copy, never a string
//!   cell), so it holds no Value (plan rule 8). A strong LRU of [`CACHE_ENTRIES`] programs
//!   (BP R8): memory stays bounded however many patterns a script makes; a pattern longer
//!   than [`CACHE_MAX_UNITS`] is never cached. A SyntaxError is never cached.
//! - Annex B legacy features (the "legacy RegExp features" proposal; M5-P2): RegExpBuiltinExec
//!   records the last match of a RegExp of the current realm ([`note_match`]); the `%RegExp%`
//!   accessors read it ([`legacy_value`]). The state is Rust memory per realm, behind the
//!   table's root source (the strings) and weak source (the realm key).

use core::cell::RefCell;
use core::sync::atomic::Ordering;
use std::rc::Rc;

use ep9_heap::Heap;
use ep9_regexp::{Backend, ExecResult, Flags, Input, Span};
use ep9_value::Throw;

use crate::abi::ObjectClass;
use crate::atoms;
use crate::builtins::{self, IntrinsicId};
use crate::conv::{Res, describe};
use crate::error::{FrameRef, throw_error};
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::interp::{Cx, interrupt, slow};
use crate::kinds;
use crate::object::{self, attr};
use crate::runtime::{ErrorKind, Runtime, val};
use crate::string::{self, JsStr};

/// The RegExp backend of this build: the one place that names it (MC-w14-19).
type Engine = ep9_regexp::Rx0;
/// A compiled program of [`Engine`]: Rust memory, no Value.
type Program = <Engine as Backend>::Program;

/// The `[[GroupName]]` of each capture group of a program, by group number - 1 (`None` for
/// an unnamed group), as UTF-16 code units. Empty when the pattern has no named group
/// (shared: every match record of the program holds a clone of the `Rc`). Rust memory, no
/// Value.
pub(crate) type Names = Rc<[Option<Box<[u16]>>]>;

/// The internal slots of a RegExp object (the table in the module doc).
pub(crate) mod slot {
    /// `[[OriginalSource]]`: the pattern text (a string).
    pub const SOURCE: usize = 0;
    /// `[[OriginalFlags]]`: the flags string, in the order `dgimsuvy`.
    pub const FLAGS: usize = 1;
    /// `[[RegExpRecord]]`: the flag bits (int32).
    pub const RECORD: usize = 2;
    /// `[[RegExpMatcher]]`: the program's table index (int32), or undefined (not compiled).
    pub const MATCHER: usize = 3;
    /// `[[Realm]]` (Annex B legacy features): the `%RegExp%` of the realm that made the
    /// object (undefined in an isolate without the RegExp builtins).
    pub const REALM: usize = 4;
    /// The slot count.
    pub const COUNT: usize = 5;
    /// The `[[RegExpRecord]]` bit of `[[LegacyFeaturesEnabled]]` (above the 8 flag bits of
    /// `ep9_regexp::Flags`, which read only the low byte).
    pub const LEGACY: i32 = 1 << 8;
}

// ---- the program table ------------------------------------------------------------------------

/// A compiled program and its group names ([`group_names`]): Rust memory, no Value. Shared
/// (`Rc`) by every table entry and cache entry of the same (source, flags) (M5-P2 REGEXP2).
pub(crate) struct Compiled {
    program: Program,
    names: Names,
}

/// One table entry: a program and its owner, or a free entry (`owner` 0, no program).
struct Entry {
    /// The RegExp object's bits, held weakly (the weak source clears it); 0 = free.
    owner: u64,
    compiled: Option<Rc<Compiled>>,
}

/// The entries, and the free indices to reuse.
#[derive(Default)]
struct Programs {
    entries: Vec<Entry>,
    free: Vec<u32>,
}

impl Programs {
    /// Frees the entries whose owner is dead. Returns how many it freed.
    fn sweep(&mut self, mut is_live: impl FnMut(u64) -> bool) -> usize {
        let mut freed = 0;
        for (i, e) in self.entries.iter_mut().enumerate() {
            if e.owner != 0 && !is_live(e.owner) {
                e.owner = 0;
                e.compiled = None;
                self.free.push(i as u32);
                freed += 1;
            }
        }
        freed
    }
}

/// The most programs the cache keeps ([`ProgramCache`]; BP R8: a small strong LRU, est.).
const CACHE_ENTRIES: usize = 64;

/// The longest pattern (code units) the cache keeps; a longer one compiles each time.
const CACHE_MAX_UNITS: usize = 4096;

/// One cache entry: the key (source units, flag bits; `hash` of the units first) and the
/// program.
struct CacheEntry {
    hash: u64,
    flags: u8,
    source: Box<[u16]>,
    compiled: Rc<Compiled>,
}

/// The per-isolate program cache (M5-P2 REGEXP2; M4-P3-OPEN (4)): the last
/// [`CACHE_ENTRIES`] programs by (source units, flags), so a RegExp literal in a loop (a new
/// object per evaluation, §13.2.7.3) and `new RegExp(p, f)` with a seen pattern compile once.
/// Rust memory with no Value in it (plan rule 8): the key is the pattern text, never a string
/// cell. Most recently used first: a hit moves one place forward, a miss goes in front and the
/// last entry drops when the cache is full. A program lives while a cache entry or a table
/// entry holds it.
#[derive(Default)]
struct ProgramCache {
    entries: Vec<CacheEntry>,
    /// Programs compiled so far (tests: the hit and miss counts).
    #[cfg(test)]
    compiles: usize,
}

/// FNV-1a over the code units of `t`.
fn hash_units(t: JsStr<'_>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for i in 0..t.len() {
        h = (h ^ u64::from(t.at(i))).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Whether the text `t` equals the code units `u`.
fn same_units(t: JsStr<'_>, u: &[u16]) -> bool {
    match t {
        JsStr::Latin1(b) => b.len() == u.len() && b.iter().zip(u).all(|(&a, &b)| u16::from(a) == b),
        JsStr::Utf16(w) => w == u,
    }
}

impl ProgramCache {
    /// The program of (`t`, `flags`) if the cache holds it (it moves one place forward).
    fn find(&mut self, t: JsStr<'_>, hash: u64, flags: u8) -> Option<Rc<Compiled>> {
        let i = self
            .entries
            .iter()
            .position(|e| e.hash == hash && e.flags == flags && same_units(t, &e.source))?;
        if i > 0 {
            self.entries.swap(i - 1, i);
            return Some(Rc::clone(&self.entries[i - 1].compiled));
        }
        Some(Rc::clone(&self.entries[0].compiled))
    }

    /// Puts the program of (`source`, `flags`) in front.
    fn insert(&mut self, hash: u64, flags: u8, source: &[u16], compiled: &Rc<Compiled>) {
        if source.len() > CACHE_MAX_UNITS {
            return;
        }
        if self.entries.len() == CACHE_ENTRIES {
            self.entries.pop();
        }
        let e = CacheEntry {
            hash,
            flags,
            source: source.into(),
            compiled: Rc::clone(compiled),
        };
        self.entries.insert(0, e);
    }
}

/// The UTF-16 copy of one Latin-1 subject (module doc): the key and the code units. Rust
/// memory, no Value; the key is a plain word, never traced, valid only until the next GC.
#[derive(Default)]
struct Widened {
    /// The bits of the flat string that `units` copies; 0 = empty.
    key: u64,
    units: Vec<u16>,
    /// The number of copies made (tests: one per subject between two GCs).
    #[cfg(test)]
    made: usize,
}

impl Widened {
    /// The UTF-16 code units of the flat Latin-1 string `flat`, whose bytes are `b`: the kept
    /// copy if it is of `flat`, else a new copy (it replaces the old one). Not a GC point.
    fn units_of(&mut self, flat: u64, b: &[u8]) -> &[u16] {
        if self.key != flat {
            self.units.clear();
            self.units.extend(b.iter().map(|&c| u16::from(c)));
            self.key = flat;
            #[cfg(test)]
            {
                self.made += 1;
            }
        }
        debug_assert_eq!(self.units.len(), b.len());
        &self.units
    }

    /// Drops the copy and its memory (the weak source, in every GC).
    fn clear(&mut self) {
        self.key = 0;
        self.units = Vec::new();
    }
}

/// The legacy static values of one realm's `%RegExp%` (Annex B proposal "legacy RegExp
/// features": `[[RegExpInput]]`, `[[RegExpLastMatch]]`, `[[RegExpLastParen]]`,
/// `[[RegExpLeftContext]]`, `[[RegExpRightContext]]`, `[[RegExpParen1]]` ...
/// `[[RegExpParen9]]`). The last match is kept as its subject and spans; the getters cut the
/// strings. The strings are held by the table's root source (strong), never across a GC in
/// plain Rust memory. The realm's `%RegExp%` is held weakly: the weak source drops the state
/// of a realm whose `%RegExp%` died, so the state never keeps a realm alive.
struct LegacyRealm {
    /// The realm's `%RegExp%` (a weak word: not traced; the key).
    ctor: u64,
    /// `[[RegExpInput]]`.
    input: LegacyValue,
    /// The other slots: the subject and the spans of the last match.
    matched: LegacyValue,
    /// The spans of the last match (`caps[0]` = the whole match).
    caps: Vec<Option<Span>>,
}

/// A legacy slot: the initial empty String, the spec's `empty` (the getter throws), or a
/// string (the input, or the subject of the last match).
#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyValue {
    Initial,
    Empty,
    Str(u64),
}

/// The legacy state of every realm that has one ([`LegacyRealm`]).
#[derive(Default)]
struct Legacy {
    realms: Vec<LegacyRealm>,
}

impl Legacy {
    /// The state of the realm whose `%RegExp%` is `ctor` (a new one in its initial state).
    fn realm(&mut self, ctor: u64) -> &mut LegacyRealm {
        let i = match self.realms.iter().position(|r| r.ctor == ctor) {
            Some(i) => i,
            None => {
                self.realms.push(LegacyRealm {
                    ctor,
                    input: LegacyValue::Initial,
                    matched: LegacyValue::Initial,
                    caps: Vec::new(),
                });
                self.realms.len() - 1
            }
        };
        &mut self.realms[i]
    }

    /// The root source: every string the state holds (not the `%RegExp%` keys).
    fn trace(&self, t: &mut ep9_heap::Tracer<'_>) {
        for r in &self.realms {
            for v in [r.input, r.matched] {
                if let LegacyValue::Str(s) = v {
                    t.visit_bits(s);
                }
            }
        }
    }
}

/// The compiled programs of the isolate, indexed by the int32 a RegExp object holds; the
/// program cache; the Latin-1 copy; the legacy static values.
pub(crate) struct ProgramTable {
    inner: Rc<RefCell<Programs>>,
    /// The program cache ([`ProgramCache`]; no Value, no GC hook).
    cache: RefCell<ProgramCache>,
    /// The UTF-16 copy of the last Latin-1 subject ([`Widened`]).
    widened: Rc<RefCell<Widened>>,
    /// The legacy static values ([`Legacy`]), behind a root source.
    legacy: Rc<RefCell<Legacy>>,
}

impl ProgramTable {
    /// The empty table of a new isolate. `heap` is the isolate's heap, before its first
    /// allocation: the table's weak source (it holds an `Rc` clone of the entries) frees the
    /// entries whose owner died, after marking, empties the Latin-1 copy and drops the legacy
    /// state of a dead realm; its root source traces the strings of the legacy static values.
    pub(crate) fn new(heap: &Heap) -> ProgramTable {
        let inner = Rc::new(RefCell::new(Programs::default()));
        let widened = Rc::new(RefCell::new(Widened::default()));
        let legacy = Rc::new(RefCell::new(Legacy::default()));
        let (table, copy) = (Rc::clone(&inner), Rc::clone(&widened));
        let realms = Rc::clone(&legacy);
        heap.add_weak_source(move |w| {
            let mut t = table
                .try_borrow_mut()
                .expect("the RegExp program table is borrowed across a GC point");
            t.sweep(|b| w.is_live(val(b)));
            copy.try_borrow_mut()
                .expect("the RegExp subject copy is borrowed across a GC point")
                .clear();
            realms
                .try_borrow_mut()
                .expect("the RegExp legacy state is borrowed across a GC point")
                .realms
                .retain(|r| w.is_live(val(r.ctor)));
        });
        let roots = Rc::clone(&legacy);
        heap.add_root_source(move |t| {
            roots
                .try_borrow()
                .expect("the RegExp legacy state is borrowed across a GC point")
                .trace(t);
        });
        ProgramTable {
            inner,
            cache: RefCell::new(ProgramCache::default()),
            widened,
            legacy,
        }
    }

    /// Stores `compiled` for the RegExp object `owner`; returns its index. Not a GC point.
    fn insert(&self, owner: u64, compiled: Rc<Compiled>) -> i32 {
        let mut t = self.inner.borrow_mut();
        let e = Entry {
            owner,
            compiled: Some(compiled),
        };
        let i = match t.free.pop() {
            Some(i) => {
                t.entries[i as usize] = e;
                i as usize
            }
            None => {
                t.entries.push(e);
                t.entries.len() - 1
            }
        };
        i32::try_from(i).expect("fewer than 2^31 RegExp programs")
    }

    /// Replaces the program of entry `index` (RegExp.prototype.compile). Not a GC point.
    fn replace(&self, index: i32, compiled: Rc<Compiled>) {
        let mut t = self.inner.borrow_mut();
        let e = &mut t.entries[index as usize];
        debug_assert!(e.owner != 0, "a live RegExp holds a live program index");
        e.compiled = Some(compiled);
    }

    /// Runs program `index` (not a GC point: regress allocates only Rust memory).
    fn exec(
        &self,
        index: i32,
        input: Input<'_>,
        start: usize,
        caps: &mut Vec<Option<Span>>,
        poll: &mut dyn FnMut() -> bool,
    ) -> ExecResult {
        let t = self.inner.borrow();
        let c = t.entries[index as usize]
            .compiled
            .as_ref()
            .expect("a live RegExp holds a live program index");
        Engine::exec(&c.program, input, start, caps, poll)
    }

    /// The group names of program `index` (a clone of the shared `Rc`).
    fn names(&self, index: i32) -> Names {
        let t = self.inner.borrow();
        let c = t.entries[index as usize]
            .compiled
            .as_ref()
            .expect("a live RegExp holds a live program index");
        Rc::clone(&c.names)
    }

    /// The number of entries that hold a program (tests).
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        let t = self.inner.borrow();
        t.entries.iter().filter(|e| e.compiled.is_some()).count()
    }

    /// The number of programs compiled so far (tests: a cache hit compiles none).
    #[cfg(test)]
    pub(crate) fn compiles(&self) -> usize {
        self.cache.borrow().compiles
    }

    /// The number of programs the cache holds (tests).
    #[cfg(test)]
    pub(crate) fn cached(&self) -> usize {
        self.cache.borrow().entries.len()
    }

    /// The number of realms with legacy static state (tests).
    #[cfg(test)]
    pub(crate) fn legacy_realms(&self) -> usize {
        self.legacy.borrow().realms.len()
    }

    /// Sets `[[RegExpInput]]` of the realm whose `%RegExp%` is `ctor` to the string `s`
    /// (tests: an isolate with no RegExp builtins keys a realm by any object of it).
    #[cfg(test)]
    pub(crate) fn set_legacy_input_of(&self, ctor: u64, s: u64) {
        self.legacy.borrow_mut().realm(ctor).input = LegacyValue::Str(s);
    }

    /// The number of Latin-1 subjects copied to UTF-16 so far (tests).
    #[cfg(test)]
    pub(crate) fn widenings(&self) -> usize {
        self.widened.borrow().made
    }

    /// The table index of `re`'s program if it has one (tests).
    #[cfg(test)]
    pub(crate) fn index_of(re: u64) -> Option<i32> {
        let m = object::slot_get(re, slot::MATCHER);
        (m != UNDEFINED).then(|| int32_of(m))
    }
}

// ---- RegExp objects ---------------------------------------------------------------------------

/// True if `v` is a RegExp object (it has a `[[RegExpMatcher]]` slot). Any Value.
pub(crate) fn is_regexp(v: u64) -> bool {
    object::slot_class(v) == Some(ObjectClass::RegExp.id())
}

/// RegExpAlloc + the slot stores of RegExpInitialize (§22.2.3.1, §22.2.3.3): a RegExp object
/// with prototype `proto`, pattern `source` (a string), `flags` and `matcher` (an int32 index
/// or undefined), and an own `lastIndex` = 0 ({w}, not enumerable, not configurable). Its
/// `[[Realm]]` is the current realm and `[[LegacyFeaturesEnabled]]` is true (the RegExp
/// constructor clears it for a NewTarget that is not `%RegExp%`). GC point; `proto` and
/// `source` stay in Rust locals (conservative scan).
fn new_regexp(rt: &Runtime, proto: u64, source: u64, flags: Flags, matcher: u64) -> u64 {
    let flags_text = object::new_string(rt, &flags.to_letters());
    let o = object::new_slot_object(rt, proto, ObjectClass::RegExp.id(), slot::COUNT);
    object::slot_set(rt, o, slot::SOURCE, source);
    object::slot_set(rt, o, slot::FLAGS, flags_text);
    let record = i32::from(flags.bits()) | slot::LEGACY;
    object::slot_set(rt, o, slot::RECORD, int32_bits(record));
    object::slot_set(rt, o, slot::MATCHER, matcher);
    let realm = builtins::intrinsic(rt, IntrinsicId::RegExp);
    object::slot_set(rt, o, slot::REALM, realm);
    let key = atoms::intern_str(rt, "lastIndex");
    object::own_define(rt, o, key, int32_bits(0), attr::WRITABLE);
    o
}

/// The prototype of a RegExp literal: the current realm's %RegExp.prototype%, or
/// %Object.prototype% in an isolate whose builtin table does not make it (ep9_vm tests).
fn literal_proto(rt: &Runtime) -> u64 {
    let p = builtins::intrinsic(rt, IntrinsicId::RegExpPrototype);
    if kinds::is_object(p) {
        p
    } else {
        builtins::intrinsic(rt, IntrinsicId::ObjectPrototype)
    }
}

/// The UTF-16 code units of the string `s` (any string kind; not a GC point).
fn units_of(s: u64) -> Vec<u16> {
    string::with_text(s, |t| t.to_utf16())
}

/// `CreateRegExpLiteral` (the cold body; pattern constant `pattern`, Literal slot `slot`,
/// `flags` the RegExp flag bits) at `pc` in frame `fp`: a new RegExp object whose program
/// compiles at its first exec (from the program cache when the pattern was seen). The
/// Literal slot records nothing yet (no boilerplate: every evaluation makes a new object,
/// §13.2.7.3). Returns 0 with an exception pending only if the flags operand is not a flag
/// set.
#[cold]
#[inline(never)]
pub(crate) fn create_literal(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    pattern: u32,
    _slot: u32,
    flags: u16,
) -> u64 {
    cx.rt().enter(fp, pc);
    let Ok(bits) = u8::try_from(flags) else {
        throw_error(
            cx,
            ErrorKind::SyntaxError,
            "Invalid regular expression flags",
        );
        return 0;
    };
    let rt = cx.rt();
    let source = FrameRef(fp).info().constant(pattern);
    debug_assert!(kinds::is_string(source), "the pattern constant is a string");
    new_regexp(
        rt,
        literal_proto(rt),
        source,
        Flags::from_bits(bits),
        UNDEFINED,
    )
}

/// The SyntaxError of a pattern or flags that do not compile.
fn syntax_error(cx: &mut Cx, message: &str) -> Throw {
    throw_error(cx, ErrorKind::SyntaxError, message)
}

/// The flags of the string `flags` (§22.2.3.3 steps 5-6), else the SyntaxError.
fn parse_flags(cx: &mut Cx, flags: u64) -> Res<Flags> {
    let flag_units = units_of(flags);
    match Flags::parse(&flag_units) {
        Some(f) => Ok(f),
        None => {
            let msg = format!(
                "Invalid flags supplied to RegExp constructor '{}'",
                String::from_utf16_lossy(&flag_units)
            );
            Err(syntax_error(cx, &msg))
        }
    }
}

/// RegExpAlloc(proto) then RegExpInitialize(obj, pattern, flags) (§22.2.3.1, §22.2.3.3) for
/// the strings `pattern` and `flags` (the caller ran ToString): a SyntaxError for a flag that
/// is not one of `dgimsuvy`, a flag twice, `u` with `v`, or a pattern that does not compile.
/// The program comes from the cache or compiles now. GC point.
pub(crate) fn create(cx: &mut Cx, proto: u64, pattern: u64, flags: u64) -> Res<u64> {
    let f = parse_flags(cx, flags)?;
    let compiled = compiled(cx, pattern, f)?;
    let rt = cx.rt();
    // Not compiled into the table yet: the object must exist to own the entry.
    let o = new_regexp(rt, proto, pattern, f, UNDEFINED);
    let i = rt.regexp_programs.insert(o, compiled);
    object::slot_set(rt, o, slot::MATCHER, int32_bits(i));
    Ok(o)
}

/// The slot steps of RegExpInitialize (§22.2.3.3 steps 5-15) on the existing RegExp object
/// `re` (Annex B RegExp.prototype.compile, B.2.4.1) for the strings `pattern` and `flags`:
/// the flags and the program are checked first (a SyntaxError changes nothing), then
/// `[[OriginalSource]]`, `[[OriginalFlags]]`, `[[RegExpRecord]]` (the legacy bit kept) and
/// `[[RegExpMatcher]]` change. No `lastIndex` step (the caller's Set). A TypeError if `re`
/// is not a RegExp object. GC point.
pub(crate) fn initialize(cx: &mut Cx, re: u64, pattern: u64, flags: u64) -> Res<()> {
    if !is_regexp(re) {
        let msg = format!("{} is not a RegExp object", describe(re));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let f = parse_flags(cx, flags)?;
    let compiled = compiled(cx, pattern, f)?;
    let rt = cx.rt();
    // GC point: `re` and `pattern` are in this frame.
    let flags_text = object::new_string(rt, &f.to_letters());
    let legacy = int32_of(object::slot_get(re, slot::RECORD)) & slot::LEGACY;
    object::slot_set(rt, re, slot::SOURCE, pattern);
    object::slot_set(rt, re, slot::FLAGS, flags_text);
    object::slot_set(
        rt,
        re,
        slot::RECORD,
        int32_bits(i32::from(f.bits()) | legacy),
    );
    let m = object::slot_get(re, slot::MATCHER);
    if m == UNDEFINED {
        let i = rt.regexp_programs.insert(re, compiled);
        object::slot_set(rt, re, slot::MATCHER, int32_bits(i));
    } else {
        rt.regexp_programs.replace(int32_of(m), compiled);
    }
    core::hint::black_box(&re);
    Ok(())
}

/// The program index of the RegExp object `re`, compiling it at the first exec of a literal
/// (from the program cache when the pattern was seen). GC point only through the SyntaxError.
fn program_index(cx: &mut Cx, re: u64) -> Res<i32> {
    let m = object::slot_get(re, slot::MATCHER);
    if m != UNDEFINED {
        return Ok(int32_of(m));
    }
    let source = object::slot_get(re, slot::SOURCE);
    let flags = Flags::from_bits(int32_of(object::slot_get(re, slot::RECORD)) as u8);
    let p = compiled(cx, source, flags)?;
    let rt = cx.rt();
    let i = rt.regexp_programs.insert(re, p);
    object::slot_set(rt, re, slot::MATCHER, int32_bits(i));
    Ok(i)
}

/// The program of the pattern string `source` with `flags`: from the program cache, else
/// compiled now (and cached). A SyntaxError if it does not compile (never cached). GC point
/// only through the error.
fn compiled(cx: &mut Cx, source: u64, flags: Flags) -> Res<Rc<Compiled>> {
    let rt = cx.rt();
    let table = &rt.regexp_programs;
    let (hit, hash) = string::with_text(source, |t| {
        let hash = hash_units(t);
        (table.cache.borrow_mut().find(t, hash, flags.bits()), hash)
    });
    if let Some(c) = hit {
        return Ok(c);
    }
    let units = units_of(source);
    let c = match Engine::compile(&units, flags) {
        Ok(program) => Rc::new(Compiled {
            program,
            names: group_names(&units, flags),
        }),
        Err(e) => return Err(syntax_error(cx, &e.message)),
    };
    let mut cache = table.cache.borrow_mut();
    #[cfg(test)]
    {
        cache.compiles += 1;
    }
    cache.insert(hash, flags.bits(), &units, &c);
    Ok(c)
}

/// The `[[GroupName]]` of each capture group of `pattern` (§22.2.1.1 CapturingGroupName; the
/// groups are numbered by their left parentheses, §22.2.2.1), for a pattern that compiled.
/// A scan, not a parser: it skips an escape (a backslash and the next code unit) and a
/// character class (`[` ... `]`, nested classes with `v`), counts each `(` not followed by
/// `?`, and reads the name of `(?<name>` (not `(?<=`, `(?<!`). A name's `\uXXXX` and
/// `\u{...}` escapes are decoded to UTF-16. Empty if the pattern has no named group.
pub(crate) fn group_names(pattern: &[u16], flags: Flags) -> Names {
    const BACKSLASH: u16 = b'\\' as u16;
    let nested = flags.contains(Flags::UNICODE_SETS);
    let unit = |i: usize| pattern.get(i).copied();
    let (mut names, mut any) = (Vec::new(), false);
    let (mut i, mut class_depth) = (0, 0usize);
    while i < pattern.len() {
        let c = pattern[i];
        if c == BACKSLASH {
            i += 2;
            continue;
        }
        if class_depth > 0 {
            if c == u16::from(b']') {
                class_depth -= 1;
            } else if c == u16::from(b'[') && nested {
                class_depth += 1;
            }
            i += 1;
            continue;
        }
        if c == u16::from(b'[') {
            class_depth = 1;
        } else if c == u16::from(b'(') {
            if unit(i + 1) != Some(u16::from(b'?')) {
                names.push(None);
            } else if unit(i + 2) == Some(u16::from(b'<'))
                && !matches!(unit(i + 3), Some(0x3D | 0x21))
            {
                // `(?<name>`: the name ends at the first `>` (an escape in a name has none).
                let start = i + 3;
                let end = pattern[start..]
                    .iter()
                    .position(|&u| u == u16::from(b'>'))
                    .map_or(pattern.len(), |n| start + n);
                names.push(Some(decode_name(&pattern[start..end])));
                any = true;
                i = end;
            }
        }
        i += 1;
    }
    if any { Rc::from(names) } else { Rc::from([]) }
}

/// A group name as UTF-16: `\uXXXX` and `\u{X...}` decoded, every other code unit kept.
fn decode_name(raw: &[u16]) -> Box<[u16]> {
    let hex = |s: &[u16]| -> Option<u32> {
        let t = String::from_utf16(s).ok()?;
        u32::from_str_radix(&t, 16).ok()
    };
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == u16::from(b'\\') && raw.get(i + 1) == Some(&u16::from(b'u')) {
            if raw.get(i + 2) == Some(&u16::from(b'{')) {
                if let Some(close) = raw[i + 3..].iter().position(|&u| u == u16::from(b'}'))
                    && let Some(c) = hex(&raw[i + 3..i + 3 + close]).and_then(char::from_u32)
                {
                    let mut buf = [0u16; 2];
                    out.extend_from_slice(c.encode_utf16(&mut buf));
                    i += 4 + close;
                    continue;
                }
            } else if let Some(u) = raw.get(i + 2..i + 6).and_then(hex) {
                out.push(u as u16);
                i += 6;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    out.into_boxed_slice()
}

/// The matcher of RegExpBuiltinExec (§22.2.7.2) without the `lastIndex` steps: the first
/// match of the RegExp object `re` in the string `s` at or after code unit `index` (a sticky
/// RegExp: only at `index`), as capture spans in code units; `None` for no match (also when
/// `index` > the length). A TypeError if `re` is not a RegExp object; a SyntaxError if a
/// literal's pattern does not compile. Interrupts: the backend polls the interrupt word, and
/// a pending request runs `slow::interrupt` (callbacks, termination) before a retry. With a
/// match: the program's group names too. GC point (the flat string, an error).
pub(crate) fn exec(
    cx: &mut Cx,
    re: u64,
    s: u64,
    index: usize,
) -> Res<Option<(Vec<Option<Span>>, Names)>> {
    if !is_regexp(re) {
        let msg = format!("{} is not a RegExp object", describe(re));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let program = program_index(cx, re)?;
    // GC points below: the flatten, an interrupt callback, an error. The table holds `re`
    // only weakly, so `re` must stay in a Rust local (conservative scan) until the program
    // has run: the `black_box` after the loop keeps it live, or an optimized build may drop
    // it after `program_index` and the GC frees the program (Codex M4-P2 R1B). `s` stays
    // live until the flatten, then `flat` does.
    let flat = string::flatten(cx, s);
    let word = cx.interrupt;
    let mut caps = Vec::new();
    let out = loop {
        let rt = cx.rt();
        let r = {
            // Not a GC point from here to the end of the block (the weak source borrows it).
            let mut widened = rt.regexp_programs.widened.borrow_mut();
            let input = match string::view(cx, flat) {
                JsStr::Latin1(b) => Input::Utf16(widened.units_of(flat, b)),
                JsStr::Utf16(u) => Input::Utf16(u),
            };
            let mut poll = || {
                // SAFETY: `cx.interrupt` points at the runtime's interrupt word, which lives
                // as long as the Runtime (an Arc it holds; `slow::interrupt`).
                let bits = unsafe { (*word).load(Ordering::Relaxed) };
                bits & (interrupt::TERMINATE | interrupt::REQUEST) != 0 || rt.terminating.get()
            };
            rt.regexp_programs
                .exec(program, input, index, &mut caps, &mut poll)
        };
        match r {
            ExecResult::Match => break Ok(Some((caps, rt.regexp_programs.names(program)))),
            ExecResult::NoMatch => break Ok(None),
            ExecResult::Interrupted => {
                if slow::interrupt(cx, core::ptr::null_mut()) == 0 {
                    break Err(Throw);
                }
            }
            ExecResult::BudgetExceeded => {
                let msg = "Maximum regular expression backtrack budget exceeded";
                break Err(throw_error(cx, ErrorKind::RangeError, msg));
            }
        }
    };
    core::hint::black_box(&re);
    out
}

// ---- Annex B legacy features --------------------------------------------------------------------

/// The current realm's `%RegExp%` if `re` was made in the current realm (its `[[Realm]]`),
/// else `None` (also in an isolate without the RegExp builtins).
fn same_realm_ctor(rt: &Runtime, re: u64) -> Option<u64> {
    let ctor = builtins::intrinsic(rt, IntrinsicId::RegExp);
    (kinds::is_object(ctor) && object::slot_get(re, slot::REALM) == ctor).then_some(ctor)
}

/// Whether `re` (a RegExp object) has `[[LegacyFeaturesEnabled]]` true and was made in the
/// current realm: the checks of RegExp.prototype.compile (legacy proposal, B.2.4.1 steps 3-6).
pub(crate) fn legacy_features(rt: &Runtime, re: u64) -> bool {
    is_regexp(re)
        && int32_of(object::slot_get(re, slot::RECORD)) & slot::LEGACY != 0
        && same_realm_ctor(rt, re).is_some()
}

/// Sets `[[LegacyFeaturesEnabled]]` of the RegExp object `re` to false (RegExpAlloc for a
/// NewTarget that is not the current realm's `%RegExp%`). Not a GC point.
pub(crate) fn disable_legacy_features(rt: &Runtime, re: u64) {
    if is_regexp(re) {
        let record = int32_of(object::slot_get(re, slot::RECORD)) & !slot::LEGACY;
        object::slot_set(rt, re, slot::RECORD, int32_bits(record));
    }
}

/// The legacy step of RegExpBuiltinExec after a match of the RegExp object `re` in the
/// string `s` with spans `caps` (legacy proposal): in `re`'s realm, with
/// `[[LegacyFeaturesEnabled]]` true, UpdateLegacyRegExpStaticProperties(%RegExp%, S,
/// lastIndex, e, capturedValues); with it false, InvalidateLegacyRegExpStaticProperties
/// (%RegExp%); in another realm nothing. Not a GC point (Rust memory only).
pub(crate) fn note_match(rt: &Runtime, re: u64, s: u64, caps: &[Option<Span>]) {
    if !is_regexp(re) {
        return;
    }
    let Some(ctor) = same_realm_ctor(rt, re) else {
        return;
    };
    let enabled = int32_of(object::slot_get(re, slot::RECORD)) & slot::LEGACY != 0;
    let mut legacy = rt.regexp_programs.legacy.borrow_mut();
    let r = legacy.realm(ctor);
    if enabled {
        r.input = LegacyValue::Str(s);
        r.matched = LegacyValue::Str(s);
        r.caps.clear();
        r.caps.extend_from_slice(caps);
    } else {
        r.input = LegacyValue::Empty;
        r.matched = LegacyValue::Empty;
        r.caps.clear();
    }
}

/// The legacy static values (the getters of `RegExp.input` ... `RegExp.$9`), by index:
/// 0 `[[RegExpInput]]`, 1 `[[RegExpLastMatch]]`, 2 `[[RegExpLastParen]]`, 3
/// `[[RegExpLeftContext]]`, 4 `[[RegExpRightContext]]`, 5 + n - 1 `[[RegExpParen<n>]]`
/// (n = 1 ... 9).
pub(crate) const LEGACY_SLOTS: u32 = 14;

/// The value of legacy slot `which` ([`LEGACY_SLOTS`]) of the current realm's `%RegExp%`:
/// `None` for the spec's `empty` (the getter throws its TypeError), else the string (the
/// empty String before any match). An undefined capture reads as the empty String (V8).
/// GC point (the new string).
pub(crate) fn legacy_value(rt: &Runtime, which: u32) -> Option<u64> {
    assert!(which < LEGACY_SLOTS, "a legacy slot index");
    let ctor = builtins::intrinsic(rt, IntrinsicId::RegExp);
    let empty = || rt.atom(atoms::Atom::Empty);
    let (input, matched, caps) = {
        let mut legacy = rt.regexp_programs.legacy.borrow_mut();
        let r = legacy.realm(ctor);
        (r.input, r.matched, r.caps.clone())
    };
    // From here GC points: the strings are held by the root source and by these locals.
    if which == 0 {
        return match input {
            LegacyValue::Initial => Some(empty()),
            LegacyValue::Empty => None,
            LegacyValue::Str(s) => Some(s),
        };
    }
    let s = match matched {
        LegacyValue::Initial => return Some(empty()),
        LegacyValue::Empty => return None,
        LegacyValue::Str(s) => s,
    };
    let len = string::str_len(s);
    let (start, end) = caps.first().copied().flatten().unwrap_or((0, 0));
    let span = match which {
        1 => Some((start, end)),
        2 => {
            if caps.len() > 1 {
                caps[caps.len() - 1]
            } else {
                None
            }
        }
        3 => Some((0, start)),
        4 => Some((end, len)),
        n => caps.get((n - 4) as usize).copied().flatten(),
    };
    let out = match span {
        Some((from, to)) => cut(rt, s, from, to),
        None => empty(),
    };
    core::hint::black_box(&s);
    Some(out)
}

/// SetLegacyRegExpStaticProperty(%RegExp%, ..., `[[RegExpInput]]`, s) after the caller's
/// checks and ToString: `[[RegExpInput]]` of the current realm's `%RegExp%` = the string `s`.
/// Not a GC point.
pub(crate) fn set_legacy_input(rt: &Runtime, s: u64) {
    debug_assert!(kinds::is_string(s));
    let ctor = builtins::intrinsic(rt, IntrinsicId::RegExp);
    rt.regexp_programs.legacy.borrow_mut().realm(ctor).input = LegacyValue::Str(s);
}

/// The code units `from..to` of the string `s` as a new string (a copy). GC point.
fn cut(rt: &Runtime, s: u64, from: usize, to: usize) -> u64 {
    if from == 0 && to == string::str_len(s) {
        return s;
    }
    // Copied out first: the view of a flat cell must not live across the allocation.
    let units: Vec<u16> = string::with_text(s, |t| (from..to).map(|i| t.at(i)).collect());
    string::new_string_utf16(rt, &units)
}

#[cfg(test)]
mod group_name_tests {
    use super::{Flags, group_names};

    /// The names of `pattern` as strings ("-" for an unnamed group).
    fn names(pattern: &str, flags: &str) -> Vec<String> {
        let p: Vec<u16> = pattern.encode_utf16().collect();
        let f: Vec<u16> = flags.encode_utf16().collect();
        let n = group_names(&p, Flags::parse(&f).unwrap());
        n.iter()
            .map(|x| x.as_deref().map_or("-".into(), String::from_utf16_lossy))
            .collect()
    }

    #[test]
    fn groups_are_counted_by_left_parenthesis() {
        let p = r"(a)(?<x>b)(?:c)(?=d)(?<=e)(?<!f)(?<y>g)(?i:h)";
        assert_eq!(names(p, ""), ["-", "x", "y"]);
        // No named group: no names at all (the record has no `groups` object).
        assert!(names(r"(a)(b)", "").is_empty());
    }

    #[test]
    fn escapes_and_classes_hide_parentheses() {
        assert_eq!(names(r"\((?<w>a)[(](?<z>b)", ""), ["w", "z"]);
        assert_eq!(names(r"[\]((](?<q>x)", ""), ["q"]);
        // With v a class nests: the "(" inside the inner class is no group.
        assert_eq!(names(r"[[(]--[a]](?<v>x)", "v"), ["v"]);
    }

    #[test]
    fn name_escapes_are_decoded() {
        assert_eq!(names(r"(?<ab>.)(?<\u{63}>.)", ""), ["ab", "c"]);
        assert_eq!(names("(?<\u{1d49c}>.)", "u"), ["\u{1d49c}"]);
        assert_eq!(names(r"(?<𝒜>.)", ""), ["\u{1d49c}"]);
    }
}
