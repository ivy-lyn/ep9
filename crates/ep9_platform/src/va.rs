//! Address-space reservations, the cage and the code range (H7).
//!
//! ```text
//!   0        min_base                                                        2^47 = CAGE_LIMIT
//!   |  ...   |   slot k-1   |   slot k (base = k * align)           |   ...  |
//!                           [ cage: cage_size ......... ][ code range ]
//!                           ^ heap pages (H9)              ^ JIT code + mmapped units
//!   never used: TSRS_RESERVATION = [0x4001_0000_0000, +32 GiB)
//! ```
//!
//! - Every byte of the cage and the code range is below 2^47. So a cell pointer always fits
//!   the NaN-box payload (H6), and the in-cage test of the conservative scan is one subtract
//!   and one compare.
//! - The code range starts at the first byte after the cage.
//! - The search tries aligned slots in a seeded order and reserves each slot at its exact
//!   address. A slot that is in use, or above the VA limit of the host, is skipped.
//! - Everything starts inaccessible. `commit` makes pages read-write; `decommit` drops them.

use std::error::Error;
use std::fmt;
use std::io;
use std::ops::Range;

use crate::sys;

#[cfg(not(target_pointer_width = "64"))]
compile_error!("ep9_platform needs a 64-bit target");

/// H7: every cage and code-range byte is below this address.
pub const CAGE_LIMIT: usize = 1 << 47;
/// H7 default cage size.
pub const DEFAULT_CAGE_SIZE: usize = 64 << 30;
/// Default code-range size: the reach of an arm64 direct call, so every call inside the code
/// range can be a direct call.
pub const DEFAULT_CODE_RANGE_SIZE: usize = 128 << 20;
/// Default cage alignment. It is a multiple of the 256 KiB heap page (H9) and of the 2 MiB and
/// 1 GiB huge pages, and it keeps the slot count small (2^15 slots below 2^47).
pub const DEFAULT_CAGE_ALIGN: usize = 4 << 30;
/// Lowest default cage base. 64-bit macOS maps a 4 GiB zero page below this address.
pub const DEFAULT_MIN_BASE: usize = 4 << 30;
/// tsrs (in-process type checking) maps this fixed range on unix and aborts if it cannot
/// [R09]. The default cage never overlaps it.
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
}

impl Reservation {
    /// Reserves `size` bytes anywhere. `size` is a multiple of [`allocation_granularity`].
    pub fn reserve(size: usize) -> io::Result<Self> {
        check_size(size)?;
        let base = sys::reserve(0, size)?;
        Ok(Self { base, size })
    }

    /// Reserves exactly `[addr, addr + size)`. Returns `Ok(None)` if the range is not free.
    /// `addr` and `size` are multiples of [`allocation_granularity`].
    pub fn reserve_at(addr: usize, size: usize) -> io::Result<Option<Self>> {
        check_size(size)?;
        if addr == 0 || !addr.is_multiple_of(allocation_granularity()) {
            return Err(invalid(
                "address is 0 or not a multiple of the allocation granularity",
            ));
        }
        if addr.checked_add(size).is_none() {
            return Err(invalid("range wraps around the address space"));
        }
        // Build the value only on success: dropping it would release a range we do not own.
        Ok(sys::reserve_at(addr, size)?.then(|| Self { base: addr, size }))
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
        // SAFETY: the range is inside this reservation (`subrange`), and the caller guarantees
        // nothing still uses it.
        unsafe { sys::decommit(addr, len) }
    }

