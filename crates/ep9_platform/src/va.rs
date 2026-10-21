//! Address-space reservations, the cage and the process code range (H7).
//!
//! ```text
//!   0        min_base                                                        2^47 = CAGE_LIMIT
//!   |  ...   |   slot k (base = k * align)    |  ...  |   slot j      |  ...  |
//!            [ cage A: cage_size .... ]  ...           [ code range ]
//!            ^ heap pages (H9), one cage per isolate   ^ one per process (CodeRange::process):
//!                                                        JIT code + mmapped units, in chunks
//!   never used: TSRS_RESERVATION = [0x4001_0000_0000, +32 GiB)
//! ```
//!
//! - Every byte of a cage and of the code range is below 2^47. So a cell pointer always fits
//!   the NaN-box payload (H6), and the in-cage test of the conservative scan is one subtract
//!   and one compare. A return pc in the code range fits a native word (H07-Q5).
//! - [`CodeRange::process`] reserves the one code range of the process at its first call
//!   (128 MiB, [`DEFAULT_CODE_RANGE_SIZE`]). Isolates take [`CodeChunk`]s from it, so code of
//!   every isolate and every shared stub stay within direct-call reach of each other.
//! - A cage reserves no code range by default (`code_range_size` 0). A cage with a non-zero
//!   `code_range_size` still gets a plain reservation right after itself
//!   ([`Cage::code_range`]); no code goes there.
//! - Cages and the code range use the same search: aligned slots in a seeded order, each
//!   reserved at its exact address. A slot that is in use, above the VA limit of the host, or
//!   refused by the host (valgrind refuses hints that overlap its own mappings: EINVAL), is
//!   skipped. The VA limit can be below 2^47: x64 Windows user space ends 64 KiB below it, so
//!   a slot that ends at 2^47 (the top slot of the default cage) is never free there.
//! - Everything starts inaccessible. `commit` makes pages read-write; `decommit` drops them.
//! - Exception (`jit` feature, macOS arm64): the code range is mapped with `MAP_JIT` and
//!   read-write-execute protection, because macOS forbids `MAP_JIT` with `MAP_FIXED` (so the
//!   slot address is a hint, and the result is checked) and forbids any `mprotect` of a
//!   `MAP_JIT` page after it is RWX. Each thread still sees it as RX or RW only
//!   (`pthread_jit_write_protect_np`). See the `jit` module. Without `jit` the code range is a
//!   plain reservation.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io;
use std::ops::Range;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::sys;

#[cfg(not(target_pointer_width = "64"))]
compile_error!("ep9_platform needs a 64-bit target");

/// H7: every cage and code-range byte is below this address.
pub const CAGE_LIMIT: usize = 1 << 47;
/// H7 default cage size.
pub const DEFAULT_CAGE_SIZE: usize = 64 << 30;
/// Size of the process code range ([`CodeRange::process`]) on every target: the reach of an
/// arm64 direct call, so every call inside the code range can be a direct call.
pub const DEFAULT_CODE_RANGE_SIZE: usize = 128 << 20;
/// Default cage alignment. It is a multiple of the 256 KiB heap page (H9) and of the 2 MiB and
/// 1 GiB huge pages, and it keeps the slot count small (2^15 slots below 2^47).
pub const DEFAULT_CAGE_ALIGN: usize = 4 << 30;
/// Lowest default cage base. 64-bit macOS maps a 4 GiB zero page below this address.
pub const DEFAULT_MIN_BASE: usize = 4 << 30;
/// tsrs (in-process type checking) maps this fixed range on unix and aborts if it cannot
/// \[R09\]. The default cage never overlaps it.
pub const TSRS_RESERVATION: Range<usize> = 0x4001_0000_0000..0x4001_0000_0000 + (32 << 30);

/// Largest distance a direct call or branch can reach on this architecture.
#[cfg(target_arch = "aarch64")]
pub const DIRECT_BRANCH_REACH: usize = 128 << 20;
/// Largest distance a direct call or branch can reach on this architecture.
#[cfg(target_arch = "x86_64")]
pub const DIRECT_BRANCH_REACH: usize = 2 << 30;
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("ep9_platform supports aarch64 and x86_64 only");

/// Size of a VM page on this host (16 KiB on macOS arm64, 4 KiB on most others).
pub fn page_size() -> usize {
    sys::page_size()
}

/// Reservation bases are multiples of this: the page size on unix, 64 KiB on Windows.
pub fn allocation_granularity() -> usize {
    sys::granularity()
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// A range of address space that this process owns. No access until committed.
/// Drop releases the whole range.
#[derive(Debug)]
pub struct Reservation {
    base: usize,
    size: usize,
    kind: Kind,
}

/// How the range was mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Inaccessible until committed (every reservation except the one below).
    Plain,
    /// macOS arm64 code range: one `MAP_JIT` mapping, RWX from the start (see the module doc).
    MapJit,
}

