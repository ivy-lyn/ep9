//! Code-unit search on borrowed string payloads (OXC-PERF wave 2 STR-SEARCH-SIMD): the
//! search of StringIndexOf (§6.1.4.1) and of the String.prototype.lastIndexOf scan
//! (§22.1.3.11), with no copy of either string and no call per code unit.
//!
//! ```text
//! a step (no GC point; the caller takes the views just before it)
//!   scan   the next candidate positions (at most the budget left) for the first unit of the
//!          search string: find_unit (forward) or rfind_unit (backward)
//!   compare the candidate (a one-unit search is found at the scan hit), at most POLL_UNITS
//!          units per pass: forward from unit 1 (the scan proved unit 0 equal), backward
//!          from unit 0 (last_index_of_step says why)
//!   Poll   when POLL_UNITS units of work are done; the caller polls (a GC point), takes the
//!          views again and calls the next step with the same Cursor
//! work: 1 per position whose first unit differs, plus 1 per unit of each candidate that is
//! compared or proved by the scan (the count of the old per-position loop, so a long search
//! still polls)
//! ```
//!
//! The first-unit scan has one body per instruction set. [`Kernel::detect`] picks one at run
//! time, once per process (cached in an atomic); every body gives the result of the scalar one:
//!
//! | target | bodies | how the body is picked |
//! |---|---|---|
//! | x86-64 | AVX2 (32 bytes), SSE2 (16 bytes; the x86-64 base), scalar | `is_x86_feature_detected!("avx2")` |
//! | aarch64 (little-endian, `neon` in the build) | NEON (16 bytes; the aarch64 base), scalar | `is_aarch64_feature_detected!("neon")`, true at compile time |
//! | aarch64 big-endian, or no `neon` in the build | scalar | - |
//! | wasm32 | simd128 (16 bytes), scalar | at compile time: `cfg(target_feature = "simd128")` |
//! | other | scalar | - |
//!
//! No AVX-512 body: a run-time-detected AVX-512 leaf is allowed (a compile-time `avx512f` build is
//! not), but a 64-byte vector helps no measured row. SSE4.2 (`pcmpestri`) has no gain over SSE2
//! `pcmpeqb` for a one-unit scan, so it has no body. `EP9_SIMD_SEARCH` (`scalar`, `sse2`, `avx2`,
//! `neon`, `simd128`), read at the first detect, picks another body for a test run; a body that the
//! CPU or the build does not have is never picked (the detected one is used).
//!
//! The NEON bodies build only where `neon` is a feature of the whole build (the aarch64 base)
//! and the target is little-endian: the `shrn #4` mask puts unit `i` at bits `4i..4i+4` only
//! on a little-endian target (the technique note: napi-rs/escape-simd 26e04a0,
//! `crates/escape-simd/src/simd/bits.rs:59-80`, its big-endian fix b07b55b). So the NEON bodies
//! inline into the search step (no call), and aarch64_be takes the scalar body.
//!
//! The file uses only `core` and `std` (no crate item), so a standalone build of it proves the
//! wasm32 forms: ep9_vm itself does not build for wasm32.

use core::sync::atomic::{AtomicU8, Ordering};

/// Code units of work per poll (M2-P2-POLL; the `SearchWork` count of ep9_builtins).
pub(crate) const POLL_UNITS: usize = 256;

/// One code unit of a flat payload: a Latin-1 byte or a UTF-16 code unit.
pub(crate) trait Unit: Copy + Eq + 'static {
    /// The code unit as UTF-16.
    fn widen(self) -> u16;
    /// `s` as bytes, if `Self` is `u8`.
    fn bytes(s: &[Self]) -> Option<&[u8]>;
    /// `s` as UTF-16 units, if `Self` is `u16`.
    fn wide(s: &[Self]) -> Option<&[u16]>;
    /// The first `unit` in `hay`, by the body of `k`.
    fn find(k: Kernel, hay: &[Self], unit: u16) -> Option<usize>;
    /// The last `unit` in `hay` (scalar).
    fn rfind(hay: &[Self], unit: u16) -> Option<usize>;
}

