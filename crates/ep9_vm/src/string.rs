//! Strings (H19 v0): flat sequential strings, the hash field, the string hash and the read API.
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | SEQ_LATIN1 | 1 | +8 length (u32), +12 hash field (u32), +16 bytes | 16 + len |
//! | SEQ_UTF16 | 2 | +8 length (u32), +12 hash field (u32), +16 UTF-16 code units | 16 + 2 len |
//!
//! - The words after word0 are raw data, not Values. String kinds trace nothing, and the size
//!   comes from the kind and the length (H9 "heap always parsable").
//! - word0 flag bit 0 ([`crate::kinds::FLAG_ATOM`]) marks an atom: the one interned cell of
//!   its text in this isolate (`atoms.rs`).
//! - Kinds 3-7 (cons, sliced, thin, external) are reserved for M1 and share this header, so
//!   the length is always the u32 at +8.
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
//! and UTF-16 forms of one text hash alike. The key is chosen once per process, at random,
//! or from `EP9_HASH_SEED`. A hash never goes into bytecode, units, snapshots or property
//! order (W4-HASH, H12).
//!
//! Read API for new code: [`flatten`] (a GC point once ropes exist; the identity in v0),
//! then [`view`], whose borrow of the `Cx` ends at the next `&mut Cx` call. [`str_view`] lets
//! the caller pick the lifetime; its call sites move to `view` by M1a (W4-STRVIEW). VM code
//! may still hold a `JsStr` across an allocation because the heap never moves cells and the
//! conservative scan keeps interior pointers (H19, `crates/ep9_heap/src/trace.rs` find_cell).

use std::sync::OnceLock;

use ep9_heap::{CellKind, CellPtr, CellSize, Heap};
use ep9_value::Value;

use crate::frame::UNDEFINED;
use crate::interp::Cx;
use crate::isolate::Isolate;
use crate::kinds::{self as kind, FLAG_ATOM, cell_kind, is_cell_bits, is_string};
use crate::object::cell;
use crate::runtime::Runtime;

mod strf {
    pub const LENGTH: usize = 8;
    pub const HASH: usize = 12;
    pub const DATA: usize = 16;
}

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

/// The code units of a live string cell, for any lifetime the caller picks. The caller keeps
/// the cell live while it uses the view (cells never move; strings never change). Legacy
/// API outside this file: new code uses [`flatten`] and [`view`] (W4-STRVIEW).
pub(crate) fn str_view<'a>(b: u64) -> JsStr<'a> {
    units(b)
}

/// The code units of a live flat string cell (this file's own reads: hashing, compares,
/// [`view`]).
fn units<'a>(b: u64) -> JsStr<'a> {
    debug_assert!(is_string(b));
    let c = cell(b);
    let len = len_of(c);
    // SAFETY: a live string cell holds `len` code units of its kind's encoding from +16
    // (layout table), and its size covers them (`latin1_size`, `utf16_size`). +16 is
    // 8-aligned, so the u16 slice is aligned.
    unsafe {
        if c.header().kind() == kind::SEQ_UTF16.get() {
            JsStr::Utf16(core::slice::from_raw_parts(c.field::<u16>(strf::DATA), len))
        } else {
            JsStr::Latin1(core::slice::from_raw_parts(c.field::<u8>(strf::DATA), len))
        }
    }
}

/// The length of the string `b` in code units.
pub(crate) fn str_len(b: u64) -> usize {
    len_of(cell(b))
}

/// The flat form of the string `s` (H19). A GC point once strings can be ropes (M1); the
/// identity in v0, where every string is SEQ_LATIN1 or SEQ_UTF16.
pub(crate) fn flatten(_cx: &mut Cx, s: u64) -> u64 {
    debug_assert!(matches!(cell_kind(s), 1 | 2), "v0 strings are flat");
    s
}

/// The code units of the flat string `s`. The view borrows `cx`, so it ends before the next
/// `&mut Cx` call (a GC point); [`Isolate::string_view`] shows the rule with a compile_fail
/// test.
pub(crate) fn view<'c>(_cx: &'c Cx, s: u64) -> JsStr<'c> {
    units(s)
}

