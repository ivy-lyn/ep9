//! Heap verifier (03-gc §5 "Tooling", H9 "heap always parsable").
//!
//! | Check | Where |
//! |---|---|
//! | Page header: magic, kind, chunk index | every small page |
//! | No mark bit set outside a GC (generational, sticky marks: no mark bit outside a cell start, and a marked (old) cell is not WHITE, so a store into it takes the barrier's slow path) | small pages, LOS headers |
//! | Generational (S1 step 1): each remembered-list entry is a distinct GREY cell start of its page, and the page has no other GREY cell; a dirty card means the dirty byte is set; after a GC every list is empty and every card clean | small pages, LOS headers |
//! | Cells walk in address order from the VO bitmap; none overlaps the next; none crosses the page end | small pages |
//! | Every cell kind is registered; size is at least 16 B, at most 8 KiB in a page, above 8 KiB in LOS | all cells |
//! | gc_state is BLACK, WHITE or GREY | all cells |
//! | Shape-space kinds only in the shape region, and only those there | all cells |
//! | With a shape space: a non-zero word0 ShapeId names a live shape-space cell (H08-Q1) | all cells |
//! | Every visited slot: not 0, valid `Value` bits; a cell reference is the start of a live cell | all cells and every root source |
//! | LOS: header, chunk run and committed size match the chunk table | every large object |
//!
//! The verifier first finishes the lazy sweep, so dead cells are gone and VO bits are exact.
//!
//! Two entries, one set of checks:
//!
//! | Entry | How |
//! |---|---|
//! | [`Heap::verify`] (any time) | finish the sweep, walk every page by VO bits, check and trace every cell, then every root |
//! | after a GC with `verify_after_gc` | the GC's checked mark already checked every slot of every live cell, every strong root and every weak range (trace.rs); the sweep checks every live cell, and every dead cell's extent before it zaps it (heap.rs); this file checks that the sweeps kept exactly the cells the mark pushed (a mark bit set before the GC would hide a cell from the mark), and adds the page and LOS headers and the weak handles. No second trace, no second root walk (V9-D22 step 1) |
//!
//! The live cells after a GC are exactly the cells its mark traced, so the two entries check
//! the same slots. A planted dangling slot fails both (tests/verify.rs).

use core::sync::atomic::Ordering::Relaxed;

