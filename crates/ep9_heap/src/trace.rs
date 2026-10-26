//! Marking: the [`Tracer`] that kinds and root sources call, the interior-pointer lookup for
//! the conservative scan, and the [`WeakVisitor`] that clears weak slots at GC end.
//!
//! ```text
//! roots ----------------------> Tracer::visit* --> mark bit (page bitmap / LOS header)
//!   CSS word w: find_cell(w), find_cell(w - 1)        |  newly marked: gc_state = BLACK, push
//!   precise: handles, scratch, ranges, sources        v
//! drain: pop cell -> KindTable[word0.kind].trace(cell, tracer) -> Tracer::visit* -> ...
//!        with a shape space: word0.shape_id != 0 -> mark cage base + shape_id (H08-Q1)
//! weak:  Tracer::visit_weak records the slot; after the fixpoint, a slot whose cell is
//!        unmarked is cleared to `undefined`
//! ephemeron (key, value), e.g. a WeakMap entry: the key slot is weak; the value is traced
//!        only once the key is marked
//!   visit_ephemeron: key undefined (cleared) -> visit_weak(value)
//!                    key not a cell, or already marked -> visit(value) now
//!                    else -> pending list
//!   drain_to_fixpoint: drain; then rounds of {pending entry whose key is now marked ->
//!        visit(value); drain} until a round marks nothing; after EPHEMERON_ROUNDS rounds the
//!        rest goes to a map key -> values, and each cell the drain pops looks itself up
//!        (linear: each entry is resolved once, each popped cell costs one lookup)
//!   after the fixpoint the keys left are dead: each value slot becomes a weak slot, so it is
//!        cleared with its key unless its cell is live by another path
//! ```
//!
//! The shape that word0 names is a strong edge of every cell, so no trace function visits
//! it. Without a shape space, word0's shape_id is not a heap reference and is ignored.
//!
//! The same `Tracer` runs in verify mode for the heap verifier: it checks each edge instead of
//! marking. A checked mark (`verify_after_gc`, M4-P1) does both: it marks, and it checks each
//! edge, weak slot and word0 shape the way the verifier does, so the verify after that GC does
//! not trace the heap again (V9-D22 step 1).
//!
//! ```text
//! edge(addr), mark mode:  chunk_index -> Small: offset >= first cell, VO bit -> mark bit
//!                                     -> LargeHead: addr == cell start -> header mark
//!                         (one chunk lookup for the cell-start test and the mark)
//!                         not a cell start: checked mark -> error; else debug assert
//! ```
//!
//! A mark bit that is already set when the checked mark reaches a cell means "traced in this
//! GC". A bit left from before the GC would make the mark skip that cell (no trace, no
//! check), so the verify after the GC compares the cells the sweep keeps with the cells this
//! tracer pushed ([`Tracer::pushed_cells`], verify.rs).

use core::hash::{BuildHasherDefault, Hasher};
use core::ptr::NonNull;
use core::sync::atomic::Ordering::Relaxed;
use std::collections::HashMap;

use ep9_value::Value;

use crate::barrier::{Slot, gc_state, is_cell_bits};
use crate::cell::{CellPtr, KindTable, Space};
use crate::layout::{FIRST_CELL_OFFSET, Large, MAX_SMALL_SIZE, PAGE_MASK};
use crate::memory::{Chunk, Memory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Mark,
    Verify,
}

/// Where the verifier is, for its messages.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Context {
    Root(&'static str),
    Cell(CellPtr, u8),
}