impl Reservation {
    /// Reserves `size` bytes anywhere. `size` is a multiple of [`allocation_granularity`].
    pub fn reserve(size: usize) -> io::Result<Self> {
        check_size(size)?;
        let base = sys::reserve(0, size)?;
        Ok(Self {
            base,
            size,
            kind: Kind::Plain,
        })
    }

    /// Reserves exactly `[addr, addr + size)`. Returns `Ok(None)` if the range is not free, is
    /// above the host's user address space, or the host refuses it. `addr` and `size` are
    /// multiples of [`allocation_granularity`].
    pub fn reserve_at(addr: usize, size: usize) -> io::Result<Option<Self>> {
        check_at(addr, size)?;
        // Build the value only on success: dropping it would release a range we do not own.
        Ok(sys::reserve_at(addr, size)?.then(|| Self {
            base: addr,
            size,
            kind: Kind::Plain,
        }))
    }

    /// Reserves exactly `[addr, addr + size)` for generated code (`jit`). Returns `Ok(None)` if
    /// the range is not free.
    ///
    /// | Target | Mapping |
    /// |---|---|
    /// | macOS arm64 | one `MAP_JIT` mapping, RWX; per-thread W^X (`jit::WxMode::MapJit`) |
    /// | other | the same as [`Reservation::reserve_at`] |
    #[cfg(any(feature = "jit", test))]
    pub fn reserve_code_range_at(addr: usize, size: usize) -> io::Result<Option<Self>> {
        check_at(addr, size)?;
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        {
            Ok(sys::reserve_map_jit_at(addr, size)?.then(|| Self {
                base: addr,
                size,
                kind: Kind::MapJit,
            }))
        }
        #[cfg(not(all(target_vendor = "apple", target_arch = "aarch64")))]
        {
            Self::reserve_at(addr, size)
        }
    }

    /// True for a macOS arm64 code range from `Reservation::reserve_code_range_at` (`jit`).
    /// [`Reservation::commit`] and [`Reservation::decommit`] refuse such a range; use
    /// `jit::CodeMemory`.
    pub fn is_map_jit(&self) -> bool {
        self.kind == Kind::MapJit
    }

    pub fn base(&self) -> usize {
        self.base
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// First byte after the reservation.
    pub fn end(&self) -> usize {
        self.base + self.size
    }

    pub fn range(&self) -> Range<usize> {
        self.base..self.end()
    }

    /// One subtract and one compare.
    #[inline]
    pub fn contains(&self, addr: usize) -> bool {
        addr.wrapping_sub(self.base) < self.size
    }

    /// Makes `[base + offset, base + offset + len)` readable and writable. Pages that are already
    /// committed keep their contents. New pages read as zero. Both values are page multiples.
    /// Executable pages lose execute access; JIT code will use its own W^X calls (`jit`).
    pub fn commit(&self, offset: usize, len: usize) -> io::Result<()> {
        let addr = self.subrange(offset, len)?;
        self.check_plain()?;
        // SAFETY: `subrange` checked that the page-aligned range lies inside this reservation.
        // Commit only sets read-write protection; it never moves, frees or changes memory, so
        // no reference into the range becomes invalid.
        unsafe { sys::commit(addr, len) }
    }

    /// Drops the pages of `[base + offset, base + offset + len)` and makes them inaccessible.
    /// The range stays reserved. A later commit gives zero-filled pages.
    ///
    /// # Safety
    /// No live reference or pointer that will be dereferenced points into the range.
    pub unsafe fn decommit(&self, offset: usize, len: usize) -> io::Result<()> {
        let addr = self.subrange(offset, len)?;
        self.check_plain()?;
        // SAFETY: the range is inside this reservation (`subrange`), and the caller guarantees
        // nothing still uses it.
        unsafe { sys::decommit(addr, len) }
    }

    /// `MAP_JIT` pages cannot change protection after they are RWX (EACCES), and a plain
    /// `MAP_FIXED` remap would drop `MAP_JIT` for good.
    fn check_plain(&self) -> io::Result<()> {
        if self.kind == Kind::MapJit {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a MAP_JIT code range is committed through ep9_platform::jit::CodeMemory",
            ));
        }
        Ok(())
    }

    /// Checks `[offset, offset + len)` (page multiples, inside) and returns its address.
    pub(crate) fn subrange(&self, offset: usize, len: usize) -> io::Result<usize> {
        let ps = page_size();
        if len == 0 || !offset.is_multiple_of(ps) || !len.is_multiple_of(ps) {
            return Err(invalid("offset and length must be non-zero page multiples"));
        }
        if offset > self.size || len > self.size - offset {
            return Err(invalid("range is outside the reservation"));
        }
        Ok(self.base + offset)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        // SAFETY: this value owns the range. Callers that hand out pointers into it must keep
        // the reservation alive (the same rule as for any owned allocation).
        let r = unsafe { sys::release(self.base, self.size) };
        debug_assert!(r.is_ok(), "release failed: {r:?}");
    }
}

