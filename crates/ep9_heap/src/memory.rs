//! The heap reservation and its 256 KiB chunks.
//!
//! ```text
//! cage heap part (ep9_platform::Cage, below 2^47, H7)
//! [ chunk 0 | chunk 1 | chunk 2 ......................... | chunk high-1 ]  (never used) ...
//!   Small     Free      LargeHead LargeTail(2) LargeTail(2)   Free{committed}
//!
//! with a shape region of N chunks (HeapConfig::shape_space_bytes = N * 256 KiB, H08-Q1):
//! [ shape 0 | ... | shape N-1 | chunk N | chunk N+1 ............ | chunk high-1 ] ...
//!   Small (shape pages) or Free  ordinary chunks, as above
//!   ^ cage base: offset 0 is a page header, never a cell, so ShapeId 0 means "none"
//! ```
//!
//! - A chunk is one small page or one part of a large object (LOS).
//! - The shape region is chunks `[0, N)`. Its pages are small pages ([`Chunk::Small`]) that
//!   hold only shape-space cells. It has its own cursor (`shape_next`, the next chunk never
//!   used) and its own free set; the ordinary free set, the page cache and LOS never hold a
//!   region chunk. A freed shape page is decommitted. The region never moves.
//! - The chunk table maps any address in `[base, base + high * 256 KiB)` to its chunk in O(1).
//!   The conservative scan uses it for interior pointers into large objects.
//! - Small pages are committed whole. A large object commits only up to the OS page after its
//!   cell; the rest of its last chunk stays reserved.
//! - Up to [`MAX_CACHED_FREE_PAGES`] freed small pages stay committed for reuse (the cache);
//!   other freed chunks are decommitted. A new small page takes the lowest cached chunk, else
//!   the lowest free chunk. A large object takes the lowest free run and evicts cached chunks
//!   when the commit would pass `max_committed`, so the cache never causes an out-of-memory.
//! - Under Miri the reservation is one aligned block from the global allocator: Miri cannot
//!   map fixed addresses, but its allocations sit far below 2^47.

use std::collections::BTreeSet;
use std::ptr::NonNull;

use crate::layout::{PAGE_SIZE, Page};

/// Freed small pages that stay committed for reuse.
pub(crate) const MAX_CACHED_FREE_PAGES: usize = 16;

/// State of one 256 KiB chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chunk {
    Free {
        committed: bool,
    },
    Small,
    /// First chunk of a large object of `chunks` chunks.
    LargeHead {
        chunks: u32,
    },
    /// Another chunk of the large object that starts at chunk `head`.
    LargeTail {
        head: u32,
    },
}

/// Why the reservation could not be made.
#[derive(Debug)]
pub enum HeapError {
    /// `ep9_platform` could not reserve the cage.
    Cage(ep9_platform::CageError),
    /// The stack bounds of the current thread are unknown (needed by the conservative scan).
    Stack(std::io::Error),
    /// The configuration is not usable (the message says why).
    Config(&'static str),
}

impl std::fmt::Display for HeapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeapError::Cage(e) => write!(f, "heap reservation failed: {e}"),
            HeapError::Stack(e) => write!(f, "stack bounds unknown: {e}"),
            HeapError::Config(m) => write!(f, "invalid heap config: {m}"),
        }
    }
}

impl std::error::Error for HeapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HeapError::Cage(e) => Some(e),
            HeapError::Stack(e) => Some(e),
            HeapError::Config(_) => None,
        }
    }
}

pub(crate) struct Memory {
    /// Base of the reservation, with provenance over all of it.
    base: NonNull<u8>,
    size: usize,
    /// Chunks `[0, shape_chunks)` form the shape region (0: no shape space).
    shape_chunks: usize,
    /// The lowest region chunk that was never used.
    shape_next: usize,
    /// Region chunks below `shape_next` that are free (decommitted).
    shape_free: BTreeSet<u32>,
    #[cfg(not(miri))]
    cage: ep9_platform::Cage,
    #[cfg(miri)]
    layout: std::alloc::Layout,
    chunks: Vec<Chunk>,
    free: BTreeSet<u32>,
    /// The free chunks that stay committed (a subset of `free`).
    cached: BTreeSet<u32>,
    committed: usize,
    max_committed: usize,
    os_page: usize,
}

