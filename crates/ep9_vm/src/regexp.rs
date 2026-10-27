//! RegExp objects in the VM (owners M4-P2 RX0A, then M4-P3 REGEXP1; V9-D15, V9-A7): the
//! RegExp object (a SLOT_OBJECT of `ObjectClass::RegExp`), `CreateRegExpLiteral` and the
//! per-isolate program table.
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
//! | 2 `RECORD` | `[[RegExpRecord]]` | the flag bits (int32; `ep9_regexp::Flags`) |
//! | 3 `MATCHER` | `[[RegExpMatcher]]` | the int32 index of the program, or undefined before the first exec of a literal |
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
    /// The slot count.
    pub const COUNT: usize = 4;
}

// ---- the program table ------------------------------------------------------------------------

/// One table entry: a program and its owner, or a free entry (`owner` 0, no program).
struct Entry {
    /// The RegExp object's bits, held weakly (the weak source clears it); 0 = free.
    owner: u64,
    program: Option<Program>,
    /// The group names of `program` ([`group_names`]); empty for a free entry.
    names: Names,
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
                e.program = None;
                e.names = Rc::from([]);
                self.free.push(i as u32);
                freed += 1;
            }
        }
        freed
    }
}

/// The compiled programs of the isolate, indexed by the int32 a RegExp object holds.
pub(crate) struct ProgramTable {
    inner: Rc<RefCell<Programs>>,
}

impl ProgramTable {
    /// The empty table of a new isolate. `heap` is the isolate's heap, before its first
    /// allocation: the table's weak source (it holds an `Rc` clone of the entries) frees the
    /// entries whose owner died, after marking.
    pub(crate) fn new(heap: &Heap) -> ProgramTable {
        let inner = Rc::new(RefCell::new(Programs::default()));
        let table = Rc::clone(&inner);
        heap.add_weak_source(move |w| {
            let mut t = table
                .try_borrow_mut()
                .expect("the RegExp program table is borrowed across a GC point");
            t.sweep(|b| w.is_live(val(b)));
        });
        ProgramTable { inner }
    }

    /// Stores `program` for the RegExp object `owner`; returns its index. Not a GC point.
    fn insert(&self, owner: u64, (program, names): (Program, Names)) -> i32 {
        let mut t = self.inner.borrow_mut();
        let e = Entry {
            owner,
            program: Some(program),
            names,
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
        let program = t.entries[index as usize]
            .program
            .as_ref()
            .expect("a live RegExp holds a live program index");
        Engine::exec(program, input, start, caps, poll)
    }

    /// The group names of program `index` (a clone of the shared `Rc`).
    fn names(&self, index: i32) -> Names {
        Rc::clone(&self.inner.borrow().entries[index as usize].names)
    }

    /// The number of entries that hold a program (tests).
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        let t = self.inner.borrow();
        t.entries.iter().filter(|e| e.program.is_some()).count()
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
/// or undefined), and an own `lastIndex` = 0 ({w}, not enumerable, not configurable). GC
/// point; `proto` and `source` stay in Rust locals (conservative scan).
fn new_regexp(rt: &Runtime, proto: u64, source: u64, flags: Flags, matcher: u64) -> u64 {
    let flags_text = object::new_string(rt, &flags.to_letters());
    let o = object::new_slot_object(rt, proto, ObjectClass::RegExp.id(), slot::COUNT);
    object::slot_set(rt, o, slot::SOURCE, source);
    object::slot_set(rt, o, slot::FLAGS, flags_text);
    object::slot_set(rt, o, slot::RECORD, int32_bits(i32::from(flags.bits())));
    object::slot_set(rt, o, slot::MATCHER, matcher);
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
/// compiles at its first exec. The Literal slot records nothing yet (no boilerplate:
/// every evaluation makes a new object, §13.2.7.3). Returns 0 with an exception pending only
/// if the flags operand is not a flag set.
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

/// RegExpAlloc(proto) then RegExpInitialize(obj, pattern, flags) (§22.2.3.1, §22.2.3.3) for
/// the strings `pattern` and `flags` (the caller ran ToString): a SyntaxError for a flag that
/// is not one of `dgimsuvy`, a flag twice, `u` with `v`, or a pattern that does not compile.
/// The program compiles now. GC point.
pub(crate) fn create(cx: &mut Cx, proto: u64, pattern: u64, flags: u64) -> Res<u64> {
    let flag_units = units_of(flags);
    let Some(f) = Flags::parse(&flag_units) else {
        let msg = format!(
            "Invalid flags supplied to RegExp constructor '{}'",
            String::from_utf16_lossy(&flag_units)
        );
        return Err(syntax_error(cx, &msg));
    };
    let program = compile(cx, &units_of(pattern), f)?;
    let rt = cx.rt();
    // Not compiled into the table yet: the object must exist to own the entry.
    let o = new_regexp(rt, proto, pattern, f, UNDEFINED);
    let i = rt.regexp_programs.insert(o, program);
    object::slot_set(rt, o, slot::MATCHER, int32_bits(i));
    Ok(o)
}

/// The program index of the RegExp object `re`, compiling it at the first exec of a literal.
/// GC point only through the SyntaxError.
fn program_index(cx: &mut Cx, re: u64) -> Res<i32> {
    let m = object::slot_get(re, slot::MATCHER);
    if m != UNDEFINED {
        return Ok(int32_of(m));
    }
    let source = object::slot_get(re, slot::SOURCE);
    let flags = Flags::from_bits(int32_of(object::slot_get(re, slot::RECORD)) as u8);
    let p = compile(cx, &units_of(source), flags)?;
    let rt = cx.rt();
    let i = rt.regexp_programs.insert(re, p);
    object::slot_set(rt, re, slot::MATCHER, int32_bits(i));
    Ok(i)
}

/// Compiles `pattern` with `flags` and scans its group names. A SyntaxError if it does not
/// compile. GC point only through the error.
fn compile(cx: &mut Cx, pattern: &[u16], flags: Flags) -> Res<(Program, Names)> {
    match Engine::compile(pattern, flags) {
        Ok(p) => Ok((p, group_names(pattern, flags))),
        Err(e) => Err(syntax_error(cx, &e.message)),
    }
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
            let input = match string::view(cx, flat) {
                JsStr::Latin1(b) => Input::Latin1(b),
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