impl Unit for u8 {
    #[inline(always)]
    fn widen(self) -> u16 {
        u16::from(self)
    }
    #[inline(always)]
    fn bytes(s: &[u8]) -> Option<&[u8]> {
        Some(s)
    }
    #[inline(always)]
    fn wide(_: &[u8]) -> Option<&[u16]> {
        None
    }
    #[inline(always)]
    fn find(k: Kernel, hay: &[u8], unit: u16) -> Option<usize> {
        k.find_u8(hay, u8::try_from(unit).ok()?)
    }
    #[inline(always)]
    fn rfind(hay: &[u8], unit: u16) -> Option<usize> {
        let b = u8::try_from(unit).ok()?;
        hay.iter().rposition(|&c| c == b)
    }
}

impl Unit for u16 {
    #[inline(always)]
    fn widen(self) -> u16 {
        self
    }
    #[inline(always)]
    fn bytes(_: &[u16]) -> Option<&[u8]> {
        None
    }
    #[inline(always)]
    fn wide(s: &[u16]) -> Option<&[u16]> {
        Some(s)
    }
    #[inline(always)]
    fn find(k: Kernel, hay: &[u16], unit: u16) -> Option<usize> {
        k.find_u16(hay, unit)
    }
    #[inline(always)]
    fn rfind(hay: &[u16], unit: u16) -> Option<usize> {
        hay.iter().rposition(|&c| c == unit)
    }
}

/// Whether two runs of code units of equal length hold the same units (either encoding). A
/// slice `==` of a length that is not a constant is a call to `bcmp`. Except on x86-64, a run
/// shorter than 16 units is compared in a loop here: the call costs more than the units of a
/// short search. On x86-64 the loop costs 8 instructions per unit, and the glibc `bcmp` of
/// fewer than 32 bytes is a short vector path (about 11 to 20 instructions), so the call
/// stays (W3-SIMD x86 fix: with the loop, the micro-search row ran more instructions there).
#[inline(always)]
pub(crate) fn units_eq<A: Unit, B: Unit>(a: &[A], b: &[B]) -> bool {
    debug_assert_eq!(a.len(), b.len());
    if !cfg!(target_arch = "x86_64") && a.len() < 16 {
        return a.iter().zip(b).all(|(x, y)| x.widen() == y.widen());
    }
    if let (Some(x), Some(y)) = (A::bytes(a), B::bytes(b)) {
        return x == y;
    }
    if let (Some(x), Some(y)) = (A::wide(a), B::wide(b)) {
        return x == y;
    }
    a.iter().zip(b).all(|(x, y)| x.widen() == y.widen())
}

/// The bodies of the first-unit scan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Path {
    Scalar = 1,
    Sse2 = 2,
    Avx2 = 3,
    Neon = 4,
    Simd128 = 5,
}

impl Path {
    /// Every path, in order.
    pub(crate) const ALL: [Path; 5] = [
        Path::Scalar,
        Path::Sse2,
        Path::Avx2,
        Path::Neon,
        Path::Simd128,
    ];

    /// The `EP9_SIMD_SEARCH` name of the path.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Path::Scalar => "scalar",
            Path::Sse2 => "sse2",
            Path::Avx2 => "avx2",
            Path::Neon => "neon",
            Path::Simd128 => "simd128",
        }
    }

    /// Whether this build and the running CPU can run the path's body.
    pub(crate) fn available(self) -> bool {
        match self {
            Path::Scalar => true,
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

/// True where the NEON bodies build (the `arm` module doc); the macro is a constant there,
/// because `neon` is a feature of the whole build.
#[cfg(all(
    target_arch = "aarch64",
    target_feature = "neon",
    target_endian = "little"
))]
fn neon_detected() -> bool {
    std::arch::is_aarch64_feature_detected!("neon")
}