impl Memory {
    /// Reserves the heap part of a cage (`cage_size` bytes) and allows `max_committed` bytes to
    /// be committed at a time. No shape region (the unit tests' entry).
    #[cfg(test)]
    pub(crate) fn new(
        cage: &ep9_platform::CageConfig,
        max_committed: usize,
    ) -> Result<Memory, HeapError> {
        Self::with_shape_space(cage, max_committed, 0)
    }

    /// As `Memory::new` (test builds only), with the first `shape_bytes` of the reservation as
    /// the shape region (0: none). The checks are those of [`check_shape_space`], plus: the
    /// region fits the reservation (under Miri the reservation is smaller than the cage).
    pub(crate) fn with_shape_space(
        cage: &ep9_platform::CageConfig,
        max_committed: usize,
        shape_bytes: usize,
    ) -> Result<Memory, HeapError> {
        if cage.cage_size < PAGE_SIZE || !cage.cage_size.is_multiple_of(PAGE_SIZE) {
            return Err(HeapError::Config(
                "cage_size must be a non-zero multiple of 256 KiB",
            ));
        }
        check_shape_space(cage, shape_bytes)?;
        let mut m = Self::reserve(cage, max_committed)?;
        if shape_bytes > m.size {
            return Err(HeapError::Config(
                "shape_space_bytes is larger than the heap reservation",
            ));
        }
        // The region's chunks exist from the start, so ordinary chunks begin at index N.
        m.shape_chunks = shape_bytes / PAGE_SIZE;
        m.chunks = vec![Chunk::Free { committed: false }; m.shape_chunks];
        Ok(m)
    }

    #[cfg(not(miri))]
    fn reserve(
        config: &ep9_platform::CageConfig,
        max_committed: usize,
    ) -> Result<Memory, HeapError> {
        let mut config = config.clone();
        // Chunks must be 256 KiB aligned.
        config.align = config.align.max(PAGE_SIZE);
        let cage = ep9_platform::Cage::reserve(&config).map_err(HeapError::Cage)?;
        let base_addr = cage.heap().base();
        // The OS returned this range from mmap/VirtualAlloc; its address is exposed, so this
        // pointer may access the whole reservation.
        let base = NonNull::new(std::ptr::with_exposed_provenance_mut::<u8>(base_addr))
            .ok_or(HeapError::Config("cage base is null"))?;
        Ok(Memory {
            base,
            shape_chunks: 0,
            shape_next: 0,
            shape_free: BTreeSet::new(),
            size: cage.heap().size(),
            cage,
            chunks: Vec::new(),
            free: BTreeSet::new(),
            cached: BTreeSet::new(),
            committed: 0,
            max_committed,
            os_page: ep9_platform::page_size(),
        })
    }

    #[cfg(miri)]
    fn reserve(
        config: &ep9_platform::CageConfig,
        max_committed: usize,
    ) -> Result<Memory, HeapError> {
        // Miri: a small aligned block stands in for the cage.
        let size = config.cage_size.min(32 * PAGE_SIZE);
        let layout = std::alloc::Layout::from_size_align(size, PAGE_SIZE)
            .map_err(|_| HeapError::Config("bad Miri layout"))?;
        // SAFETY: the layout has a non-zero size.
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        let base = NonNull::new(p).ok_or(HeapError::Config("Miri allocation failed"))?;
        Ok(Memory {
            base,
            shape_chunks: 0,
            shape_next: 0,
            shape_free: BTreeSet::new(),
            size,
            layout,
            chunks: Vec::new(),
            free: BTreeSet::new(),
            cached: BTreeSet::new(),
            committed: 0,
            max_committed,
            os_page: 4096,
        })
    }

    pub(crate) fn base(&self) -> NonNull<u8> {
        self.base
    }

    pub(crate) fn base_addr(&self) -> usize {
        self.base.addr().get()
    }

    /// Total chunks the reservation can hold.
    pub(crate) fn max_chunks(&self) -> usize {
        self.size / PAGE_SIZE
    }

    /// Bytes of the shape region (0: no shape space).
    #[inline]
    pub(crate) fn shape_bytes(&self) -> usize {
        self.shape_chunks * PAGE_SIZE
    }

    /// True if chunk `idx` belongs to the shape region.
    #[inline]
    pub(crate) fn is_shape_chunk(&self, idx: usize) -> bool {
        idx < self.shape_chunks
    }