fn check_at(addr: usize, size: usize) -> io::Result<()> {
    check_size(size)?;
    if addr == 0 || !addr.is_multiple_of(allocation_granularity()) {
        return Err(invalid(
            "address is 0 or not a multiple of the allocation granularity",
        ));
    }
    if addr.checked_add(size).is_none() {
        return Err(invalid("range wraps around the address space"));
    }
    Ok(())
}

fn check_size(size: usize) -> io::Result<()> {
    if size == 0 || !size.is_multiple_of(allocation_granularity()) {
        return Err(invalid(
            "size must be a non-zero multiple of the allocation granularity",
        ));
    }
    Ok(())
}

/// Where and how big a cage is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CageConfig {
    /// Heap cage size. A multiple of [`allocation_granularity`].
    pub cage_size: usize,
    /// Size of a per-cage code range right after the cage: 0 (the default) for none. JIT code
    /// and units go to the process code range ([`CodeRange::process`]). A non-zero size is a
    /// multiple of the granularity and at most [`DIRECT_BRANCH_REACH`]; it reserves a plain
    /// range ([`Cage::code_range`]) that no code uses.
    pub code_range_size: usize,
    /// Cage base alignment: a power of two, at least the granularity.
    pub align: usize,
    /// Lowest cage base.
    pub min_base: usize,
    /// The cage (and a per-cage code range) ends at or below this address. At most
    /// [`CAGE_LIMIT`].
    pub limit: usize,
    /// Address ranges that the cage (and a per-cage code range) never overlaps.
    pub avoid: Vec<Range<usize>>,
    /// First slot to try. `None` derives a seed from ASLR'd addresses and the clock.
    pub seed: Option<u64>,
}

impl Default for CageConfig {
    fn default() -> Self {
        Self {
            cage_size: DEFAULT_CAGE_SIZE,
            code_range_size: 0,
            align: DEFAULT_CAGE_ALIGN,
            min_base: DEFAULT_MIN_BASE,
            limit: CAGE_LIMIT,
            avoid: vec![TSRS_RESERVATION],
            seed: None,
        }
    }
}

impl CageConfig {
    fn total(&self) -> usize {
        self.cage_size + self.code_range_size
    }

    fn validate(&self) -> Result<(), CageError> {
        let g = allocation_granularity();
        let bad = |m| Err(CageError::InvalidConfig(m));
        if self.cage_size == 0 || !self.cage_size.is_multiple_of(g) {
            return bad("cage_size must be a non-zero multiple of the allocation granularity");
        }
        if !self.code_range_size.is_multiple_of(g) {
            return bad("code_range_size must be 0 or a multiple of the allocation granularity");
        }
        if self.code_range_size > DIRECT_BRANCH_REACH {
            return bad("code_range_size is larger than the direct-branch reach");
        }
        let total = self.cage_size.checked_add(self.code_range_size);
        check_placement(total, self.align, self.min_base, self.limit)
            .map_err(CageError::InvalidConfig)
    }
}

/// The placement rules that cages and the code range share: `align` is a power of two and at
/// least the granularity, `limit` is at most 2^47, and `span` bytes fit between `min_base`
/// and `limit` (`None`: the span overflowed).
fn check_placement(
    span: Option<usize>,
    align: usize,
    min_base: usize,
    limit: usize,
) -> Result<(), &'static str> {
    if !align.is_power_of_two() || align < allocation_granularity() {
        return Err("align must be a power of two and at least the allocation granularity");
    }
    if limit > CAGE_LIMIT {
        return Err("limit is above 2^47");
    }
    let fits = span.is_some_and(|t| min_base.checked_add(t).is_some_and(|end| end <= limit));
    if !fits {
        return Err("the reservation does not fit between min_base and limit");
    }
    Ok(())
}

