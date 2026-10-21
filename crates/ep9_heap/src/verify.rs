//! Heap verifier (03-gc §5 "Tooling", H9 "heap always parsable").
//!
//! | Check | Where |
//! |---|---|
//! | Page header: magic, kind, chunk index | every small page |
//! | No mark bit set outside a GC | small pages, LOS headers |
//! | Cells walk in address order from the VO bitmap; none overlaps the next; none crosses the page end | small pages |
//! | Every cell kind is registered; size is at least 16 B, at most 8 KiB in a page, above 8 KiB in LOS | all cells |
//! | gc_state is BLACK, WHITE or GREY | all cells |
//! | Shape-space kinds only in the shape region, and only those there | all cells |
//! | With a shape space: a non-zero word0 ShapeId names a live shape-space cell (H08-Q1) | all cells |
//! | Every visited slot: not 0, valid `Value` bits; a cell reference is the start of a live cell | all cells and every root source |
//! | LOS: header, chunk run and committed size match the chunk table | every large object |
//!
//! The verifier first finishes the lazy sweep, so dead cells are gone and VO bits are exact.

use core::sync::atomic::Ordering::Relaxed;

use crate::barrier::gc_state;
use crate::cell::{CellPtr, Space};
use crate::heap::Heap;
use crate::layout::{
    BITMAP_WORDS, FIRST_CELL_OFFSET, LARGE_CELL_OFFSET, Large, MAX_SMALL_SIZE, PAGE_KIND_LARGE,
    PAGE_KIND_SMALL, PAGE_MAGIC, PAGE_SIZE,
};
use crate::memory::Chunk;
use crate::trace::{Context, Mode, Tracer};

/// What a clean verifier run saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub small_cells: usize,
    pub small_bytes: usize,
    pub large_cells: usize,
    pub large_bytes: usize,
    pub small_pages: usize,
}

