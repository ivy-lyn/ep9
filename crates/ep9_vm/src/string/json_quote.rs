//! QuoteJSONString (§25.5.2.3) on borrowed string payloads, and the text buffer of one
//! JSON.stringify call (OXC-PERF wave 3 W3-JSON-QUOTE).
//!
//! ```text
//! JsonText      One(Vec<u8>) until the first code unit above 0xFF, then Two(Vec<u16>)
//!               (widen: one pass, at most once per stringify call)
//! quote_latin1  Latin-1 source -> One (the main path) or Two (a widening copy)
//! quote_utf16   UTF-16 source  -> One (stops at the first unit above 0xFF that is not a lone
//!               surrogate: widen, then go on as Two from that unit) or Two
//! per chunk     at most CHUNK source units: one reserve of chunk * 6 + 2 + SLACK units, then
//!               raw-pointer stores with no capacity check, and one set_len at the chunk end
//! a body        load W units, store all W (the reserve covers the slack), then the mask of
//!               the units that need an escape; mask 0: advance W; else advance to the first
//!               set unit and escape a run of units in a scalar loop until a clean unit
//! the tail      no read past the end: one overlapped last load at end - W when the units
//!               before the current one were copied as they are (so the store at the same
//!               offset back writes the same units again), else SWAR on u64, then scalar
//! ```
//!
//! A unit needs an escape when it is below 0x20, `"` (0x22) or `\` (0x5C) (Table 77 and
//! step 2.b of the spec). A UTF-16 source also stops at a surrogate ((u & 0xF800) == 0xD800):
//! a scalar check copies a valid pair as it is and escapes a lone one as `\udxxx` (lowercase
//! hex). For a one-byte output it also stops at a unit above 0xFF. 0x7F and 0x80-0xFF are
//! copied as they are; a Latin-1 source stays one-byte. A lone surrogate escapes to ASCII, so
//! it does not force a widen; only a valid pair or another unit above 0xFF does.
//!
//! One body per instruction set; [`Kernel::detect`] picks one at run time, once per process
//! (cached in an atomic). Every body gives the result of [`Path::Scalar`], the per-unit loop
//! that `ep9_builtins` `json/stringify.rs` `quote` ran before this module (the FASTDIFF
//! switched-off path):
//!
//! | target | bodies | how the body is picked |
//! |---|---|---|
//! | x86-64 | AVX2 (32 bytes), SSE2 (16 bytes; the x86-64 base), SWAR, scalar | `is_x86_feature_detected!("avx2")`; AVX2 only for a chunk of at least 32 units |
//! | aarch64 | NEON (16 bytes; the aarch64 base), SWAR, scalar | `is_aarch64_feature_detected!("neon")` |
//! | wasm32 | simd128 (16 bytes), SWAR, scalar | at compile time: `cfg(target_feature = "simd128")` |
//! | other | SWAR, scalar | - |
//!
//! No AVX-512 body: the hot strings are short (object keys average 7.4 units in the
//! json-stringify-inspector payload), so a 64-byte vector has no step to take; backlog.
//! `EP9_SIMD_JSON` (`scalar`, `swar`, `sse2`, `avx2`, `neon`, `simd128`), read at the first
//! detect, picks another body for a test run; a body that the CPU or the build does not have
//! is never picked (the detected one is used).
//!
//! Techniques (M6-ORCH-858: learned, not copied; the escape set is the spec's table):
//! - One-byte output first, one widen to two-byte: V8 40f4b69c3b
//!   src/json/json-stringifier.cc:1828-1842 (`ChangeEncoding`) and :3590-3620
//!   (`FastJsonStringify`: a one-byte result string copied in one pass).
//! - The reserve of the worst case (6 units per source unit) before raw stores: escape-simd
//!   26e04a0 crates/escape-simd/src/json.rs:17-26 (`escape_into`); V8 json-stringifier.cc:148-157
//!   (`EscapedLengthIfCurrentPartFits`).
//! - The store before the check: escape-simd 26e04a0 crates/escape-simd/src/simd/neon.rs:242-256
//!   and avx2.rs:244-258 (from commit d4d94e3).
//! - The escape table of (length, 8 padded bytes), all 8 written and the pointer moved by the
//!   length: escape-simd 26e04a0 crates/escape-simd/src/simd/util.rs:30-57; V8
//!   json-stringifier.cc:463-491 (`JsonEscapeTable`, 8-byte entries).
//! - Unsigned `<=` on SSE2 and AVX2 as `max` then `cmpeq`: escape-simd 26e04a0
//!   crates/escape-simd/src/simd/avx2.rs:96-102.
//! - The NEON mask by a narrowing shift (`vshrn` by 4, 4 bits per byte): escape-simd 26e04a0
//!   crates/escape-simd/src/simd/neon.rs:63-73.
//! - The SWAR test on a u64 (subtract, xor, `!x & msb`): V8 json-stringifier.cc:524-555
//!   (`NeedsEscape`). This module also takes the lowest set lane as the first unit: a borrow
//!   starts only at a lane that matches, so the lanes below the first match are exact.
//! - The overlapped last load: V8 json-stringifier.cc:3499-3504.
//! - The AVX2 length gate in the plain wrapper: memchr 2.7.6
//!   src/arch/x86_64/avx2/memchr.rs:179-217.
//!
//! The file uses only `core` and `std` (no crate item), so a standalone build of it proves the
//! wasm32 forms: ep9_vm itself does not build for wasm32.

use core::sync::atomic::{AtomicU8, Ordering};

/// The most source units per reserve (a 6x reserve of a long string would be huge).
pub(crate) const CHUNK: usize = 4096;

/// Output units past the worst-case length that a body may store into: the widest vector
/// store (32 units: AVX2 bytes, or 32 widened units) and the 8 units of a table write.
pub(crate) const SLACK: usize = 32;

/// The most output units per source unit (`\u00xx`).
const GROW: usize = 6;

/// The AVX2 body takes only a chunk of at least this many units; a shorter one takes SSE2.
#[cfg(target_arch = "x86_64")]
const AVX2_MIN: usize = 32;

// ---- the text buffer -----------------------------------------------------------------------

/// The text of one JSON.stringify call, as it is made: one-byte (Latin-1) code units until the
/// first unit above 0xFF, then UTF-16 code units. `Cx::json_text_finish` makes the string.
///
/// ```
/// use ep9_vm::JsonText;
/// let mut t = JsonText::new();
/// t.push_ascii(b"[1,");
/// t.push_units(&[0x2603]);
/// assert_eq!(t.len(), 4);
/// ```
#[derive(Debug)]
pub struct JsonText {
    pub(crate) repr: Repr,
}

/// The units of a [`JsonText`].
#[derive(Debug)]
pub(crate) enum Repr {
    /// Every unit is at most 0xFF.
    One(Vec<u8>),
    /// Some unit is above 0xFF (the widen ran when the first one came).
    Two(Vec<u16>),
}

impl Default for JsonText {
    fn default() -> JsonText {
        JsonText::new()
    }
}

impl JsonText {
    /// An empty one-byte text.
    pub const fn new() -> JsonText {
        JsonText {
            repr: Repr::One(Vec::new()),
        }
    }