/// The slot search of cages and the code range (module doc). Tries `[base, base + span)` for
/// each `align`-aligned base with `min_base <= base` and `base + span <= limit`, starting at
/// slot `seed % slots` and wrapping around. Slots that overlap `avoid` are skipped without a
/// try. `try_at(base, tried)` returns `Ok(None)` if the slot is not free. The caller checked
/// the values with [`check_placement`].
fn search_slots<T>(
    span: usize,
    align: usize,
    min_base: usize,
    limit: usize,
    avoid: &[Range<usize>],
    seed: Option<u64>,
    mut try_at: impl FnMut(usize, usize) -> Result<Option<T>, CageError>,
) -> Result<T, CageError> {
    let first = min_base.div_ceil(align);
    let last = (limit - span) / align;
    if first > last {
        return Err(CageError::NoFreeSlot { slots: 0, tried: 0 });
    }
    let slots = last - first + 1;
    let seed = seed.unwrap_or_else(default_seed);
    let start = (seed % slots as u64) as usize;
    let mut tried = 0;
    for i in 0..slots {
        let base = (first + (start + i) % slots) * align;
        if avoid.iter().any(|a| overlaps(&(base..base + span), a)) {
            continue;
        }
        tried += 1;
        if let Some(found) = try_at(base, tried)? {
            return Ok(found);
        }
    }
    Err(CageError::NoFreeSlot { slots, tried })
}

/// Why a cage or the code range could not be reserved.
#[derive(Debug)]
pub enum CageError {
    InvalidConfig(&'static str),
    /// Every slot was in use or above the host VA limit.
    NoFreeSlot {
        slots: usize,
        tried: usize,
    },
    Os(io::Error),
    /// The startup self-check found a broken invariant (a bug).
    SelfCheck(&'static str),
}

impl fmt::Display for CageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(m) => write!(f, "invalid cage or code-range config: {m}"),
            Self::NoFreeSlot { slots, tried } => {
                write!(
                    f,
                    "no free slot below 2^47 ({tried} of {slots} slots tried)"
                )
            }
            Self::Os(e) => write!(f, "cage or code-range reservation failed: {e}"),
            Self::SelfCheck(m) => write!(f, "cage or code-range self-check failed: {m}"),
        }
    }
}

impl Error for CageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Os(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CageError {
    fn from(e: io::Error) -> Self {
        Self::Os(e)
    }
}

/// A heap cage (H7), plus a code range right after it only when `code_range_size > 0`.
/// Drop releases both.
#[derive(Debug)]
pub struct Cage {
    heap: Reservation,
    code: Option<Reservation>,
    config: CageConfig,
    tried: usize,
}

impl Cage {
    /// Reserves the cage (and a per-cage code range if `code_range_size > 0`), then runs
    /// [`Cage::self_check`].
    pub fn reserve(config: &CageConfig) -> Result<Self, CageError> {
        config.validate()?;
        let found = search_slots(
            config.total(),
            config.align,
            config.min_base,
            config.limit,
            &config.avoid,
            config.seed,
            |base, tried| {
                let Some(heap) = Reservation::reserve_at(base, config.cage_size)? else {
                    return Ok(None);
                };
                let code = if config.code_range_size == 0 {
                    None
                } else {
                    // Returning on `None` drops `heap`, which releases the cage part again.
                    let Some(code) = Reservation::reserve_at(heap.end(), config.code_range_size)?
                    else {
                        return Ok(None);
                    };
                    Some(code)
                };
                Ok(Some(Self {
                    heap,
                    code,
                    config: config.clone(),
                    tried,
                }))
            },
        )?;
        found.self_check()?;
        Ok(found)
    }

    /// The H7 startup self-check. [`Cage::reserve`] already ran it; it is cheap to run again.
    pub fn self_check(&self) -> Result<(), CageError> {
        let c = &self.config;
        let h = self.heap.range();
        check(h.start.is_multiple_of(c.align), "cage base is not aligned")?;
        check(h.start >= c.min_base, "cage base is below min_base")?;
        check(h.len() == c.cage_size, "cage size differs from the config")?;
        check(
            self.code.is_some() == (c.code_range_size > 0),
            "cage code range present without code_range_size, or missing",
        )?;
        let mut end = h.end;
        if let Some(code) = &self.code {
            let k = code.range();
            check(
                k.len() == c.code_range_size,
                "code-range size differs from the config",
            )?;
            check(
                k.start == h.end,
                "code range does not start at the cage end",
            )?;
            check(
                k.len() <= DIRECT_BRANCH_REACH,
                "code range is larger than the branch reach",
            )?;
            end = k.end;
        }
        check(
            end <= c.limit && c.limit <= CAGE_LIMIT,
            "cage or code range reaches 2^47",
        )?;
        let span = h.start..end;
        check(
            !c.avoid.iter().any(|a| overlaps(&span, a)),
            "cage overlaps an avoided range",
        )?;
        Ok(())
    }

    /// The heap part (H9 pages live here).
    pub fn heap(&self) -> &Reservation {
        &self.heap
    }

    /// The per-cage code range: `Some` only when `code_range_size > 0`.
    pub fn code_range(&self) -> Option<&Reservation> {
        self.code.as_ref()
    }

    /// In-cage test for the conservative scan: one subtract and one compare.
    #[inline]
    pub fn contains(&self, addr: usize) -> bool {
        self.heap.contains(addr)
    }

    /// How many slots the search tried (1 when the first slot was free).
    pub fn slots_tried(&self) -> usize {
        self.tried
    }

    pub fn config(&self) -> &CageConfig {
        &self.config
    }
}

/// `Ok` if `ok`, else the self-check error `m`.
fn check(ok: bool, m: &'static str) -> Result<(), CageError> {
    if ok {
        Ok(())
    } else {
        Err(CageError::SelfCheck(m))
    }
}

/// Where and how big the code range is. [`CodeRange::process`] uses the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeRangeConfig {
    /// A multiple of [`allocation_granularity`], at most [`DIRECT_BRANCH_REACH`].
    pub size: usize,
    /// Base alignment: a power of two, at least the granularity.
    pub align: usize,
    /// Lowest base.
    pub min_base: usize,
    /// The range ends at or below this address. At most [`CAGE_LIMIT`].
    pub limit: usize,
    /// Address ranges that the code range never overlaps.
    pub avoid: Vec<Range<usize>>,
    /// First slot to try. `None` derives a seed from ASLR'd addresses and the clock.
    pub seed: Option<u64>,
}

