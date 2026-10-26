//! Strings (H19): flat sequential strings, ropes (CONS) and slices (SLICED), the hash field,
//! the string hash and the read API.
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | SEQ_LATIN1 | 1 | +8 length (u32), +12 hash field (u32), +16 bytes | 16 + len |
//! | SEQ_UTF16 | 2 | +8 length (u32), +12 hash field (u32), +16 UTF-16 code units | 16 + 2 len |
//! | CONS | 3 | +8 length (u32), +12 hash field (u32), +16 first, +24 second (strings) | 32 |
//! | SLICED | 4 | +8 length (u32), +12 hash field (u32), +16 parent (a SEQ string), +24 offset (int32) | 32 |
//!
//! - The words after word0 of SEQ cells are raw data, not Values. SEQ kinds trace nothing,
//!   and the size comes from the kind and the length (H9 "heap always parsable"). CONS and
//!   SLICED trace their two words at +16 and +24 (the +8 word is raw).
//! - word0 flag bit 0 ([`crate::kinds::FLAG_ATOM`]) marks an atom: the one interned cell of
//!   its text in this isolate (`atoms.rs`). Flag bit 1 ([`FLAG_TWO_BYTE`]) marks a CONS cell
//!   with a UTF-16 leaf (its flat form is SEQ_UTF16).
//! - Kinds 5-7 (thin, external) are reserved and share this header, so the length is always
//!   the u32 at +8.
//!
//! Ropes (M4-P1 STR1; H19, V9-D13). `a + b` makes a CONS cell when the result has at least
//! [`MIN_CONS_LENGTH`] code units, and a substring of at least [`MIN_SLICE_LENGTH`] units
//! is a SLICED cell over a SEQ parent ([`substring`]). [`flatten`] of a CONS writes the flat
//! form once and turns the cell into (first = flat, second = ""), V8 style: a flattened CONS
//! reads as its first. A SLICED cell is flat for reading (its units are a range of the
//! parent's), so [`flatten`] returns it as is.
//!
//! Hash field (u32 at +12; 0 = not computed yet), V8 Name style:
//!
//! ```text
//! bit 0      computed
//! bit 1      an array index below 2^24 ("0" .. "16777215"): bits 2-25 hold the index
//! bits 2-31  otherwise: 30 bits of the seeded hash of the code units
//! ```
//!
//! The hash is SipHash-1-3 over the UTF-16 code-unit values (little-endian), so the Latin-1
//! and UTF-16 forms of one text hash alike, and a rope hashes like its flat form. The key is
//! chosen once per process, at random, or from `EP9_HASH_SEED`. A hash never goes into
//! bytecode, units, snapshots or property order (W4-HASH, H12).
//!
//! Read API: [`flatten`] (a GC point: it may allocate the flat form), then [`view`], whose
//! borrow of the `Cx` ends at the next `&mut Cx` call. The helpers that take no `Cx`
//! ([`equals`], [`equals_text`], [`compare`], [`hash`], [`hash_field`], [`array_index`],
//! [`to_rust_string`], [`with_text`]) read ropes without a GC allocation (a walk over the
//! rope). [`str_view`] is the legacy flat-only read; no VM code outside tests calls it since
//! M4-P1 (W4-STRVIEW, V9-A2). VM code may still hold a `JsStr` across an allocation because
//! the heap never moves cells and the conservative scan keeps interior pointers (H19,
//! `crates/ep9_heap/src/trace.rs` find_cell); an in-place flatten replaces the CONS fields
//! but never frees the old leaves while a view of them is on the stack.

use std::sync::OnceLock;

use ep9_heap::{CellKind, CellPtr, CellSize, Heap};
use ep9_value::Value;

use crate::atoms::Atom;
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::interp::Cx;
use crate::isolate::Isolate;
use crate::kinds::{self as kind, FLAG_ATOM, cell_kind, is_cell_bits, is_string};
use crate::object::{cell, init, rd, wr};
use crate::runtime::Runtime;

mod strf {
    pub const LENGTH: usize = 8;
    pub const HASH: usize = 12;
    pub const DATA: usize = 16;
    /// CONS: the first part (a string).
    pub const FIRST: usize = 16;
    /// CONS: the second part (a string; "" once the cell is flattened).
    pub const SECOND: usize = 24;
    /// SLICED: the parent (a SEQ string).
    pub const PARENT: usize = 16;
    /// SLICED: the offset in the parent (int32).
    pub const OFFSET: usize = 24;
    /// The size of a CONS or SLICED cell.
    pub const ROPE_SIZE: usize = 32;
}

/// word0 flag bit 1 of a CONS cell: some leaf is UTF-16, so the flat form is SEQ_UTF16.
pub(crate) const FLAG_TWO_BYTE: u16 = 0x0002;

/// `a + b` of at least this many code units is a CONS cell (H19; V8 `ConsString::kMinLength`).
pub(crate) const MIN_CONS_LENGTH: usize = 13;

/// A substring of at least this many code units is a SLICED cell (H19; V8
/// `SlicedString::kMinLength`).
pub(crate) const MIN_SLICE_LENGTH: usize = 13;

/// Largest string length in code units (V8: 2^29 - 24). Longer results throw RangeError
/// ("Invalid string length"); see [`concat()`].
pub(crate) const MAX_STRING_LENGTH: usize = (1 << 29) - 24;

// ---- hash field ------------------------------------------------------------------------------

/// The hash field holds a value.
const HASH_COMPUTED: u32 = 1;
/// The string is an array index below [`MAX_CACHED_INDEX`]; bits 2-25 hold it.
const HASH_IS_INDEX: u32 = 2;
const HASH_SHIFT: u32 = 2;
/// Array indices below this are cached in the hash field (24 bits).
pub(crate) const MAX_CACHED_INDEX: u32 = 1 << 24;
/// Mask of a table hash (30 bits).
pub(crate) const HASH_MASK: u32 = (1 << 30) - 1;