impl Isolate {
    /// The code units of `v` if it is a string, without a copy. The view does not root the
    /// string, so it borrows the isolate exclusively: no isolate call can run while it lives.
    /// That covers the `&mut self` calls that run JS or collect ([`Isolate::collect_garbage`])
    /// and the `&self` ones that allocate or collect ([`Isolate::heap`] gives `Heap::collect`
    /// and `Heap::alloc`). V8's `String::ValueView` has the same rule. A raw `Value` is not a
    /// root: `v` must come from the latest call into the isolate. Copy the text out first:
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
    /// across a `&self` call that collects, also with the views stored on the Rust heap where
    /// the conservative scan does not see them (`E0502`),
    ///
    /// ```compile_fail,E0502
    /// use ep9_vm::{Isolate, IsolateConfig, Value};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let v = Value::UNDEFINED;
    /// let mut views = Vec::new();
    /// views.extend(iso.string_view(v));
    /// iso.heap().collect();
    /// assert!(views.is_empty());
    /// ```
    ///
    /// and across a `&self` call that allocates (`E0502`):
    ///
    /// ```compile_fail,E0502
    /// use ep9_vm::{Isolate, IsolateConfig, Value};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let v = Value::UNDEFINED;
    /// let view = iso.string_view(v);
    /// let _ = iso.heap().alloc(ep9_heap::KindId::new(1).unwrap(), 16);
    /// assert!(view.is_none());
    /// ```
    pub fn string_view(&mut self, v: Value) -> Option<JsStr<'_>> {
        let b = v.to_bits();
        is_string(b).then(|| view(&self.cx, b))
    }
}

/// The string of `s[i]` (`i < str_len(s)`): the single-character atom for a Latin-1 code unit
/// (one table per isolate, filled lazily), a new string otherwise. GC point.
pub(crate) fn char_at(rt: &Runtime, s: u64, i: usize) -> u64 {
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
/// caller throws RangeError "Invalid string length".
pub(crate) fn concat(rt: &Runtime, a: u64, b: u64) -> Option<u64> {
    let (la, lb) = (str_len(a), str_len(b));
    if lb == 0 {
        return Some(a);
    }
    if la == 0 {
        return Some(b);
    }
    let len = la.checked_add(lb).filter(|&n| n <= MAX_STRING_LENGTH)?;
    let (va, vb) = (units(a), units(b));
    // v0: flat concatenation (H19 adds ropes for length >= 13).
    let r = match (va, vb) {
        (JsStr::Latin1(x), JsStr::Latin1(y)) => {
            let c = alloc_string(rt, len, false);
            // SAFETY: the new cell has room for la + lb bytes; the sources are live strings
            // (the caller holds them) and do not overlap the new cell.
            unsafe {
                let d = c.field::<u8>(strf::DATA);
                core::ptr::copy_nonoverlapping(x.as_ptr(), d, la);
                core::ptr::copy_nonoverlapping(y.as_ptr(), d.add(la), lb);
            }
            c
        }
        _ => {
            let c = alloc_string(rt, len, true);
            // SAFETY: the new cell has room for len u16 code units.
            let d = unsafe { core::slice::from_raw_parts_mut(c.field::<u16>(strf::DATA), len) };
            for (i, x) in d.iter_mut().enumerate() {
                *x = if i < la { va.at(i) } else { vb.at(i - la) };
            }
            c
        }
    };
    Some(r.to_value().to_bits())
}

// ---- atoms, hashes and keys (used by atoms.rs and key.rs) ------------------------------------

/// True if `b` is an atom: a string cell with the ATOM flag.
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
    let f = units(b).hash_field();
    set_hash_field_raw(c, f);
    f
}

/// Stores the hash field of a new string whose text was hashed before allocation.
pub(crate) fn set_hash_field(b: u64, field: u32) {
    debug_assert!(field & HASH_COMPUTED != 0);
    debug_assert_eq!(field, units(b).hash_field());
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
    units(b).array_index()
}

/// Equal code units.
pub(crate) fn equals(a: u64, b: u64) -> bool {
    a == b || units(a).eq(&units(b))
}

/// Equal code units with a text that is not a cell.
pub(crate) fn equals_text(a: u64, text: JsStr<'_>) -> bool {
    units(a).eq(&text)
}

/// The text of the string `b` (lone surrogates become U+FFFD).
pub(crate) fn to_rust_string(b: u64) -> String {
    units(b).to_string_lossy()
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