/// Visits GC references. Kinds get one in their trace function; root sources get one in their
/// callback.
///
/// | Method | For |
/// |---|---|
/// | [`Tracer::visit`], [`Tracer::visit_slots`] | `Value` slots of a cell or a root |
/// | [`Tracer::visit_value`], [`Tracer::visit_bits`] | a `Value` copy; a raw VM-stack word |
/// | [`Tracer::visit_cell`] | a raw cell pointer field (not a `Value`) |
/// | [`Tracer::visit_weak`] | a weak `Value` slot inside a cell: cleared at GC end if its cell dies |
/// | [`Tracer::visit_ephemeron`] | a (key, value) pair inside a cell: the key is weak, the value is strong only while the key is live |
///
/// A trace function or root source must not call into the [`crate::Heap`].
pub struct Tracer<'h> {
    mem: &'h Memory,
    kinds: &'h KindTable,
    mode: Mode,
    /// Check every visited slot as the verifier does: always in verify mode, and in mark mode
    /// for a checked mark ([`Tracer::new_checked`]).
    check: bool,
    /// Bytes of the shape region (0: no shape space).
    shape_bytes: usize,
    stack: Vec<CellPtr>,
    weak: Vec<NonNull<Slot>>,
    /// Ephemerons whose key was not marked when they were visited: (key slot, value slot).
    ephemerons: Vec<(NonNull<Slot>, NonNull<Slot>)>,
    /// The linear form of the fixpoint (after [`EPHEMERON_ROUNDS`] rounds): key address ->
    /// the value slots that wait for it. `None` before.
    waiting: Option<EphemeronMap>,
    /// Rounds the last [`Tracer::drain_to_fixpoint`] ran, and whether it went linear.
    pub(crate) ephemeron_rounds: u32,
    pub(crate) ephemeron_linear: bool,
    pub(crate) marked_cells: usize,
    pub(crate) marked_bytes: usize,
    /// Cells whose mark bit this tracer set (each is pushed once). After a checked mark, the
    /// checked sweep keeps exactly these cells, unless a mark bit was set before the GC
    /// (verify.rs).
    pub(crate) pushed_cells: usize,
    pub(crate) errors: Vec<String>,
    pub(crate) context: Context,
}

/// Errors kept by one verifier run; more are counted, not stored.
pub(crate) const MAX_ERRORS: usize = 32;

/// Rounds of the ephemeron fixpoint before it switches to the linear form. Each round scans
/// every pending entry, so a chain of n entries met in the wrong order would cost n rounds of
/// n (V8 bounds its rounds the same way and then goes linear). Real chains are short: most
/// GCs end in one or two rounds.
pub(crate) const EPHEMERON_ROUNDS: u32 = 8;

/// Key address -> the value slots that wait for that key (the linear form of the fixpoint).
type EphemeronMap = HashMap<usize, Vec<NonNull<Slot>>, BuildHasherDefault<AddrHasher>>;

/// A fixed hash of a cell address for [`EphemeronMap`]: no random state, so a GC does the
/// same work in every run (instruction counts, H12-Q2), and the map never leaves the GC.
#[derive(Default)]
struct AddrHasher(u64);

impl Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_usize(&mut self, addr: usize) {
        // Cells are 8-byte aligned: drop the 3 zero bits, then a Fibonacci multiply spreads
        // the rest over the high bits too.
        self.0 = ((addr as u64) >> 3).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

impl<'h> Tracer<'h> {
    pub(crate) fn new(
        mem: &'h Memory,
        kinds: &'h KindTable,
        mode: Mode,
        stack: Vec<CellPtr>,
        weak: Vec<NonNull<Slot>>,
    ) -> Tracer<'h> {
        debug_assert!(stack.is_empty() && weak.is_empty());
        Tracer {
            mem,
            kinds,
            mode,
            check: mode == Mode::Verify,
            shape_bytes: mem.shape_bytes(),
            stack,
            weak,
            ephemerons: Vec::new(),
            waiting: None,
            ephemeron_rounds: 0,
            ephemeron_linear: false,
            marked_cells: 0,
            marked_bytes: 0,
            pushed_cells: 0,
            errors: Vec::new(),
            context: Context::Root("none"),
        }
    }

