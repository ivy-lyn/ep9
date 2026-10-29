//! Slack (owner M5-P2 SLACK; the H17 "Slack tracking" row): the in-object capacity of the
//! objects that `new F` makes.
//!
//! ```text
//!  call::ordinary_create_from_constructor(F)
//!    proto = F.prototype ──> instance_capacity(F, proto)
//!                              Some(n) ─> object::new_object_with_capacity(proto, n)  root (proto, OBJECT, n)
//!                              None    ─> object::new_object(proto)                   root (proto, OBJECT, GENEROUS)
//!    n = the distinct names of F's own `this.<name> =` stores          (scan, cached by code)
//!      + the data properties of proto whose value is not callable (the instances' likely
//!        shadows: `F.prototype.left = null`) and that F does not store, at most GENEROUS
//!  construct_ic::miss caches the root shape of that first instance; a hit allocates from it
//! ```
//!
//! Rules:
//! - One capacity per (constructor code, prototype): every instance of `new F` starts from
//!   the same root shape, so the ICs see one shape family per constructor, as before.
//! - The count is read from the bytecode, so no instance runs first with another capacity.
//!   A run-time tracker that sizes the later instances after the first N (V8 slack tracking)
//!   would start a second root shape family: measured in M5-P2, the T0 ICs then see two
//!   shapes per constructor, which cost more than the smaller objects saved (richards +12.7%,
//!   deltablue +9.1% instructions). V8 shrinks its map tree in place instead; here the
//!   capacity lives in each shape's `counts` word (`shape.rs`), and a live object never
//!   shrinks (H21 P5).
//! - [`None`] (the generous capacity, today's 8) when the count is unknown: `this` leaves
//!   the frame (a call or a store that reads it, a closure, a context slot, a direct eval),
//!   a keyed store, a derived constructor, no store at all, or more names than [`MAX_NAMES`].
//!   A field added after the constructor returns goes out of line (props.rs SLOTS), as a
//!   ninth field does today.
//! - The count is a size hint, never a correctness input: any capacity is a right one. A
//!   linear scan does not follow jumps, so a register that holds `this` on one path only may
//!   count as holding it, or not.
//! - It reads no execution policy (rule 40, M-R9): the bytecode and the prototype only, so
//!   every preset and both feedback modes make the same shapes (`tests::slack`).
//! - The scan result is a function of the code bytes alone, so a per-thread cache keyed by
//!   the bytes (length and a 64-bit hash) keeps it ([`CACHE_ENTRIES`] entries, direct mapped):
//!   a cold construct (a megamorphic site, `Reflect.construct`, a site with no vector) costs
//!   a hash of the code, not a decode (measured: 8,285 instructions per cold construct of
//!   deltablue's `Variable` without the cache). The cache holds no Value (rule 8): constant
//!   indices and counts.
//! - Not a GC point; it allocates nothing (fixed arrays).

use ep9_bytecode::{Instructions, Opcode, OperandKind, Register, RuntimeId};

use crate::function_info::fi_flags;
use crate::kinds::{self as kind, is_callable};
use crate::object::{self, cell, obj, rd};
use crate::runtime::Runtime;
use crate::shape::{self, slf};

/// The in-object capacity of a new object whose prototype is not %Object.prototype% when no
/// count is known (`object::new_object`; the capacity before M5-P2).
pub(crate) const GENEROUS: u8 = 8;

/// The most distinct `this.<name> =` names one scan keeps; more gives [`None`].
pub(crate) const MAX_NAMES: usize = 64;

/// The registers a scan follows (locals and parameters below this index each).
const TRACKED_REGISTERS: u32 = 64;

/// The entries of the per-thread scan cache.
pub(crate) const CACHE_ENTRIES: usize = 64;

/// The names one cache entry keeps; a scan with more is not cached (it runs again).
const CACHED_NAMES: usize = 16;

/// One scan result: the code's length and hash, and the names (`n` = `NO_COUNT` for a scan
/// that gave [`None`]).
#[derive(Clone, Copy)]
struct Entry {
    len: u32,
    hash: u64,
    n: u8,
    names: [u32; CACHED_NAMES],
}

/// `Entry::n` of a scan that gave [`None`].
const NO_COUNT: u8 = u8::MAX;

/// An entry no code matches (a function's code is never `u32::MAX` bytes long).
const EMPTY: Entry = Entry {
    len: u32::MAX,
    hash: 0,
    n: NO_COUNT,
    names: [0; CACHED_NAMES],
};

std::thread_local! {
    static CACHE: core::cell::RefCell<[Entry; CACHE_ENTRIES]> =
        const { core::cell::RefCell::new([EMPTY; CACHE_ENTRIES]) };
}

