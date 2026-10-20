//! Page geometry and side metadata (H9).
//!
//! ```text
//! small page (256 KiB, 256 KiB aligned)          large object (LOS, one run of whole chunks)
//! +0      PageHeader (64 B)                      +0    LargeHeader (64 B): size, chunks, mark
//! +64     mark bitmap   1 bit / 8 B  (4 KiB)     +64   the cell (word0 first)
//! +4160   VO bitmap     1 bit / 8 B  (4 KiB)     ...   committed up to the OS page after the cell
//! +8256   line marks    1 B / 256 B line (1 KiB)
//! +9472   cells (bump allocation into free lines; first line = 37)
//! ```
//!
//! | Item | Value | Why |
//! |---|---|---|
//! | Page | 256 KiB, aligned | H9; `addr & PAGE_MASK` finds the page header |
//! | Granule | 8 B | H09-Q1 default: cells are 8-byte aligned; 1 bit per granule |
//! | Line | 256 B | Immix line; line marks are ready for S3 evacuation |
//! | Header | in the page | H09-Q2 decided in S0 core: one mask finds all metadata, as JIT code needs |
//! | Small cell | 16 B ..= 8 KiB | H09-Q3 default: LOS above 8 KiB |
//! | Medium cell | above 256 B | Immix overflow allocation, so a medium cell never wastes a hole |
//!
//! The bitmaps cover the whole page, metadata included; their first bits stay 0.

use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

/// Page size and alignment (H9).
pub const PAGE_SIZE: usize = 256 << 10;
/// `addr & PAGE_MASK` is the base of the page that holds `addr` (small pages, LOS heads).
pub const PAGE_MASK: usize = !(PAGE_SIZE - 1);
/// Cell alignment and bitmap granularity.
pub const GRANULE: usize = 8;
/// Immix line.
pub const LINE_SIZE: usize = 256;
pub const LINES_PER_PAGE: usize = PAGE_SIZE / LINE_SIZE;
/// Smallest cell: word0 plus one word (05-gc-design "minimum object 16 bytes").
pub const MIN_CELL_SIZE: usize = 16;
/// Largest cell in a small page. Larger cells go to the large-object space (LOS).
pub const MAX_SMALL_SIZE: usize = 8 << 10;
/// Cells above this size are medium: they use the overflow allocator when the hole is short.
pub const MAX_LINE_CELL: usize = LINE_SIZE;

pub(crate) const HEADER_BYTES: usize = 64;
pub(crate) const BITMAP_WORDS: usize = PAGE_SIZE / GRANULE / 64;
pub(crate) const MARK_BITS_OFFSET: usize = HEADER_BYTES;
pub(crate) const VO_BITS_OFFSET: usize = MARK_BITS_OFFSET + BITMAP_WORDS * 8;
pub(crate) const LINE_MARKS_OFFSET: usize = VO_BITS_OFFSET + BITMAP_WORDS * 8;
/// Offset of the first cell in a small page.
pub const FIRST_CELL_OFFSET: usize =
    (LINE_MARKS_OFFSET + LINES_PER_PAGE).next_multiple_of(LINE_SIZE);
pub(crate) const FIRST_LINE: usize = FIRST_CELL_OFFSET / LINE_SIZE;
/// Bytes for cells in one small page.
pub const PAGE_CELL_BYTES: usize = PAGE_SIZE - FIRST_CELL_OFFSET;
/// Offset of the cell in the first chunk of a large object.
pub const LARGE_CELL_OFFSET: usize = 64;

/// Freed memory is filled with this word when zapping is on. As a Value it is invalid (int32
/// tag with junk in bits 32-48) and it is not a cell, so a stale read shows up in the verifier
/// and in `Value::from_bits_unchecked`'s debug check.
pub const ZAP_WORD: u64 = 0xFFFF_DEAD_DEAD_BEEF;

pub(crate) const PAGE_MAGIC: u32 = 0xE9E9_0256;
pub(crate) const PAGE_KIND_SMALL: u8 = 1;
pub(crate) const PAGE_KIND_LARGE: u8 = 2;