    /// A mark-mode tracer that also checks each edge, weak slot and word0 shape (errors in
    /// [`Tracer::errors`]), so that no verify pass has to trace the heap again.
    pub(crate) fn new_checked(
        mem: &'h Memory,
        kinds: &'h KindTable,
        stack: Vec<CellPtr>,
        weak: Vec<NonNull<Slot>>,
    ) -> Tracer<'h> {
        let mut t = Tracer::new(mem, kinds, Mode::Mark, stack, weak);
        t.check = true;
        t
    }

    /// Returns the work lists (empty) so the heap can keep their capacity.
    pub(crate) fn into_lists(mut self) -> (Vec<CellPtr>, Vec<NonNull<Slot>>) {
        self.stack.clear();
        self.weak.clear();
        debug_assert!(self.ephemerons.is_empty() && self.waiting.is_none());
        (self.stack, self.weak)
    }

    pub(crate) fn mem(&self) -> &'h Memory {
        self.mem
    }

    pub(crate) fn kinds(&self) -> &'h KindTable {
        self.kinds
    }

    /// Cells marked but not traced yet. Right after the conservative scan: the CSS roots.
    pub(crate) fn pending(&self) -> usize {
        self.stack.len()
    }

    /// A strong `Value` slot.
    #[inline]
    pub fn visit(&mut self, slot: &Slot) {
        self.visit_bits(slot.bits());
    }

    pub fn visit_slots(&mut self, slots: &[Slot]) {
        for s in slots {
            self.visit(s);
        }
    }

    #[inline]
    pub fn visit_value(&mut self, v: Value) {
        self.visit_bits(v.to_bits());
    }

    /// A raw `Value` word (VM stack, handle table). The bits 0 are never a `Value`: debug
    /// builds assert, release builds skip the word and never dereference it (H06-empty).
    #[inline]
    pub fn visit_bits(&mut self, bits: u64) {
        if bits == 0 {
            if self.check {
                self.error(format_args!("slot holds the bits 0"));
                return;
            }
            debug_assert!(bits != 0, "a GC-visible slot holds the bits 0 (H06-empty)");
            return;
        }
        if self.check && !Value::is_valid_bits(bits) {
            self.error(format_args!("slot holds invalid Value bits {bits:#018x}"));
            return;
        }
        if is_cell_bits(bits) {
            self.edge(bits as usize);
        }
    }

    /// A raw cell pointer field.
    #[inline]
    pub fn visit_cell(&mut self, cell: CellPtr) {
        self.edge(cell.addr());
    }

    /// A weak `Value` slot inside a live cell. The heap clears it to `undefined` at GC end if
    /// its cell is not marked by then. Use [`crate::Heap::add_weak_source`] for weak slots
    /// outside the heap.
    pub fn visit_weak(&mut self, slot: &Slot) {
        let bits = slot.bits();
        match self.mode {
            Mode::Mark => {
                debug_assert!(
                    self.mem.contains(core::ptr::from_ref(slot).addr()),
                    "visit_weak on a slot outside the heap"
                );
                // The verifier's checks of a weak slot, before the slot can be cleared: a
                // dangling weak reference would otherwise be cleared without a word.
                if self.check && !self.check_weak_bits(bits) {
                    return;
                }
                debug_assert!(bits != 0, "a weak slot holds the bits 0 (H06-empty)");
                if bits != 0 && is_cell_bits(bits) {
                    self.weak.push(NonNull::from(slot));
                }
            }
            Mode::Verify => self.visit_bits(bits),
        }
    }

    /// An ephemeron: `value` is strong only while the cell in `key` is live (a WeakMap entry;
    /// 03-gc "Weak semantics"). `key` is a weak slot as [`Tracer::visit_weak`] treats it: it
    /// is cleared to `undefined` at GC end if its cell dies, and then `value` is cleared too,
    /// unless its own cell is live by another path. A key of `undefined` is that cleared form,
    /// so its `value` stays weak ([`Tracer::visit_weak`]) at every later GC. Any other key that
    /// is not a cell keeps `value` as a strong slot would. Both slots lie inside the live cell
    /// being traced.
    ///
    /// A value is traced once its key is marked, from any path, also when the key is marked
    /// later in this GC (the fixpoint, module doc). So a value that reaches only its own key,
    /// or a cycle of entries whose keys only the values reach, does not keep itself alive.
    pub fn visit_ephemeron(&mut self, key: &Slot, value: &Slot) {
        if self.mode == Mode::Verify {
            self.visit_bits(key.bits());
            self.visit_bits(value.bits());
            return;
        }
        debug_assert!(
            self.mem.contains(core::ptr::from_ref(value).addr()),
            "visit_ephemeron on a slot outside the heap"
        );
        let kb = key.bits();
        if kb == Value::UNDEFINED.to_bits() {
            // A key an earlier GC cleared: the value stays weak, so it dies with its last
            // other path (a strong visit here would keep it alive for good).
            self.visit_weak(value);
            return;
        }
        let errors = self.errors.len();
        self.visit_weak(key);
        if (self.check && self.errors.len() != errors)
            || kb == 0
            || !is_cell_bits(kb)
            || is_marked(self.mem, kb as usize)
        {
            // A live key (or a bad one, which the check just reported): the value is strong.
            self.visit(value);
            return;
        }
        let vb = value.bits();
        // The value may wait to the end and then be cleared as a weak slot: check it now.
        if self.check && !self.check_weak_bits(vb) {
            return;
        }
        if vb == 0 || !is_cell_bits(vb) {
            return;
        }
        match &mut self.waiting {
            Some(map) => map
                .entry(kb as usize)
                .or_default()
                .push(NonNull::from(value)),
            None => self
                .ephemerons
                .push((NonNull::from(key), NonNull::from(value))),
        }
    }

    /// The verifier's checks of a weak slot's bits (not 0, valid `Value` bits, a cell
    /// reference is the start of a live cell). False after an error.
    fn check_weak_bits(&mut self, bits: u64) -> bool {
        if bits == 0 {
            self.error(format_args!("slot holds the bits 0"));
            return false;
        }
        if !Value::is_valid_bits(bits) {
            self.error(format_args!("slot holds invalid Value bits {bits:#018x}"));
            return false;
        }
        if is_cell_bits(bits) && cell_start(self.mem, bits as usize).is_none() {
            self.error(format_args!(
                "reference {:#x} is not the start of a live cell",
                bits as usize
            ));
            return false;
        }
        true
    }

    fn edge(&mut self, addr: usize) {
        match self.mode {
            Mode::Mark => {
                if !self.mark_start(addr) {
                    if self.check {
                        self.error(format_args!(
                            "reference {addr:#x} is not the start of a live cell"
                        ));
                    } else {
                        debug_assert!(false, "{addr:#x} is not the start of a cell of this heap");
                    }
                }
            }
            Mode::Verify => {
                if cell_start(self.mem, addr).is_none() {
                    self.error(format_args!(
                        "reference {addr:#x} is not the start of a live cell"
                    ));
                }
            }
        }
    }

    /// The cell-start test and the mark in one chunk lookup: false if no live cell starts at
    /// `addr`; otherwise marks the cell and pushes it if it was not marked.
    #[inline]
    fn mark_start(&mut self, addr: usize) -> bool {
        if !addr.is_multiple_of(8) {
            return false;
        }
        let Some(idx) = self.mem.chunk_index(addr) else {
            return false;
        };
        let newly = match self.mem.chunk(idx) {
            Chunk::Small => {
                let page = self.mem.page(idx);
                if addr - page.addr() < FIRST_CELL_OFFSET || !page.vo(addr) {
                    return false;
                }
                page.mark(addr)
            }
            Chunk::LargeHead { .. } => {
                // SAFETY: chunk `idx` is the committed first chunk of a large object.
                let large = unsafe { Large::from_base(self.mem.chunk_base(idx)) };
                if addr != large.cell_addr() {
                    return false;
                }
                large.header().mark.swap(1, Relaxed) == 0
            }
            _ => return false,
        };
        if newly {
            // SAFETY: a live cell of this heap starts at `addr` (checked above).
            let c = unsafe { CellPtr::from_raw(self.mem.ptr(addr).cast()) };
            self.pushed(c);
        }
        true
    }

    /// Marks a cell start; pushes it if it was not marked.
    fn mark(&mut self, c: CellPtr) -> bool {
        if !set_mark(self.mem, c.addr()) {
            return false;
        }
        self.pushed(c);
        true
    }

    /// A cell whose mark bit this tracer just set.
    #[inline]
    fn pushed(&mut self, c: CellPtr) {
        // S2 marker rule, minus the fence S0 does not need: set BLACK, then read the fields.
        // SAFETY: `c` is a cell start of this heap (VO bit or LOS header).
        unsafe { c.header() }.set_gc_state(gc_state::BLACK);
        self.pushed_cells += 1;
        self.stack.push(c);
    }

    /// Conservative candidate `word` (with the end rule: `word - 1` too). Returns the newly
    /// marked cells through `log`.
    pub(crate) fn mark_conservative(&mut self, word: usize, mut log: Option<&mut Vec<CellPtr>>) {
        for cand in [word, word.wrapping_sub(1)] {
            if let Some(c) = find_cell(self.mem, self.kinds, cand)
                && self.mark(c)
                && let Some(l) = log.as_deref_mut()
            {
                l.push(c);
            }
        }
    }

    /// Traces until the mark stack is empty. With a shape space, it also marks the shape that
    /// each traced cell's word0 names. Ephemerons whose key is still unmarked stay pending
    /// ([`Tracer::drain_to_fixpoint`] resolves them).
    pub(crate) fn drain(&mut self) {
        while let Some(c) = self.stack.pop() {
            self.trace_one(c);
        }
    }

    /// [`Tracer::drain`] in the linear form of the ephemeron fixpoint: each popped cell first
    /// releases the values that wait for it as a key. Rare, so out of line: the hot drain
    /// keeps the only inlined copy of `trace_one`.
    #[cold]
    #[inline(never)]
    fn drain_linear(&mut self) {
        while let Some(c) = self.stack.pop() {
            if let Some(values) = self.waiting.as_mut().and_then(|m| m.remove(&c.addr())) {
                for v in values {
                    // SAFETY: the value slot lies in a cell that this GC traced, so it stays
                    // valid until the GC ends (nothing is freed while marking).
                    self.visit(unsafe { v.as_ref() });
                }
            }
            self.trace_one_outlined(c);
        }
    }

    #[inline(never)]
    fn trace_one_outlined(&mut self, c: CellPtr) {
        self.trace_one(c);
    }

    /// Marks everything reachable from the roots given so far, ephemerons included: drains,
    /// then traces the value of each pending ephemeron whose key got marked, and repeats until
    /// nothing new is marked (module doc). After it, the pending entries have dead keys: their
    /// value slots join the weak slots, so [`Tracer::clear_weak_slots`] clears each one whose
    /// cell is unmarked. Called once per GC, after the last root.
    pub(crate) fn drain_to_fixpoint(&mut self) {
        self.drain();
        let mut rounds = 0;
        while !self.ephemerons.is_empty() {
            if rounds == EPHEMERON_ROUNDS {
                self.ephemerons_linear();
                break;
            }
            rounds += 1;
            self.resolve_ephemerons();
            if self.stack.is_empty() {
                // No key was marked since the last round: the rest stays dead.
                break;
            }
            self.drain();
        }
        self.ephemeron_rounds = rounds;
        let dead = self.ephemerons.drain(..).map(|(_, v)| v);
        self.weak.extend(dead);
        if let Some(map) = self.waiting.take() {
            self.weak.extend(map.into_values().flatten());
        }
    }

    /// One round: traces the value of every pending ephemeron whose key is marked now, and
    /// drops it from the list.
    fn resolve_ephemerons(&mut self) {
        let mut i = 0;
        while i < self.ephemerons.len() {
            let (k, v) = self.ephemerons[i];
            // SAFETY: both slots lie in cells that this GC traced; nothing is freed while
            // marking, so they stay valid.
            let kb = unsafe { k.as_ref() }.bits();
            if is_marked(self.mem, kb as usize) {
                self.ephemerons.swap_remove(i);
                // SAFETY: as above.
                self.visit(unsafe { v.as_ref() });
            } else {
                i += 1;
            }
        }
    }

    /// The linear form: one more round (so every key left is unmarked), then a map key ->
    /// values, and a drain in which each popped cell releases its waiting values. New
    /// ephemerons with an unmarked key go straight into the map ([`Tracer::visit_ephemeron`]).
    fn ephemerons_linear(&mut self) {
        self.ephemeron_linear = true;
        self.resolve_ephemerons();
        let mut map = EphemeronMap::default();
        for (k, v) in self.ephemerons.drain(..) {
            // SAFETY: as in `resolve_ephemerons`.
            let kb = unsafe { k.as_ref() }.bits();
            map.entry(kb as usize).or_default().push(v);
        }
        self.waiting = Some(map);
        self.drain_linear();
    }

    /// Traces one marked cell.
    #[inline(always)]
    fn trace_one(&mut self, c: CellPtr) {
        // SAFETY: only marked cell starts are pushed.
        let h = unsafe { c.header() };
        let kind = h.kind();
        if self.check {
            self.context = Context::Cell(c, kind);
        }
        let Some(k) = self.kinds.get(kind).copied() else {
            if self.check {
                self.error(format_args!("unregistered kind {kind}"));
                return;
            }
            panic!("heap corrupt: cell {c:?} has the unregistered kind {kind}");
        };
        self.marked_cells += 1;
        let size = self.kinds.cell_size(c);
        if self.check {
            self.check_marked(c, kind, size);
        }
        self.marked_bytes += size.unwrap_or(0);
        if self.shape_bytes != 0 {
            let id = h.shape_id().get() as usize;
            if id != 0 {
                self.mark_shape(c, id);
            }
        }
        (k.trace)(c, self);
    }

    /// A checked mark's checks of a cell it traces (the context names it), besides its
    /// slots and word0 shape: a size, and the shape region holds exactly the shape-space
    /// kinds. The checked sweep checks the size class and the cell order (heap.rs); the
    /// verify after the GC checks LOS cells (verify.rs). No gc_state check: the mark has just
    /// set BLACK.
    fn check_marked(&mut self, c: CellPtr, kind: u8, size: Option<usize>) {
        if size.is_none() {
            self.error(format_args!("size overflows"));
        }
        let in_region = c.addr().wrapping_sub(self.mem.base_addr()) < self.shape_bytes;
        let space = self.kinds.space(kind);
        if (space == Space::Shape) != in_region {
            self.error(format_args!(
                "a {space:?}-space cell {} the shape region",
                if in_region { "inside" } else { "outside" }
            ));
        }
    }

    /// Marks the shape cell at offset `id` of the region (the word0 shape of `host`). Inlined
    /// into the drain: it runs for every traced cell with a shape.
    #[inline(always)]
    fn mark_shape(&mut self, host: CellPtr, id: usize) {
        match self.shape_at(id) {
            Some(s) => {
                self.mark(s);
            }
            None if self.check => self.error(format_args!(
                "word0 ShapeId {id:#x} does not name a live shape-space cell"
            )),
            None => debug_assert!(false, "{host:?}: word0 ShapeId {id:#x} names no shape cell"),
        }
    }

    /// The live shape-space cell that starts at offset `id` of the shape region, if any.
    fn shape_at(&self, id: usize) -> Option<CellPtr> {
        if id >= self.shape_bytes {
            return None;
        }
        let c = cell_start(self.mem, self.mem.base_addr() + id)?;
        // SAFETY: `cell_start` found a live cell start (VO bit) in a committed page.
        let kind = unsafe { c.header() }.kind();
        (self.kinds.space(kind) == Space::Shape).then_some(c)
    }

    /// Verify mode: the word0 shape of the current cell. With a shape space, a non-zero id
    /// must name a live shape-space cell.
    pub(crate) fn verify_word0_shape(&mut self, id: u32) {
        if self.shape_bytes != 0 && id != 0 && self.shape_at(id as usize).is_none() {
            self.error(format_args!(
                "word0 ShapeId {id:#x} does not name a live shape-space cell"
            ));
        }
    }

    /// After the fixpoint: clears every recorded weak slot whose cell is unmarked.
    pub(crate) fn clear_weak_slots(&mut self) -> usize {
        let mut cleared = 0;
        for s in self.weak.drain(..) {
            // SAFETY: the slot lies in a cell that was marked (only live cells are traced), so
            // its memory stays valid until this GC ends.
            let slot = unsafe { s.as_ref() };
            let bits = slot.bits();
            if is_cell_bits(bits) && !is_marked(self.mem, bits as usize) {
                slot.gc_store(Value::UNDEFINED);
                cleared += 1;
            }
        }
        cleared
    }

    pub(crate) fn error(&mut self, msg: core::fmt::Arguments<'_>) {
        record_error(&mut self.errors, self.context, msg);
    }
}

