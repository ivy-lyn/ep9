//! Strings (week-3 layout; H19 replaces it). Flat only: one kind for both encodings.
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | STRING | 1 | +8 length (int32), +16 encoding (int32: 0 Latin-1, 1 UTF-16), +24 code units | 24 + len or 24 + 2 len |
//!
//! Read API for new code: [`flatten`] (a GC point; identity while every string is flat), then
//! [`view`], whose borrow of the `Cx` ends at the next `&mut Cx` call. [`str_view`] lets the
//! caller pick the lifetime; its call sites move to `view` (H19, by M1a).

use ep9_heap::{CellKind, CellPtr, CellSize, Heap};

use crate::frame::int32_bits;
use crate::interp::Cx;
use crate::kinds::{self as kind, is_string};
use crate::object::{cell, init, rd_i32};
use crate::runtime::Runtime;

mod strf {
    pub const LENGTH: usize = 8;
    pub const ENCODING: usize = 16;
    pub const DATA: usize = 24;
}

/// Largest string length in code units (V8: 2^29 - 24).
pub(crate) const MAX_STRING_LENGTH: usize = (1 << 29) - 24;

fn trace_none(_: CellPtr, _: &mut ep9_heap::Tracer<'_>) {}

pub(crate) fn string_size(c: CellPtr) -> usize {
    let len = rd_i32(c, strf::LENGTH) as usize;
    let enc = rd_i32(c, strf::ENCODING) as usize;
    strf::DATA + (len << enc)
}

/// Registers the string kind.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::STRING,
        CellKind {
            name: "string",
            size: CellSize::Dynamic(string_size),
            trace: trace_none,
        },
    )
    .expect("register a VM cell kind");
}

/// The code units of a string (H19 `JsStr`, flat only).
#[derive(Clone, Copy, Debug)]
pub(crate) enum JsStr<'a> {
    Latin1(&'a [u8]),
    Utf16(&'a [u16]),
}

impl<'a> JsStr<'a> {
    pub fn len(&self) -> usize {
        match self {
            JsStr::Latin1(b) => b.len(),
            JsStr::Utf16(u) => u.len(),
        }
    }

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

    pub fn eq(&self, other: &JsStr<'_>) -> bool {
        match (self, other) {
            (JsStr::Latin1(a), JsStr::Latin1(b)) => a == b,
            (JsStr::Utf16(a), JsStr::Utf16(b)) => a == b,
            _ => self.len() == other.len() && (0..self.len()).all(|i| self.at(i) == other.at(i)),
        }
    }

    /// Code-unit order (IsLessThan on strings, §7.2.13).
    pub fn cmp(&self, other: &JsStr<'_>) -> core::cmp::Ordering {
        let n = self.len().min(other.len());
        for i in 0..n {
            match self.at(i).cmp(&other.at(i)) {
                core::cmp::Ordering::Equal => {}
                o => return o,
            }
        }
        self.len().cmp(&other.len())
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
}

/// The code units of a live string cell. The caller keeps the cell live while it uses the
/// view (strings never move or change).
pub(crate) fn str_view<'a>(b: u64) -> JsStr<'a> {
    debug_assert!(is_string(b));
    let c = cell(b);
    let len = rd_i32(c, strf::LENGTH) as usize;
    let utf16 = rd_i32(c, strf::ENCODING) == 1;
    // SAFETY: a live string cell holds `len` code units of its encoding from offset 24
    // (layout table); its size covers them (string_size).
    unsafe {
        if utf16 {
            JsStr::Utf16(core::slice::from_raw_parts(c.field::<u16>(strf::DATA), len))
        } else {
            JsStr::Latin1(core::slice::from_raw_parts(c.field::<u8>(strf::DATA), len))
        }
    }
}

pub(crate) fn str_len(b: u64) -> usize {
    rd_i32(cell(b), strf::LENGTH) as usize
}

/// The flat form of the string `s` (H19). A GC point once strings can be ropes; the identity
/// while every string is flat.
#[allow(dead_code)] // wk4-stub
pub(crate) fn flatten(_cx: &mut Cx, s: u64) -> u64 {
    debug_assert!(is_string(s));
    s
}

/// The code units of the flat string `s`. The view borrows `cx`, so it ends before the next
/// `&mut Cx` call (a GC point).
#[allow(dead_code)] // wk4-stub
pub(crate) fn view<'c>(_cx: &'c Cx, s: u64) -> JsStr<'c> {
    str_view(s)
}

/// The one-code-unit string of `s[i]` (`i < str_len(s)`). GC point.
pub(crate) fn char_at(rt: &Runtime, s: u64, i: usize) -> u64 {
    let u = [str_view(s).at(i)];
    new_string_utf16(rt, &u)
}

fn alloc_string(rt: &Runtime, len: usize, utf16: bool) -> CellPtr {
    assert!(len <= MAX_STRING_LENGTH, "string too long");
    let size = strf::DATA + (len << usize::from(utf16));
    let c = rt.alloc(kind::STRING, size.max(16));
    init(c, strf::LENGTH, int32_bits(len as i32));
    init(c, strf::ENCODING, int32_bits(i32::from(utf16)));
    c
}

pub(crate) fn new_string_latin1(rt: &Runtime, bytes: &[u8]) -> u64 {
    let c = alloc_string(rt, bytes.len(), false);
    // SAFETY: the new cell has room for `len` bytes at offset 24 (alloc_string).
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
    let c = alloc_string(rt, units.len(), true);
    // SAFETY: the new cell has room for `len` u16 at offset 24 (8-aligned).
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

/// `a + b` for two strings. `None` if the result is too long (RangeError).
pub(crate) fn concat(rt: &Runtime, a: u64, b: u64) -> Option<u64> {
    let (la, lb) = (str_len(a), str_len(b));
    if lb == 0 {
        return Some(a);
    }
    if la == 0 {
        return Some(b);
    }
    let len = la.checked_add(lb).filter(|&n| n <= MAX_STRING_LENGTH)?;
    let (va, vb) = (str_view(a), str_view(b));
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