const _: () = {
    assert!(PAGE_SIZE.is_power_of_two());
    assert!(FIRST_CELL_OFFSET == 9472 && FIRST_LINE == 37);
    assert!(LINE_MARKS_OFFSET + LINES_PER_PAGE <= FIRST_CELL_OFFSET);
    assert!(MAX_SMALL_SIZE <= PAGE_CELL_BYTES);
    assert!(size_of::<PageHeader>() <= HEADER_BYTES);
    assert!(size_of::<LargeHeader>() <= LARGE_CELL_OFFSET);
    assert!(MIN_CELL_SIZE.is_multiple_of(GRANULE));
};

/// Size that the allocator reserves for a request of `size` bytes: at least
/// [`MIN_CELL_SIZE`], rounded up to [`GRANULE`]. `None` on overflow.
pub const fn cell_size_for(size: usize) -> Option<usize> {
    let s = if size < MIN_CELL_SIZE {
        MIN_CELL_SIZE
    } else {
        size
    };
    s.checked_next_multiple_of(GRANULE)
}

/// Header at offset 0 of every small page.
#[repr(C)]
pub(crate) struct PageHeader {
    pub(crate) magic: u32,
    pub(crate) kind: u8,
    _pad: [u8; 3],
    /// Index of this chunk in the heap reservation.
    pub(crate) chunk: u32,
}

/// Header at offset 0 of the first chunk of a large object.
#[repr(C)]
pub(crate) struct LargeHeader {
    pub(crate) magic: u32,
    pub(crate) kind: u8,
    /// Mark bit of the one cell (side metadata for LOS).
    pub(crate) mark: AtomicU8,
    _pad: [u8; 2],
    pub(crate) chunks: u32,
    pub(crate) chunk: u32,
    /// Cell size in bytes (from `cell_size_for`).
    pub(crate) size: usize,
    /// Bytes committed from the chunk base.
    pub(crate) committed: usize,
}

/// A small page, by its base pointer. The pointer carries the provenance of the heap
/// reservation, so every address inside the page can be reached from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Page(NonNull<u8>);

impl Page {
    /// # Safety
    /// `base` is the 256 KiB aligned base of a committed small page of this heap.
    pub(crate) unsafe fn from_base(base: NonNull<u8>) -> Page {
        debug_assert!(base.addr().get().is_multiple_of(PAGE_SIZE));
        Page(base)
    }

    pub(crate) fn addr(self) -> usize {
        self.0.addr().get()
    }

    /// Writes a fresh header and clears all metadata.
    pub(crate) fn init(self, chunk: u32) {
        // SAFETY: the page is committed (constructor contract) and the metadata area
        // [0, FIRST_CELL_OFFSET) belongs to this page; no reference into it is live while the
        // allocator sets the page up.
        unsafe {
            self.0.as_ptr().write_bytes(0, FIRST_CELL_OFFSET);
            self.0.cast::<PageHeader>().write(PageHeader {
                magic: PAGE_MAGIC,
                kind: PAGE_KIND_SMALL,
                _pad: [0; 3],
                chunk,
            });
        }
    }

    pub(crate) fn header(&self) -> &PageHeader {
        // SAFETY: offset 0 of a committed page holds a PageHeader (`init`). It is read only.
        unsafe { self.0.cast::<PageHeader>().as_ref() }
    }

    fn bitmap_word(&self, offset: usize, i: usize) -> &AtomicU64 {
        debug_assert!(i < BITMAP_WORDS);
        // SAFETY: both bitmaps lie inside the committed metadata area, 8-byte aligned. They are
        // only accessed as atomics, so shared references are fine.
        unsafe { self.0.add(offset + i * 8).cast::<AtomicU64>().as_ref() }
    }

    pub(crate) fn mark_word(&self, i: usize) -> &AtomicU64 {
        self.bitmap_word(MARK_BITS_OFFSET, i)
    }

    pub(crate) fn vo_word(&self, i: usize) -> &AtomicU64 {
        self.bitmap_word(VO_BITS_OFFSET, i)
    }

    /// (word index, bit mask) of the granule at `addr`.
    pub(crate) fn bit(self, addr: usize) -> (usize, u64) {
        let g = (addr - self.addr()) / GRANULE;
        (g / 64, 1u64 << (g % 64))
    }