/// The cage defaults, with [`DEFAULT_CODE_RANGE_SIZE`]: 4 GiB-aligned slots from 4 GiB to
/// 2^47, the tsrs range avoided, a random first slot.
impl Default for CodeRangeConfig {
    fn default() -> Self {
        Self {
            size: DEFAULT_CODE_RANGE_SIZE,
            align: DEFAULT_CAGE_ALIGN,
            min_base: DEFAULT_MIN_BASE,
            limit: CAGE_LIMIT,
            avoid: vec![TSRS_RESERVATION],
            seed: None,
        }
    }
}

impl CodeRangeConfig {
    fn validate(&self) -> Result<(), CageError> {
        let bad = |m| Err(CageError::InvalidConfig(m));
        if self.size == 0 || !self.size.is_multiple_of(allocation_granularity()) {
            return bad(
                "code-range size must be a non-zero multiple of the allocation granularity",
            );
        }
        if self.size > DIRECT_BRANCH_REACH {
            return bad("code-range size is larger than the direct-branch reach");
        }
        check_placement(Some(self.size), self.align, self.min_base, self.limit)
            .map_err(CageError::InvalidConfig)
    }
}

/// The process code range (H07-Q6): one reservation for JIT code and mmapped units of every
/// isolate, below 2^47 and outside [`TSRS_RESERVATION`]. Code gets its pages as
/// [`CodeChunk`]s; with `jit`, `CodeChunk::commit` turns a chunk into `jit::CodeMemory`.
///
/// | Build | Mapping |
/// |---|---|
/// | `jit` (or `cfg(test)`) on macOS arm64 | one `MAP_JIT` RWX mapping at a hinted address ([`Reservation::reserve_code_range_at`]) |
/// | every other build | a plain reservation, inaccessible until committed |
///
/// The range is never released: [`CodeRange::process`] keeps it for the life of the process.
/// [`CodeRange::reserve`] makes a range of its own (tests, tools); drop releases that one.
#[derive(Debug)]
pub struct CodeRange {
    range: Reservation,
    /// Free page runs, offset -> length (both page multiples), never adjacent.
    free: Mutex<BTreeMap<usize, usize>>,
    tried: usize,
}