fn trace_none(_: CellPtr, _: &mut ep9_heap::Tracer<'_>) {}

/// CONS and SLICED: the two Value words at +16 and +24 (the +8 word is raw length and hash).
fn trace_rope(c: CellPtr, t: &mut ep9_heap::Tracer<'_>) {
    // SAFETY: the heap calls trace functions with live cells of the registered kind; +16 and
    // +24 of a CONS or SLICED cell are Value slots (layout table).
    t.visit_slots(unsafe { c.slots(strf::FIRST, 2) });
}

/// The length of a live string cell.
#[inline(always)]
fn len_of(c: CellPtr) -> usize {
    // SAFETY: `c` is a live string cell (callers check the kind or hold a string Value); every
    // string kind has a u32 length at +8, inside its 16-byte minimum size.
    unsafe { c.field::<u32>(strf::LENGTH).read() as usize }
}

#[inline(always)]
fn hash_field_raw(c: CellPtr) -> u32 {
    // SAFETY: as in `len_of`; the hash field is the u32 at +12.
    unsafe { c.field::<u32>(strf::HASH).read() }
}

#[inline(always)]
fn set_hash_field_raw(c: CellPtr, field: u32) {
    // SAFETY: as in `len_of`. Strings are immutable except this cache word, which no view
    // covers (views start at +16), and an isolate runs on one thread.
    unsafe { c.field::<u32>(strf::HASH).write(field) }
}

fn latin1_size(c: CellPtr) -> usize {
    strf::DATA + len_of(c)
}

fn utf16_size(c: CellPtr) -> usize {
    strf::DATA + 2 * len_of(c)
}

/// Registers the string kinds.
pub(crate) fn register_kinds(heap: &Heap) {
    let kinds = [
        (
            kind::SEQ_LATIN1,
            "string-latin1",
            latin1_size as fn(CellPtr) -> usize,
        ),
        (kind::SEQ_UTF16, "string-utf16", utf16_size),
    ];
    for (id, name, size) in kinds {
        heap.register_kind(
            id,
            CellKind {
                name,
                size: CellSize::Dynamic(size),
                trace: trace_none,
            },
        )
        .expect("register a VM cell kind");
    }
    for (id, name) in [(kind::CONS, "string-cons"), (kind::SLICED, "string-sliced")] {
        heap.register_kind(
            id,
            CellKind {
                name,
                size: CellSize::Fixed(strf::ROPE_SIZE),
                trace: trace_rope,
            },
        )
        .expect("register a VM cell kind");
    }
}

// ---- views ---------------------------------------------------------------------------------

/// The code units of a flat string (H19 `JsStr`): Latin-1 bytes or UTF-16 code units. The
/// two forms of one text are equal (`==`), and `<` is code-unit order (§7.2.13):
///
/// ```
/// use ep9_vm::JsStr;
/// assert_eq!(JsStr::Latin1(b"ab"), JsStr::Utf16(&[0x61, 0x62]));
/// assert!(JsStr::Latin1(b"ab") < JsStr::Utf16(&[0x61, 0x62, 0]));
/// assert!(JsStr::Latin1(b"\xff\xff") < JsStr::Utf16(&[0x100]));
/// ```
#[derive(Clone, Copy, Debug)]
pub enum JsStr<'a> {
    Latin1(&'a [u8]),
    Utf16(&'a [u16]),
}

impl<'a> JsStr<'a> {
    /// The length in UTF-16 code units.
    pub fn len(&self) -> usize {
        match self {
            JsStr::Latin1(b) => b.len(),
            JsStr::Utf16(u) => u.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Code unit `i` (`i < len()`).
    pub fn at(&self, i: usize) -> u16 {
        match self {
            JsStr::Latin1(b) => u16::from(b[i]),
            JsStr::Utf16(u) => u[i],
        }
    }

    pub fn to_utf16(self) -> Vec<u16> {
        match self {
            JsStr::Latin1(b) => b.iter().map(|&c| u16::from(c)).collect(),
            JsStr::Utf16(u) => u.to_vec(),
        }
    }

    /// Rust text; a lone surrogate becomes U+FFFD.
    pub fn to_string_lossy(self) -> String {
        match self {
            JsStr::Latin1(b) => b.iter().map(|&c| char::from(c)).collect(),
            JsStr::Utf16(u) => String::from_utf16_lossy(u),
        }
    }

    /// CanonicalNumericIndexString restricted to array indices: "0", "1", ... "4294967294".
    pub fn array_index(&self) -> Option<u32> {
        let n = self.len();
        if n == 0 || n > 10 {
            return None;
        }
        let first = self.at(0);
        if first == u16::from(b'0') {
            return (n == 1).then_some(0);
        }
        let mut v: u64 = 0;
        for i in 0..n {
            let c = self.at(i);
            if !(u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                return None;
            }
            v = v * 10 + u64::from(c - u16::from(b'0'));
        }
        (v < u64::from(u32::MAX)).then_some(v as u32)
    }

    /// The hash field of this text (computed bit set). See the module docs.
    fn hash_field(&self) -> u32 {
        if let Some(i) = self.array_index()
            && i < MAX_CACHED_INDEX
        {
            return HASH_COMPUTED | HASH_IS_INDEX | (i << HASH_SHIFT);
        }
        let h = match self {
            JsStr::Latin1(b) => sip13_units(b),
            JsStr::Utf16(u) => sip13_units(u),
        };
        HASH_COMPUTED | ((h as u32 & HASH_MASK) << HASH_SHIFT)
    }
}

/// Equal code units (either encoding).
impl PartialEq<JsStr<'_>> for JsStr<'_> {
    fn eq(&self, other: &JsStr<'_>) -> bool {
        match (self, other) {
            (JsStr::Latin1(a), JsStr::Latin1(b)) => a == b,
            (JsStr::Utf16(a), JsStr::Utf16(b)) => a == b,
            _ => self.len() == other.len() && (0..self.len()).all(|i| self.at(i) == other.at(i)),
        }
    }
}

impl Eq for JsStr<'_> {}

/// Code-unit order (IsLessThan on strings, §7.2.13).
impl Ord for JsStr<'_> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let n = self.len().min(other.len());
        for i in 0..n {
            match self.at(i).cmp(&other.at(i)) {
                core::cmp::Ordering::Equal => {}
                o => return o,
            }
        }
        self.len().cmp(&other.len())
    }
}

impl PartialOrd for JsStr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The code units of a live flat string cell (SEQ, SLICED or a flattened CONS), for any
/// lifetime the caller picks. The caller keeps the cell live while it uses the view (cells
/// never move; strings never change). Legacy API: VM code uses [`flatten`] and [`view`], or a
/// rope-safe helper (W4-STRVIEW, V9-A2); panics on a CONS that is not flattened.
#[cfg_attr(not(test), allow(dead_code))] // callers: tests only
pub(crate) fn str_view<'a>(b: u64) -> JsStr<'a> {
    units(b)
}

/// The code units of a SEQ cell.
#[inline(always)]
fn seq_units<'a>(c: CellPtr, utf16: bool) -> JsStr<'a> {
    let len = len_of(c);
    // SAFETY: a live SEQ string cell holds `len` code units of its kind's encoding from +16
    // (layout table), and its size covers them (`latin1_size`, `utf16_size`). +16 is
    // 8-aligned, so the u16 slice is aligned.
    unsafe {
        if utf16 {
            JsStr::Utf16(core::slice::from_raw_parts(c.field::<u16>(strf::DATA), len))
        } else {
            JsStr::Latin1(core::slice::from_raw_parts(c.field::<u8>(strf::DATA), len))
        }
    }
}

/// The code units of a flat string `b` (SEQ, SLICED or a flattened CONS); `None` for a CONS
/// that is not flattened. Not a GC point.
#[inline]
fn flat_units<'a>(b: u64) -> Option<JsStr<'a>> {
    debug_assert!(is_string(b));
    let c = cell(b);
    // SAFETY: a string Value held by the VM is a live cell (crate invariant, object.rs).
    match unsafe { c.header().kind() } {
        1 => Some(seq_units(c, false)),
        2 => Some(seq_units(c, true)),
        _ => non_seq_units(b),
    }
}

/// Is `b` a flat string (SEQ, SLICED or a flattened CONS), whose code units can be read
/// without a flatten? The test of the `FlatStr` downcast (`abi/local.rs`). Not a GC point.
pub(crate) fn is_flat(b: u64) -> bool {
    is_string(b) && flat_units(b).is_some()
}

#[inline(never)]
fn non_seq_units<'a>(b: u64) -> Option<JsStr<'a>> {
    let c = cell(b);
    match cell_kind(b) {
        3 => {
            let second = rd(c, strf::SECOND);
            (len_of(cell(second)) == 0).then(|| units(rd(c, strf::FIRST)))
        }
        4 => {
            let (parent, off) = (rd(c, strf::PARENT), int32_of(rd(c, strf::OFFSET)) as usize);
            let len = len_of(c);
            Some(match units(parent) {
                JsStr::Latin1(x) => JsStr::Latin1(&x[off..off + len]),
                JsStr::Utf16(x) => JsStr::Utf16(&x[off..off + len]),
            })
        }
        k => panic!("string kind {k} has no reader yet"),
    }
}

/// The code units of a flat string cell (this file's own reads: hashing, compares, [`view`]).
/// Panics on a CONS that is not flattened: callers check [`flat_units`] first or flatten.
fn units<'a>(b: u64) -> JsStr<'a> {
    match flat_units(b) {
        Some(v) => v,
        None => panic!("a flat read of a rope that is not flattened (flatten it first)"),
    }
}

/// The length of the string `b` in code units.
pub(crate) fn str_len(b: u64) -> usize {
    len_of(cell(b))
}

/// Does the flat form of `b` need UTF-16 storage? Not a GC point.
fn is_two_byte(b: u64) -> bool {
    match cell_kind(b) {
        1 => false,
        2 => true,
        // SAFETY: `b` is a live CONS cell.
        3 => (unsafe { cell(b).header().flags() } & FLAG_TWO_BYTE) != 0,
        4 => is_two_byte(rd(cell(b), strf::PARENT)),
        k => panic!("string kind {k} has no reader yet"),
    }
}

/// The parts of a CONS cell that is not flattened.
fn cons_parts(b: u64) -> (u64, u64) {
    debug_assert_eq!(cell_kind(b), kind::CONS.get());
    let c = cell(b);
    (rd(c, strf::FIRST), rd(c, strf::SECOND))
}

/// The leaves of the string `b` in order (each a flat view), by a walk over the rope that
/// allocates nothing on the JS heap (the parts still to visit are on a Rust `Vec`, empty for
/// a flat string).
struct Leaves<'a> {
    next: Option<u64>,
    /// Right parts still to visit (cells, so no raw Value list lives here; not a GC point).
    stack: Vec<CellPtr>,
    _view: core::marker::PhantomData<JsStr<'a>>,
}

impl<'a> Leaves<'a> {
    fn new(b: u64) -> Leaves<'a> {
        Leaves {
            next: Some(b),
            stack: Vec::new(),
            _view: core::marker::PhantomData,
        }
    }
}