    /// The shape region plus the chunks handed out at least once: `[0, high)`.
    pub(crate) fn high(&self) -> usize {
        self.chunks.len()
    }

    pub(crate) fn committed_bytes(&self) -> usize {
        self.committed
    }

    pub(crate) fn os_page(&self) -> usize {
        self.os_page
    }

    /// True if `addr` lies in a chunk that was ever used. One subtract and one compare.
    #[inline]
    pub(crate) fn in_use_range(&self, addr: usize) -> bool {
        addr.wrapping_sub(self.base_addr()) < self.chunks.len() * PAGE_SIZE
    }

    /// True if `addr` lies inside the reservation.
    #[inline]
    pub(crate) fn contains(&self, addr: usize) -> bool {
        addr.wrapping_sub(self.base_addr()) < self.size
    }

    #[inline]
    pub(crate) fn chunk_index(&self, addr: usize) -> Option<usize> {
        let off = addr.wrapping_sub(self.base_addr());
        (off < self.chunks.len() * PAGE_SIZE).then_some(off / PAGE_SIZE)
    }

    pub(crate) fn chunk(&self, idx: usize) -> Chunk {
        self.chunks[idx]
    }

    /// Pointer to `addr`, with the provenance of the reservation.
    #[inline]
    pub(crate) fn ptr(&self, addr: usize) -> NonNull<u8> {
        debug_assert!(self.contains(addr), "{addr:#x} outside the heap");
        // SAFETY: `addr` is inside the reservation (checked in debug; every caller derives it
        // from a chunk or cell of this heap), so the offset stays in bounds and is not null.
        unsafe { self.base.add(addr - self.base_addr()) }
    }

    pub(crate) fn chunk_base(&self, idx: usize) -> NonNull<u8> {
        self.ptr(self.base_addr() + idx * PAGE_SIZE)
    }

    pub(crate) fn page(&self, idx: usize) -> Page {
        debug_assert_eq!(self.chunks[idx], Chunk::Small);
        // SAFETY: chunk `idx` is a committed small page.
        unsafe { Page::from_base(self.chunk_base(idx)) }
    }