    pub(crate) fn vo(&self, addr: usize) -> bool {
        let (w, m) = self.bit(addr);
        self.vo_word(w).load(Relaxed) & m != 0
    }

    pub(crate) fn set_vo(&self, addr: usize) {
        let (w, m) = self.bit(addr);
        let word = self.vo_word(w);
        // Only the mutator thread writes VO bits (allocation and sweep), so load + store is
        // enough; no RMW needed.
        word.store(word.load(Relaxed) | m, Relaxed);
    }

    pub(crate) fn is_marked(&self, addr: usize) -> bool {
        let (w, m) = self.bit(addr);
        self.mark_word(w).load(Relaxed) & m != 0
    }

    /// Sets the mark bit; true if it was clear.
    pub(crate) fn mark(&self, addr: usize) -> bool {
        let (w, m) = self.bit(addr);
        let word = self.mark_word(w);
        let old = word.load(Relaxed);
        if old & m != 0 {
            return false;
        }
        word.store(old | m, Relaxed);
        true
    }

    /// The nearest cell start at or below `addr`, looking back at most `max_back` bytes and
    /// never below the first cell.
    pub(crate) fn prev_vo(&self, addr: usize, max_back: usize) -> Option<usize> {
        let first = FIRST_CELL_OFFSET / GRANULE;
        let g = (addr - self.addr()) / GRANULE;
        if g < first {
            return None;
        }
        let lo = g.saturating_sub(max_back / GRANULE).max(first);
        let mut w = g / 64;
        // Bits 0..=g%64 of the first word.
        let mut bits = self.vo_word(w).load(Relaxed) & (u64::MAX >> (63 - g % 64));
        loop {
            if bits != 0 {
                let s = w * 64 + 63 - bits.leading_zeros() as usize;
                return (s >= lo).then(|| self.addr() + s * GRANULE);
            }
            if w == 0 || w * 64 <= lo {
                return None;
            }
            w -= 1;
            bits = self.vo_word(w).load(Relaxed);
        }
    }

    /// Pointer to the line-mark bytes (one per line, 0 = free).
    pub(crate) fn line_marks(self) -> *mut u8 {
        // SAFETY: the line marks lie inside the committed metadata area.
        unsafe { self.0.as_ptr().add(LINE_MARKS_OFFSET) }
    }

    pub(crate) fn line_used(self, line: usize) -> bool {
        debug_assert!(line < LINES_PER_PAGE);
        // SAFETY: `line` < LINES_PER_PAGE, inside the line-mark array. Only the mutator thread
        // reads and writes line marks.
        unsafe { self.line_marks().add(line).read() != 0 }
    }
}

/// A large object, by the base pointer of its first chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Large(NonNull<u8>);

impl Large {
    /// # Safety
    /// `base` is the base of the first chunk of a committed large object of this heap whose
    /// header was written by [`Large::init`].
    pub(crate) unsafe fn from_base(base: NonNull<u8>) -> Large {
        debug_assert!(base.addr().get().is_multiple_of(PAGE_SIZE));
        Large(base)
    }

    /// Writes the header of a new large object.
    ///
    /// # Safety
    /// `[base, base + committed)` is committed memory of this heap that nothing else uses,
    /// and `committed >= LARGE_CELL_OFFSET + size`.
    pub(crate) unsafe fn init(
        base: NonNull<u8>,
        chunk: u32,
        chunks: u32,
        size: usize,
        committed: usize,
    ) -> Large {
        debug_assert!(committed >= LARGE_CELL_OFFSET + size);
        // SAFETY: caller contract: the header area is committed and unused.
        unsafe {
            base.cast::<LargeHeader>().write(LargeHeader {
                magic: PAGE_MAGIC,
                kind: PAGE_KIND_LARGE,
                mark: AtomicU8::new(0),
                _pad: [0; 2],
                chunks,
                chunk,
                size,
                committed,
            })
        };
        Large(base)
    }

    pub(crate) fn header(&self) -> &LargeHeader {
        // SAFETY: constructor contract: a LargeHeader sits at offset 0. Its only mutable
        // field is an atomic.
        unsafe { self.0.cast::<LargeHeader>().as_ref() }
    }

    pub(crate) fn cell_addr(self) -> usize {
        self.0.addr().get() + LARGE_CELL_OFFSET
    }