impl<'a> Iterator for Leaves<'a> {
    type Item = JsStr<'a>;

    fn next(&mut self) -> Option<JsStr<'a>> {
        let mut b = match self.next.take() {
            Some(b) => b,
            None => self.stack.pop()?.to_value().to_bits(),
        };
        loop {
            if let Some(v) = flat_units(b) {
                return Some(v);
            }
            // Visit `first`, then `second`, then the parts pending before.
            let (first, second) = cons_parts(b);
            if let Some(n) = self.next.replace(second) {
                self.stack.push(cell(n));
            }
            b = first;
        }
    }
}

/// The code units of the string `b` in order (any kind; not a GC point).
struct Units<'a> {
    leaves: Leaves<'a>,
    cur: JsStr<'a>,
    i: usize,
}

impl<'a> Units<'a> {
    fn new(b: u64) -> Units<'a> {
        Units {
            leaves: Leaves::new(b),
            cur: JsStr::Latin1(&[]),
            i: 0,
        }
    }
}

impl Iterator for Units<'_> {
    type Item = u16;

    #[inline]
    fn next(&mut self) -> Option<u16> {
        while self.i >= self.cur.len() {
            self.cur = self.leaves.next()?;
            self.i = 0;
        }
        let u = self.cur.at(self.i);
        self.i += 1;
        Some(u)
    }
}