/// Adds one verifier error at `context`; keeps the first [`MAX_ERRORS`].
#[cold]
pub(crate) fn record_error(
    errors: &mut Vec<String>,
    context: Context,
    msg: core::fmt::Arguments<'_>,
) {
    if errors.len() < MAX_ERRORS {
        let at = match context {
            Context::Root(r) => format!("root {r}"),
            Context::Cell(c, k) => format!("cell {c:?} (kind {k})"),
        };
        errors.push(format!("{at}: {msg}"));
    } else if errors.len() == MAX_ERRORS {
        errors.push("more errors not shown".into());
    }
}

/// Passed to weak sources after marking. A slot whose cell is unmarked is dead.
///
/// After a checked mark (a GC with `verify_after_gc`), [`WeakVisitor::visit`] also checks each
/// slot the way the verifier does, before it clears it: the weak root ranges are checked here,
/// so the verify after that GC walks no root a second time.
pub struct WeakVisitor<'h> {
    mem: &'h Memory,
    pub(crate) cleared: usize,
    check: bool,
    pub(crate) errors: Vec<String>,
}

impl<'h> WeakVisitor<'h> {
    pub(crate) fn new(mem: &'h Memory) -> WeakVisitor<'h> {
        WeakVisitor {
            mem,
            cleared: 0,
            check: false,
            errors: Vec::new(),
        }
    }

    /// A visitor whose `visit` checks each slot first (errors in `errors`, appended to
    /// `errors`).
    pub(crate) fn new_checked(mem: &'h Memory, errors: Vec<String>) -> WeakVisitor<'h> {
        WeakVisitor {
            mem,
            cleared: 0,
            check: true,
            errors,
        }
    }

    /// False only for a cell that this GC did not mark.
    pub fn is_live(&self, v: Value) -> bool {
        !v.is_cell() || is_marked(self.mem, v.to_bits() as usize)
    }

    /// Clears `slot` to `undefined` if it holds a dead cell. Returns true if it cleared.
    pub fn visit(&mut self, slot: &Slot) -> bool {
        let bits = slot.bits();
        if self.check && !self.check_weak(bits) {
            return false;
        }
        if bits != 0 && is_cell_bits(bits) && !is_marked(self.mem, bits as usize) {
            slot.gc_store(Value::UNDEFINED);
            self.cleared += 1;
            return true;
        }
        false
    }

    /// The verifier's checks of a weak root slot: not 0, valid `Value` bits, a cell reference
    /// is the start of a cell (live or dead in this GC; the sweep has not run yet). False on
    /// an error: the slot is left as it is.
    fn check_weak(&mut self, bits: u64) -> bool {
        let ctx = Context::Root("weak range");
        if bits == 0 {
            record_error(&mut self.errors, ctx, format_args!("slot holds the bits 0"));
        } else if !Value::is_valid_bits(bits) {
            record_error(
                &mut self.errors,
                ctx,
                format_args!("slot holds invalid Value bits {bits:#018x}"),
            );
        } else if is_cell_bits(bits) && cell_start(self.mem, bits as usize).is_none() {
            record_error(
                &mut self.errors,
                ctx,
                format_args!(
                    "reference {:#x} is not the start of a live cell",
                    bits as usize
                ),
            );
        } else {
            return true;
        }
        false
    }
}

/// The cell that starts exactly at `addr`, if a live one does (VO bit, or a LOS header).
pub(crate) fn cell_start(mem: &Memory, addr: usize) -> Option<CellPtr> {
    if !addr.is_multiple_of(8) {
        return None;
    }
    let idx = mem.chunk_index(addr)?;
    let ok = match mem.chunk(idx) {
        Chunk::Small => {
            let page = mem.page(idx);
            addr - page.addr() >= FIRST_CELL_OFFSET && page.vo(addr)
        }
        Chunk::LargeHead { .. } => {
            // SAFETY: chunk `idx` is the committed first chunk of a large object.
            let large = unsafe { Large::from_base(mem.chunk_base(idx)) };
            addr == large.cell_addr()
        }
        _ => false,
    };
    // SAFETY: a live cell of this heap starts at `addr` (checked above); the pointer gets the
    // reservation's provenance.
    ok.then(|| unsafe { CellPtr::from_raw(mem.ptr(addr).cast()) })
}

/// Interior-pointer lookup for the conservative scan: the live cell that contains `addr`.
pub(crate) fn find_cell(mem: &Memory, kinds: &KindTable, addr: usize) -> Option<CellPtr> {
    let idx = mem.chunk_index(addr)?;
    match mem.chunk(idx) {
        Chunk::Small => {
            let page = mem.page(idx);
            let start = page.prev_vo(addr, MAX_SMALL_SIZE)?;
            // SAFETY: a VO bit marks the start of an allocated cell in this committed page.
            let c = unsafe { CellPtr::from_raw(mem.ptr(start).cast()) };
            let size = kinds.cell_size(c)?;
            (addr < start + size).then_some(c)
        }
        Chunk::LargeHead { .. } => large_containing(mem, idx, addr),
        Chunk::LargeTail { head } => large_containing(mem, head as usize, addr),
        Chunk::Free { .. } => None,
    }
}

fn large_containing(mem: &Memory, head: usize, addr: usize) -> Option<CellPtr> {
    // SAFETY: chunk `head` is the committed first chunk of a large object.
    let large = unsafe { Large::from_base(mem.chunk_base(head)) };
    let start = large.cell_addr();
    let size = large.header().size;
    // SAFETY: the LOS cell starts right after the header.
    (start <= addr && addr < start + size)
        .then(|| unsafe { CellPtr::from_raw(large.cell_ptr().cast()) })
}

/// Sets the mark bit of the cell starting at `addr`; true if it was clear.
fn set_mark(mem: &Memory, addr: usize) -> bool {
    let Some(idx) = mem.chunk_index(addr) else {
        return false;
    };
    match mem.chunk(idx) {
        Chunk::Small => mem.page(idx).mark(addr),
        Chunk::LargeHead { .. } => {
            // SAFETY: as in `cell_start`.
            let large = unsafe { Large::from_base(mem.chunk_base(idx)) };
            large.header().mark.swap(1, Relaxed) == 0
        }
        _ => false,
    }
}

/// True if the cell starting at `addr` is marked. Anything that is not a cell of this heap
/// counts as unmarked.
pub(crate) fn is_marked(mem: &Memory, addr: usize) -> bool {
    let Some(idx) = mem.chunk_index(addr) else {
        return false;
    };
    match mem.chunk(idx) {
        Chunk::Small => mem.page(idx).is_marked(addr),
        Chunk::LargeHead { .. } => {
            debug_assert_eq!(addr & PAGE_MASK, mem.chunk_base(idx).addr().get());
            // SAFETY: as in `cell_start`.
            let large = unsafe { Large::from_base(mem.chunk_base(idx)) };
            large.header().mark.load(Relaxed) != 0
        }
        _ => false,
    }
}