    /// Every chunk index in state `Small`.
    pub(crate) fn small_pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == Chunk::Small)
            .map(|(i, _)| i)
    }

    /// Every chunk index in state `LargeHead`.
    pub(crate) fn large_heads(&self) -> impl Iterator<Item = usize> + '_ {
        self.chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, Chunk::LargeHead { .. }))
            .map(|(i, _)| i)
    }

    fn commit(&mut self, idx: usize, len: usize) -> bool {
        #[cfg(not(miri))]
        {
            if self
                .cage
                .heap()
                .commit(idx * PAGE_SIZE, len.next_multiple_of(self.os_page))
                .is_err()
            {
                return false;
            }
        }
        #[cfg(miri)]
        let _ = (idx, len);
        true
    }

    /// # Safety
    /// No live reference or pointer that will be dereferenced points into the range.
    unsafe fn decommit(&mut self, idx: usize, len: usize) {
        #[cfg(not(miri))]
        {
            // SAFETY: caller contract.
            let r = unsafe {
                self.cage
                    .heap()
                    .decommit(idx * PAGE_SIZE, len.next_multiple_of(self.os_page))
            };
            debug_assert!(r.is_ok(), "decommit failed: {r:?}");
        }
        #[cfg(miri)]
        {
            // Decommitted pages read as zero after the next commit.
            // SAFETY: the range is inside the reservation and unused (caller contract).
            unsafe { self.chunk_base(idx).as_ptr().write_bytes(0, len) };
        }
    }

    /// Takes one chunk for a small page and commits it. The caller writes the page header.
    pub(crate) fn alloc_small_page(&mut self) -> Option<usize> {
        // A cached chunk is already committed: it needs no commit and cannot pass the limit.
        if let Some(i) = self.cached.pop_first() {
            debug_assert_eq!(self.chunks[i as usize], Chunk::Free { committed: true });
            self.free.remove(&i);
            self.chunks[i as usize] = Chunk::Small;
            return Some(i as usize);
        }
        let (idx, pushed) = match self.free.first().copied() {
            Some(i) => (i as usize, false),
            None => {
                if self.chunks.len() >= self.max_chunks() {
                    return None;
                }
                self.chunks.push(Chunk::Free { committed: false });
                (self.chunks.len() - 1, true)
            }
        };
        debug_assert_eq!(self.chunks[idx], Chunk::Free { committed: false });
        if self.committed + PAGE_SIZE > self.max_committed || !self.commit(idx, PAGE_SIZE) {
            if pushed {
                self.chunks.pop();
            }
            return None;
        }
        self.committed += PAGE_SIZE;
        self.free.remove(&(idx as u32));
        self.chunks[idx] = Chunk::Small;
        Some(idx)
    }

    /// Takes the lowest free chunk of the shape region for a shape page and commits it. The
    /// caller writes the page header. `None` when the region is full or the commit limit
    /// is reached; cached ordinary pages are evicted first, so the cache never causes it.
    pub(crate) fn alloc_shape_page(&mut self) -> Option<usize> {
        let idx = match self.shape_free.first() {
            Some(&i) => i as usize,
            None if self.shape_next < self.shape_chunks => self.shape_next,
            None => return None,
        };
        debug_assert_eq!(self.chunks[idx], Chunk::Free { committed: false });
        if self.committed - self.cached.len() * PAGE_SIZE + PAGE_SIZE > self.max_committed {
            return None;
        }
        while self.committed + PAGE_SIZE > self.max_committed {
            let i = *self
                .cached
                .last()
                .expect("the check above leaves enough cached chunks");
            self.evict_cached(i);
        }
        if !self.commit(idx, PAGE_SIZE) {
            return None;
        }
        self.committed += PAGE_SIZE;
        if idx == self.shape_next {
            self.shape_next += 1;
        } else {
            self.shape_free.remove(&(idx as u32));
        }
        self.chunks[idx] = Chunk::Small;
        Some(idx)
    }

    /// Returns a swept, empty small page. It stays committed while the cache has room. A shape
    /// page goes back to the shape region, decommitted.
    pub(crate) fn free_small_page(&mut self, idx: usize) {
        debug_assert_eq!(self.chunks[idx], Chunk::Small);
        if self.is_shape_chunk(idx) {
            // SAFETY: the page holds no live cell (the sweep found none), and the heap keeps no
            // pointer into a free chunk.
            unsafe { self.decommit(idx, PAGE_SIZE) };
            self.committed -= PAGE_SIZE;
            self.chunks[idx] = Chunk::Free { committed: false };
            self.shape_free.insert(idx as u32);
            return;
        }
        if self.cached.len() < MAX_CACHED_FREE_PAGES {
            self.cached.insert(idx as u32);
            self.chunks[idx] = Chunk::Free { committed: true };
        } else {
            // SAFETY: the page holds no live cell (the sweep found none), and the heap keeps no
            // pointer into a free chunk.
            unsafe { self.decommit(idx, PAGE_SIZE) };
            self.committed -= PAGE_SIZE;
            self.chunks[idx] = Chunk::Free { committed: false };
        }
        self.free.insert(idx as u32);
    }

    /// Takes `n` consecutive chunks for a large object and commits the first `commit` bytes.
    pub(crate) fn alloc_large(&mut self, n: usize, commit: usize) -> Option<usize> {
        debug_assert!(n >= 1 && commit <= n * PAGE_SIZE);
        let commit = commit.next_multiple_of(self.os_page);
        // Lowest run of n free chunks; a run that reaches `high` may grow past it.
        let mut run_start = usize::MAX;
        let mut run_len = 0usize;
        let mut found = None;
        for &f in &self.free {
            let f = f as usize;
            if run_len > 0 && f == run_start + run_len {
                run_len += 1;
            } else {
                run_start = f;
                run_len = 1;
            }
            if run_len == n {
                found = Some(run_start);
                break;
            }
        }
        let start = match found {
            Some(s) => s,
            None => {
                let tail = if run_len > 0 && run_start + run_len == self.chunks.len() {
                    run_start
                } else {
                    self.chunks.len()
                };
                if tail + n > self.max_chunks() {
                    return None;
                }
                tail
            }
        };
        // Cached chunks count as committed. Check the limit as if every cached chunk were
        // decommitted, then decommit the ones in the run (they are recommitted as part of the
        // object) and, highest first, as many others as the limit needs.
        if self.committed - self.cached.len() * PAGE_SIZE + commit > self.max_committed {
            return None;
        }
        while let Some(&i) = self.cached.range(start as u32..(start + n) as u32).next() {
            self.evict_cached(i);
        }
        while self.committed + commit > self.max_committed {
            let i = *self
                .cached
                .last()
                .expect("the check above leaves enough cached chunks");
            self.evict_cached(i);
        }
        let old_len = self.chunks.len();
        while self.chunks.len() < start + n {
            self.chunks.push(Chunk::Free { committed: false });
        }
        if !self.commit(start, commit) {
            self.chunks.truncate(old_len);
            return None;
        }
        self.committed += commit;
        for i in start..start + n {
            self.free.remove(&(i as u32));
            self.chunks[i] = if i == start {
                Chunk::LargeHead { chunks: n as u32 }
            } else {
                Chunk::LargeTail { head: start as u32 }
            };
        }
        Some(start)
    }

    /// Decommits a cached chunk. It stays free.
    fn evict_cached(&mut self, i: u32) {
        let was_cached = self.cached.remove(&i);
        debug_assert!(was_cached && self.chunks[i as usize] == Chunk::Free { committed: true });
        // SAFETY: a free chunk holds no live cell and nothing points into it.
        unsafe { self.decommit(i as usize, PAGE_SIZE) };
        self.committed -= PAGE_SIZE;
        self.chunks[i as usize] = Chunk::Free { committed: false };
    }

    /// Frees a large object: decommits its `committed` bytes and frees its chunks.
    ///
    /// # Safety
    /// No live reference or pointer that will be dereferenced points into the object.
    pub(crate) unsafe fn free_large(&mut self, idx: usize, committed: usize) {
        let Chunk::LargeHead { chunks } = self.chunks[idx] else {
            panic!("chunk {idx} is not a large object head");
        };
        let committed = committed.next_multiple_of(self.os_page);
        // SAFETY: caller contract.
        unsafe { self.decommit(idx, committed) };
        self.committed -= committed;
        for i in idx..idx + chunks as usize {
            self.chunks[i] = Chunk::Free { committed: false };
            self.free.insert(i as u32);
        }
    }
}