/// Code unit `i` of the string `b` (any kind; `i < str_len(b)`), by a walk down the rope:
/// O(depth), no GC point. For readers that hold no `&mut Cx`, such as a check of the first
/// unit of a name that may be an atom of rope kind (`interp/slow/lookup.rs`
/// `is_private_name`; a fixer item of M4-P1).
#[cfg_attr(not(test), allow(dead_code))] // caller: lookup.rs is_private_name (fixer, M4-P1)
pub(crate) fn unit_at(b: u64, i: usize) -> u16 {
    debug_assert!(i < str_len(b));
    let (mut b, mut i) = (b, i);
    loop {
        if let Some(v) = flat_units(b) {
            return v.at(i);
        }
        let (first, second) = cons_parts(b);
        let lf = str_len(first);
        if i < lf {
            b = first;
        } else {
            (b, i) = (second, i - lf);
        }
    }
}

/// A destination of [`write_flat`]: Latin-1 bytes or UTF-16 code units.
trait FlatUnit: Copy {
    fn copy_from(dst: &mut [Self], src: JsStr<'_>);
}

impl FlatUnit for u8 {
    fn copy_from(dst: &mut [u8], src: JsStr<'_>) {
        match src {
            JsStr::Latin1(x) => dst.copy_from_slice(x),
            // A one-byte rope has only Latin-1 leaves (FLAG_TWO_BYTE); a SEQ_UTF16 leaf whose
            // units all fit (tests make one) narrows exactly.
            JsStr::Utf16(x) => {
                for (d, &u) in dst.iter_mut().zip(x) {
                    debug_assert!(u <= 0xFF);
                    *d = u as u8;
                }
            }
        }
    }
}

impl FlatUnit for u16 {
    fn copy_from(dst: &mut [u16], src: JsStr<'_>) {
        match src {
            JsStr::Latin1(x) => {
                for (d, &u) in dst.iter_mut().zip(x) {
                    *d = u16::from(u);
                }
            }
            JsStr::Utf16(x) => dst.copy_from_slice(x),
        }
    }
}

/// Writes the code units of the string `b` (any kind) to `dst` (`dst.len() == str_len(b)`).
/// Not a GC point. A part that is flat is written at once; the walk goes on into the other
/// part, so the work stack grows only where both parts of a CONS are ropes.
fn write_flat<T: FlatUnit>(b: u64, dst: &mut [T]) {
    debug_assert_eq!(dst.len(), str_len(b));
    let mut work: Vec<(CellPtr, usize)> = Vec::new();
    let mut cur = Some((b, 0usize));
    while let Some((s, at)) = cur.take() {
        if let Some(v) = flat_units(s) {
            T::copy_from(&mut dst[at..at + v.len()], v);
        } else {
            let (first, second) = cons_parts(s);
            let mid = at + str_len(first);
            match (flat_units(first), flat_units(second)) {
                (Some(x), Some(y)) => {
                    T::copy_from(&mut dst[at..mid], x);
                    T::copy_from(&mut dst[mid..mid + y.len()], y);
                }
                (Some(x), None) => {
                    T::copy_from(&mut dst[at..mid], x);
                    cur = Some((second, mid));
                }
                (None, Some(y)) => {
                    T::copy_from(&mut dst[mid..mid + y.len()], y);
                    cur = Some((first, at));
                }
                (None, None) => {
                    work.push((cell(second), mid));
                    cur = Some((first, at));
                }
            }
        }
        if cur.is_none() {
            cur = work.pop().map(|(c, at)| (c.to_value().to_bits(), at));
        }
    }
}

/// The text of the string `b` (any kind) as a view for `f`: the cell's own units when it is
/// flat, else a Rust copy (Latin-1 when every leaf is). Not a GC point.
pub(crate) fn with_text<R>(b: u64, f: impl FnOnce(JsStr<'_>) -> R) -> R {
    if let Some(v) = flat_units(b) {
        return f(v);
    }
    if is_two_byte(b) {
        let mut buf = vec![0u16; str_len(b)];
        write_flat(b, &mut buf);
        f(JsStr::Utf16(&buf))
    } else {
        let mut buf = vec![0u8; str_len(b)];
        write_flat(b, &mut buf);
        f(JsStr::Latin1(&buf))
    }
}

/// The flat form of the string `s` (H19, V9-D13): `s` itself when it is SEQ or SLICED; for a
/// CONS the flat SEQ cell, which also becomes the CONS's first part (second = ""), so the
/// next flatten of the same CONS allocates nothing. GC point (the flat cell). The caller
/// holds every Value it needs after the call in a root (a Rust local: conservative scan).
pub(crate) fn flatten(cx: &mut Cx, s: u64) -> u64 {
    flatten_rt(cx.rt(), s)
}

/// [`flatten`] for callers that hold the runtime (GC point).
pub(crate) fn flatten_rt(rt: &Runtime, s: u64) -> u64 {
    debug_assert!(is_string(s));
    match cell_kind(s) {
        1 | 2 | 4 => s,
        3 => flatten_cons(rt, s),
        k => panic!("string kind {k} has no reader yet"),
    }
}

#[inline(never)]
fn flatten_cons(rt: &Runtime, s: u64) -> u64 {
    let c = cell(s);
    if str_len(rd(c, strf::SECOND)) == 0 {
        return rd(c, strf::FIRST);
    }
    let len = len_of(c);
    let two = is_two_byte(s);
    // GC point: `s` is in a Rust local (conservative scan), and it holds its parts.
    let f = alloc_string(rt, len, two);
    // SAFETY: the new cell has room for `len` units of its encoding at +16 (alloc_string);
    // no other reference to its data exists yet, and the parts are other cells.
    unsafe {
        if two {
            write_flat(
                s,
                core::slice::from_raw_parts_mut(f.field::<u16>(strf::DATA), len),
            );
        } else {
            write_flat(
                s,
                core::slice::from_raw_parts_mut(f.field::<u8>(strf::DATA), len),
            );
        }
    }
    // Same text, same hash field.
    set_hash_field_raw(f, hash_field_raw(c));
    let fb = f.to_value().to_bits();
    // Barriered stores: the CONS may be older than the last GC point.
    wr(&rt.heap, c, strf::FIRST, fb);
    wr(&rt.heap, c, strf::SECOND, rt.atom(Atom::Empty));
    fb
}

/// The code units of the flat string `s` (the result of [`flatten`], or any SEQ or SLICED
/// string). The view borrows `cx`, so it ends before the next `&mut Cx` call (a GC point);
/// [`Isolate::string_view`] shows the rule with a compile_fail test. Panics on a CONS that is
/// not flattened (debug and release: a read of its +16 would not be code units).
pub(crate) fn view<'c>(_cx: &'c Cx, s: u64) -> JsStr<'c> {
    units(s)
}