    /// The length in UTF-16 code units.
    pub fn len(&self) -> usize {
        match &self.repr {
            Repr::One(v) => v.len(),
            Repr::Two(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends `text`, one code unit per byte (Latin-1; ASCII in practice).
    pub fn push_ascii(&mut self, text: &[u8]) {
        match &mut self.repr {
            Repr::One(v) => v.extend_from_slice(text),
            Repr::Two(v) => v.extend(text.iter().map(|&b| u16::from(b))),
        }
    }

    /// Appends the code units `units` as they are. The first unit above 0xFF widens the text.
    pub fn push_units(&mut self, units: &[u16]) {
        if let Repr::One(v) = &mut self.repr {
            match units.iter().position(|&u| u > 0xFF) {
                None => {
                    v.extend(units.iter().map(|&u| u as u8));
                    return;
                }
                Some(at) => {
                    v.extend(units[..at].iter().map(|&u| u as u8));
                    self.widen();
                    if let Repr::Two(w) = &mut self.repr {
                        w.extend_from_slice(&units[at..]);
                    }
                    return;
                }
            }
        }
        if let Repr::Two(w) = &mut self.repr {
            w.extend_from_slice(units);
        }
    }

    /// Appends one code unit (widens the text if it is above 0xFF).
    #[inline]
    pub(crate) fn push_unit(&mut self, u: u16) {
        match &mut self.repr {
            Repr::One(v) if u <= 0xFF => v.push(u as u8),
            Repr::One(_) => {
                self.widen();
                self.push_unit(u);
            }
            Repr::Two(v) => v.push(u),
        }
    }

    /// Turns a one-byte text into a two-byte one (one pass). Nothing for a two-byte text.
    #[cold]
    #[inline(never)]
    pub(crate) fn widen(&mut self) {
        if let Repr::One(v) = &self.repr {
            let mut w = Vec::with_capacity(v.capacity().max(v.len() + SLACK));
            w.extend(v.iter().map(|&b| u16::from(b)));
            self.repr = Repr::Two(w);
        }
    }

    /// True for a two-byte text.
    #[cfg_attr(not(test), allow(dead_code))] // callers: tests only
    pub(crate) fn is_two_byte(&self) -> bool {
        matches!(self.repr, Repr::Two(_))
    }

    /// The code units as UTF-16 (tests).
    #[cfg_attr(not(test), allow(dead_code))] // callers: tests only
    pub(crate) fn to_utf16(&self) -> Vec<u16> {
        match &self.repr {
            Repr::One(v) => v.iter().map(|&b| u16::from(b)).collect(),
            Repr::Two(v) => v.clone(),
        }
    }
}

// ---- the escape table (the spec's Table 77 and UnicodeEscape; ep9's own form) ---------------

const HEX: &[u8; 16] = b"0123456789abcdef";

/// The escape of each unit that needs one: index 0x00-0x1F a control unit, 32 `"`, 33 `\`.
/// Each entry is (length, the escape padded to 8 bytes).
const ESC: [(u8, [u8; 8]); 34] = esc_table();

/// [`ESC`] widened to UTF-16 units.
const ESC16: [(u8, [u16; 8]); 34] = esc16_table();

const fn esc_table() -> [(u8, [u8; 8]); 34] {
    let mut t = [(0u8, [0u8; 8]); 34];
    let mut c = 0;
    while c < 32 {
        // UnicodeEscape(C): "\u" and 4 lowercase hex digits.
        t[c] = (
            6,
            [b'\\', b'u', b'0', b'0', HEX[c >> 4], HEX[c & 0xF], 0, 0],
        );
        c += 1;
    }
    // Table 77 (JSON single character escape sequences).
    t[0x08] = (2, [b'\\', b'b', 0, 0, 0, 0, 0, 0]);
    t[0x09] = (2, [b'\\', b't', 0, 0, 0, 0, 0, 0]);
    t[0x0A] = (2, [b'\\', b'n', 0, 0, 0, 0, 0, 0]);
    t[0x0C] = (2, [b'\\', b'f', 0, 0, 0, 0, 0, 0]);
    t[0x0D] = (2, [b'\\', b'r', 0, 0, 0, 0, 0, 0]);
    t[32] = (2, [b'\\', b'"', 0, 0, 0, 0, 0, 0]);
    t[33] = (2, [b'\\', b'\\', 0, 0, 0, 0, 0, 0]);
    t
}

const fn esc16_table() -> [(u8, [u16; 8]); 34] {
    let b = esc_table();
    let mut t = [(0u8, [0u16; 8]); 34];
    let mut i = 0;
    while i < 34 {
        let mut j = 0;
        while j < 8 {
            t[i].1[j] = b[i].1[j] as u16;
            j += 1;
        }
        t[i].0 = b[i].0;
        i += 1;
    }
    t
}

/// The [`ESC`] index of a unit below 0x20, `"` or `\`.
#[inline(always)]
fn esc_index(u: u16) -> usize {
    debug_assert!(u < 0x20 || u == 0x22 || u == 0x5C);
    match u {
        0x22 => 32,
        0x5C => 33,
        // The mask is a no-op for a unit below 0x20; it lets the compiler drop the bounds
        // check of the table read (no panic path in a body).
        _ => usize::from(u & 0x1F),
    }
}

/// UnicodeEscape(u) of a lone surrogate: `\u` and 4 lowercase hex digits, padded to 8 bytes.
#[inline(always)]
fn unicode_escape(u: u16) -> [u8; 8] {
    let h = |s: u16| HEX[usize::from((u >> s) & 0xF)];
    [b'\\', b'u', h(12), h(8), h(4), h(0), 0, 0]
}

#[inline(always)]
fn is_surrogate(u: u16) -> bool {
    (u & 0xF800) == 0xD800
}

#[inline(always)]
fn is_lead(u: u16) -> bool {
    (u & 0xFC00) == 0xD800
}

#[inline(always)]
fn is_trail(u: u16) -> bool {
    (u & 0xFC00) == 0xDC00
}

// ---- the reference body (the per-unit loop of the old quote) -------------------------------

/// [`Path::Scalar`]: the per-unit loop that `json/stringify.rs` `quote` ran before this
/// module, moved here; it writes into a [`JsonText`] (the FASTDIFF switched-off path).
fn reference<S: Copy + Into<u16>>(src: &[S], out: &mut JsonText) {
    // 1. Let product be the String value consisting solely of the code unit 0x0022.
    out.push_unit(0x22);
    // 2. For each code point C of StringToCodePoints(value), do
    let mut i = 0;
    while i < src.len() {
        let c: u16 = src[i].into();
        i += 1;
        // a. If C is listed in the "Code Point" column of Table 77, use its escape.
        let escape = match c {
            0x08 => Some(b'b'),
            0x09 => Some(b't'),
            0x0A => Some(b'n'),
            0x0C => Some(b'f'),
            0x0D => Some(b'r'),
            0x22 => Some(b'"'),
            0x5C => Some(b'\\'),
            _ => None,
        };
        if let Some(e) = escape {
            out.push_unit(0x5C);
            out.push_unit(u16::from(e));
            continue;
        }
        // A surrogate pair is one code point: copied as it is.
        if (0xD800..0xDC00).contains(&c)
            && let Some(next) = src.get(i).map(|&n| Into::<u16>::into(n))
            && (0xDC00..0xE000).contains(&next)
        {
            out.push_unit(c);
            out.push_unit(next);
            i += 1;
            continue;
        }
        // b. Else if C < 0x20 or C is a leading or trailing surrogate: UnicodeEscape(C).
        if c < 0x20 || (0xD800..0xE000).contains(&c) {
            out.push_unit(0x5C);
            out.push_unit(u16::from(b'u'));
            for shift in [12, 8, 4, 0] {
                out.push_unit(u16::from(HEX[usize::from((c >> shift) & 0xF)]));
            }
            continue;
        }
        // c. Else, append UTF16EncodeCodePoint(C).
        out.push_unit(c);
    }
    // 3. Set product to the string-concatenation of product and the code unit 0x0022.
    out.push_unit(0x22);
}

// ---- the paths and the dispatch ------------------------------------------------------------

/// The quote bodies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Path {
    /// The reference per-unit loop (FASTDIFF off).
    Scalar = 1,
    /// u64 lanes: 8 bytes or 4 UTF-16 units per step.
    Swar = 2,
    Sse2 = 3,
    Avx2 = 4,
    Neon = 5,
    Simd128 = 6,
}

impl Path {
    /// Every path, in order.
    pub(crate) const ALL: [Path; 6] = [
        Path::Scalar,
        Path::Swar,
        Path::Sse2,
        Path::Avx2,
        Path::Neon,
        Path::Simd128,
    ];

    /// The `EP9_SIMD_JSON` name of the path.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Path::Scalar => "scalar",
            Path::Swar => "swar",
            Path::Sse2 => "sse2",
            Path::Avx2 => "avx2",
            Path::Neon => "neon",
            Path::Simd128 => "simd128",
        }
    }