use crate::barrier::gc_state;
use crate::cell::{CellPtr, KindTable, Space};
use crate::heap::Heap;
use crate::layout::{
    BITMAP_WORDS, FIRST_CELL_OFFSET, LARGE_CELL_OFFSET, Large, MAX_SMALL_SIZE, PAGE_KIND_LARGE,
    PAGE_KIND_SMALL, PAGE_MAGIC, PAGE_SIZE,
};
use crate::memory::Chunk;
use crate::trace::{Context, Mode, Tracer, record_error};

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
        let sticky = inner.s.generational;

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
                    let mut grey = 0usize;
                    for w in 0..BITMAP_WORDS {
                        let mut vo = page.vo_word(w).load(Relaxed);
                        let outside = if sticky { !vo } else { u64::MAX };
                        let marks = page.mark_word(w).load(Relaxed);
                        if marks & outside != 0 {
                            t.error(format_args!("small page {idx}: mark bit set outside a GC"));
                        }
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
                            // SAFETY: as above.
                            let st = unsafe { c.header() }.gc_state();
                            grey += usize::from(st == gc_state::GREY);
                            if sticky && marks & (1 << bit) != 0 && st == gc_state::WHITE {
                                t.error(format_args!(
                                    "small page {idx}: cell {addr:#x} is marked (old) but WHITE"
                                ));
                            }
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
                    check_remembered(&mut t, idx, grey);
                }
                Chunk::LargeHead { chunks } => {
                    let Some(c) = check_large_header(&mut t, idx, chunks, sticky) else {
                        continue;
                    };
                    check_cards(&mut t, idx);
                    if let Some(size) = check_cell(&mut t, c, true, false) {
                        check_large_size(&mut t, idx, c, size);
                        r.large_cells += 1;
                        r.large_bytes += size;
                    }
                }
                Chunk::LargeTail { head } => check_large_tail(&mut t, idx, head),
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

    /// The verify right after a GC whose mark was checked (`errors`: what the checked mark
    /// found; `pushed`: the cells it marked; `large_kept`: the large objects its LOS sweep
    /// kept). Finishes the sweep with the cell checks, checks that the sweeps kept exactly the
    /// cells the mark pushed, then checks the page and LOS headers and the weak handles. Not
    /// a GC point.
    pub(crate) fn verify_after_collect(
        &self,
        errors: Vec<String>,
        pushed: usize,
        large_kept: usize,
    ) -> Result<(), VerifyError> {
        let mut inner_ref = self.inner.borrow_mut();
        let mut errors = errors;
        let small_kept = inner_ref.finish_sweeping_checked(&mut errors);
        // A cell whose mark bit was set before the GC is kept but was never pushed, so the
        // mark neither traced nor checked it (its children may be freed). Each pushed cell
        // stays marked until its sweep, so the counts differ exactly when such a cell exists.
        let kept = small_kept + large_kept;
        if kept != pushed {
            record_error(
                &mut errors,
                Context::Root("mark"),
                format_args!(
                    "the sweeps kept {kept} cells but the mark pushed {pushed}: {} cell(s) \
                     marked before the GC were not traced or checked",
                    kept.abs_diff(pushed)
                ),
            );
        }
        let inner = &*inner_ref;
        let handles = self.handles.borrow();
        let mem = &inner.mem;
        let mut t = Tracer::new(mem, &inner.kinds, Mode::Verify, Vec::new(), Vec::new());
        t.errors = errors;
        let sticky = inner.s.generational;

        for idx in 0..mem.high() {
            match mem.chunk(idx) {
                Chunk::Small => {
                    let page = mem.page(idx);
                    let h = page.header();
                    if h.magic != PAGE_MAGIC || h.kind != PAGE_KIND_SMALL || h.chunk as usize != idx
                    {
                        t.context = Context::Root("page header");
                        t.error(format_args!("small page {idx}: bad header"));
                    }
                    // The GC emptied every list after its mark (generational).
                    if h.remembered.len() != 0 {
                        t.context = Context::Root("remembered list");
                        t.error(format_args!(
                            "small page {idx}: remembered list not empty after a GC"
                        ));
                    }
                }
                Chunk::LargeHead { chunks } => {
                    // The cell's slots were checked when the mark traced it.
                    if let Some(c) = check_large_header(&mut t, idx, chunks, sticky) {
                        // SAFETY: chunk `idx` is the committed head of a large object.
                        let h = unsafe { Large::from_base(mem.chunk_base(idx)) };
                        if h.header().dirty.load(Relaxed) != 0 {
                            t.error(format_args!(
                                "large object at chunk {idx}: dirty after a GC"
                            ));
                        }
                        let kinds = t.kinds();
                        // `sweep_large` freed every unmarked object, so this one is live.
                        if let Some(size) = cell_checks(kinds, c, true, false, &mut t.errors) {
                            check_large_size(&mut t, idx, c, size);
                        }
                    }
                }
                Chunk::LargeTail { head } => check_large_tail(&mut t, idx, head),
                Chunk::Free { .. } => {}
            }
        }
        handles.verify_weak(&mut t);

        if t.errors.is_empty() {
            Ok(())
        } else {
            Err(VerifyError {
                errors: core::mem::take(&mut t.errors),
            })
        }
    }
}