#[cfg(not(all(
    target_arch = "aarch64",
    target_feature = "neon",
    target_endian = "little"
)))]
fn neon_detected() -> bool {
    false
}

/// A first-unit scan body that this CPU can run. The field is private, so a `Kernel` comes
/// only from [`Kernel::detect`], [`Kernel::scalar`] or [`Kernel::new`], which check the CPU:
/// that check is what makes the `unsafe` calls of the bodies sound.
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

impl Kernel {
    /// The best body for this CPU (or the `EP9_SIMD_SEARCH` one), detected once per process.
    #[inline]
    pub(crate) fn detect() -> Kernel {
        #[cfg(test)]
        if let Some(p) = FORCED.with(core::cell::Cell::get) {
            return Kernel(p);
        }
        match DETECTED.load(Ordering::Relaxed) {
            2 => Kernel(Path::Sse2),
            3 => Kernel(Path::Avx2),
            4 => Kernel(Path::Neon),
            5 => Kernel(Path::Simd128),
            1 => Kernel(Path::Scalar),
            _ => Kernel::detect_slow(),
        }
    }

    #[cold]
    #[inline(never)]
    fn detect_slow() -> Kernel {
        let p = pick(std::env::var("EP9_SIMD_SEARCH").ok().as_deref());
        DETECTED.store(p as u8, Ordering::Relaxed);
        Kernel(p)
    }

    /// The scalar body.
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

    /// The index of the first `b` in `hay`.
    #[inline]
    pub(crate) fn find_u8(self, hay: &[u8], b: u8) -> Option<usize> {
        match self.0 {
            // SAFETY: a Kernel of this path exists only when the CPU has AVX2 (`Kernel` doc).
            #[cfg(target_arch = "x86_64")]
            Path::Avx2 => unsafe { x86::find_u8_avx2(hay, b) },
            // SAFETY: SSE2 is part of the x86-64 base.
            #[cfg(target_arch = "x86_64")]
            Path::Sse2 => unsafe { x86::find_u8_sse2(hay, b) },
            #[cfg(all(
                target_arch = "aarch64",
                target_feature = "neon",
                target_endian = "little"
            ))]
            // SAFETY: this arm builds only with `neon` on for the whole build.
            Path::Neon => unsafe { arm::find_u8_neon(hay, b) },
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            Path::Simd128 => wasm::find_u8(hay, b),
            _ => scalar_find_u8(hay, b),
        }
    }

    /// The index of the first `u` in `hay`.
    #[inline]
    pub(crate) fn find_u16(self, hay: &[u16], u: u16) -> Option<usize> {
        match self.0 {
            // SAFETY: a Kernel of this path exists only when the CPU has AVX2 (`Kernel` doc).
            #[cfg(target_arch = "x86_64")]
            Path::Avx2 => unsafe { x86::find_u16_avx2(hay, u) },
            // SAFETY: SSE2 is part of the x86-64 base.
            #[cfg(target_arch = "x86_64")]
            Path::Sse2 => unsafe { x86::find_u16_sse2(hay, u) },
            #[cfg(all(
                target_arch = "aarch64",
                target_feature = "neon",
                target_endian = "little"
            ))]
            // SAFETY: this arm builds only with `neon` on for the whole build.
            Path::Neon => unsafe { arm::find_u16_neon(hay, u) },
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            Path::Simd128 => wasm::find_u16(hay, u),
            _ => scalar_find_u16(hay, u),
        }
    }
}

/// The path for an `EP9_SIMD_SEARCH` value: that path if this CPU and build have it, else
/// the best one that they have.
pub(crate) fn pick(env: Option<&str>) -> Path {
    let best = [Path::Avx2, Path::Sse2, Path::Neon, Path::Simd128]
        .into_iter()
        .find(|p| p.available())
        .unwrap_or(Path::Scalar);
    env.and_then(|v| Path::ALL.into_iter().find(|p| p.name() == v))
        .filter(|p| p.available())
        .unwrap_or(best)
}