/// The first errors of a failed verifier run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} heap error(s)", self.errors.len())?;
        for e in &self.errors {
            write!(f, "\n  {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for VerifyError {}

impl Heap {
    /// Checks the whole heap and every root. Not a GC point; it changes nothing but the lazy
    /// sweep state.
    pub fn verify(&self) -> Result<VerifyReport, VerifyError> {
        let mut inner_ref = self.inner.borrow_mut();
        inner_ref.finish_sweeping();
        let inner = &*inner_ref;
        let mut roots = self.roots.borrow_mut();
        let handles = self.handles.borrow();
        let scratch = self.scratch.borrow();
        let mem = &inner.mem;
        let kinds = &inner.kinds;
        let mut t = Tracer::new(mem, kinds, Mode::Verify, Vec::new(), Vec::new());
        let mut r = VerifyReport::default();

        for idx in 0..mem.high() {
            match mem.chunk(idx) {
                Chunk::Small => {
                    r.small_pages += 1;
                    let page = mem.page(idx);
                    t.context = Context::Root("page header");
                    let h = page.header();
                    if h.magic != PAGE_MAGIC || h.kind != PAGE_KIND_SMALL || h.chunk as usize != idx
                    {
                        t.error(format_args!("small page {idx}: bad header"));
                        continue;
                    }
                    let mut prev_end = page.addr() + FIRST_CELL_OFFSET;
                    for w in 0..BITMAP_WORDS {
                        if page.mark_word(w).load(Relaxed) != 0 {
                            t.error(format_args!("small page {idx}: mark bit set outside a GC"));
                        }
                        let mut vo = page.vo_word(w).load(Relaxed);
                        while vo != 0 {
                            let bit = vo.trailing_zeros() as usize;
                            vo &= vo - 1;
                            let addr = page.addr() + (w * 64 + bit) * 8;
                            if addr < prev_end {
                                t.error(format_args!(
                                    "small page {idx}: cell {addr:#x} overlaps the previous cell \
                                     or the metadata (ends {prev_end:#x})"
                                ));
                            }
                            // SAFETY: a VO bit marks a cell start whose word0 the allocator
                            // wrote; the page is committed.
                            let c = unsafe { CellPtr::from_raw(mem.ptr(addr).cast()) };
                            match check_cell(&mut t, c, false, mem.is_shape_chunk(idx)) {
                                Some(size) => {
                                    if addr + size > page.addr() + PAGE_SIZE {
                                        t.error(format_args!(
                                            "cell {addr:#x} of {size} B crosses the page end"
                                        ));
                                    }
                                    prev_end = addr + size;
                                    r.small_cells += 1;
                                    r.small_bytes += size;
                                }
                                None => prev_end = addr + 8,
                            }
                        }
                    }
                }
                Chunk::LargeHead { chunks } => {
                    // SAFETY: chunk `idx` is the committed head of a large object.
                    let large = unsafe { Large::from_base(mem.chunk_base(idx)) };
                    let h = large.header();
                    t.context = Context::Root("large header");
                    let total = LARGE_CELL_OFFSET + h.size;
                    if h.magic != PAGE_MAGIC
                        || h.kind != PAGE_KIND_LARGE
                        || h.chunk as usize != idx
                        || h.chunks != chunks
                        || total.div_ceil(PAGE_SIZE) != chunks as usize
                        || h.committed < total
                    {
                        t.error(format_args!("large object at chunk {idx}: bad header"));
                        continue;
                    }
                    if h.mark.load(Relaxed) != 0 {
                        t.error(format_args!(
                            "large object at chunk {idx}: mark set outside a GC"
                        ));
                    }
                    for i in idx + 1..idx + chunks as usize {
                        if mem.chunk(i) != (Chunk::LargeTail { head: idx as u32 }) {
                            t.error(format_args!(
                                "chunk {i} is not a tail of large object {idx}"
                            ));
                        }
                    }
                    // SAFETY: the LOS cell starts right after the header.
                    let c = unsafe { CellPtr::from_raw(large.cell_ptr().cast()) };
                    if let Some(size) = check_cell(&mut t, c, true, false) {
                        if size != h.size {
                            t.error(format_args!(
                                "large cell {c:?}: kind size {size} != header size {}",
                                h.size
                            ));
                        }
                        r.large_cells += 1;
                        r.large_bytes += size;
                    }
                }
                Chunk::LargeTail { head } => {
                    if !matches!(mem.chunk(head as usize), Chunk::LargeHead { .. }) {
                        t.context = Context::Root("chunk table");
                        t.error(format_args!("tail chunk {idx} points to non-head {head}"));
                    }
                }
                Chunk::Free { .. } => {}
            }
        }

        handles.visit_strong(&mut t);
        handles.verify_weak(&mut t);
        t.context = Context::Root("scratch roots");
        for &b in scratch.iter() {
            t.visit_bits(b);
        }
        roots.visit_strong(&mut t, true);

        if t.errors.is_empty() {
            Ok(r)
        } else {
            Err(VerifyError {
                errors: core::mem::take(&mut t.errors),
            })
        }
    }
}

/// Checks one cell and traces its slots in verify mode. Returns its size, or `None` when the
/// kind is unknown (the size cannot be known then). `in_region`: the cell lies in the shape
/// region.
fn check_cell(t: &mut Tracer<'_>, c: CellPtr, large: bool, in_region: bool) -> Option<usize> {
    // SAFETY: callers pass a cell start (VO bit or LOS header) in committed memory.
    let h = unsafe { c.header() };
    let kind = h.kind();
    t.context = Context::Cell(c, kind);
    let Some(k) = t.kinds().get(kind).copied() else {
        t.error(format_args!("unregistered kind {kind}"));
        return None;
    };
    let Some(size) = t.kinds().cell_size(c) else {
        t.error(format_args!("size overflows"));
        return None;
    };
    if large != (size > MAX_SMALL_SIZE) {
        t.error(format_args!(
            "{} cell has size {size}",
            if large { "large" } else { "small" }
        ));
    }
    let st = h.gc_state();
    if !matches!(st, gc_state::BLACK | gc_state::WHITE | gc_state::GREY) {
        t.error(format_args!("gc_state {st} is not BLACK, WHITE or GREY"));
    }
    let space = t.kinds().space(kind);
    if (space == Space::Shape) != in_region {
        t.error(format_args!(
            "a {space:?}-space cell {} the shape region",
            if in_region { "inside" } else { "outside" }
        ));
    }
    t.verify_word0_shape(h.shape_id().get());
    (k.trace)(c, t);
    Some(size)
}