    fn subrange(&self, offset: usize, len: usize) -> io::Result<usize> {
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

fn check_size(size: usize) -> io::Result<()> {
    if size == 0 || !size.is_multiple_of(allocation_granularity()) {
        return Err(invalid(
            "size must be a non-zero multiple of the allocation granularity",
        ));
    }
    Ok(())
}

/// Where and how big the cage and the code range are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CageConfig {
    /// Heap cage size. A multiple of [`allocation_granularity`].
    pub cage_size: usize,
    /// Code-range size, at most [`DIRECT_BRANCH_REACH`]. A multiple of the granularity.
    pub code_range_size: usize,
    /// Cage base alignment: a power of two, at least the granularity.
    pub align: usize,
    /// Lowest cage base.
    pub min_base: usize,
    /// The code range ends at or below this address. At most [`CAGE_LIMIT`].
    pub limit: usize,
    /// Address ranges that the cage and the code range never overlap.
    pub avoid: Vec<Range<usize>>,
    /// First slot to try. `None` derives a seed from ASLR'd addresses and the clock.
    pub seed: Option<u64>,
}

impl Default for CageConfig {
    fn default() -> Self {
        Self {
            cage_size: DEFAULT_CAGE_SIZE,
            code_range_size: DEFAULT_CODE_RANGE_SIZE,
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
        if self.code_range_size == 0 || !self.code_range_size.is_multiple_of(g) {
            return bad(
                "code_range_size must be a non-zero multiple of the allocation granularity",
            );
        }
        if self.code_range_size > DIRECT_BRANCH_REACH {
            return bad("code_range_size is larger than the direct-branch reach");
        }
        if !self.align.is_power_of_two() || self.align < g {
            return bad("align must be a power of two and at least the allocation granularity");
        }
        if self.limit > CAGE_LIMIT {
            return bad("limit is above 2^47");
        }
        let fits = self
            .cage_size
            .checked_add(self.code_range_size)
            .is_some_and(|t| {
                self.min_base
                    .checked_add(t)
                    .is_some_and(|end| end <= self.limit)
            });
        if !fits {
            return bad("cage and code range do not fit between min_base and limit");
        }
        Ok(())
    }
}

/// Why a cage could not be reserved.
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
            Self::InvalidConfig(m) => write!(f, "invalid cage config: {m}"),
            Self::NoFreeSlot { slots, tried } => {
                write!(
                    f,
                    "no free cage slot below 2^47 ({tried} of {slots} slots tried)"
                )
            }
            Self::Os(e) => write!(f, "cage reservation failed: {e}"),
            Self::SelfCheck(m) => write!(f, "cage self-check failed: {m}"),
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

/// The heap cage plus the code range right after it (H7). Drop releases both.
#[derive(Debug)]
pub struct Cage {
    heap: Reservation,
    code: Reservation,
    config: CageConfig,
    tried: usize,
}

impl Cage {
    /// Reserves the cage and the code range, then runs [`Cage::self_check`].
    pub fn reserve(config: &CageConfig) -> Result<Self, CageError> {
        config.validate()?;
        let total = config.total();
        let first = config.min_base.div_ceil(config.align);
        let last = (config.limit - total) / config.align;
        if first > last {
            return Err(CageError::NoFreeSlot { slots: 0, tried: 0 });
        }
        let slots = last - first + 1;
        let seed = config.seed.unwrap_or_else(default_seed);
        let start = (seed % slots as u64) as usize;
        let mut tried = 0;
        for i in 0..slots {
            let base = (first + (start + i) % slots) * config.align;
            let span = base..base + total;
            if config.avoid.iter().any(|a| overlaps(&span, a)) {
                continue;
            }
            tried += 1;
            let Some(heap) = Reservation::reserve_at(base, config.cage_size)? else {
                continue;
            };
            // Dropping `heap` on the `continue` path releases the cage part again.
            let Some(code) = Reservation::reserve_at(heap.end(), config.code_range_size)? else {
                continue;
            };
            let cage = Self {
                heap,
                code,
                config: config.clone(),
                tried,
            };
            cage.self_check()?;
            return Ok(cage);
        }
        Err(CageError::NoFreeSlot { slots, tried })
    }

    /// The H7 startup self-check. [`Cage::reserve`] already ran it; it is cheap to run again.
    pub fn self_check(&self) -> Result<(), CageError> {
        let c = &self.config;
        let (h, k) = (self.heap.range(), self.code.range());
        let check = |ok: bool, m| {
            if ok {
                Ok(())
            } else {
                Err(CageError::SelfCheck(m))
            }
        };
        check(h.start.is_multiple_of(c.align), "cage base is not aligned")?;
        check(h.start >= c.min_base, "cage base is below min_base")?;
        check(h.len() == c.cage_size, "cage size differs from the config")?;
        check(
            k.len() == c.code_range_size,
            "code-range size differs from the config",
        )?;
        check(
            k.start == h.end,
            "code range does not start at the cage end",
        )?;
        check(
            k.end <= c.limit && c.limit <= CAGE_LIMIT,
            "cage or code range reaches 2^47",
        )?;
        check(
            k.len() <= DIRECT_BRANCH_REACH,
            "code range is larger than the branch reach",
        )?;
        let span = h.start..k.end;
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

    /// The code range (JIT code and mmapped units).
    pub fn code(&self) -> &Reservation {
        &self.code
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
        ];
        for c in cases {
            assert!(
                matches!(c.validate(), Err(CageError::InvalidConfig(_))),
                "{c:?}"
            );
        }
    }
}