impl Isolate {
    /// The code units of `v` if it is a live string cell of this isolate's heap, without a
    /// copy (a rope is flattened first, H19: that allocation is the call's only GC point, and
    /// the flat form is kept); `None` for any other `v`. A raw `Value` is not a root, so the method checks `v`
    /// before it reads it (H20-Q4, Codex W4-1): a `Value` of a freed cell, of another heap
    /// or of an isolate that was dropped, or bits that do not start a cell, give `None`. If a
    /// freed cell's memory now holds a new live string, the view shows that string: the text
    /// is wrong, but the read is in bounds. The default build gives safe code no raw `Value`
    /// of a cell of this heap (no `Isolate::heap`, and `Value::from_cell_ptr` and
    /// `Value::from_bits_unchecked` are `unsafe`); the spike-native API does.
    ///
    /// The view does not root the string, so it borrows the isolate exclusively: no isolate
    /// call can run while it lives. That covers the `&mut self` calls that run JS or collect
    /// ([`Isolate::collect_garbage`]) and the `&self` ones (in the spike-native build,
    /// `Isolate::heap` gives `Heap::collect` and `Heap::alloc`). V8's `String::ValueView`
    /// has the same rule. Copy the text out first:
    ///
    /// ```
    /// use ep9_vm::{Isolate, IsolateConfig, Value};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let v = Value::UNDEFINED; // a raw string Value, for example from the spike-native API
    /// let text = iso.string_view(v).map(|s| s.to_string_lossy());
    /// iso.collect_garbage();
    /// assert_eq!(text, None);
    /// ```
    ///
    /// A cell of another heap is not read (the embedder can make an `ep9_heap::Heap` of its
    /// own):
    ///
    /// ```
    /// use ep9_heap::{CellKind, CellPtr, CellSize, Heap, HeapConfig, KindId, Tracer};
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// fn size(_: CellPtr) -> usize {
    ///     32
    /// }
    /// fn trace(_: CellPtr, _: &mut Tracer<'_>) {}
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let other = Heap::new(HeapConfig::default()).unwrap();
    /// // Kind 1 is the VM's SEQ_LATIN1 string kind; this cell has no string layout.
    /// let k = KindId::new(1).unwrap();
    /// other
    ///     .register_kind(k, CellKind { name: "fake", size: CellSize::Dynamic(size), trace })
    ///     .unwrap();
    /// let v = other.alloc(k, 32).unwrap().to_value();
    /// assert!(iso.string_view(v).is_none());
    /// drop(other);
    /// assert!(iso.string_view(v).is_none());
    /// ```
    ///
    /// Keeping the view across a GC point does not compile: across a `&mut self` call
    /// (`E0499`),
    ///
    /// ```compile_fail,E0499
    /// use ep9_vm::{Isolate, IsolateConfig, Value};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let v = Value::UNDEFINED;
    /// let view = iso.string_view(v);
    /// iso.collect_garbage();
    /// assert!(view.is_none());
    /// ```
    ///
    /// and across a `&self` call, also with the views stored on the Rust heap where the
    /// conservative scan does not see them (`E0502`):
    ///
    /// ```compile_fail,E0502
    /// use ep9_vm::{Isolate, IsolateConfig, Value};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let v = Value::UNDEFINED;
    /// let mut views = Vec::new();
    /// views.extend(iso.string_view(v));
    /// let _ = iso.gc_stats();
    /// assert!(views.is_empty());
    /// ```
    pub fn string_view(&mut self, v: Value) -> Option<JsStr<'_>> {
        let b = v.to_bits();
        // `Heap::is_cell` finishes the lazy sweep, then accepts only the start of a live cell
        // in this heap's cage, so `is_string` reads the word0 of a live cell (H20-Q4).
        let live = is_cell_bits(b) && self.rt.heap.is_cell(b as usize);
        if !(live && is_string(b)) {
            return None;
        }
        // GC point for a rope only: `b` is in this frame (conservative scan).
        let f = flatten_rt(&self.rt, b);
        Some(view(&self.cx, f))
    }
}

/// The string of `s[i]` (`i < str_len(s)`): the single-character atom for a Latin-1 code unit
/// (one table per isolate, filled lazily), a new string otherwise. GC point (a rope is
/// flattened first, so a loop over `s[i]` reads it in O(1) per unit).
pub(crate) fn char_at(rt: &Runtime, s: u64, i: usize) -> u64 {
    let s = flatten_rt(rt, s);
    let u = units(s).at(i);
    match u8::try_from(u) {
        Ok(c) => single_char(rt, c),
        Err(_) => new_string_utf16(rt, &[u]),
    }
}

/// The atom of the one-character Latin-1 string `c`. GC point on the first use of `c`.
pub(crate) fn single_char(rt: &Runtime, c: u8) -> u64 {
    let a = rt.single_char(c);
    if a != UNDEFINED {
        return a;
    }
    let a = crate::atoms::intern_latin1(rt, &[c]);
    rt.set_single_char(c, a);
    a
}