impl CodeRange {
    /// The code range of this process. The first call reserves it with
    /// [`CodeRangeConfig::default`] and runs [`CodeRange::self_check`]; every later call, on
    /// any thread, returns the same range. A failed first call reserves nothing, and the next
    /// call tries again.
    pub fn process() -> Result<&'static CodeRange, CageError> {
        static PROCESS: OnceLock<CodeRange> = OnceLock::new();
        static INIT: Mutex<()> = Mutex::new(());
        if let Some(r) = PROCESS.get() {
            return Ok(r);
        }
        let _init = INIT.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(r) = PROCESS.get() {
            return Ok(r);
        }
        let r = Self::reserve(&CodeRangeConfig::default())?;
        // INIT is held and PROCESS is empty, so this stores `r`.
        Ok(PROCESS.get_or_init(|| r))
    }

    /// Reserves a code range of its own (see the type doc for the mapping) and runs
    /// [`CodeRange::self_check`].
    pub fn reserve(config: &CodeRangeConfig) -> Result<CodeRange, CageError> {
        config.validate()?;
        #[cfg(any(feature = "jit", test))]
        let reserve: fn(usize, usize) -> io::Result<Option<Reservation>> =
            Reservation::reserve_code_range_at;
        #[cfg(not(any(feature = "jit", test)))]
        let reserve: fn(usize, usize) -> io::Result<Option<Reservation>> = Reservation::reserve_at;
        let range = search_slots(
            config.size,
            config.align,
            config.min_base,
            config.limit,
            &config.avoid,
            config.seed,
            |base, tried| Ok(reserve(base, config.size)?.map(|r| (r, tried))),
        )?;
        let (range, tried) = range;
        let free = BTreeMap::from([(0, range.size())]);
        let r = CodeRange {
            range,
            free: Mutex::new(free),
            tried,
        };
        r.check_config(config)?;
        Ok(r)
    }

    /// The H7 checks: below 2^47, at most the direct-branch reach, outside the tsrs range.
    pub fn self_check(&self) -> Result<(), CageError> {
        let k = self.range.range();
        check(k.end <= CAGE_LIMIT, "code range reaches 2^47")?;
        check(
            k.len() <= DIRECT_BRANCH_REACH,
            "code range is larger than the branch reach",
        )?;
        check(
            !overlaps(&k, &TSRS_RESERVATION),
            "code range overlaps the tsrs range",
        )
    }

    /// [`CodeRange::self_check`] plus the config's own placement.
    fn check_config(&self, c: &CodeRangeConfig) -> Result<(), CageError> {
        let k = self.range.range();
        check(k.start.is_multiple_of(c.align), "code range is not aligned")?;
        check(k.start >= c.min_base, "code range is below min_base")?;
        check(k.len() == c.size, "code-range size differs from the config")?;
        check(k.end <= c.limit, "code range ends above the limit")?;
        check(
            !c.avoid.iter().any(|a| overlaps(&k, a)),
            "code range overlaps an avoided range",
        )?;
        self.self_check()
    }

    /// Takes `len` bytes (rounded up to whole pages) as one chunk: the lowest free run that
    /// fits. Thread-safe. Errors: `InvalidInput` for 0, `OutOfMemory` if no free run fits.
    pub fn alloc(&self, len: usize) -> io::Result<CodeChunk<'_>> {
        let ps = page_size();
        let Some(len) = len.checked_next_multiple_of(ps).filter(|&l| l != 0) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a code chunk needs a non-zero length that fits the code range",
            ));
        };
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((&offset, &run)) = free.iter().find(|&(_, &run)| run >= len) else {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "the code range has no free run of this length",
            ));
        };
        free.remove(&offset);
        if run > len {
            free.insert(offset + len, run - len);
        }
        Ok(CodeChunk {
            range: self,
            offset,
            len,
        })
    }

    /// Gives a chunk's pages back and merges them with free neighbours.
    fn release(&self, mut offset: usize, mut len: usize) {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(next) = free.remove(&(offset + len)) {
            len += next;
        }
        if let Some((&prev, &prev_len)) = free.range(..offset).next_back()
            && prev + prev_len == offset
        {
            free.remove(&prev);
            offset = prev;
            len += prev_len;
        }
        debug_assert!(
            free.range(offset..offset + len).next().is_none(),
            "code chunk released twice"
        );
        free.insert(offset, len);
    }

    /// Bytes in free runs.
    pub fn free_bytes(&self) -> usize {
        let free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        free.values().sum()
    }

    pub fn base(&self) -> usize {
        self.range.base()
    }

    pub fn size(&self) -> usize {
        self.range.size()
    }

    /// First byte after the range.
    pub fn end(&self) -> usize {
        self.range.end()
    }

    pub fn range(&self) -> Range<usize> {
        self.range.range()
    }

    /// One subtract and one compare.
    #[inline]
    pub fn contains(&self, addr: usize) -> bool {
        self.range.contains(addr)
    }

    /// True if the range is one `MAP_JIT` mapping (macOS arm64 with `jit`; see the type doc).
    pub fn is_map_jit(&self) -> bool {
        self.range.is_map_jit()
    }

    /// How many slots the search tried (1 when the first slot was free).
    pub fn slots_tried(&self) -> usize {
        self.tried
    }

    /// The reservation. Crate-private: outside this crate, pages of the range change only
    /// through a [`CodeChunk`], so no chunk sees another's protection change.
    #[cfg_attr(not(any(feature = "jit", test)), allow(dead_code))]
    pub(crate) fn reservation(&self) -> &Reservation {
        &self.range
    }
}

/// Pages `[offset, offset + len)` of a [`CodeRange`] that belong to one owner until drop.
/// Chunks never overlap. Drop gives the pages back to the range (it does not decommit them;
/// `jit::CodeMemory` does that when it drops). With `jit`, `CodeChunk::commit` makes the
/// chunk code memory. A chunk never handed to `jit::CodeMemory` is reserved and inaccessible.
#[derive(Debug)]
pub struct CodeChunk<'r> {
    range: &'r CodeRange,
    offset: usize,
    len: usize,
}