#[cfg(test)]
std::thread_local! {
    /// Tests: the scans [`cached_store_names`] ran (cache misses).
    pub(crate) static SCANS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// A 64-bit hash of `code` (FNV-1a over 8-byte words, then a murmur3 finalizer). Not a GC
/// point.
fn code_hash(code: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let (words, rest) = code.as_chunks::<8>();
    for &w in words {
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    for &b in rest {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^ (h >> 33)
}

/// [`this_store_names`] through the per-thread cache (module docs). Not a GC point.
pub(crate) fn cached_store_names(code: &[u8], names: &mut [u32; MAX_NAMES]) -> Option<usize> {
    let len = code.len() as u32;
    let hash = code_hash(code);
    let i = hash as usize % CACHE_ENTRIES;
    let hit = CACHE.with(|c| {
        let e = c.borrow()[i];
        (e.len == len && e.hash == hash).then(|| {
            let n = usize::from(e.n);
            if e.n != NO_COUNT {
                names[..n].copy_from_slice(&e.names[..n]);
            }
            e.n
        })
    });
    if let Some(n) = hit {
        return (n != NO_COUNT).then_some(usize::from(n));
    }
    #[cfg(test)]
    SCANS.with(|c| c.set(c.get() + 1));
    let r = this_store_names(code, names);
    let n = match r {
        Some(n) if n <= CACHED_NAMES => n as u8,
        Some(_) => return r,
        None => NO_COUNT,
    };
    let mut e = Entry {
        len,
        hash,
        n,
        names: [0; CACHED_NAMES],
    };
    if n != NO_COUNT {
        e.names[..usize::from(n)].copy_from_slice(&names[..usize::from(n)]);
    }
    CACHE.with(|c| c.borrow_mut()[i] = e);
    r
}

/// Where the scan found `this`: the accumulator and two register masks.
#[derive(Default)]
struct Holders {
    acc: bool,
    locals: u64,
    params: u64,
}

impl Holders {
    fn holds(&self, r: Register) -> bool {
        if r.is_receiver() {
            return true;
        }
        if let Some(i) = r.local_index() {
            return i < TRACKED_REGISTERS && self.locals & (1 << i) != 0;
        }
        if let Some(i) = r.parameter_index() {
            return i < TRACKED_REGISTERS && self.params & (1 << i) != 0;
        }
        false
    }

    /// Records that `r` now holds `this` (`on`) or not. False if the scan cannot follow it.
    fn set(&mut self, r: Register, on: bool) -> bool {
        if r.is_receiver() {
            // A write of the receiver register: not followed.
            return false;
        }
        let (mask, i) = if let Some(i) = r.local_index() {
            (&mut self.locals, i)
        } else if let Some(i) = r.parameter_index() {
            (&mut self.params, i)
        } else {
            return !on;
        };
        if i >= TRACKED_REGISTERS {
            return !on;
        }
        if on {
            *mask |= 1 << i;
        } else {
            *mask &= !(1 << i);
        }
        true
    }
}

/// The distinct constant-pool indices of the names `code` stores on `this`
/// (`SetNamedProperty` / `DefineNamedOwnProperty` on a register that holds it), or [`None`]
/// when `this` may leave the frame or the scan cannot follow it. A linear scan: it does not
/// follow jumps, so a register that holds `this` on one path only counts as holding it (a
/// count, never a correctness input). Not a GC point.
pub(crate) fn this_store_names(code: &[u8], names: &mut [u32; MAX_NAMES]) -> Option<usize> {
    let mut h = Holders::default();
    let mut n = 0;
    for insn in Instructions::new(code) {
        let insn = insn.ok()?;
        let op = insn.opcode;
        let info = op.info();
        match op {
            Opcode::Ldar => {
                h.acc = h.holds(insn.register(0));
                continue;
            }
            Opcode::Star => {
                if !h.set(insn.register(0), h.acc) {
                    return None;
                }
                continue;
            }
            Opcode::Mov => {
                let on = h.holds(insn.register(0));
                if !h.set(insn.register(1), on) {
                    return None;
                }
                continue;
            }
            Opcode::Return => continue,
            Opcode::SetNamedProperty | Opcode::DefineNamedOwnProperty => {
                // Operands: object register, name constant, slot; the value in acc.
                if h.acc {
                    return None;
                }
                if h.holds(insn.register(0)) {
                    let k = insn.unsigned(1);
                    if !names[..n].contains(&k) {
                        if n == MAX_NAMES {
                            return None;
                        }
                        names[n] = k;
                        n += 1;
                    }
                }
                continue;
            }
            Opcode::GetNamedProperty => {
                // Reads `this.<name>`: `this` stays in the frame.
                h.acc = false;
                continue;
            }
            _ => {}
        }
        if h.acc && info.acc.reads() {
            return None;
        }
        let count = insn.operand_count();
        let mut i = 0;
        while i < count {
            let k = insn.operand_kind(i);
            match k {
                OperandKind::Reg | OperandKind::RegInOut => {
                    let r = insn.register(i);
                    if h.holds(r) {
                        return None;
                    }
                }
                OperandKind::RegOut => {
                    if !h.set(insn.register(i), false) {
                        return None;
                    }
                }
                OperandKind::RegList | OperandKind::RegOutList => {
                    let first = insn.register(i).to_operand();
                    let len = insn.unsigned(i + 1) as i32;
                    for j in 0..len {
                        let r = Register::from_operand(first + j);
                        if k == OperandKind::RegList && h.holds(r) {
                            return None;
                        }
                        if k == OperandKind::RegOutList && !h.set(r, false) {
                            return None;
                        }
                    }
                    i += 1;
                }
                OperandKind::RegPair | OperandKind::RegOutPair | OperandKind::RegOutTriple => {
                    let first = insn.register(i).to_operand();
                    let len = if k == OperandKind::RegOutTriple { 3 } else { 2 };
                    for j in 0..len {
                        let r = Register::from_operand(first + j);
                        if k == OperandKind::RegPair && h.holds(r) {
                            return None;
                        }
                        if k != OperandKind::RegPair && !h.set(r, false) {
                            return None;
                        }
                    }
                }
                OperandKind::Runtime
                    if RuntimeId::from_u16(insn.operand(i) as u16)
                        == Some(RuntimeId::ResolvePossiblyDirectEval) =>
                {
                    return None;
                }
                _ => {}
            }
            i += 1;
        }
        if info.acc.writes() {
            h.acc = false;
        }
    }
    Some(n)
}

/// The value of the own data property at field `field` of the fast-mode object `o` with
/// shape `s`. Not a GC point.
fn field_value(o: u64, s: u64, field: usize) -> u64 {
    let (cap, k) = shape::field_layout(s);
    let c = cell(o);
    if field < cap {
        rd(c, obj::FIXED + 8 * (object::fixed_count(k) + field))
    } else {
        rd(cell(rd(c, obj::SLOTS)), slf::DATA + 8 * (field - cap))
    }
}

/// True if the own property `i` of `proto` (shape `s`) is a data property whose value is not
/// callable: a likely instance field (`F.prototype.left = null`). Not a GC point.
fn is_proto_field(proto: u64, s: u64, i: usize) -> bool {
    let d = shape::details_at(s, i);
    !d.is_accessor() && !is_callable(field_value(proto, s, usize::from(d.field())))
}

/// The in-object capacity of the instances `new f` makes with the prototype `proto` (module
/// docs), or [`None`] for [`GENEROUS`]. Not a GC point.
pub(crate) fn instance_capacity(rt: &Runtime, f: u64, proto: u64) -> Option<usize> {
    if !object::is_function(f) {
        return None;
    }
    let fi = object::function_info(f);
    let need = fi_flags::BYTECODE | fi_flags::CONSTRUCTOR;
    if fi.flags & need != need || fi.flags & fi_flags::DERIVED != 0 || fi.code.is_null() {
        return None;
    }
    // SAFETY: a bytecode FunctionInfo's code is its unit function's bytecode, which the unit
    // keeps at this address while the info lives (`FunctionInfo::bytecode`).
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let mut names = [0u32; MAX_NAMES];
    let n = cached_store_names(code, &mut names)?;
    if n == 0 {
        return None;
    }
    let mut extra = 0;
    if kind::is_object(proto) {
        let ps = shape::shape_of(proto);
        if !shape::is_dictionary(ps) {
            extra = (0..shape::count(ps))
                .filter(|&i| is_proto_field(proto, ps, i))
                .count();
            // A name the constructor stores and the prototype has counts once.
            for &k in &names[..n] {
                if extra == 0 {
                    break;
                }
                // SAFETY: `k` is a verified String constant index of this function.
                let key = unsafe { (*fi.consts.add(k as usize)).bits() };
                if let Some(found) = shape::lookup(rt, ps, key)
                    && is_proto_field(proto, ps, found.index)
                {
                    extra -= 1;
                }
            }
        }
    }
    Some((n + extra.min(usize::from(GENEROUS))).min(obj::MAX_INOBJECT))
}