/// The scalar body: the index of the first `b` in `hay`.
#[inline]
pub(crate) fn scalar_find_u8(hay: &[u8], b: u8) -> Option<usize> {
    hay.iter().position(|&c| c == b)
}

/// The scalar body: the index of the first `u` in `hay`.
#[inline]
pub(crate) fn scalar_find_u16(hay: &[u16], u: u16) -> Option<usize> {
    hay.iter().position(|&c| c == u)
}

/// x86-64 bodies. Each loads whole vectors inside the slice only: a slice shorter than one
/// vector goes to a narrower body, and the last step loads the last full vector of the slice,
/// which overlaps units that the step before tested (they held no match, so their mask bits
/// are 0).
#[cfg(target_arch = "x86_64")]
mod x86 {
    use core::arch::x86_64::{
        __m128i, __m256i, _mm_cmpeq_epi8, _mm_cmpeq_epi16, _mm_loadu_si128, _mm_movemask_epi8,
        _mm_set1_epi8, _mm_set1_epi16, _mm256_cmpeq_epi8, _mm256_cmpeq_epi16, _mm256_loadu_si256,
        _mm256_movemask_epi8, _mm256_set1_epi8, _mm256_set1_epi16,
    };

    /// SSE2: the first `b` in `hay`, 16 bytes per step.
    ///
    /// # Safety
    /// The CPU has SSE2 (always true on x86-64).
    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn find_u8_sse2(hay: &[u8], b: u8) -> Option<usize> {
        let n = hay.len();
        if n < 16 {
            return super::scalar_find_u8(hay, b);
        }
        let v = _mm_set1_epi8(b as i8);
        let mut i = 0;
        loop {
            let at = i.min(n - 16);
            // SAFETY: at + 16 <= n, so the 16 bytes are in `hay`.
            let x = unsafe { _mm_loadu_si128(hay.as_ptr().add(at).cast::<__m128i>()) };
            let m = _mm_movemask_epi8(_mm_cmpeq_epi8(x, v)) as u32;
            if m != 0 {
                return Some(at + m.trailing_zeros() as usize);
            }
            i += 16;
            if i >= n {
                return None;
            }
        }
    }

    /// SSE2: the first `u` in `hay`, 8 units per step (2 mask bits per unit).
    ///
    /// # Safety
    /// The CPU has SSE2 (always true on x86-64).
    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn find_u16_sse2(hay: &[u16], u: u16) -> Option<usize> {
        let n = hay.len();
        if n < 8 {
            return super::scalar_find_u16(hay, u);
        }
        let v = _mm_set1_epi16(u as i16);
        let mut i = 0;
        loop {
            let at = i.min(n - 8);
            // SAFETY: at + 8 <= n, so the 8 units are in `hay`.
            let x = unsafe { _mm_loadu_si128(hay.as_ptr().add(at).cast::<__m128i>()) };
            let m = _mm_movemask_epi8(_mm_cmpeq_epi16(x, v)) as u32;
            if m != 0 {
                return Some(at + (m.trailing_zeros() / 2) as usize);
            }
            i += 8;
            if i >= n {
                return None;
            }
        }
    }

    /// AVX2: the first `b` in `hay`, 32 bytes per step.
    ///
    /// # Safety
    /// The CPU has AVX2 (`is_x86_feature_detected!("avx2")`).
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn find_u8_avx2(hay: &[u8], b: u8) -> Option<usize> {
        let n = hay.len();
        if n < 32 {
            // SAFETY: AVX2 includes SSE2.
            return unsafe { find_u8_sse2(hay, b) };
        }
        let v = _mm256_set1_epi8(b as i8);
        let mut i = 0;
        loop {
            let at = i.min(n - 32);
            // SAFETY: at + 32 <= n, so the 32 bytes are in `hay`.
            let x = unsafe { _mm256_loadu_si256(hay.as_ptr().add(at).cast::<__m256i>()) };
            let m = _mm256_movemask_epi8(_mm256_cmpeq_epi8(x, v)) as u32;
            if m != 0 {
                return Some(at + m.trailing_zeros() as usize);
            }
            i += 32;
            if i >= n {
                return None;
            }
        }
    }