impl<'r> CodeChunk<'r> {
    /// The range this chunk is part of.
    pub fn code_range(&self) -> &'r CodeRange {
        self.range
    }

    /// Offset from the range base (a page multiple).
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Length in bytes (a non-zero page multiple).
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false: [`CodeRange::alloc`] rejects 0.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// First byte of the chunk.
    pub fn base(&self) -> usize {
        self.range.base() + self.offset
    }

    /// `base()..base() + len()`.
    pub fn addr_range(&self) -> Range<usize> {
        self.base()..self.base() + self.len
    }
}

impl Drop for CodeChunk<'_> {
    fn drop(&mut self) {
        self.range.release(self.offset, self.len);
    }
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Seed from ASLR'd stack and text addresses plus the clock, mixed with splitmix64.
fn default_seed() -> u64 {
    let local = 0u8;
    let stack = std::ptr::addr_of!(local) as u64;
    let text = default_seed as *const () as u64;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut z = stack ^ text.rotate_left(21) ^ nanos.rotate_left(42);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_is_half_open() {
        assert!(overlaps(&(0..10), &(9..20)));
        assert!(!overlaps(&(0..10), &(10..20)));
        assert!(!overlaps(&(10..20), &(0..10)));
    }

    #[test]
    fn default_config_is_valid() {
        CageConfig::default().validate().unwrap();
        CodeRangeConfig::default().validate().unwrap();
        assert_eq!(
            CageConfig::default().code_range_size,
            0,
            "a cage has no code range by default"
        );
    }

    #[test]
    fn rejects_bad_configs() {
        let base = CageConfig::default();
        let cases = [
            CageConfig {
                cage_size: 0,
                ..base.clone()
            },
            CageConfig {
                code_range_size: DIRECT_BRANCH_REACH * 2,
                ..base.clone()
            },
            CageConfig {
                align: 3 << 30,
                ..base.clone()
            },
            CageConfig {
                limit: CAGE_LIMIT * 2,
                ..base.clone()
            },
            CageConfig {
                min_base: CAGE_LIMIT - (1 << 30),
                ..base.clone()
            },
            CageConfig {
                code_range_size: allocation_granularity() / 2,
                ..base.clone()
            },
        ];
        for c in cases {
            assert!(
                matches!(c.validate(), Err(CageError::InvalidConfig(_))),
                "{c:?}"
            );
        }
        let base = CodeRangeConfig::default();
        let cases = [
            CodeRangeConfig {
                size: 0,
                ..base.clone()
            },
            CodeRangeConfig {
                size: allocation_granularity() / 2,
                ..base.clone()
            },
            CodeRangeConfig {
                size: DIRECT_BRANCH_REACH * 2,
                ..base.clone()
            },
            CodeRangeConfig {
                align: 3 << 30,
                ..base.clone()
            },
            CodeRangeConfig {
                limit: CAGE_LIMIT * 2,
                ..base.clone()
            },
            CodeRangeConfig {
                min_base: CAGE_LIMIT - (64 << 20),
                ..base.clone()
            },
        ];
        for c in cases {
            assert!(
                matches!(c.validate(), Err(CageError::InvalidConfig(_))),
                "{c:?}"
            );
        }
    }

    /// Seed `slots - 1` makes the search try the slot that ends at `limit` first, then wrap to
    /// the lowest slot. `sp_va_skips_slot_above_host_limit` uses these seeds: on x64 Windows
    /// that top slot is above the user address space (CI run 39632519735).
    #[test]
    fn last_seed_tries_the_top_slot_first() {
        let cage = CageConfig::default();
        let code = CodeRangeConfig {
            align: DEFAULT_CODE_RANGE_SIZE,
            ..CodeRangeConfig::default()
        };
        for (span, align, min_base, avoid) in [
            (cage.total(), cage.align, cage.min_base, &cage.avoid),
            (code.size, code.align, code.min_base, &code.avoid),
        ] {
            let seed = ((CAGE_LIMIT - span) / align - min_base.div_ceil(align)) as u64;
            let mut tried = Vec::new();
            let found = search_slots(
                span,
                align,
                min_base,
                CAGE_LIMIT,
                avoid,
                Some(seed),
                |b, n| {
                    tried.push(b);
                    Ok((n == 2).then_some(b))
                },
            );
            assert_eq!(found.unwrap(), min_base);
            assert_eq!(tried, [CAGE_LIMIT - span, min_base]);
        }
    }

    fn small_cage(code_range_size: usize) -> CageConfig {
        CageConfig {
            cage_size: allocation_granularity(),
            code_range_size,
            ..CageConfig::default()
        }
    }

    #[test]
    fn cage_without_code_range() {
        // A fixed slot far from where the OS puts its own mappings (low on macOS, near 2^47 on
        // Linux), so only this cage can be there.
        let cage = Cage::reserve(&CageConfig {
            seed: Some(3000),
            ..small_cage(0)
        })
        .unwrap();
        assert!(cage.code_range().is_none());
        cage.self_check().unwrap();
        // The bytes right after the cage are not reserved by it.
        let g = allocation_granularity();
        let after = Reservation::reserve_at(cage.heap().end(), g).unwrap();
        assert!(
            after.is_some(),
            "the slot after a cage without a code range is free"
        );
    }

    #[test]
    fn cage_with_code_range_keeps_it() {
        let size = 1 << 20;
        let cage = Cage::reserve(&small_cage(size)).unwrap();
        let k = cage.code_range().expect("a per-cage code range");
        assert_eq!((k.base(), k.size()), (cage.heap().end(), size));
        assert!(
            !k.is_map_jit(),
            "a per-cage code range is a plain reservation"
        );
    }

    /// A private 1 MiB range: alloc rounds to pages, takes the lowest fit, refuses 0 and
    /// overflow, and drop merges free runs again.
    #[test]
    fn code_range_chunks() {
        let r = CodeRange::reserve(&CodeRangeConfig {
            size: 1 << 20,
            ..CodeRangeConfig::default()
        })
        .unwrap();
        r.self_check().unwrap();
        assert_eq!(r.size(), 1 << 20);
        assert!(r.base().is_multiple_of(DEFAULT_CAGE_ALIGN) && r.end() <= CAGE_LIMIT);
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        assert!(
            r.is_map_jit(),
            "unit tests build as jit: MAP_JIT on macOS arm64"
        );
        let ps = page_size();
        let pages = r.size() / ps;
        assert_eq!(r.alloc(0).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            r.alloc(usize::MAX).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let a = r.alloc(1).unwrap();
        assert_eq!((a.offset(), a.len(), a.base()), (0, ps, r.base()));
        let b = r.alloc(ps + 1).unwrap();
        assert_eq!((b.offset(), b.len()), (ps, 2 * ps));
        let c = r.alloc(ps).unwrap();
        assert_eq!(c.offset(), 3 * ps);
        assert!(r.contains(c.base()) && c.addr_range().end <= r.end());
        assert_eq!(r.free_bytes(), r.size() - 4 * ps);
        // Too big for what is left.
        assert_eq!(
            r.alloc(r.size()).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        // The freed middle run is the lowest fit again.
        drop(b);
        let d = r.alloc(2 * ps).unwrap();
        assert_eq!(d.offset(), ps);
        drop(d);
        let e = r.alloc(ps).unwrap();
        assert_eq!(e.offset(), ps, "lowest first");
        // Fill the range page by page, then nothing is left.
        let rest: Vec<_> = (0..pages - 3).map(|_| r.alloc(ps).unwrap()).collect();
        assert_eq!(r.free_bytes(), 0);
        assert_eq!(r.alloc(1).unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        // Drop in a mixed order: every other page first (holes), then the rest (each one
        // merges with a free run on both sides).
        drop(c);
        drop(a);
        let (odd, even): (Vec<_>, Vec<_>) =
            rest.into_iter().partition(|ch| ch.offset() / ps % 2 == 1);
        drop(odd);
        drop(even);
        drop(e);
        assert_eq!(r.free_bytes(), r.size());
        let all = r.alloc(r.size()).expect("one merged run");
        assert_eq!(all.offset(), 0);
    }

    /// Chunks taken on many threads at once never overlap.
    #[test]
    fn code_range_chunks_from_threads() {
        let r = CodeRange::reserve(&CodeRangeConfig {
            size: 4 << 20,
            ..CodeRangeConfig::default()
        })
        .unwrap();
        let ps = page_size();
        let chunks: Vec<CodeChunk<'_>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..8)
                .map(|t| {
                    let r = &r;
                    s.spawn(move || {
                        (0..8)
                            .map(|i| r.alloc(ps * (1 + (t + i) % 3)).unwrap())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut spans: Vec<Range<usize>> = chunks.iter().map(CodeChunk::addr_range).collect();
        spans.sort_unstable_by_key(|s| s.start);
        for w in spans.windows(2) {
            assert!(w[0].end <= w[1].start, "chunks overlap: {w:?}");
        }
        let used: usize = spans.iter().map(|s| s.len()).sum();
        assert_eq!(r.free_bytes(), r.size() - used);
        drop(chunks);
        assert_eq!(r.free_bytes(), r.size());
    }
}