// ---- allocation ------------------------------------------------------------------------------

/// A new string cell of `len` code units (data not written yet). GC point. The caller checks
/// `len <= MAX_STRING_LENGTH` for lengths that JS code controls ([`concat()`] does).
fn alloc_string(rt: &Runtime, len: usize, utf16: bool) -> CellPtr {
    assert!(
        len <= MAX_STRING_LENGTH,
        "string of {len} code units is over the limit"
    );
    let (k, size) = if utf16 {
        (kind::SEQ_UTF16, strf::DATA + 2 * len)
    } else {
        (kind::SEQ_LATIN1, strf::DATA + len)
    };
    let c = rt.alloc(k, size);
    // SAFETY: the new cell has at least 16 bytes; +8 and +12 are its length and hash field.
    unsafe {
        c.field::<u32>(strf::LENGTH).write(len as u32);
        c.field::<u32>(strf::HASH).write(0);
    }
    c
}

pub(crate) fn new_string_latin1(rt: &Runtime, bytes: &[u8]) -> u64 {
    let c = alloc_string(rt, bytes.len(), false);
    // SAFETY: the new cell has room for `len` bytes at +16 (alloc_string).
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), c.field::<u8>(strf::DATA), bytes.len())
    };
    c.to_value().to_bits()
}

/// A string from UTF-16 code units. Latin-1 storage when every unit fits.
pub(crate) fn new_string_utf16(rt: &Runtime, units: &[u16]) -> u64 {
    if units.iter().all(|&u| u <= 0xFF) {
        let bytes: Vec<u8> = units.iter().map(|&u| u as u8).collect();
        return new_string_latin1(rt, &bytes);
    }
    new_seq_utf16(rt, units)
}

/// A SEQ_UTF16 string of `units` (the caller checked that one unit is above 0xFF, or a
/// test wants the cross-encoding case).
fn new_seq_utf16(rt: &Runtime, units: &[u16]) -> u64 {
    let c = alloc_string(rt, units.len(), true);
    // SAFETY: the new cell has room for `len` u16 at +16 (8-aligned).
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), c.field::<u16>(strf::DATA), units.len())
    };
    c.to_value().to_bits()
}