/// The shape-space rules of [`crate::HeapConfig::shape_space_bytes`] that do not need the
/// reservation: 0 (off), or a power of two from 1 MiB to 4 GiB and at most the cage size. Outside
/// Miri the cage must be aligned to at least 4 GiB, so a ShapeId (the offset from the cage base)
/// is also the low 32 bits of the shape's address (JSC StructureID). Miri's stand-in
/// reservation is aligned to 256 KiB only, so Miri skips that rule; ids still use the subtract.
pub(crate) fn check_shape_space(
    cage: &ep9_platform::CageConfig,
    shape_bytes: usize,
) -> Result<(), HeapError> {
    if shape_bytes == 0 {
        return Ok(());
    }
    if !shape_bytes.is_power_of_two() || !(MIN_SHAPE_SPACE..=MAX_SHAPE_SPACE).contains(&shape_bytes)
    {
        return Err(HeapError::Config(
            "shape_space_bytes must be 0 or a power of two from 1 MiB to 4 GiB",
        ));
    }
    if shape_bytes > cage.cage_size {
        return Err(HeapError::Config(
            "shape_space_bytes is larger than the cage",
        ));
    }
    #[cfg(not(miri))]
    if cage.align < SHAPE_CAGE_ALIGN {
        return Err(HeapError::Config(
            "a shape space needs cage.align >= 4 GiB (a ShapeId is the low 32 bits of an address)",
        ));
    }
    Ok(())
}

/// Smallest non-zero shape space.
pub(crate) const MIN_SHAPE_SPACE: usize = 1 << 20;
/// Largest shape space: every offset fits a u32 ShapeId.
pub(crate) const MAX_SHAPE_SPACE: usize = 1 << 32;
/// Cage alignment that a shape space needs outside Miri.
#[cfg_attr(miri, allow(dead_code))]
pub(crate) const SHAPE_CAGE_ALIGN: usize = 1 << 32;

const _: () = assert!(MIN_SHAPE_SPACE.is_multiple_of(PAGE_SIZE));