    /// AVX2: the first `u` in `hay`, 16 units per step (2 mask bits per unit).
    ///
    /// # Safety
    /// The CPU has AVX2 (`is_x86_feature_detected!("avx2")`).
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn find_u16_avx2(hay: &[u16], u: u16) -> Option<usize> {
        let n = hay.len();
        if n < 16 {
            // SAFETY: AVX2 includes SSE2.
            return unsafe { find_u16_sse2(hay, u) };
        }
        let v = _mm256_set1_epi16(u as i16);
        let mut i = 0;
        loop {
            let at = i.min(n - 16);
            // SAFETY: at + 16 <= n, so the 16 units are in `hay`.
            let x = unsafe { _mm256_loadu_si256(hay.as_ptr().add(at).cast::<__m256i>()) };
            let m = _mm256_movemask_epi8(_mm256_cmpeq_epi16(x, v)) as u32;
            if m != 0 {
                return Some(at + (m.trailing_zeros() / 2) as usize);
            }
            i += 16;
            if i >= n {
                return None;
            }
        }
    }
}

/// aarch64 bodies (NEON), in the x86 form. The compare mask narrows to a u64 with `shrn #4`:
/// 4 bits per byte, so 4 bits per Latin-1 unit and 8 bits per UTF-16 unit. Built only when
/// `neon` is on for the whole build and the target is little-endian (the file doc): on a
/// big-endian target the 16 nibbles come out in the reverse order.
#[cfg(all(
    target_arch = "aarch64",
    target_feature = "neon",
    target_endian = "little"
))]
mod arm {
    use core::arch::aarch64::{
        uint8x16_t, vceqq_u8, vceqq_u16, vdupq_n_u8, vdupq_n_u16, vget_lane_u64, vld1q_u8,
        vld1q_u16, vreinterpret_u64_u8, vreinterpretq_u8_u16, vreinterpretq_u16_u8, vshrn_n_u16,
    };

    /// The 64-bit mask of a byte compare: 4 bits per byte, byte `i` at bits `4i..4i+4` (the
    /// module is little-endian only). No `target_feature` (a safe fn with one must not be
    /// called where the feature is off): `neon` is on for the whole build.
    #[inline(always)]
    fn mask(eq: uint8x16_t) -> u64 {
        // SAFETY: the module builds only with `neon` on for the whole build, so the CPU has
        // NEON.
        unsafe {
            vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(vreinterpretq_u16_u8(
                eq,
            ))))
        }
    }

    /// NEON: the first `b` in `hay`, 16 bytes per step.
    ///
    /// # Safety
    /// The CPU has NEON (`is_aarch64_feature_detected!("neon")`).
    #[target_feature(enable = "neon")]
    pub(super) unsafe fn find_u8_neon(hay: &[u8], b: u8) -> Option<usize> {
        let n = hay.len();
        if n < 16 {
            return super::scalar_find_u8(hay, b);
        }
        let v = vdupq_n_u8(b);
        let mut i = 0;
        loop {
            let at = i.min(n - 16);
            // SAFETY: at + 16 <= n, so the 16 bytes are in `hay`.
            let x = unsafe { vld1q_u8(hay.as_ptr().add(at)) };
            let m = mask(vceqq_u8(x, v));
            if m != 0 {
                return Some(at + (m.trailing_zeros() / 4) as usize);
            }
            i += 16;
            if i >= n {
                return None;
            }
        }
    }

    /// NEON: the first `u` in `hay`, 8 units per step.
    ///
    /// # Safety
    /// The CPU has NEON (`is_aarch64_feature_detected!("neon")`).
    #[target_feature(enable = "neon")]
    pub(super) unsafe fn find_u16_neon(hay: &[u16], u: u16) -> Option<usize> {
        let n = hay.len();
        if n < 8 {
            return super::scalar_find_u16(hay, u);
        }
        let v = vdupq_n_u16(u);
        let mut i = 0;
        loop {
            let at = i.min(n - 8);
            // SAFETY: at + 8 <= n, so the 8 units are in `hay`.
            let x = unsafe { vld1q_u16(hay.as_ptr().add(at)) };
            let m = mask(vreinterpretq_u8_u16(vceqq_u16(x, v)));
            if m != 0 {
                return Some(at + (m.trailing_zeros() / 8) as usize);
            }
            i += 8;
            if i >= n {
                return None;
            }
        }
    }
}