    /// Whether this build and the running CPU can run the path's body.
    pub(crate) fn available(self) -> bool {
        match self {
            Path::Scalar | Path::Swar => true,
            // SSE2 is part of the x86-64 base.
            Path::Sse2 => cfg!(target_arch = "x86_64"),
            Path::Avx2 => avx2_detected(),
            Path::Neon => neon_detected(),
            Path::Simd128 => cfg!(all(target_arch = "wasm32", target_feature = "simd128")),
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn avx2_detected() -> bool {
    std::is_x86_feature_detected!("avx2")
}

#[cfg(not(target_arch = "x86_64"))]
fn avx2_detected() -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
fn neon_detected() -> bool {
    std::arch::is_aarch64_feature_detected!("neon")
}

#[cfg(not(target_arch = "aarch64"))]
fn neon_detected() -> bool {
    false
}

/// A quote body that this CPU can run. The field is private, so a `Kernel` comes only from
/// [`Kernel::detect`], [`Kernel::scalar`] or [`Kernel::new`], which check the CPU: that check
/// is what makes the `unsafe` calls of the bodies sound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Kernel(Path);

/// The detected path (`Path as u8`), 0 before the first detect.
static DETECTED: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    /// Test builds: the path that [`Kernel::detect`] gives on this thread ([`with_path`]).
    static FORCED: core::cell::Cell<Option<Path>> = const { core::cell::Cell::new(None) };
}

/// Test builds: runs `f` with [`Kernel::detect`] giving `p` on this thread. `None` if this CPU
/// does not have `p`.
#[cfg(test)]
pub(crate) fn with_path<R>(p: Path, f: impl FnOnce() -> R) -> Option<R> {
    if !p.available() {
        return None;
    }
    let old = FORCED.with(|c| c.replace(Some(p)));
    let r = f();
    FORCED.with(|c| c.set(old));
    Some(r)
}

/// The path for an `EP9_SIMD_JSON` value: that path if this CPU and build have it, else the
/// best one that they have.
pub(crate) fn pick(env: Option<&str>) -> Path {
    let best = [Path::Avx2, Path::Sse2, Path::Neon, Path::Simd128]
        .into_iter()
        .find(|p| p.available())
        .unwrap_or(Path::Swar);
    env.and_then(|v| Path::ALL.into_iter().find(|p| p.name() == v))
        .filter(|p| p.available())
        .unwrap_or(best)
}

impl Kernel {
    /// The best body for this CPU (or the `EP9_SIMD_JSON` one), detected once per process.
    #[inline]
    pub(crate) fn detect() -> Kernel {
        #[cfg(test)]
        if let Some(p) = FORCED.with(core::cell::Cell::get) {
            return Kernel(p);
        }
        match DETECTED.load(Ordering::Relaxed) {
            1 => Kernel(Path::Scalar),
            2 => Kernel(Path::Swar),
            3 => Kernel(Path::Sse2),
            4 => Kernel(Path::Avx2),
            5 => Kernel(Path::Neon),
            6 => Kernel(Path::Simd128),
            _ => Kernel::detect_slow(),
        }
    }

    #[cold]
    #[inline(never)]
    fn detect_slow() -> Kernel {
        let p = pick(std::env::var("EP9_SIMD_JSON").ok().as_deref());
        DETECTED.store(p as u8, Ordering::Relaxed);
        Kernel(p)
    }

    /// The reference body.
    pub(crate) const fn scalar() -> Kernel {
        Kernel(Path::Scalar)
    }

    /// The body of `p`, if this CPU has it.
    #[cfg_attr(not(test), allow(dead_code))] // callers: tests only
    pub(crate) fn new(p: Path) -> Option<Kernel> {
        p.available().then_some(Kernel(p))
    }

    /// The path of this body.
    #[cfg_attr(not(test), allow(dead_code))] // callers: tests only
    pub(crate) fn path(self) -> Path {
        self.0
    }

    /// Runs the body of this kernel on the chunk `src[from..end]`.
    ///
    /// # Safety
    /// `from < end <= src.len()`, and `dst` has room for `(end - from) * GROW + SLACK` units.
    #[inline]
    unsafe fn run<P: Pair>(self, src: &[P::S], from: usize, end: usize, dst: *mut P::D) -> Done {
        match self.0 {
            // SAFETY: the caller's bound; a Kernel of Avx2 exists only when the CPU has AVX2
            // (`Kernel` doc).
            #[cfg(target_arch = "x86_64")]
            Path::Avx2 if end - from >= AVX2_MIN => unsafe { x86::avx2::<P>(src, from, end, dst) },
            // SAFETY: the caller's bound; SSE2 is part of the x86-64 base.
            #[cfg(target_arch = "x86_64")]
            Path::Avx2 | Path::Sse2 => unsafe { x86::sse2::<P>(src, from, end, dst) },
            // SAFETY: the caller's bound; a Kernel of Neon exists only when the CPU has NEON.
            #[cfg(target_arch = "aarch64")]
            Path::Neon => unsafe { arm::neon::<P>(src, from, end, dst) },
            // SAFETY: the caller's bound; the build has simd128 (the cfg).
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            Path::Simd128 => unsafe { drive::<wasm::V128, P>(src, from, end, dst) },
            // SAFETY: the caller's bound; SWAR needs no CPU feature.
            _ => unsafe { drive::<Swar, P>(src, from, end, dst) },
        }
    }
}

// ---- the quote -----------------------------------------------------------------------------

/// Appends QuoteJSONString(`src`) of a Latin-1 source to `out`, by the body of `k`.
pub(crate) fn quote_latin1(k: Kernel, src: &[u8], out: &mut JsonText) {
    if k.0 == Path::Scalar {
        return reference(src, out);
    }
    out.push_ascii(b"\"");
    let n = src.len();
    match &mut out.repr {
        Repr::One(v) => chunks(v, n, 0, |from, end, dst| {
            // SAFETY: `chunks` gives from < end <= n and the reserve of the bound.
            unsafe { k.run::<L1One>(src, from, end, dst) }
        }),
        Repr::Two(v) => chunks(v, n, 0, |from, end, dst| {
            // SAFETY: as above.
            unsafe { k.run::<L1Two>(src, from, end, dst) }
        }),
    };
    out.push_ascii(b"\"");
}

/// Appends QuoteJSONString(`src`) of a UTF-16 source to `out`, by the body of `k`.
pub(crate) fn quote_utf16(k: Kernel, src: &[u16], out: &mut JsonText) {
    if k.0 == Path::Scalar {
        return reference(src, out);
    }
    out.push_ascii(b"\"");
    let n = src.len();
    let mut i = 0;
    if let Repr::One(v) = &mut out.repr {
        let stop = chunks(v, n, 0, |from, end, dst| {
            // SAFETY: `chunks` gives from < end <= n and the reserve of the bound.
            unsafe { k.run::<U16One>(src, from, end, dst) }
        });
        match stop {
            None => {
                out.push_ascii(b"\"");
                return;
            }
            // The unit at `at` needs two-byte output: widen, then go on from it.
            Some(at) => {
                i = at;
                out.widen();
            }
        }
    }
    if let Repr::Two(v) = &mut out.repr {
        chunks(v, n, i, |from, end, dst| {
            // SAFETY: as above.
            unsafe { k.run::<U16Two>(src, from, end, dst) }
        });
    }
    out.push_ascii(b"\"");
}

/// The result of a body on one chunk.
struct Done {
    /// The next source unit (`end`, or `end + 1` after a pair across the chunk end, or the
    /// unit that needs a widen).
    i: usize,
    /// Output units written.
    wrote: usize,
    /// The unit at `i` needs two-byte output (U16One only).
    widen: bool,
}

/// Runs `body` on the chunks of `n` source units from `i`: per chunk one reserve, then the
/// body's raw stores, then one `set_len`. `Some(i)` if the body stopped for a widen at `i`.
#[inline(always)]
fn chunks<D: Copy>(
    v: &mut Vec<D>,
    n: usize,
    mut i: usize,
    mut body: impl FnMut(usize, usize, *mut D) -> Done,
) -> Option<usize> {
    while i < n {
        let end = (i + CHUNK).min(n);
        v.reserve((end - i) * GROW + 2 + SLACK);
        let len = v.len();
        // SAFETY: `len <= capacity`, so the pointer is inside (or one past) the allocation.
        let dst = unsafe { v.as_mut_ptr().add(len) };
        let d = body(i, end, dst);
        debug_assert!(d.wrote <= (end - i) * GROW && len + d.wrote <= v.capacity());
        // SAFETY: the body initialized `d.wrote` units from `len` (each store below `wrote` is
        // a unit of the output), inside the reserve.
        unsafe { v.set_len(len + d.wrote) };
        i = d.i;
        if d.widen {
            return Some(i);
        }
    }
    None
}

// ---- the body (generic over the lanes and the source and output kinds) ---------------------

/// One vector width of the units of an instruction set.
trait Lanes {
    /// Bytes per step of a Latin-1 source.
    const W8: usize;
    /// Units per step of a UTF-16 source.
    const W16: usize;
    /// Latin-1 -> one-byte: stores the `W8` bytes at `s` to `d`; the escape mask.
    ///
    /// # Safety
    /// `W8` bytes at `s` are readable and `W8` bytes at `d` writable.
    unsafe fn l1_one(s: *const u8, d: *mut u8) -> u64;
    /// Latin-1 -> two-byte: stores the `W8` bytes at `s` to `d`, widened; the escape mask.
    ///
    /// # Safety
    /// `W8` bytes at `s` are readable and `W8` units at `d` writable.
    unsafe fn l1_two(s: *const u8, d: *mut u16) -> u64;
    /// UTF-16 -> one-byte: stores the `W16` units at `s` to `d`, narrowed (a unit above 0xFF
    /// stores some byte, later written over); the mask of the units below 0x20, `"`, `\` and
    /// above 0xFF.
    ///
    /// # Safety
    /// `W16` units at `s` are readable and `W16` bytes at `d` writable.
    unsafe fn u16_one(s: *const u16, d: *mut u8) -> u64;
    /// UTF-16 -> two-byte: stores the `W16` units at `s` to `d`; the mask of the units below
    /// 0x20, `"`, `\` and the surrogates.
    ///
    /// # Safety
    /// `W16` units at `s` are readable and `W16` units at `d` writable.
    unsafe fn u16_two(s: *const u16, d: *mut u16) -> u64;
    /// The index of the first unit set in a mask of a Latin-1 step (the mask is not 0).
    fn first8(m: u64) -> usize;
    /// The index of the first unit set in a mask of a UTF-16 step (the mask is not 0).
    fn first16(m: u64) -> usize;
}

/// A source kind and an output kind.
trait Pair {
    type S: Copy;
    type D: Copy;
    /// Units per step of `L`.
    fn w<L: Lanes>() -> usize;
    /// One step of `L` ([`Lanes`]).
    ///
    /// # Safety
    /// As the `Lanes` method.
    unsafe fn step<L: Lanes>(s: *const Self::S, d: *mut Self::D) -> u64;
    fn first<L: Lanes>(m: u64) -> usize;
    /// The scalar test: the unit needs an escape (or a pair check, or a widen).
    fn needs(u: Self::S) -> bool;
    /// A unit that needs no escape, in the output kind.
    fn copy(u: Self::S) -> Self::D;
    /// Writes the escape of `src[*i]` (a unit that [`Pair::needs`]) at `dst + *d` and moves
    /// both; false (and nothing written) if the unit needs two-byte output first.
    ///
    /// # Safety
    /// `*i < src.len()`, and `dst + *d` has room for 8 units.
    unsafe fn escape(src: &[Self::S], i: &mut usize, dst: *mut Self::D, d: &mut usize) -> bool;
}

/// Writes a table escape of a unit below 0x20, `"` or `\` at `dst + *d` (all 8 bytes).
///
/// # Safety
/// `dst + *d` has room for 8 bytes.
#[inline(always)]
unsafe fn esc_one(u: u16, dst: *mut u8, d: &mut usize) {
    let e = &ESC[esc_index(u)];
    // SAFETY: the caller's room for 8 bytes; an unaligned store of the padded entry.
    unsafe { dst.add(*d).cast::<[u8; 8]>().write_unaligned(e.1) };
    *d += usize::from(e.0);
}

/// [`esc_one`] for a two-byte output.
///
/// # Safety
/// `dst + *d` has room for 8 units.
#[inline(always)]
unsafe fn esc_two(u: u16, dst: *mut u16, d: &mut usize) {
    let e = &ESC16[esc_index(u)];
    // SAFETY: the caller's room for 8 units.
    unsafe { dst.add(*d).cast::<[u16; 8]>().write_unaligned(e.1) };
    *d += usize::from(e.0);
}

/// Latin-1 source, one-byte output.
struct L1One;
/// Latin-1 source, two-byte output.
struct L1Two;
/// UTF-16 source, one-byte output.
struct U16One;
/// UTF-16 source, two-byte output.
struct U16Two;

#[inline(always)]
fn needs8(b: u8) -> bool {
    b < 0x20 || b == b'"' || b == b'\\'
}

impl Pair for L1One {
    type S = u8;
    type D = u8;
    #[inline(always)]
    fn w<L: Lanes>() -> usize {
        L::W8
    }
    #[inline(always)]
    unsafe fn step<L: Lanes>(s: *const u8, d: *mut u8) -> u64 {
        // SAFETY: the caller's bound.
        unsafe { L::l1_one(s, d) }
    }
    #[inline(always)]
    fn first<L: Lanes>(m: u64) -> usize {
        L::first8(m)
    }
    #[inline(always)]
    fn needs(u: u8) -> bool {
        needs8(u)
    }
    #[inline(always)]
    fn copy(u: u8) -> u8 {
        u
    }
    #[inline(always)]
    unsafe fn escape(src: &[u8], i: &mut usize, dst: *mut u8, d: &mut usize) -> bool {
        // SAFETY: `*i < src.len()` and the room for 8 bytes (the caller's bound).
        unsafe { esc_one(u16::from(*src.get_unchecked(*i)), dst, d) };
        *i += 1;
        true
    }
}

impl Pair for L1Two {
    type S = u8;
    type D = u16;
    #[inline(always)]
    fn w<L: Lanes>() -> usize {
        L::W8
    }
    #[inline(always)]
    unsafe fn step<L: Lanes>(s: *const u8, d: *mut u16) -> u64 {
        // SAFETY: the caller's bound.
        unsafe { L::l1_two(s, d) }
    }
    #[inline(always)]
    fn first<L: Lanes>(m: u64) -> usize {
        L::first8(m)
    }
    #[inline(always)]
    fn needs(u: u8) -> bool {
        needs8(u)
    }
    #[inline(always)]
    fn copy(u: u8) -> u16 {
        u16::from(u)
    }
    #[inline(always)]
    unsafe fn escape(src: &[u8], i: &mut usize, dst: *mut u16, d: &mut usize) -> bool {
        // SAFETY: `*i < src.len()` and the room for 8 units (the caller's bound).
        unsafe { esc_two(u16::from(*src.get_unchecked(*i)), dst, d) };
        *i += 1;
        true
    }
}

impl Pair for U16One {
    type S = u16;
    type D = u8;
    #[inline(always)]
    fn w<L: Lanes>() -> usize {
        L::W16
    }
    #[inline(always)]
    unsafe fn step<L: Lanes>(s: *const u16, d: *mut u8) -> u64 {
        // SAFETY: the caller's bound.
        unsafe { L::u16_one(s, d) }
    }
    #[inline(always)]
    fn first<L: Lanes>(m: u64) -> usize {
        L::first16(m)
    }
    #[inline(always)]
    fn needs(u: u16) -> bool {
        u < 0x20 || u == 0x22 || u == 0x5C || u > 0xFF
    }
    #[inline(always)]
    fn copy(u: u16) -> u8 {
        u as u8
    }
    #[inline(always)]
    unsafe fn escape(src: &[u16], i: &mut usize, dst: *mut u8, d: &mut usize) -> bool {
        // SAFETY: the caller's `*i < src.len()`.
        let u = unsafe { *src.get_unchecked(*i) };
        if u <= 0xFF {
            // SAFETY: the room for 8 bytes.
            unsafe { esc_one(u, dst, d) };
        } else if is_surrogate(u) && !(is_lead(u) && src.get(*i + 1).is_some_and(|&t| is_trail(t)))
        {
            // A lone surrogate: an ASCII escape, no widen.
            // SAFETY: the room for 8 bytes.
            unsafe {
                dst.add(*d)
                    .cast::<[u8; 8]>()
                    .write_unaligned(unicode_escape(u))
            };
            *d += 6;
        } else {
            // A valid pair or another unit above 0xFF: the output must widen.
            return false;
        }
        *i += 1;
        true
    }
}

impl Pair for U16Two {
    type S = u16;
    type D = u16;
    #[inline(always)]
    fn w<L: Lanes>() -> usize {
        L::W16
    }
    #[inline(always)]
    unsafe fn step<L: Lanes>(s: *const u16, d: *mut u16) -> u64 {
        // SAFETY: the caller's bound.
        unsafe { L::u16_two(s, d) }
    }
    #[inline(always)]
    fn first<L: Lanes>(m: u64) -> usize {
        L::first16(m)
    }
    #[inline(always)]
    fn needs(u: u16) -> bool {
        u < 0x20 || u == 0x22 || u == 0x5C || is_surrogate(u)
    }
    #[inline(always)]
    fn copy(u: u16) -> u16 {
        u
    }
    #[inline(always)]
    unsafe fn escape(src: &[u16], i: &mut usize, dst: *mut u16, d: &mut usize) -> bool {
        // SAFETY: the caller's `*i < src.len()`.
        let u = unsafe { *src.get_unchecked(*i) };
        if !is_surrogate(u) {
            // SAFETY: the room for 8 units.
            unsafe { esc_two(u, dst, d) };
            *i += 1;
        } else if is_lead(u)
            && let Some(&t) = src.get(*i + 1)
            && is_trail(t)
        {
            // A valid pair (also across a vector or chunk end): copied as it is.
            // SAFETY: the room for 8 units.
            unsafe { dst.add(*d).cast::<[u16; 2]>().write_unaligned([u, t]) };
            *d += 2;
            *i += 2;
        } else {
            let e = unicode_escape(u).map(u16::from);
            // SAFETY: the room for 8 units.
            unsafe { dst.add(*d).cast::<[u16; 8]>().write_unaligned(e) };
            *d += 6;
            *i += 1;
        }
        true
    }
}

/// Escapes the run of units from `*i` (the first one needs an escape) until a clean unit or
/// `end`. False if a unit needs a widen (nothing written for it).
///
/// # Safety
/// `*i < end <= src.len()`, and the room of the chunk bound at `dst + *d`.
#[inline(always)]
unsafe fn run<P: Pair>(
    src: &[P::S],
    i: &mut usize,
    end: usize,
    dst: *mut P::D,
    d: &mut usize,
) -> bool {
    loop {
        // SAFETY: the caller's bound; each escape takes at most 6 units of room per source
        // unit it moves past (8 written, 2 of them in the slack).
        if !unsafe { P::escape(src, i, dst, d) } {
            return false;
        }
        // SAFETY: `*i < end <= src.len()` when read.
        if *i >= end || !P::needs(unsafe { *src.get_unchecked(*i) }) {
            return true;
        }
    }
}

/// The quote body of `L` on the chunk `src[from..end]`, written at `dst` (the units between
/// the quotes; the caller writes the quotes).
///
/// # Safety
/// `from < end <= src.len()`; `dst` has room for `(end - from) * GROW + SLACK` units.
#[inline(always)]
unsafe fn drive<L: Lanes, P: Pair>(src: &[P::S], from: usize, end: usize, dst: *mut P::D) -> Done {
    debug_assert!(from < end && end <= src.len());
    let s = src.as_ptr();
    let w = P::w::<L>();
    // `clean`: the units `clean..i` were copied as they are, to `d - (i - clean)..d`.
    let (mut i, mut d, mut clean) = (from, 0usize, from);
    // Room: each source unit before `i` took at most GROW output units, so `d <= GROW * (i -
    // from)`; a store at `d` writes at most SLACK units (a vector, or the 8 of an escape).
    while i + w <= end {
        // SAFETY: `i + w <= end`, so the w units are in `src`; the store is in the room.
        let m = unsafe { P::step::<L>(s.add(i), dst.add(d)) };
        if m == 0 {
            i += w;
            d += w;
            continue;
        }
        let k = P::first::<L>(m);
        i += k;
        d += k;
        // SAFETY: `i < end` (a set lane is inside the step), the room as above.
        if !unsafe { run::<P>(src, &mut i, end, dst, &mut d) } {
            return Done {
                i,
                wrote: d,
                widen: true,
            };
        }
        clean = i;
    }
    // The tail, shorter than a vector: one overlapped load of the last w units, when the
    // `back` units before `i` were copied as they are (their output is the same units again,
    // so the store at `d - back` writes the same text; their mask lanes are 0).
    if i < end && end >= w && i - clean >= w - (end - i) {
        let back = w - (end - i);
        // SAFETY: `end - w >= clean >= 0`, so the w units are in `src`; `d - back >= 0` (the
        // `back` copied units), and the store ends at `d + (end - i)`, in the room.
        let m = unsafe { P::step::<L>(s.add(end - w), dst.add(d - back)) };
        if m == 0 {
            d += end - i;
            i = end;
        } else {
            let k = P::first::<L>(m);
            debug_assert!(k >= back, "a lane of a copied unit is set");
            i += k - back;
            d += k - back;
            // SAFETY: as in the loop.
            if !unsafe { run::<P>(src, &mut i, end, dst, &mut d) } {
                return Done {
                    i,
                    wrote: d,
                    widen: true,
                };
            }
        }
    }
    // SWAR steps, then one unit at a time.
    let ws = P::w::<Swar>();
    while i + ws <= end {
        // SAFETY: as in the vector loop.
        let m = unsafe { P::step::<Swar>(s.add(i), dst.add(d)) };
        if m == 0 {
            i += ws;
            d += ws;
            continue;
        }
        let k = P::first::<Swar>(m);
        i += k;
        d += k;
        // SAFETY: as in the vector loop.
        if !unsafe { run::<P>(src, &mut i, end, dst, &mut d) } {
            return Done {
                i,
                wrote: d,
                widen: true,
            };
        }
    }
    while i < end {
        // SAFETY: `i < end <= src.len()`.
        let u = unsafe { *src.get_unchecked(i) };
        if P::needs(u) {
            // SAFETY: as in the vector loop.
            if !unsafe { run::<P>(src, &mut i, end, dst, &mut d) } {
                return Done {
                    i,
                    wrote: d,
                    widen: true,
                };
            }
        } else {
            // SAFETY: one unit at `d`, in the room.
            unsafe { dst.add(d).write(P::copy(u)) };
            i += 1;
            d += 1;
        }
    }
    Done {
        i,
        wrote: d,
        widen: false,
    }
}

// ---- SWAR (u64 lanes; every target) --------------------------------------------------------

/// u64 lanes: 8 Latin-1 bytes or 4 UTF-16 units. The lanes are in little-endian order on
/// every target (`from_le_bytes`), so lane j is unit j.
struct Swar;

const L8: u64 = 0x0101_0101_0101_0101;
const H8: u64 = 0x8080_8080_8080_8080;
const L16: u64 = 0x0001_0001_0001_0001;
const H16: u64 = 0x8000_8000_8000_8000;

/// The escape mask of 8 Latin-1 bytes: bit 7 of byte j for a byte below 0x20, `"` or `\`.
/// The lowest set byte is exact (a borrow starts only at a byte that matches; higher bytes may
/// be false positives, so only the first set byte is used).
#[inline(always)]
fn swar8(w: u64) -> u64 {
    let lt = w.wrapping_sub(L8 * 0x20);
    let q = (w ^ (L8 * 0x22)).wrapping_sub(L8);
    let b = (w ^ (L8 * 0x5C)).wrapping_sub(L8);
    (lt | q | b) & !w & H8
}

/// The mask of 4 UTF-16 units below 0x20, `"` or `\` (bit 15 of lane j; the lowest exact).
#[inline(always)]
fn swar16_small(w: u64) -> u64 {
    let lt = w.wrapping_sub(L16 * 0x20);
    let q = (w ^ (L16 * 0x22)).wrapping_sub(L16);
    let b = (w ^ (L16 * 0x5C)).wrapping_sub(L16);
    (lt | q | b) & !w & H16
}

/// The mask of 4 UTF-16 units that are surrogates (a zero test of `(u & 0xF800) ^ 0xD800`).
#[inline(always)]
fn swar16_surrogate(w: u64) -> u64 {
    let x = (w & (L16 * 0xF800)) ^ (L16 * 0xD800);
    x.wrapping_sub(L16) & !x & H16
}

/// The mask of 4 UTF-16 units above 0xFF (exact in every lane: no carry leaves a lane).
#[inline(always)]
fn swar16_big(w: u64) -> u64 {
    (((w & (L16 * 0xFF00)) >> 8) + L16 * 0x7FFF) & H16
}

/// 4 UTF-16 units at `s` as u64 lanes.
///
/// # Safety
/// 4 units at `s` are readable.
#[inline(always)]
unsafe fn load16x4(s: *const u16) -> u64 {
    // SAFETY: the caller's bound; an unaligned read of 4 units.
    let a = unsafe { s.cast::<[u16; 4]>().read_unaligned() };
    u64::from(a[0]) | u64::from(a[1]) << 16 | u64::from(a[2]) << 32 | u64::from(a[3]) << 48
}

impl Lanes for Swar {
    const W8: usize = 8;
    const W16: usize = 4;
    #[inline(always)]
    unsafe fn l1_one(s: *const u8, d: *mut u8) -> u64 {
        // SAFETY: the caller's bound: 8 bytes each.
        unsafe {
            let b = s.cast::<[u8; 8]>().read_unaligned();
            d.cast::<[u8; 8]>().write_unaligned(b);
            swar8(u64::from_le_bytes(b))
        }
    }
    #[inline(always)]
    unsafe fn l1_two(s: *const u8, d: *mut u16) -> u64 {
        // SAFETY: the caller's bound: 8 bytes, 8 units.
        unsafe {
            let b = s.cast::<[u8; 8]>().read_unaligned();
            d.cast::<[u16; 8]>().write_unaligned(b.map(u16::from));
            swar8(u64::from_le_bytes(b))
        }
    }
    #[inline(always)]
    unsafe fn u16_one(s: *const u16, d: *mut u8) -> u64 {
        // SAFETY: the caller's bound: 4 units, 4 bytes.
        unsafe {
            let a = s.cast::<[u16; 4]>().read_unaligned();
            d.cast::<[u8; 4]>().write_unaligned(a.map(|u| u as u8));
            let w = load16x4(s);
            swar16_small(w) | swar16_big(w)
        }
    }
    #[inline(always)]
    unsafe fn u16_two(s: *const u16, d: *mut u16) -> u64 {
        // SAFETY: the caller's bound: 4 units each.
        unsafe {
            let a = s.cast::<[u16; 4]>().read_unaligned();
            d.cast::<[u16; 4]>().write_unaligned(a);
            let w = load16x4(s);
            swar16_small(w) | swar16_surrogate(w)
        }
    }
    #[inline(always)]
    fn first8(m: u64) -> usize {
        (m.trailing_zeros() / 8) as usize
    }
    #[inline(always)]
    fn first16(m: u64) -> usize {
        (m.trailing_zeros() / 16) as usize
    }
}

// ---- x86-64 ----------------------------------------------------------------------------------

/// x86-64 bodies: SSE2 (the base) and AVX2. The AVX2 lanes take the UTF-16 steps 16 units at a
/// time and the widening and narrowing steps through two 128-bit halves.
#[cfg(target_arch = "x86_64")]
mod x86 {
    use core::arch::x86_64::{
        __m128i, __m256i, _mm_and_si128, _mm_cmpeq_epi8, _mm_cmpeq_epi16, _mm_loadu_si128,
        _mm_max_epu8, _mm_movemask_epi8, _mm_or_si128, _mm_packus_epi16, _mm_set1_epi8,
        _mm_set1_epi16, _mm_setzero_si128, _mm_storel_epi64, _mm_storeu_si128, _mm_subs_epu16,
        _mm_unpackhi_epi8, _mm_unpacklo_epi8, _mm256_and_si256, _mm256_castsi256_si128,
        _mm256_cmpeq_epi8, _mm256_cmpeq_epi16, _mm256_cvtepu8_epi16, _mm256_extracti128_si256,
        _mm256_loadu_si256, _mm256_max_epu8, _mm256_movemask_epi8, _mm256_or_si256,
        _mm256_set1_epi8, _mm256_set1_epi16, _mm256_setzero_si256, _mm256_storeu_si256,
        _mm256_subs_epu16,
    };

    use super::{Done, Lanes, Pair, drive};

    /// SSE2 body.
    ///
    /// # Safety
    /// The bound of `drive`. The CPU has SSE2 (always true on x86-64).
    #[target_feature(enable = "sse2")]
    #[inline]
    pub(super) unsafe fn sse2<P: Pair>(
        src: &[P::S],
        from: usize,
        end: usize,
        dst: *mut P::D,
    ) -> Done {
        // SAFETY: the caller's bound.
        unsafe { drive::<Sse2, P>(src, from, end, dst) }
    }

    /// AVX2 body (a leaf: it calls nothing but the inlined steps).
    ///
    /// # Safety
    /// The bound of `drive`. The CPU has AVX2 (`is_x86_feature_detected!("avx2")`).
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn avx2<P: Pair>(
        src: &[P::S],
        from: usize,
        end: usize,
        dst: *mut P::D,
    ) -> Done {
        // SAFETY: the caller's bound.
        unsafe { drive::<Avx2, P>(src, from, end, dst) }
    }

    /// The SSE2 escape mask of 16 bytes (a byte <= 0x1F as max(v, 0x1F) == 0x1F).
    #[inline(always)]
    unsafe fn mask8(v: __m128i) -> u64 {
        // SAFETY: the caller's SSE2.
        unsafe {
            let c = _mm_set1_epi8(0x1F);
            let lt = _mm_cmpeq_epi8(_mm_max_epu8(v, c), c);
            let q = _mm_cmpeq_epi8(v, _mm_set1_epi8(0x22));
            let b = _mm_cmpeq_epi8(v, _mm_set1_epi8(0x5C));
            _mm_movemask_epi8(_mm_or_si128(lt, _mm_or_si128(q, b))) as u32 as u64
        }
    }

    /// The SSE2 mask of 8 units below 0x20, `"` or `\` (2 bits per unit).
    #[inline(always)]
    unsafe fn small16(v: __m128i) -> __m128i {
        // SAFETY: the caller's SSE2.
        unsafe {
            let z = _mm_setzero_si128();
            let lt = _mm_cmpeq_epi16(_mm_subs_epu16(v, _mm_set1_epi16(0x1F)), z);
            let q = _mm_cmpeq_epi16(v, _mm_set1_epi16(0x22));
            let b = _mm_cmpeq_epi16(v, _mm_set1_epi16(0x5C));
            _mm_or_si128(lt, _mm_or_si128(q, b))
        }
    }

    /// SSE2: 16 bytes or 8 UTF-16 units per step.
    struct Sse2;

    impl Lanes for Sse2 {
        const W8: usize = 16;
        const W16: usize = 8;
        #[inline(always)]
        unsafe fn l1_one(s: *const u8, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 16 bytes each.
            unsafe {
                let v = _mm_loadu_si128(s.cast::<__m128i>());
                _mm_storeu_si128(d.cast::<__m128i>(), v);
                mask8(v)
            }
        }
        #[inline(always)]
        unsafe fn l1_two(s: *const u8, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 16 bytes, 16 units.
            unsafe {
                let v = _mm_loadu_si128(s.cast::<__m128i>());
                let z = _mm_setzero_si128();
                _mm_storeu_si128(d.cast::<__m128i>(), _mm_unpacklo_epi8(v, z));
                _mm_storeu_si128(d.add(8).cast::<__m128i>(), _mm_unpackhi_epi8(v, z));
                mask8(v)
            }
        }
        #[inline(always)]
        unsafe fn u16_one(s: *const u16, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 8 units, 8 bytes.
            unsafe {
                let v = _mm_loadu_si128(s.cast::<__m128i>());
                _mm_storel_epi64(d.cast::<__m128i>(), _mm_packus_epi16(v, v));
                let fits = _mm_cmpeq_epi16(
                    _mm_and_si128(v, _mm_set1_epi16(0xFF00u16 as i16)),
                    _mm_setzero_si128(),
                );
                let big = (_mm_movemask_epi8(fits) as u32) ^ 0xFFFF;
                ((_mm_movemask_epi8(small16(v)) as u32) | big) as u64
            }
        }
        #[inline(always)]
        unsafe fn u16_two(s: *const u16, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 8 units each.
            unsafe {
                let v = _mm_loadu_si128(s.cast::<__m128i>());
                _mm_storeu_si128(d.cast::<__m128i>(), v);
                let sur = _mm_cmpeq_epi16(
                    _mm_and_si128(v, _mm_set1_epi16(0xF800u16 as i16)),
                    _mm_set1_epi16(0xD800u16 as i16),
                );
                _mm_movemask_epi8(_mm_or_si128(small16(v), sur)) as u32 as u64
            }
        }
        #[inline(always)]
        fn first8(m: u64) -> usize {
            m.trailing_zeros() as usize
        }
        #[inline(always)]
        fn first16(m: u64) -> usize {
            (m.trailing_zeros() / 2) as usize
        }
    }

    /// The AVX2 escape mask of 32 bytes.
    ///
    /// # Safety
    /// The CPU has AVX2.
    #[inline(always)]
    unsafe fn mask8x32(v: __m256i) -> u64 {
        // SAFETY: the caller's AVX2.
        unsafe {
            let c = _mm256_set1_epi8(0x1F);
            let lt = _mm256_cmpeq_epi8(_mm256_max_epu8(v, c), c);
            let q = _mm256_cmpeq_epi8(v, _mm256_set1_epi8(0x22));
            let b = _mm256_cmpeq_epi8(v, _mm256_set1_epi8(0x5C));
            _mm256_movemask_epi8(_mm256_or_si256(lt, _mm256_or_si256(q, b))) as u32 as u64
        }
    }

    /// The AVX2 mask of 16 units below 0x20, `"` or `\`.
    ///
    /// # Safety
    /// The CPU has AVX2.
    #[inline(always)]
    unsafe fn small16x16(v: __m256i) -> __m256i {
        // SAFETY: the caller's AVX2.
        unsafe {
            let z = _mm256_setzero_si256();
            let lt = _mm256_cmpeq_epi16(_mm256_subs_epu16(v, _mm256_set1_epi16(0x1F)), z);
            let q = _mm256_cmpeq_epi16(v, _mm256_set1_epi16(0x22));
            let b = _mm256_cmpeq_epi16(v, _mm256_set1_epi16(0x5C));
            _mm256_or_si256(lt, _mm256_or_si256(q, b))
        }
    }

    /// AVX2: 32 bytes or 16 UTF-16 units per step.
    struct Avx2;

    impl Lanes for Avx2 {
        const W8: usize = 32;
        const W16: usize = 16;
        #[inline(always)]
        unsafe fn l1_one(s: *const u8, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 32 bytes each; AVX2 (only `avx2` runs this).
            unsafe {
                let v = _mm256_loadu_si256(s.cast::<__m256i>());
                _mm256_storeu_si256(d.cast::<__m256i>(), v);
                mask8x32(v)
            }
        }
        #[inline(always)]
        unsafe fn l1_two(s: *const u8, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 32 bytes, 32 units; AVX2.
            unsafe {
                let v = _mm256_loadu_si256(s.cast::<__m256i>());
                let lo = _mm256_cvtepu8_epi16(_mm256_castsi256_si128(v));
                let hi = _mm256_cvtepu8_epi16(_mm256_extracti128_si256::<1>(v));
                _mm256_storeu_si256(d.cast::<__m256i>(), lo);
                _mm256_storeu_si256(d.add(16).cast::<__m256i>(), hi);
                mask8x32(v)
            }
        }
        #[inline(always)]
        unsafe fn u16_one(s: *const u16, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 16 units, 16 bytes; AVX2.
            unsafe {
                let v = _mm256_loadu_si256(s.cast::<__m256i>());
                let n =
                    _mm_packus_epi16(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
                _mm_storeu_si128(d.cast::<__m128i>(), n);
                let fits = _mm256_cmpeq_epi16(
                    _mm256_and_si256(v, _mm256_set1_epi16(0xFF00u16 as i16)),
                    _mm256_setzero_si256(),
                );
                let big = !(_mm256_movemask_epi8(fits) as u32);
                ((_mm256_movemask_epi8(small16x16(v)) as u32) | big) as u64
            }
        }
        #[inline(always)]
        unsafe fn u16_two(s: *const u16, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 16 units each; AVX2.
            unsafe {
                let v = _mm256_loadu_si256(s.cast::<__m256i>());
                _mm256_storeu_si256(d.cast::<__m256i>(), v);
                let sur = _mm256_cmpeq_epi16(
                    _mm256_and_si256(v, _mm256_set1_epi16(0xF800u16 as i16)),
                    _mm256_set1_epi16(0xD800u16 as i16),
                );
                _mm256_movemask_epi8(_mm256_or_si256(small16x16(v), sur)) as u32 as u64
            }
        }
        #[inline(always)]
        fn first8(m: u64) -> usize {
            m.trailing_zeros() as usize
        }
        #[inline(always)]
        fn first16(m: u64) -> usize {
            (m.trailing_zeros() / 2) as usize
        }
    }
}

// ---- aarch64 ---------------------------------------------------------------------------------

/// aarch64 body: NEON (the base). The mask is a narrowing shift by 4 of the compare result:
/// 4 bits per byte, or 8 bits per UTF-16 unit.
#[cfg(target_arch = "aarch64")]
mod arm {
    use core::arch::aarch64::{
        uint8x16_t, uint16x8_t, vandq_u16, vceqq_u8, vceqq_u16, vcgtq_u16, vcleq_u8, vcleq_u16,
        vdupq_n_u8, vdupq_n_u16, vget_lane_u64, vget_low_u8, vld1q_u8, vld1q_u16, vmovl_high_u8,
        vmovl_u8, vmovn_u16, vorrq_u8, vorrq_u16, vreinterpret_u64_u8, vreinterpretq_u16_u8,
        vshrn_n_u16, vst1_u8, vst1q_u8, vst1q_u16,
    };

    use super::{Done, Lanes, Pair, drive};

    /// NEON body.
    ///
    /// # Safety
    /// The bound of `drive`. The CPU has NEON (the aarch64 base).
    #[target_feature(enable = "neon")]
    #[inline]
    pub(super) unsafe fn neon<P: Pair>(
        src: &[P::S],
        from: usize,
        end: usize,
        dst: *mut P::D,
    ) -> Done {
        // SAFETY: the caller's bound.
        unsafe { drive::<Neon, P>(src, from, end, dst) }
    }

    /// The escape lanes of 16 bytes.
    #[inline(always)]
    unsafe fn cls8(v: uint8x16_t) -> uint8x16_t {
        // SAFETY: the caller's NEON.
        unsafe {
            let lt = vcleq_u8(v, vdupq_n_u8(0x1F));
            let q = vceqq_u8(v, vdupq_n_u8(0x22));
            let b = vceqq_u8(v, vdupq_n_u8(0x5C));
            vorrq_u8(lt, vorrq_u8(q, b))
        }
    }

    /// The lanes of 8 units below 0x20, `"` or `\`.
    #[inline(always)]
    unsafe fn small16(v: uint16x8_t) -> uint16x8_t {
        // SAFETY: the caller's NEON.
        unsafe {
            let lt = vcleq_u16(v, vdupq_n_u16(0x1F));
            let q = vceqq_u16(v, vdupq_n_u16(0x22));
            let b = vceqq_u16(v, vdupq_n_u16(0x5C));
            vorrq_u16(lt, vorrq_u16(q, b))
        }
    }

    /// 4 bits per byte lane (or 8 per u16 lane) in a u64.
    #[inline(always)]
    unsafe fn bits(m: uint16x8_t) -> u64 {
        // SAFETY: the caller's NEON.
        unsafe { vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(m))) }
    }

    /// NEON: 16 bytes or 8 UTF-16 units per step.
    struct Neon;

    impl Lanes for Neon {
        const W8: usize = 16;
        const W16: usize = 8;
        #[inline(always)]
        unsafe fn l1_one(s: *const u8, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 16 bytes each.
            unsafe {
                let v = vld1q_u8(s);
                vst1q_u8(d, v);
                bits(vreinterpretq_u16_u8(cls8(v)))
            }
        }
        #[inline(always)]
        unsafe fn l1_two(s: *const u8, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 16 bytes, 16 units.
            unsafe {
                let v = vld1q_u8(s);
                vst1q_u16(d, vmovl_u8(vget_low_u8(v)));
                vst1q_u16(d.add(8), vmovl_high_u8(v));
                bits(vreinterpretq_u16_u8(cls8(v)))
            }
        }
        #[inline(always)]
        unsafe fn u16_one(s: *const u16, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 8 units, 8 bytes.
            unsafe {
                let v = vld1q_u16(s);
                vst1_u8(d, vmovn_u16(v));
                bits(vorrq_u16(small16(v), vcgtq_u16(v, vdupq_n_u16(0xFF))))
            }
        }
        #[inline(always)]
        unsafe fn u16_two(s: *const u16, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 8 units each.
            unsafe {
                let v = vld1q_u16(s);
                vst1q_u16(d, v);
                let sur = vceqq_u16(vandq_u16(v, vdupq_n_u16(0xF800)), vdupq_n_u16(0xD800));
                bits(vorrq_u16(small16(v), sur))
            }
        }
        #[inline(always)]
        fn first8(m: u64) -> usize {
            (m.trailing_zeros() / 4) as usize
        }
        #[inline(always)]
        fn first16(m: u64) -> usize {
            (m.trailing_zeros() / 8) as usize
        }
    }
}

// ---- wasm32 simd128 --------------------------------------------------------------------------

/// wasm32 body: simd128, chosen at compile time (`-C target-feature=+simd128`).
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm {
    use core::arch::wasm32::{
        i8x16_bitmask, i8x16_eq, i16x8_bitmask, i16x8_eq, u8x16_le, u8x16_narrow_i16x8,
        u8x16_splat, u16x8_extend_high_u8x16, u16x8_extend_low_u8x16, u16x8_gt, u16x8_le,
        u16x8_splat, v128, v128_and, v128_load, v128_or, v128_store, v128_store64_lane,
    };

    use super::Lanes;

    /// The escape lanes of 16 bytes (the wasm intrinsics are safe in a simd128 build).
    #[inline(always)]
    fn cls8(v: v128) -> v128 {
        let lt = u8x16_le(v, u8x16_splat(0x1F));
        let q = i8x16_eq(v, u8x16_splat(0x22));
        let b = i8x16_eq(v, u8x16_splat(0x5C));
        v128_or(lt, v128_or(q, b))
    }

    /// The lanes of 8 units below 0x20, `"` or `\`.
    #[inline(always)]
    fn small16(v: v128) -> v128 {
        let lt = u16x8_le(v, u16x8_splat(0x1F));
        let q = i16x8_eq(v, u16x8_splat(0x22));
        let b = i16x8_eq(v, u16x8_splat(0x5C));
        v128_or(lt, v128_or(q, b))
    }

    /// simd128: 16 bytes or 8 UTF-16 units per step.
    pub(super) struct V128;

    impl Lanes for V128 {
        const W8: usize = 16;
        const W16: usize = 8;
        #[inline(always)]
        unsafe fn l1_one(s: *const u8, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 16 bytes each.
            unsafe {
                let v = v128_load(s.cast::<v128>());
                v128_store(d.cast::<v128>(), v);
                u64::from(i8x16_bitmask(cls8(v)))
            }
        }
        #[inline(always)]
        unsafe fn l1_two(s: *const u8, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 16 bytes, 16 units.
            unsafe {
                let v = v128_load(s.cast::<v128>());
                v128_store(d.cast::<v128>(), u16x8_extend_low_u8x16(v));
                v128_store(d.add(8).cast::<v128>(), u16x8_extend_high_u8x16(v));
                u64::from(i8x16_bitmask(cls8(v)))
            }
        }
        #[inline(always)]
        unsafe fn u16_one(s: *const u16, d: *mut u8) -> u64 {
            // SAFETY: the caller's bound: 8 units, 8 bytes.
            unsafe {
                let v = v128_load(s.cast::<v128>());
                v128_store64_lane::<0>(u8x16_narrow_i16x8(v, v), d.cast::<u64>());
                let m = v128_or(small16(v), u16x8_gt(v, u16x8_splat(0xFF)));
                u64::from(i16x8_bitmask(m))
            }
        }
        #[inline(always)]
        unsafe fn u16_two(s: *const u16, d: *mut u16) -> u64 {
            // SAFETY: the caller's bound: 8 units each.
            unsafe {
                let v = v128_load(s.cast::<v128>());
                v128_store(d.cast::<v128>(), v);
                let sur = i16x8_eq(v128_and(v, u16x8_splat(0xF800)), u16x8_splat(0xD800));
                u64::from(i16x8_bitmask(v128_or(small16(v), sur)))
            }
        }
        #[inline(always)]
        fn first8(m: u64) -> usize {
            m.trailing_zeros() as usize
        }
        #[inline(always)]
        fn first16(m: u64) -> usize {
            m.trailing_zeros() as usize
        }
    }
}

/// Test hooks: one chunk body into a buffer of exactly the room of the bound, with a canary
/// after it (a store past the bound changes the canary).
#[cfg(test)]
pub(crate) mod testing {
    use super::{Done, GROW, Kernel, L1One, L1Two, Pair, SLACK, U16One, U16Two};