#[cfg(miri)]
impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: `base` came from `alloc_zeroed(self.layout)` in `reserve`.
        unsafe { std::alloc::dealloc(self.base.as_ptr(), self.layout) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config() -> ep9_platform::CageConfig {
        ep9_platform::CageConfig {
            cage_size: 16 * PAGE_SIZE,
            code_range_size: 0,
            ..ep9_platform::CageConfig::default()
        }
    }

    /// H07-Q6: a heap's cage has no code range (code goes to `CodeRange::process()`), with an
    /// explicit `code_range_size: 0` and with the default config.
    #[cfg(not(miri))]
    #[test]
    fn heap_new_reserves_no_code_range() {
        // A fixed slot far from where the OS puts its own mappings (low on macOS, near 2^47 on
        // Linux), so only this cage can be there.
        let small = crate::HeapConfig {
            cage: ep9_platform::CageConfig {
                seed: Some(3001),
                ..small_config()
            },
            ..crate::HeapConfig::default()
        };
        for (config, small) in [(small, true), (crate::HeapConfig::default(), false)] {
            let heap = crate::Heap::new(config).unwrap();
            let inner = heap.inner.borrow();
            let cage = &inner.mem.cage;
            assert!(cage.code_range().is_none(), "{:?}", cage.config());
            assert_eq!(cage.config().code_range_size, 0);
            if small {
                // The 4 MiB cage ends inside its 4 GiB slot, where no other cage can start: the
                // bytes right after it are free.
                let end = cage.heap().end();
                let g = ep9_platform::allocation_granularity();
                let after = ep9_platform::Reservation::reserve_at(end, g).unwrap();
                assert!(after.is_some(), "nothing is reserved at {end:#x}");
            }
        }
    }

    #[test]
    fn rejects_unaligned_cage_size() {
        let c = ep9_platform::CageConfig {
            cage_size: PAGE_SIZE + 4096,
            ..ep9_platform::CageConfig::default()
        };
        assert!(matches!(
            Memory::new(&c, usize::MAX),
            Err(HeapError::Config(_))
        ));
    }

    #[test]
    fn chunks_small_and_large() {
        let mut m = Memory::new(&small_config(), usize::MAX).unwrap();
        assert!(m.base_addr().is_multiple_of(PAGE_SIZE));
        assert!((m.base_addr() as u64) < (1 << 47));
        let a = m.alloc_small_page().unwrap();
        let b = m.alloc_small_page().unwrap();
        assert_eq!((a, b), (0, 1));
        assert_eq!(m.committed_bytes(), 2 * PAGE_SIZE);
        // A 3-chunk object commits only what it needs.
        let l = m.alloc_large(3, PAGE_SIZE + 100).unwrap();
        assert_eq!(l, 2);
        assert_eq!(m.chunk(2), Chunk::LargeHead { chunks: 3 });
        assert_eq!(m.chunk(4), Chunk::LargeTail { head: 2 });
        assert_eq!(
            m.committed_bytes(),
            2 * PAGE_SIZE + (PAGE_SIZE + 100).next_multiple_of(m.os_page())
        );
        // Address lookup.
        let inside = m.base_addr() + 4 * PAGE_SIZE + 17;
        assert_eq!(m.chunk_index(inside), Some(4));
        assert_eq!(m.chunk_index(m.base_addr() + 5 * PAGE_SIZE), None);
        assert!(m.in_use_range(inside) && !m.in_use_range(m.base_addr() - 1));
        // Free the first page (cached, committed) and the large object (decommitted).
        m.free_small_page(a);
        assert_eq!(m.chunk(a), Chunk::Free { committed: true });
        // SAFETY: nothing points into the large object.
        unsafe { m.free_large(l, PAGE_SIZE + 100) };
        assert_eq!(m.committed_bytes(), 2 * PAGE_SIZE);
        // Reuse: lowest free chunk first; a 4-chunk run grows past the end.
        assert_eq!(m.alloc_small_page(), Some(0));
        let l2 = m.alloc_large(4, PAGE_SIZE).unwrap();
        assert_eq!(l2, 2, "chunks 2-4 are free and the run grows to 5");
        assert_eq!(m.high(), 6);
    }

    #[test]
    fn limits() {
        let mut m = Memory::new(&small_config(), 2 * PAGE_SIZE).unwrap();
        assert!(m.alloc_small_page().is_some());
        assert!(m.alloc_small_page().is_some());
        assert_eq!(m.alloc_small_page(), None, "committed-bytes limit");
        assert_eq!(m.high(), 2, "a failed allocation leaves no chunk behind");
        let mut m = Memory::new(&small_config(), usize::MAX).unwrap();
        let max = m.max_chunks();
        assert_eq!(m.alloc_large(max + 1, PAGE_SIZE), None, "reservation limit");
        assert!(m.alloc_large(max, PAGE_SIZE).is_some());
        assert_eq!(m.alloc_small_page(), None);
    }

    #[test]
    fn shape_region_chunks() {
        let region = 4 * PAGE_SIZE;
        let mut m = Memory::with_shape_space(&small_config(), usize::MAX, region).unwrap();
        assert_eq!((m.shape_bytes(), m.high()), (region, 4));
        assert!(m.is_shape_chunk(3) && !m.is_shape_chunk(4));
        // Ordinary chunks start after the region; region chunks come from its own cursor.
        assert_eq!(m.alloc_small_page(), Some(4));
        assert_eq!(m.alloc_shape_page(), Some(0));
        assert_eq!(m.alloc_shape_page(), Some(1));
        assert_eq!(m.alloc_large(2, PAGE_SIZE), Some(5));
        assert_eq!(m.chunk_index(m.base_addr() + 17), Some(0));
        assert_eq!(m.committed_bytes(), 3 * PAGE_SIZE + PAGE_SIZE);
        // A freed shape page is decommitted and goes back to the region, lowest first.
        m.free_small_page(0);
        assert_eq!(m.chunk(0), Chunk::Free { committed: false });
        assert_eq!(m.committed_bytes(), 3 * PAGE_SIZE);
        m.free_small_page(4);
        assert_eq!(
            m.chunk(4),
            Chunk::Free { committed: true },
            "ordinary page cached"
        );
        assert_eq!(m.alloc_small_page(), Some(4), "never a region chunk");
        assert_eq!(m.alloc_shape_page(), Some(0));
        assert_eq!(m.alloc_shape_page(), Some(2));
        assert_eq!(m.alloc_shape_page(), Some(3));
        assert_eq!(m.alloc_shape_page(), None, "region full");
        assert_eq!(m.alloc_small_page(), Some(7));
    }

    #[test]
    fn shape_pages_evict_cached_pages_under_the_limit() {
        let mut m = Memory::with_shape_space(&small_config(), PAGE_SIZE, 4 * PAGE_SIZE).unwrap();
        let s = m.alloc_small_page().unwrap();
        m.free_small_page(s);
        assert_eq!(m.committed_bytes(), PAGE_SIZE, "one cached page");
        assert_eq!(m.alloc_shape_page(), Some(0), "the cached page is evicted");
        assert_eq!(m.chunk(s), Chunk::Free { committed: false });
        assert_eq!(m.committed_bytes(), PAGE_SIZE);
        assert_eq!(m.alloc_shape_page(), None, "a real limit");
    }

    #[test]
    fn cached_pages_never_cause_out_of_memory() {
        let kib = 1024;
        let mut m = Memory::new(&small_config(), PAGE_SIZE + 128 * kib).unwrap();
        let l = m.alloc_large(1, 64 * kib).unwrap();
        let s = m.alloc_small_page().unwrap();
        assert_eq!((l, s), (0, 1));
        m.free_small_page(s);
        // SAFETY: nothing points into the large object.
        unsafe { m.free_large(l, 64 * kib) };
        assert_eq!(m.chunk(0), Chunk::Free { committed: false });
        assert_eq!(m.chunk(1), Chunk::Free { committed: true });
        assert_eq!(m.committed_bytes(), PAGE_SIZE);
        // A small page takes the cached chunk 1; committing chunk 0 would pass the limit.
        assert_eq!(m.alloc_small_page(), Some(1));
        assert_eq!(m.committed_bytes(), PAGE_SIZE);
        // A large object in chunk 0 evicts the cached chunk 1, which is outside its run.
        m.free_small_page(1);
        assert_eq!(m.alloc_large(1, 200 * kib), Some(0));
        assert_eq!(m.chunk(1), Chunk::Free { committed: false });
        let committed = (200 * kib).next_multiple_of(m.os_page());
        assert_eq!(m.committed_bytes(), committed);
        // Nothing left to evict: a real out-of-memory.
        assert_eq!(m.alloc_large(1, 200 * kib), None);
        assert_eq!(m.committed_bytes(), committed);
    }
}