/// wasm32 bodies (simd128, chosen at compile time), in the x86 form.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm {
    use core::arch::wasm32::{
        u8x16_bitmask, u8x16_eq, u8x16_splat, u16x8_bitmask, u16x8_eq, u16x8_splat, v128, v128_load,
    };

    /// simd128: the first `b` in `hay`, 16 bytes per step.
    pub(super) fn find_u8(hay: &[u8], b: u8) -> Option<usize> {
        let n = hay.len();
        if n < 16 {
            return super::scalar_find_u8(hay, b);
        }
        let v = u8x16_splat(b);
        let mut i = 0;
        loop {
            let at = i.min(n - 16);
            // SAFETY: at + 16 <= n, so the 16 bytes are in `hay`; v128.load needs no
            // alignment.
            let x = unsafe { v128_load(hay.as_ptr().add(at).cast::<v128>()) };
            let m = u32::from(u8x16_bitmask(u8x16_eq(x, v)));
            if m != 0 {
                return Some(at + m.trailing_zeros() as usize);
            }
            i += 16;
            if i >= n {
                return None;
            }
        }
    }

    /// simd128: the first `u` in `hay`, 8 units per step (1 mask bit per unit).
    pub(super) fn find_u16(hay: &[u16], u: u16) -> Option<usize> {
        let n = hay.len();
        if n < 8 {
            return super::scalar_find_u16(hay, u);
        }
        let v = u16x8_splat(u);
        let mut i = 0;
        loop {
            let at = i.min(n - 8);
            // SAFETY: at + 8 <= n, so the 8 units are in `hay`; v128.load needs no alignment.
            let x = unsafe { v128_load(hay.as_ptr().add(at).cast::<v128>()) };
            let m = u32::from(u16x8_bitmask(u16x8_eq(x, v)));
            if m != 0 {
                return Some(at + m.trailing_zeros() as usize);
            }
            i += 8;
            if i >= n {
                return None;
            }
        }
    }
}

// ---- the search steps --------------------------------------------------------------------

/// The state of one search between two polls.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    /// The candidate position.
    pos: usize,
    /// Units of the candidate at `pos` known equal (0: scan for the next candidate).
    matched: usize,
    /// Work since the last poll.
    work: usize,
}

impl Cursor {
    /// A search that starts at position `pos`.
    pub(crate) fn new(pos: usize) -> Cursor {
        Cursor {
            pos,
            matched: 0,
            work: 0,
        }
    }
}

/// The result of one step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    /// The search string is at this position.
    Found(usize),
    /// No candidate is left.
    NotFound,
    /// [`POLL_UNITS`] units of work are done: poll, then call the step again.
    Poll,
}

/// Compares the candidate at `c.pos` from `c.matched`: one pass of at most [`POLL_UNITS`]
/// units. `Some(true)`: the candidate is a match; `Some(false)`: it is not (`c.matched` is 0
/// again); `None`: more units are left to compare.
#[inline(always)]
fn compare<H: Unit, N: Unit>(hay: &[H], needle: &[N], c: &mut Cursor) -> Option<bool> {
    let (from, to) = (c.matched, needle.len().min(c.matched + POLL_UNITS));
    c.work += to - from;
    if units_eq(&hay[c.pos + from..c.pos + to], &needle[from..to]) {
        if to == needle.len() {
            return Some(true);
        }
        c.matched = to;
        None
    } else {
        c.matched = 0;
        Some(false)
    }
}