/// The LOS header of the large object at chunk `idx` (`chunks` chunks): magic, kind, chunk
/// index and count, committed size, no mark outside a GC (unless `sticky`: generational
/// marks stay on old objects), every tail chunk names the head. Returns the cell, or `None`
/// when the header is bad.
fn check_large_header(
    t: &mut Tracer<'_>,
    idx: usize,
    chunks: u32,
    sticky: bool,
) -> Option<CellPtr> {
    let mem = t.mem();
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
        return None;
    }
    let marked = h.mark.load(Relaxed) != 0;
    if !sticky && marked {
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
    // SAFETY: the header checks passed, so the cell's word0 is the allocator's.
    if sticky && marked && unsafe { c.header() }.gc_state() == gc_state::WHITE {
        t.error(format_args!(
            "large object at chunk {idx}: marked (old) but WHITE"
        ));
    }
    Some(c)
}

/// The remembered list of small page `idx` (S1 step 1): each entry is a GREY cell start of
/// the page, no entry repeats, and the page holds no other GREY cell (`grey`: the GREY cells
/// the walk counted). So every GREY host is listed exactly once. Empty in S0.
fn check_remembered(t: &mut Tracer<'_>, idx: usize, grey: usize) {
    let mem = t.mem();
    let page = mem.page(idx);
    // SAFETY: the verifier pushes nothing while it reads the list.
    let mut offsets = unsafe { page.header().remembered.offsets() }.to_vec();
    t.context = Context::Root("remembered list");
    for &off in &offsets {
        let addr = page.addr() + off as usize;
        let ok = crate::trace::cell_start(mem, addr)
            // SAFETY: `cell_start` found a live cell start.
            .is_some_and(|c| unsafe { c.header() }.gc_state() == gc_state::GREY);
        if !ok {
            t.error(format_args!(
                "small page {idx}: remembered entry {addr:#x} is not a GREY cell start"
            ));
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    if offsets.len() != page.header().remembered.len() {
        t.error(format_args!("small page {idx}: a remembered entry repeats"));
    }
    if offsets.len() != grey {
        t.error(format_args!(
            "small page {idx}: {grey} GREY cell(s) but {} remembered",
            offsets.len()
        ));
    }
}

/// The cards of the large object at chunk `idx` (S1 step 1): a dirty card means the dirty
/// byte is set.
fn check_cards(t: &mut Tracer<'_>, idx: usize) {
    // SAFETY: chunk `idx` is the committed head of a large object.
    let large = unsafe { Large::from_base(t.mem().chunk_base(idx)) };
    let h = large.header();
    let any = crate::remembered::cards(h).is_some_and(|c| c.iter().any(|b| b.load(Relaxed) != 0));
    if any && h.dirty.load(Relaxed) == 0 {
        t.context = Context::Root("large header");
        t.error(format_args!(
            "large object at chunk {idx}: a dirty card but no dirty byte"
        ));
    }
}

/// The kind's size of the LOS cell `c` equals its header size.
fn check_large_size(t: &mut Tracer<'_>, idx: usize, c: CellPtr, size: usize) {
    // SAFETY: chunk `idx` is the committed head of a large object.
    let large = unsafe { Large::from_base(t.mem().chunk_base(idx)) };
    let h = large.header();
    if size != h.size {
        t.error(format_args!(
            "large cell {c:?}: kind size {size} != header size {}",
            h.size
        ));
    }
}

/// A tail chunk names a head chunk.
fn check_large_tail(t: &mut Tracer<'_>, idx: usize, head: u32) {
    if !matches!(t.mem().chunk(head as usize), Chunk::LargeHead { .. }) {
        t.context = Context::Root("chunk table");
        t.error(format_args!("tail chunk {idx} points to non-head {head}"));
    }
}

/// Checks one cell and traces its slots in verify mode. Returns its size, or `None` when the
/// kind is unknown (the size cannot be known then). `in_region`: the cell lies in the shape
/// region.
fn check_cell(t: &mut Tracer<'_>, c: CellPtr, large: bool, in_region: bool) -> Option<usize> {
    // SAFETY: callers pass a cell start (VO bit or LOS header) in committed memory.
    let h = unsafe { c.header() };
    t.context = Context::Cell(c, h.kind());
    let kinds = t.kinds();
    let size = cell_checks(kinds, c, large, in_region, &mut t.errors)?;
    t.verify_word0_shape(h.shape_id().get());
    let k = kinds
        .get(h.kind())
        .copied()
        .expect("cell_checks found the kind");
    (k.trace)(c, t);
    Some(size)
}

/// The checks of one cell that need no trace: a registered kind, a size that fits the space
/// (small page or LOS), a valid gc_state, the shape region holds exactly the shape-space
/// kinds. Returns the size, or `None` when it cannot be known. Errors go to `errors`.
pub(crate) fn cell_checks(
    kinds: &KindTable,
    c: CellPtr,
    large: bool,
    in_region: bool,
    errors: &mut Vec<String>,
) -> Option<usize> {
    // SAFETY: callers pass a cell start (VO bit or LOS header) in committed memory.
    let h = unsafe { c.header() };
    let kind = h.kind();
    let ctx = Context::Cell(c, kind);
    if kinds.get(kind).is_none() {
        record_error(errors, ctx, format_args!("unregistered kind {kind}"));
        return None;
    }
    let Some(size) = kinds.cell_size(c) else {
        record_error(errors, ctx, format_args!("size overflows"));
        return None;
    };
    if large != (size > MAX_SMALL_SIZE) {
        record_error(
            errors,
            ctx,
            format_args!(
                "{} cell has size {size}",
                if large { "large" } else { "small" }
            ),
        );
    }
    let st = h.gc_state();
    if !matches!(st, gc_state::BLACK | gc_state::WHITE | gc_state::GREY) {
        record_error(
            errors,
            ctx,
            format_args!("gc_state {st} is not BLACK, WHITE or GREY"),
        );
    }
    let space = kinds.space(kind);
    if (space == Space::Shape) != in_region {
        record_error(
            errors,
            ctx,
            format_args!(
                "a {space:?}-space cell {} the shape region",
                if in_region { "inside" } else { "outside" }
            ),
        );
    }
    Some(size)
}

// ---- the E2 check (GFF-E2CHECK) ------------------------------------------------------------
//
// E2 (H11): an initializing store into a cell allocated since the last GC point needs no
// barrier. The claim is wrong when a GC ran between the allocation and the store: the host
// then survived a GC (gc_state BLACK, not WHITE), and under S1 the store of a young cell into
// it would lose an old-to-young edge. Under GC-stress (a GC at every allocation) a wrong claim
// shows up at once.
//
//   ep9_vm object::init(c, off, v)  --e2_check_on()?--> e2_check_init(c, v, caller)
//     host WHITE, or v not a cell (the barrier ignores non-cell values): fine
//     else  Report: count it per caller; print the caller at counts 1, 10, 100, ...
//           Assert: panic with the caller

/// What [`e2_check_init`] does with a wrong E2 claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum E2Check {
    /// No check (no GC-stress heap was made in this process).
    Off = 0,
    /// Count and print each wrong claim (stderr, `ep9: E2 check: ...`, and the file that
    /// `EP9_E2_REPORT` names, if set) at counts 1, 10, 100, ...; do not panic.
    Report = 1,
    /// Panic at the first wrong claim.
    Assert = 2,
}

/// The mode that a GC-stress heap turns on ([`HeapConfig::gc_stress`] > 0 at
/// [`Heap::new`], or [`Heap::set_gc_stress`] with n > 0): [`E2Check::Assert`], so a wrong
/// E2 claim panics with its caller. GC1-VERIFY built it in report mode, which listed 0 wrong
/// claims; the M4-P1 fixer switched it to assert mode after the merge (plan §3.9, V9-A20).
///
/// [`HeapConfig::gc_stress`]: crate::HeapConfig::gc_stress
pub const E2_CHECK_UNDER_GC_STRESS: E2Check = E2Check::Assert;

/// Process-wide: heaps live one per thread, and the store sites have no heap at hand. Once a
/// GC-stress heap exists, every heap of the process checks (a wrong claim is wrong with or
/// without GC-stress; GC-stress only makes it happen).
static E2_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Wrong claims seen so far in report mode: (caller, count).
static E2_SEEN: std::sync::Mutex<Vec<(&'static core::panic::Location<'static>, u64)>> =
    std::sync::Mutex::new(Vec::new());

/// True if E2 stores must call [`e2_check_init`]. One relaxed load: the store sites test it
/// inline.
#[inline(always)]
pub fn e2_check_on() -> bool {
    E2_MODE.load(Relaxed) != 0
}

/// The current mode.
pub fn e2_check_mode() -> E2Check {
    match E2_MODE.load(Relaxed) {
        0 => E2Check::Off,
        1 => E2Check::Report,
        _ => E2Check::Assert,
    }
}

/// Sets the mode for the whole process (tests; embedders that want the check without
/// GC-stress).
pub fn set_e2_check(mode: E2Check) {
    E2_MODE.store(mode as u8, Relaxed);
}

/// Turns the check on in [`E2_CHECK_UNDER_GC_STRESS`] mode unless a mode is already set.
pub(crate) fn e2_check_for_gc_stress() {
    let _ = E2_MODE.compare_exchange(
        E2Check::Off as u8,
        E2_CHECK_UNDER_GC_STRESS as u8,
        Relaxed,
        Relaxed,
    );
}

/// Checks one E2 store of `value` into `host` at `at` (the store's caller). See the section
/// comment. Not a GC point. The test is inline; a wrong claim goes to a cold function.
#[inline(always)]
pub fn e2_check_init(host: CellPtr, value: u64, at: &'static core::panic::Location<'static>) {
    // SAFETY: the store site holds `host`, a live cell of this thread's heap.
    let h = unsafe { host.header() };
    if crate::barrier::is_cell_bits(value) && value != 0 && h.gc_state() != gc_state::WHITE {
        e2_wrong_claim(h.kind(), at);
    }
}

#[cold]
#[inline(never)]
fn e2_wrong_claim(kind: u8, at: &'static core::panic::Location<'static>) {
    match e2_check_mode() {
        E2Check::Off => {}
        E2Check::Assert => panic!(
            "E2 check: barrier-free init store into a cell that survived a GC \
             (kind {kind}) at {at}"
        ),
        E2Check::Report => {
            let mut seen = E2_SEEN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let n = match seen.iter_mut().find(|(l, _)| core::ptr::eq(*l, at)) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    seen.push((at, 1));
                    1
                }
            };
            drop(seen);
            if is_power_of_ten(n) {
                let line = format!(
                    "ep9: E2 check: barrier-free init store into a cell that survived a GC \
                     (kind {kind}) at {at} (seen {n} times)"
                );
                eprintln!("{line}");
                e2_report_to_file(&line);
            }
        }
    }
}

/// `EP9_E2_REPORT=<file>`: report lines are also appended to that file, one `write` per
/// line. For runners whose workers' stderr is not kept (tools/test262 pipes it and keeps only
/// a crash tail): every worker process appends to the same file.
fn e2_report_to_file(line: &str) {
    use std::io::Write;
    static PATH: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    let Some(path) = PATH.get_or_init(|| std::env::var_os("EP9_E2_REPORT").map(Into::into)) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

fn is_power_of_ten(mut n: u64) -> bool {
    while n >= 10 && n.is_multiple_of(10) {
        n /= 10;
    }
    n == 1
}

/// Report mode: every caller with a wrong claim so far and its count, in first-seen order.
pub fn e2_reports() -> Vec<(String, u64)> {
    E2_SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(l, n)| (l.to_string(), *n))
        .collect()
}