    pub(crate) fn cell_ptr(self) -> NonNull<u8> {
        // SAFETY: the cell starts inside the committed first chunk.
        unsafe { self.0.add(LARGE_CELL_OFFSET) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry() {
        assert_eq!(PAGE_SIZE, 262_144);
        assert_eq!(LINES_PER_PAGE, 1024);
        assert_eq!(BITMAP_WORDS, 512);
        assert_eq!(MARK_BITS_OFFSET, 64);
        assert_eq!(VO_BITS_OFFSET, 4160);
        assert_eq!(LINE_MARKS_OFFSET, 8256);
        assert_eq!(FIRST_CELL_OFFSET, 9472);
        assert_eq!(PAGE_CELL_BYTES, 252_672);
        // Metadata overhead of a small page: 3.6%.
        const { assert!(FIRST_CELL_OFFSET * 1000 / PAGE_SIZE < 37) };
    }

    #[test]
    fn cell_size_rounding() {
        assert_eq!(cell_size_for(0), Some(16));
        assert_eq!(cell_size_for(1), Some(16));
        assert_eq!(cell_size_for(16), Some(16));
        assert_eq!(cell_size_for(17), Some(24));
        assert_eq!(cell_size_for(8192), Some(8192));
        assert_eq!(cell_size_for(usize::MAX), None);
    }

    #[test]
    fn zap_word_is_not_a_value() {
        assert!(!ep9_value::Value::is_valid_bits(ZAP_WORD));
        assert!(!crate::barrier::is_cell_bits(ZAP_WORD));
    }

    /// One aligned, zeroed page from the global allocator (works under Miri too).
    struct TestPage(NonNull<u8>);

    impl TestPage {
        fn layout() -> std::alloc::Layout {
            std::alloc::Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap()
        }

        fn new() -> TestPage {
            // SAFETY: the layout has a non-zero size.
            let p = unsafe { std::alloc::alloc_zeroed(Self::layout()) };
            TestPage(NonNull::new(p).expect("alloc"))
        }

        fn page(&self) -> Page {
            // SAFETY: a zeroed, page-aligned block of PAGE_SIZE bytes that this value owns.
            let p = unsafe { Page::from_base(self.0) };
            p.init(7);
            p
        }
    }

    impl Drop for TestPage {
        fn drop(&mut self) {
            // SAFETY: allocated in `new` with the same layout.
            unsafe { std::alloc::dealloc(self.0.as_ptr(), Self::layout()) }
        }
    }

    #[test]
    fn bitmaps_and_prev_vo() {
        let mem = TestPage::new();
        let p = mem.page();
        assert_eq!(p.header().magic, PAGE_MAGIC);
        assert_eq!(p.header().chunk, 7);
        let a = p.addr() + FIRST_CELL_OFFSET;
        let b = a + 64 * 8 + 8; // next bitmap word
        assert_eq!(p.prev_vo(a, MAX_SMALL_SIZE), None);
        p.set_vo(a);
        p.set_vo(b);
        assert!(p.vo(a) && p.vo(b) && !p.vo(a + 8));
        assert_eq!(p.prev_vo(a, MAX_SMALL_SIZE), Some(a));
        assert_eq!(p.prev_vo(a + 40, MAX_SMALL_SIZE), Some(a));
        assert_eq!(p.prev_vo(b - 8, MAX_SMALL_SIZE), Some(a));
        assert_eq!(p.prev_vo(b, MAX_SMALL_SIZE), Some(b));
        assert_eq!(p.prev_vo(b + 4096, MAX_SMALL_SIZE), Some(b));
        // Look-back limit.
        assert_eq!(p.prev_vo(b + 4096, 4000), None);
        assert_eq!(
            p.prev_vo(p.addr() + 64, MAX_SMALL_SIZE),
            None,
            "metadata area"
        );
        // Marks.
        assert!(!p.is_marked(a));
        assert!(p.mark(a));
        assert!(!p.mark(a), "second mark reports already set");
        assert!(p.is_marked(a) && !p.is_marked(b));
        // Line marks start free.
        assert!((0..LINES_PER_PAGE).all(|l| !p.line_used(l)));
    }
}