pub(crate) fn new_string(rt: &Runtime, s: &str) -> u64 {
    if s.is_ascii() {
        return new_string_latin1(rt, s.as_bytes());
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    new_string_utf16(rt, &units)
}

/// `a + b` for two strings. `None` if the result is longer than [`MAX_STRING_LENGTH`]; the
/// caller throws RangeError "Invalid string length". GC point. A result of at least
/// [`MIN_CONS_LENGTH`] code units is a CONS cell (O(1); H19), a shorter one a flat copy.
pub(crate) fn concat(rt: &Runtime, a: u64, b: u64) -> Option<u64> {
    let (la, lb) = (str_len(a), str_len(b));
    if lb == 0 {
        return Some(a);
    }
    if la == 0 {
        return Some(b);
    }
    let len = la.checked_add(lb).filter(|&n| n <= MAX_STRING_LENGTH)?;
    if len >= MIN_CONS_LENGTH {
        return Some(new_cons(rt, a, b, len));
    }
    // Both parts are shorter than MIN_CONS_LENGTH, so they are flat (SEQ); `write_flat`
    // reads any kind anyway.
    let two = is_two_byte(a) || is_two_byte(b);
    let c = alloc_string(rt, len, two);
    // SAFETY: the new cell has room for `len` units of its encoding at +16 (alloc_string);
    // the sources are live strings (the caller holds them) and other cells.
    unsafe {
        if two {
            let d = core::slice::from_raw_parts_mut(c.field::<u16>(strf::DATA), len);
            write_flat(a, &mut d[..la]);
            write_flat(b, &mut d[la..]);
        } else {
            let d = core::slice::from_raw_parts_mut(c.field::<u8>(strf::DATA), len);
            write_flat(a, &mut d[..la]);
            write_flat(b, &mut d[la..]);
        }
    }
    Some(c.to_value().to_bits())
}

/// A new CONS cell `first + second` of `len` code units (both parts non-empty). GC point.
fn new_cons(rt: &Runtime, first: u64, second: u64, len: usize) -> u64 {
    debug_assert!(len == str_len(first) + str_len(second) && len <= MAX_STRING_LENGTH);
    let two = is_two_byte(first) || is_two_byte(second);
    // GC point: `first` and `second` are in this frame (conservative scan).
    let c = rt.alloc(kind::CONS, strf::ROPE_SIZE);
    // SAFETY: the new cell is 32 bytes; +8 and +12 are its length and hash field.
    unsafe {
        c.field::<u32>(strf::LENGTH).write(len as u32);
        c.field::<u32>(strf::HASH).write(0);
        if two {
            let h = c.header();
            h.set_flags(h.flags() | FLAG_TWO_BYTE);
        }
    }
    // E2: initializing stores, no GC point since the allocation.
    init(c, strf::FIRST, first);
    init(c, strf::SECOND, second);
    c.to_value().to_bits()
}

/// The substring `s[from..to]` (`from <= to <= str_len(s)`), for String.prototype.substring,
/// slice and the like. GC point. A result of at least [`MIN_SLICE_LENGTH`] code units is a
/// SLICED cell over the flat parent (O(1) after the flatten; H19), a shorter one a copy.
#[cfg_attr(not(test), allow(dead_code))] // caller: the Cx substring ABI (fixer, M4-P1)
pub(crate) fn substring(rt: &Runtime, s: u64, from: usize, to: usize) -> u64 {
    let n = str_len(s);
    assert!(from <= to && to <= n, "substring {from}..{to} of {n} units");
    let len = to - from;
    if len == n {
        return s;
    }
    if len == 0 {
        return rt.atom(Atom::Empty);
    }
    // GC point for a rope: `s` is in this frame.
    let f = flatten_rt(rt, s);
    if len < MIN_SLICE_LENGTH {
        let r = match units(f) {
            JsStr::Latin1(x) => new_string_latin1(rt, &x[from..to]),
            JsStr::Utf16(x) => new_string_utf16(rt, &x[from..to]),
        };
        return r;
    }
    let (parent, off) = if cell_kind(f) == kind::SLICED.get() {
        let c = cell(f);
        (
            rd(c, strf::PARENT),
            int32_of(rd(c, strf::OFFSET)) as usize + from,
        )
    } else {
        (f, from)
    };
    // GC point: `parent` is in this frame.
    let c = rt.alloc(kind::SLICED, strf::ROPE_SIZE);
    // SAFETY: the new cell is 32 bytes; +8 and +12 are its length and hash field.
    unsafe {
        c.field::<u32>(strf::LENGTH).write(len as u32);
        c.field::<u32>(strf::HASH).write(0);
    }
    // E2: initializing stores. The offset is below 2^29, so it fits an int32.
    init(c, strf::PARENT, parent);
    init(c, strf::OFFSET, int32_bits(off as i32));
    c.to_value().to_bits()
}

// ---- atoms, hashes and keys (used by atoms.rs and key.rs) ------------------------------------

/// True if `b` is an atom: a string cell with the ATOM flag (any string kind; a rope that
/// `atoms::intern` met keeps its kind).
#[inline]
pub(crate) fn is_atom(b: u64) -> bool {
    if !is_cell_bits(b) {
        return false;
    }
    // SAFETY: a cell Value held by the VM is live (crate invariant, object.rs).
    let h = unsafe { cell(b).header() };
    matches!(h.kind(), 1..=7) && h.flags() & FLAG_ATOM != 0
}

/// Marks the string `b` as an atom. Only `atoms.rs` calls it, after it put `b` in the table.
pub(crate) fn set_atom(b: u64) {
    debug_assert!(is_string(b) && hash_field_raw(cell(b)) & HASH_COMPUTED != 0);
    // SAFETY: `b` is a live string cell (caller contract).
    let h = unsafe { cell(b).header() };
    h.set_flags(h.flags() | FLAG_ATOM);
}

/// The hash field of the string `b`, computed and cached on first use. Not a GC point.
pub(crate) fn hash_field(b: u64) -> u32 {
    let c = cell(b);
    let f = hash_field_raw(c);
    if f & HASH_COMPUTED != 0 {
        return f;
    }
    let f = match flat_units(b) {
        Some(v) => v.hash_field(),
        None => rope_hash_field(b),
    };
    set_hash_field_raw(c, f);
    f
}

/// The hash field of a CONS that is not flattened, by a walk (no JS-heap allocation).
#[inline(never)]
fn rope_hash_field(b: u64) -> u32 {
    // An array index has at most 10 digits; a rope that short (none today: CONS cells have
    // at least MIN_CONS_LENGTH units) takes the flat path on a copy.
    if str_len(b) <= 10 {
        return with_text(b, |t| t.hash_field());
    }
    let h = sip13_iter(sip_keys(), str_len(b), Units::new(b));
    HASH_COMPUTED | ((h as u32 & HASH_MASK) << HASH_SHIFT)
}

/// Stores the hash field of a new string whose text was hashed before allocation.
pub(crate) fn set_hash_field(b: u64, field: u32) {
    debug_assert!(field & HASH_COMPUTED != 0);
    debug_assert_eq!(field, with_text(b, |t| t.hash_field()));
    set_hash_field_raw(cell(b), field);
}

/// The 30-bit table hash for a hash field: the stored hash, or for a cached index the hash of
/// the index value.
pub(crate) fn table_hash(field: u32) -> u32 {
    debug_assert!(field & HASH_COMPUTED != 0);
    if field & HASH_IS_INDEX != 0 {
        let i = field >> HASH_SHIFT;
        sip13_word(u64::from(i)) as u32 & HASH_MASK
    } else {
        field >> HASH_SHIFT
    }
}

/// The 30-bit hash of the string `b` (atom tables, shape indexes). Not a GC point.
pub(crate) fn hash(b: u64) -> u32 {
    table_hash(hash_field(b))
}

/// The hash field and the table hash of a text before it is a string cell.
pub(crate) fn hash_text(text: JsStr<'_>) -> (u32, u32) {
    let f = text.hash_field();
    (f, table_hash(f))
}

/// The array index that the string `b` names, if any. Uses the cached hash field when it is
/// computed (atoms always have it).
pub(crate) fn array_index(b: u64) -> Option<u32> {
    let c = cell(b);
    let f = hash_field_raw(c);
    if f & HASH_COMPUTED != 0 {
        if f & HASH_IS_INDEX != 0 {
            return Some(f >> HASH_SHIFT);
        }
        // Every index below 10^7 < 2^24 is cached, so a short string is not an index.
        if len_of(c) <= 7 {
            return None;
        }
    }
    // An index has at most 10 digits (2^32 - 2), so a longer string (every rope) is not one:
    // the answer needs no copy of a rope's text (the profile hooks call this; they allocate
    // nothing, `profile.rs`).
    if len_of(c) > 10 {
        return None;
    }
    match flat_units(b) {
        Some(v) => v.array_index(),
        None => with_text(b, |t| t.array_index()),
    }
}

/// Equal code units (any string kinds; not a GC point).
pub(crate) fn equals(a: u64, b: u64) -> bool {
    if a == b {
        return true;
    }
    match (flat_units(a), flat_units(b)) {
        (Some(x), Some(y)) => x.eq(&y),
        _ => rope_equals(a, b),
    }
}

#[inline(never)]
fn rope_equals(a: u64, b: u64) -> bool {
    if str_len(a) != str_len(b) {
        return false;
    }
    let (fa, fb) = (hash_field_raw(cell(a)), hash_field_raw(cell(b)));
    if fa & fb & HASH_COMPUTED != 0 && fa != fb {
        return false;
    }
    Units::new(a).eq(Units::new(b))
}

/// Equal code units with a text that is not a cell (any string kind; not a GC point).
pub(crate) fn equals_text(a: u64, text: JsStr<'_>) -> bool {
    match flat_units(a) {
        Some(v) => v.eq(&text),
        None => str_len(a) == text.len() && Units::new(a).eq((0..text.len()).map(|i| text.at(i))),
    }
}

/// Code-unit order of two strings (IsLessThan, §7.2.13; any kinds; not a GC point).
pub(crate) fn compare(a: u64, b: u64) -> core::cmp::Ordering {
    match (flat_units(a), flat_units(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => Units::new(a).cmp(Units::new(b)),
    }
}

/// The text of the string `b` (any kind; lone surrogates become U+FFFD). Not a GC point.
pub(crate) fn to_rust_string(b: u64) -> String {
    with_text(b, |t| t.to_string_lossy())
}

// ---- the hash function -------------------------------------------------------------------------

/// SipHash keys of this process: from `EP9_HASH_SEED` if set, random otherwise.
fn sip_keys() -> (u64, u64) {
    static KEYS: OnceLock<(u64, u64)> = OnceLock::new();
    *KEYS.get_or_init(|| {
        let seed = match std::env::var_os("EP9_HASH_SEED") {
            Some(s) => parse_seed(&s.to_string_lossy()),
            None => random_seed(),
        };
        let k0 = splitmix64(seed);
        (k0, splitmix64(k0))
    })
}

/// `EP9_HASH_SEED`: a decimal or `0x` hexadecimal u64; any other text is hashed (FNV-1a) into
/// a seed, so every value gives a fixed seed.
pub(crate) fn parse_seed(s: &str) -> u64 {
    let t = s.trim();
    let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => t.parse::<u64>().ok(),
    };
    parsed.unwrap_or_else(|| {
        t.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    })
}

/// 64 random bits from the OS (std seeds `RandomState` from the OS random source).
fn random_seed() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(0x6570_3968_6173_6821_u64)
}

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