    /// The canary units after the room.
    const CANARY: usize = 64;

    fn chunk<P: Pair>(
        k: Kernel,
        src: &[P::S],
        from: usize,
        end: usize,
        fill: P::D,
    ) -> (Done, Vec<P::D>)
    where
        P::D: PartialEq + core::fmt::Debug,
    {
        let room = (end - from) * GROW + SLACK;
        let mut buf = vec![fill; room + CANARY];
        // SAFETY: from < end <= src.len() (the callers), and `buf` has the room of the bound.
        let d = unsafe { k.run::<P>(src, from, end, buf.as_mut_ptr()) };
        assert!(
            buf[room..].iter().all(|&c| c == fill),
            "a store past the room of the bound"
        );
        buf.truncate(d.wrote);
        (d, buf)
    }

    /// The body of `k` on `src[from..end]` (Latin-1), one-byte (`two` false) or two-byte
    /// output: (the next source unit, the units written, a widen stop).
    pub(crate) fn chunk_latin1(
        k: Kernel,
        two: bool,
        src: &[u8],
        from: usize,
        end: usize,
    ) -> (usize, Vec<u16>, bool) {
        if two {
            let (d, b) = chunk::<L1Two>(k, src, from, end, 0xA5A5);
            (d.i, b, d.widen)
        } else {
            let (d, b) = chunk::<L1One>(k, src, from, end, 0xA5);
            (d.i, b.into_iter().map(u16::from).collect(), d.widen)
        }
    }

    /// [`chunk_latin1`] for a UTF-16 source.
    pub(crate) fn chunk_utf16(
        k: Kernel,
        two: bool,
        src: &[u16],
        from: usize,
        end: usize,
    ) -> (usize, Vec<u16>, bool) {
        if two {
            let (d, b) = chunk::<U16Two>(k, src, from, end, 0xA5A5);
            (d.i, b, d.widen)
        } else {
            let (d, b) = chunk::<U16One>(k, src, from, end, 0xA5);
            (d.i, b.into_iter().map(u16::from).collect(), d.widen)
        }
    }
}