/// One step of StringIndexOf: the first position `i >= c.pos` with
/// `hay[i..i + needle.len()] == needle`. Needs `1 <= needle.len() <= hay.len()` and
/// `c.pos <= hay.len() - needle.len()`.
pub(crate) fn index_of_step<H: Unit, N: Unit>(
    k: Kernel,
    hay: &[H],
    needle: &[N],
    c: &mut Cursor,
) -> Step {
    debug_assert!(!needle.is_empty() && needle.len() <= hay.len());
    let last = hay.len() - needle.len();
    debug_assert!(c.pos <= last);
    let first = needle[0].widen();
    if H::bytes(hay).is_some() && first > 0xFF {
        // A Latin-1 string holds no unit above 0xFF.
        return Step::NotFound;
    }
    loop {
        if c.work >= POLL_UNITS {
            c.work -= POLL_UNITS;
            return Step::Poll;
        }
        if c.matched == 0 {
            let end = (last + 1).min(c.pos + (POLL_UNITS - c.work));
            match H::find(k, &hay[c.pos..end], first) {
                None => {
                    c.work += end - c.pos;
                    c.pos = end;
                    if end > last {
                        return Step::NotFound;
                    }
                    continue;
                }
                Some(d) => {
                    // The scan proved unit 0 equal: count it as 1 unit of work (the old
                    // compare did), then a one-unit search is found here and a longer one
                    // compares from unit 1 (V8 v8/v8 40f4b69c3b,
                    // src/strings/string-search.h:236-247 SingleCharSearch and :269-289
                    // LinearSearch, which compares from pattern[1]).
                    c.pos += d;
                    c.work += d + 1;
                    if needle.len() == 1 {
                        return Step::Found(c.pos);
                    }
                    c.matched = 1;
                }
            }
        }
        match compare(hay, needle, c) {
            Some(true) => return Step::Found(c.pos),
            Some(false) => {
                if c.pos == last {
                    return Step::NotFound;
                }
                c.pos += 1;
            }
            None => {}
        }
    }
}

/// One step of the lastIndexOf scan: the last position `i <= c.pos` with
/// `hay[i..i + needle.len()] == needle`. Needs `1 <= needle.len()` and
/// `c.pos + needle.len() <= hay.len()`. The backward scan is scalar.
pub(crate) fn last_index_of_step<H: Unit, N: Unit>(
    hay: &[H],
    needle: &[N],
    c: &mut Cursor,
) -> Step {
    debug_assert!(!needle.is_empty() && c.pos + needle.len() <= hay.len());
    let first = needle[0].widen();
    if H::bytes(hay).is_some() && first > 0xFF {
        return Step::NotFound;
    }
    loop {
        if c.work >= POLL_UNITS {
            c.work -= POLL_UNITS;
            return Step::Poll;
        }
        if c.matched == 0 {
            let lo = (c.pos + 1).saturating_sub(POLL_UNITS - c.work);
            match H::rfind(&hay[lo..=c.pos], first) {
                None => {
                    c.work += c.pos + 1 - lo;
                    if lo == 0 {
                        return Step::NotFound;
                    }
                    c.pos = lo - 1;
                    continue;
                }
                Some(d) => {
                    // A one-unit search is found at the scan hit, as in index_of_step. A
                    // longer one compares from unit 0 here: a compare from unit 1 needs
                    // `lo + d + 1` after the scan, and LLVM then keeps a copy of the counter
                    // and of the pointer in the backward scan loop (x86-64 asm: 2 more
                    // instructions per scanned unit; W3-SIMD x86 fix).
                    c.work += c.pos - (lo + d);
                    c.pos = lo + d;
                    if needle.len() == 1 {
                        return Step::Found(c.pos);
                    }
                }
            }
        }
        match compare(hay, needle, c) {
            Some(true) => return Step::Found(c.pos),
            Some(false) => {
                if c.pos == 0 {
                    return Step::NotFound;
                }
                c.pos -= 1;
            }
            None => {}
        }
    }
}