struct Sip {
    v: [u64; 4],
}

impl Sip {
    fn new((k0, k1): (u64, u64)) -> Sip {
        Sip {
            v: [
                k0 ^ 0x736f_6d65_7073_6575,
                k1 ^ 0x646f_7261_6e64_6f6d,
                k0 ^ 0x6c79_6765_6e65_7261,
                k1 ^ 0x7465_6462_7974_6573,
            ],
        }
    }

    #[inline(always)]
    fn round(&mut self) {
        let [mut v0, mut v1, mut v2, mut v3] = self.v;
        v0 = v0.wrapping_add(v1);
        v1 = v1.rotate_left(13) ^ v0;
        v0 = v0.rotate_left(32);
        v2 = v2.wrapping_add(v3);
        v3 = v3.rotate_left(16) ^ v2;
        v0 = v0.wrapping_add(v3);
        v3 = v3.rotate_left(21) ^ v0;
        v2 = v2.wrapping_add(v1);
        v1 = v1.rotate_left(17) ^ v2;
        v2 = v2.rotate_left(32);
        self.v = [v0, v1, v2, v3];
    }

    /// One 8-byte message word (SipHash-1-3: one compression round).
    #[inline(always)]
    fn word(&mut self, m: u64) {
        self.v[3] ^= m;
        self.round();
        self.v[0] ^= m;
    }

    /// The last word (tail bytes plus the byte length in the top byte), then 3 rounds.
    fn finish(mut self, last: u64) -> u64 {
        self.word(last);
        self.v[2] ^= 0xff;
        self.round();
        self.round();
        self.round();
        self.v[0] ^ self.v[1] ^ self.v[2] ^ self.v[3]
    }
}

/// SipHash-1-3 of the code units as UTF-16LE bytes, with explicit keys.
fn sip13_units_with<T: Copy + Into<u64>>(keys: (u64, u64), units: &[T]) -> u64 {
    let mut s = Sip::new(keys);
    let w = |u: T| -> u64 { u.into() };
    let (blocks, tail) = units.as_chunks::<4>();
    for &[a, b, c, d] in blocks {
        s.word(w(a) | w(b) << 16 | w(c) << 32 | w(d) << 48);
    }
    let mut last = ((2 * units.len()) as u64 & 0xff) << 56;
    for (j, &u) in tail.iter().enumerate() {
        last |= w(u) << (16 * j);
    }
    s.finish(last)
}

/// [`sip13_units_with`] over `len` code units from an iterator (a rope walk).
fn sip13_iter(keys: (u64, u64), len: usize, units: impl Iterator<Item = u16>) -> u64 {
    let mut s = Sip::new(keys);
    let (mut word, mut n) = (0u64, 0u32);
    for u in units {
        word |= u64::from(u) << (16 * n);
        n += 1;
        if n == 4 {
            s.word(word);
            (word, n) = (0, 0);
        }
    }
    s.finish(word | ((2 * len) as u64 & 0xff) << 56)
}

/// The process hash of code units (Latin-1 bytes or UTF-16 units hash alike).
fn sip13_units<T: Copy + Into<u64>>(units: &[T]) -> u64 {
    sip13_units_with(sip_keys(), units)
}

/// The process hash of one u64 (cached array indices).
fn sip13_word(w: u64) -> u64 {
    let mut s = Sip::new(sip_keys());
    s.word(w);
    s.finish(8 << 56)
}

#[cfg(test)]
pub(crate) mod testing {
    //! Hooks for tests/strings.rs.
    use super::*;

    pub(crate) fn sip13(keys: (u64, u64), units: &[u16]) -> u64 {
        sip13_units_with(keys, units)
    }

    pub(crate) fn sip13_latin1(keys: (u64, u64), bytes: &[u8]) -> u64 {
        sip13_units_with(keys, bytes)
    }

    pub(crate) fn keys() -> (u64, u64) {
        sip_keys()
    }

    pub(crate) fn raw_hash_field(b: u64) -> u32 {
        hash_field_raw(cell(b))
    }

    pub(crate) fn set_raw_hash_field(b: u64, f: u32) {
        set_hash_field_raw(cell(b), f);
    }

    /// A SEQ_UTF16 cell even when every unit fits Latin-1.
    pub(crate) fn new_utf16_cell(rt: &Runtime, units: &[u16]) -> u64 {
        new_seq_utf16(rt, units)
    }
}
