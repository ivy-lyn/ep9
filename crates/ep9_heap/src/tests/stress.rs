//! Randomized mutator stress (M0 S0 core, item 4).
//!
//! ```text
//! model: id -> {addr, slots: [Cell(id) | Bits(non-cell value)]}
//! roots: global handles, a VM-stack root source, scratch roots      (precise)
//!        an array of addresses in this Rust frame                    (conservative, not Miri)
//!        weak handles                                                 (weak)
//! step:  alloc (small / medium / large) | store through Slot::set at a random threshold
//!        (generational heap: at its GEN threshold, so old hosts log) |
//!        root change | weak handle | explicit collect;  GC-stress collects inside alloc
//! after every GC:  may  = closure(precise + stack array + cells the scan reported
//!                                + after an S1 minor GC: the cells that were old, i.e. live
//!                                  after the GC before it; old cells die only in a major GC)
//!                  must = closure(precise + stack array)
//!                  every model cell: live <=> in may;  must cells keep id and slots;
//!                  weak handle: cleared <=> its cell died;  stack-array cells were found
//! ```
//!
//! V1 (tests/v1.rs): [`run_controlled`] runs the generational form with a negative control
//! of 03-gc §7 at its store site, and optionally with E2 elisions:
//!
//! ```text
//! E2 (elide):  a store into a host that the region calls fresh is a plain store (Slot::init)
//!   control off:         fresh = allocated with no GC point since, not escaped (stored into a
//!                        cell, a root, a weak handle); such a host must be WHITE
//!   ElideAfterEscape:    fresh = allocated in the current region (REGION steps), escaped or not
//! other stores:  StoreSite (Slot::set; DropOneIn(n), OldOnlyFilter break it)
//! ```
//!
//! Liveness is read without forcing the lazy sweep (`live_after_gc`), so allocation keeps
//! sweeping pages lazily during the run.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::hint::black_box;
use std::rc::Rc;

use ep9_value::Value;

use super::v1::{Control, StoreSite};
use super::*;
use crate::{GlobalHandle, ScratchRoots, WeakHandle, gc_state, threshold};

/// Steps per E2 region ([`run_controlled`] with `elide`).
const REGION: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Cell(u32),
    Bits(u64),
}

struct Obj {
    addr: usize,
    slots: Vec<Edge>,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64: deterministic, so a failure is reproducible from the seed.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const STACK_ROOTS: usize = 8;

struct Run<'h> {
    heap: &'h Heap,
    rng: Rng,
    objs: HashMap<u32, Obj>,
    /// Model ids in increasing order (for reproducible random picks).
    ids: Vec<u32>,
    next_id: u32,
    globals: Vec<(GlobalHandle, Option<u32>)>,
    vm_stack: Rc<RefCell<Vec<u64>>>,
    vm_ids: Vec<Option<u32>>,
    scratch: ScratchRootsModel,
    weaks: Vec<(WeakHandle, u32)>,
    stack_ids: [Option<u32>; STACK_ROOTS],
    conservative: bool,
    /// The heap is generational: stores keep its GEN threshold.
    generational: bool,
    /// The model cells live after the last GC: old in a generational heap.
    old: Vec<u32>,
    minor_seen: u64,
    seen_gcs: u64,
    checks: u64,
    freed_seen: u64,
    retained_seen: u64,
    /// The store site of the generational stores (a negative control, or `Control::Off`).
    site: StoreSite,
    /// E2 elisions on (module doc).
    elide: bool,
    /// E2: hosts allocated with no GC point since and not escaped; hosts allocated in the
    /// current region; steps run.
    fresh: BTreeSet<u32>,
    region: BTreeSet<u32>,
    steps: usize,
    /// E2 stores made (plain stores into a host the region called fresh).
    elided: u64,
}

/// Model of the run's one ScratchRoots scope (fixed size; entries are overwritten).
struct ScratchRootsModel {
    ids: Vec<Option<u32>>,
}

impl<'h> Run<'h> {
    fn cell(&self, id: u32) -> CellPtr {
        let addr = self.objs[&id].addr;
        // SAFETY: every model object is a live cell: the checker removes freed ones right
        // after each GC, before the next allocation can reuse their memory.
        unsafe { CellPtr::from_raw(core::ptr::NonNull::new(heap_ptr(self.heap, addr)).unwrap()) }
    }

    fn value(&self, e: Edge) -> Value {
        match e {
            Edge::Cell(id) => self.cell(id).to_value(),
            // SAFETY: `Bits` edges hold bits of non-cell Values built by `random_bits`.
            Edge::Bits(b) => unsafe { Value::from_bits_unchecked(b) },
        }
    }

    fn random_bits(&mut self) -> u64 {
        match self.rng.below(4) {
            0 => Value::UNDEFINED.to_bits(),
            1 => Value::int32(self.rng.next() as i32).to_bits(),
            2 => Value::double(f64::from_bits(self.rng.next())).to_bits(),
            _ => Value::NULL.to_bits(),
        }
    }

    fn random_id(&mut self) -> Option<u32> {
        if self.ids.is_empty() {
            return None;
        }
        Some(self.ids[self.rng.below(self.ids.len())])
    }

    fn random_edge(&mut self) -> Edge {
        if self.rng.below(3) > 0
            && let Some(id) = self.random_id()
        {
            return Edge::Cell(id);
        }
        Edge::Bits(self.random_bits())
    }

    /// Allocates a node; runs the checker if a GC happened inside the allocation.
    fn alloc(&mut self, stack_roots: &mut [usize; STACK_ROOTS]) -> u32 {
        let n = match self.rng.below(100) {
            0..=69 => self.rng.below(7),         // small
            70..=97 => 31 + self.rng.below(200), // medium (> 256 B)
            _ => 1100 + self.rng.below(600),     // large (> 8 KiB)
        };
        let id = self.next_id;
        self.next_id += 1;
        let gcs = self.heap.stats().collections;
        let c = new_node(self.heap, id, n);
        let addr = c.addr();
        if self.heap.stats().collections != gcs {
            self.fresh.clear(); // a GC point ended every region-fresh host's E2 claim
        }
        self.fresh.insert(id);
        self.region.insert(id);
        self.after_gc_point(Some(addr), stack_roots);
        self.objs.insert(
            id,
            Obj {
                addr,
                slots: vec![Edge::Bits(Value::UNDEFINED.to_bits()); n],
            },
        );
        self.ids.push(id);
        id
    }

    fn after_gc_point(&mut self, new_addr: Option<usize>, stack_roots: &mut [usize; STACK_ROOTS]) {
        let gcs = self.heap.stats().collections;
        if gcs != self.seen_gcs {
            assert_eq!(gcs, self.seen_gcs + 1, "one GC per GC point");
            self.seen_gcs = gcs;
            self.check(new_addr, stack_roots);
        }
    }

    fn closure(&self, roots: impl IntoIterator<Item = u32>) -> BTreeSet<u32> {
        let mut seen = BTreeSet::new();
        let mut work: Vec<u32> = roots.into_iter().collect();
        while let Some(id) = work.pop() {
            if seen.insert(id) {
                work.extend(self.objs[&id].slots.iter().filter_map(|e| match e {
                    Edge::Cell(t) => Some(*t),
                    Edge::Bits(_) => None,
                }));
            }
        }
        seen
    }

    fn check(&mut self, new_addr: Option<usize>, stack_roots: &mut [usize; STACK_ROOTS]) {
        self.checks += 1;
        let by_addr: HashMap<usize, u32> = self.objs.iter().map(|(id, o)| (o.addr, *id)).collect();
        let precise: Vec<u32> = self
            .globals
            .iter()
            .filter_map(|g| g.1)
            .chain(self.vm_ids.iter().flatten().copied())
            .chain(self.scratch.ids.iter().flatten().copied())
            .collect();
        let stack: Vec<u32> = if self.conservative {
            self.stack_ids.iter().flatten().copied().collect()
        } else {
            Vec::new()
        };
        let mut css = Vec::new();
        for c in self.heap.last_conservative_roots() {
            let id = *by_addr
                .get(&c.addr())
                .unwrap_or_else(|| panic!("the scan found {c:?}, which is no model cell"));
            css.push(id);
        }
        let must = self.closure(precise.iter().chain(&stack).copied());
        let minor = self.heap.stats().last_minor;
        assert!(self.generational || !minor, "an S0 heap ran a minor GC");
        self.minor_seen += u64::from(minor);
        let old: &[u32] = if minor { &self.old } else { &[] };
        let may = self.closure(precise.iter().chain(&stack).chain(&css).chain(old).copied());
        for id in &stack {
            assert!(
                may.contains(id),
                "stack-array cell {id} was not found by the scan"
            );
        }
        let ids = std::mem::take(&mut self.ids);
        for &id in &ids {
            let addr = self.objs[&id].addr;
            let live = Some(addr) != new_addr && self.heap.live_after_gc(addr);
            assert_eq!(
                live,
                may.contains(&id),
                "cell {id} at {addr:#x}: live {live}, reachable {} (must {})",
                may.contains(&id),
                must.contains(&id)
            );
            if !live {
                self.objs.remove(&id);
                self.freed_seen += 1;
                continue;
            }
            if !must.contains(&id) {
                self.retained_seen += 1;
            }
            let c = self.cell(id);
            assert_eq!(node_id(c), id, "cell {id}: id word changed");
            for (i, e) in self.objs[&id].slots.iter().enumerate() {
                let want = match e {
                    Edge::Cell(t) => self.objs[t].addr as u64,
                    Edge::Bits(b) => *b,
                };
                assert_eq!(slot(c, i).bits(), want, "cell {id} slot {i}");
            }
        }
        self.ids = ids
            .into_iter()
            .filter(|id| self.objs.contains_key(id))
            .collect();
        // Every survivor is old now (sticky marks); a new cell starts young.
        self.old.clone_from(&self.ids);
        // Weak handles: cleared exactly when their cell died.
        let mut keep = Vec::new();
        for (h, id) in std::mem::take(&mut self.weaks) {
            match (self.objs.get(&id), self.heap.weak(h)) {
                (Some(o), Some(v)) => {
                    assert_eq!(v.to_bits(), o.addr as u64);
                    keep.push((h, id));
                }
                (None, None) => self.heap.drop_weak(h),
                (o, v) => panic!(
                    "weak handle to cell {id}: cell live {}, value {v:?}",
                    o.is_some()
                ),
            }
        }
        self.weaks = keep;
        // Stack-array slots whose cell died can no longer be roots: clear them.
        for (i, s) in self.stack_ids.iter_mut().enumerate() {
            if s.is_some_and(|id| !self.objs.contains_key(&id)) {
                *s = None;
                stack_roots[i] = 0;
            }
        }
        if self.checks.is_multiple_of(25) {
            self.heap.verify().expect("verifier during the stress run");
        }
    }

    fn step(&mut self, scratch: &ScratchRoots<'_>, stack_roots: &mut [usize; STACK_ROOTS]) {
        self.steps += 1;
        if self.steps.is_multiple_of(REGION) {
            self.region.clear();
        }
        match self.rng.below(100) {
            0..=34 => {
                let id = self.alloc(stack_roots);
                if self.elide {
                    // Initializing stores into the new cell before it escapes (E2).
                    for _ in 0..self.rng.below(3) {
                        let e = self.random_edge();
                        self.store(id, e);
                    }
                }
                // Make most new cells reachable from somewhere.
                match self.rng.below(6) {
                    0 => self.set_global(Some(id)),
                    1 => self.set_vm(Some(id)),
                    2 => self.set_scratch(scratch, Some(id)),
                    3 if self.conservative => self.set_stack(stack_roots, Some(id)),
                    _ => {
                        if let Some(host) = self.random_id() {
                            self.store(host, Edge::Cell(id));
                        }
                    }
                }
            }
            35..=69 => {
                if let Some(host) = self.random_id() {
                    let e = self.random_edge();
                    self.store(host, e);
                }
            }
            70..=74 => {
                let e = self.random_id();
                self.set_global(e);
            }
            75..=79 => {
                let e = if self.rng.below(2) == 0 {
                    self.random_id()
                } else {
                    None
                };
                self.set_vm(e);
            }
            80..=83 => {
                let e = if self.rng.below(2) == 0 {
                    self.random_id()
                } else {
                    None
                };
                self.set_scratch(scratch, e);
            }
            84..=89 if self.conservative => {
                let e = if self.rng.below(2) == 0 {
                    self.random_id()
                } else {
                    None
                };
                self.set_stack(stack_roots, e);
            }
            90..=94 => {
                if let Some(id) = self.random_id() {
                    let v = self.cell(id).to_value();
                    self.weaks.push((self.heap.new_weak(v), id));
                    self.fresh.remove(&id);
                }
            }
            _ => {
                if self.rng.below(4) == 0 {
                    // A generational heap: an explicit minor GC half the time.
                    if self.generational && self.rng.below(2) == 0 {
                        self.heap.collect_minor();
                    } else {
                        self.heap.collect();
                    }
                    self.fresh.clear();
                    self.after_gc_point(None, stack_roots);
                }
            }
        }
    }

    fn store(&mut self, host: u32, e: Edge) {
        let n = self.objs[&host].slots.len();
        if n == 0 {
            return;
        }
        let i = self.rng.below(n);
        let v = self.value(e);
        if let Edge::Cell(id) = e {
            self.fresh.remove(&id); // escapes into `host`
        }
        if self.generational {
            let c = self.cell(host);
            let elided = self.elide
                && if self.site.control == Control::ElideAfterEscape {
                    self.region.contains(&host)
                } else {
                    self.fresh.contains(&host)
                };
            if elided {
                if self.site.control != Control::ElideAfterEscape {
                    // The E2 check: a fresh host is young.
                    assert_eq!(
                        state_of(c),
                        gc_state::WHITE,
                        "E2 into a host that is not WHITE"
                    );
                }
                slot(c, i).init(v);
                self.elided += 1;
            } else {
                self.site.store_node(self.heap, c, i, v);
            }
        } else {
            // Any threshold: the slow path must store exactly as the fast path does.
            let th = [threshold::OFF, threshold::GEN, threshold::MARKING][self.rng.below(3)];
            self.heap.barrier().set_threshold(th);
            store(self.heap, self.cell(host), i, v);
            self.heap.barrier().set_threshold(threshold::OFF);
        }
        self.objs.get_mut(&host).unwrap().slots[i] = e;
    }

    fn set_global(&mut self, id: Option<u32>) {
        self.escape(id);
        let v = id.map_or(Value::UNDEFINED, |id| self.cell(id).to_value());
        if self.globals.len() < 16 {
            self.globals.push((self.heap.new_global(v), id));
        } else if self.rng.below(4) == 0 {
            let (h, _) = self.globals.swap_remove(self.rng.below(16));
            self.heap.drop_global(h);
        } else {
            let i = self.rng.below(self.globals.len());
            self.heap.set_global(self.globals[i].0, v);
            self.globals[i].1 = id;
        }
    }

    fn set_vm(&mut self, id: Option<u32>) {
        self.escape(id);
        let bits = id.map_or(Value::UNDEFINED.to_bits(), |id| {
            self.cell(id).to_value().to_bits()
        });
        let i = self.rng.below(self.vm_ids.len());
        self.vm_stack.borrow_mut()[i] = bits;
        self.vm_ids[i] = id;
    }

    fn set_scratch(&mut self, scratch: &ScratchRoots<'_>, id: Option<u32>) {
        self.escape(id);
        let v = id.map_or(Value::NULL, |id| self.cell(id).to_value());
        let i = self.rng.below(self.scratch.ids.len());
        scratch.set(i, v);
        self.scratch.ids[i] = id;
    }

    /// E2: a host stored into a root has escaped.
    fn escape(&mut self, id: Option<u32>) {
        if let Some(id) = id {
            self.fresh.remove(&id);
        }
    }

    fn set_stack(&mut self, stack_roots: &mut [usize; STACK_ROOTS], id: Option<u32>) {
        self.escape(id);
        let i = self.rng.below(STACK_ROOTS);
        stack_roots[i] = id.map_or(0, |id| self.objs[&id].addr);
        black_box(&mut *stack_roots);
        self.stack_ids[i] = id;
    }
}

#[derive(Debug)]
pub(super) struct Stats {
    checks: u64,
    freed: u64,
    retained: u64,
    gcs: u64,
    pub(super) minor: u64,
    /// V1 runs (one before each minor GC: these runs verify after every GC).
    pub(super) v1_runs: u64,
    /// E2 stores; stores the control made plain.
    pub(super) elided: u64,
    pub(super) dropped: u64,
}

fn state_of(c: CellPtr) -> u8 {
    // SAFETY: callers pass live cells.
    unsafe { c.header() }.gc_state()
}

fn run(seed: u64, steps: usize, gc_stress: u32) -> Stats {
    run_with(seed, steps, gc_stress, false)
}

/// `generational`: S1 with the verify after every GC (sticky marks, logs, minor GCs: under
/// GC-stress every 64th GC is major, without it the policy picks).
#[inline(never)]
fn run_with(seed: u64, steps: usize, gc_stress: u32, generational: bool) -> Stats {
    run_budget(seed, steps, gc_stress, generational, 1 << 20)
}

#[inline(never)]
fn run_budget(
    seed: u64,
    steps: usize,
    gc_stress: u32,
    generational: bool,
    initial_budget: usize,
) -> Stats {
    run_full(
        seed,
        steps,
        gc_stress,
        generational,
        initial_budget,
        Control::Off,
        false,
    )
}

/// The generational run (GC-stress `gc_stress`, or the budget policy at 0) with `control` at
/// its store site and E2 elisions if `elide` (module doc). `Err`: the text of the panic that
/// stopped it.
pub(super) fn run_controlled(
    seed: u64,
    steps: usize,
    gc_stress: u32,
    control: Control,
    elide: bool,
) -> Result<Stats, String> {
    std::panic::catch_unwind(|| run_full(seed, steps, gc_stress, true, 1 << 18, control, elide))
        .map_err(|p| {
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a panic without a text".into())
        })
}

#[inline(never)]
fn run_full(
    seed: u64,
    steps: usize,
    gc_stress: u32,
    generational: bool,
    initial_budget: usize,
    control: Control,
    elide: bool,
) -> Stats {
    let heap = test_heap(|c| {
        c.gc_stress = gc_stress;
        c.initial_budget = initial_budget;
        c.generational = generational;
        c.verify_after_gc = generational;
    });
    let vm_stack = Rc::new(RefCell::new(vec![Value::UNDEFINED.to_bits(); 12]));
    let src = Rc::clone(&vm_stack);
    heap.add_root_source(move |t| {
        for &w in src.borrow().iter() {
            t.visit_bits(w);
        }
    });
    let scratch = heap.scratch_roots();
    for _ in 0..6 {
        scratch.push(Value::NULL);
    }
    // Conservative roots: plain words in this frame. Kept in memory by black_box.
    let mut stack_roots = [0usize; STACK_ROOTS];
    black_box(&mut stack_roots);
    let mut r = Run {
        heap: &heap,
        rng: Rng(seed),
        objs: HashMap::new(),
        ids: Vec::new(),
        next_id: 1,
        globals: Vec::new(),
        vm_stack,
        vm_ids: vec![None; 12],
        scratch: ScratchRootsModel { ids: vec![None; 6] },
        weaks: Vec::new(),
        stack_ids: [None; STACK_ROOTS],
        conservative: cfg!(not(miri)),
        generational,
        old: Vec::new(),
        minor_seen: 0,
        seen_gcs: 0,
        checks: 0,
        freed_seen: 0,
        retained_seen: 0,
        site: StoreSite::new(control),
        elide,
        fresh: BTreeSet::new(),
        region: BTreeSet::new(),
        steps: 0,
        elided: 0,
    };
    for _ in 0..steps {
        r.step(&scratch, &mut stack_roots);
    }
    heap.collect();
    r.after_gc_point(None, &mut stack_roots);
    heap.verify().expect("verifier at the end");
    black_box(&stack_roots);
    let s = Stats {
        checks: r.checks,
        freed: r.freed_seen,
        retained: r.retained_seen,
        gcs: heap.stats().collections,
        minor: r.minor_seen,
        v1_runs: heap.inner.borrow().v1_runs,
        elided: r.elided,
        dropped: r.site.dropped,
    };
    assert_eq!(
        s.minor,
        heap.stats().minor_collections,
        "a minor GC the checker missed"
    );
    let hs = heap.stats();
    println!(
        "seed {seed:#x} stress {gc_stress} generational {generational}: {steps} steps, {} GCs ({} minor), {} checks, {} cells freed, \
         {} live at end, {} retained-by-scan cell-checks, {} B allocated, {} pages, {} LOS, \
         {} V1 runs, {} E2 stores, {} stores the control made plain",
        s.gcs,
        s.minor,
        s.checks,
        s.freed,
        r.objs.len(),
        s.retained,
        hs.allocated_bytes,
        hs.small_pages,
        hs.large_cells,
        s.v1_runs,
        s.elided,
        s.dropped
    );
    drop(r);
    drop(scratch);
    s
}

#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_gc_stress_1() {
    let s = run(0xE9_0001, 1500, 1);
    assert!(
        s.gcs >= 400 && s.checks == s.gcs,
        "{} GCs, {} checks",
        s.gcs,
        s.checks
    );
    assert!(s.freed > 100, "{} freed", s.freed);
}

#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_gc_stress_7() {
    let s = run(0xE9_0007, 6000, 7);
    assert!(s.gcs >= 200, "{} GCs", s.gcs);
    assert!(s.freed > 500, "{} freed", s.freed);
}

#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_budget_only() {
    // No stress: GCs come from the allocation budget and explicit collects.
    let s = run(0xE9_0000, 20_000, 0);
    assert!(s.gcs >= 20, "{} GCs", s.gcs);
    assert!(s.freed > 1000, "{} freed", s.freed);
    println!("retained by stale stack words: {} cell-checks", s.retained);
}

#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_many_seeds() {
    for seed in 0..8u64 {
        let s = run(0x5EED_0000 + seed, 1500, 3 + seed as u32);
        assert!(s.gcs > 0);
    }
}

/// The Miri variant: under Miri, precise roots only (no conservative scan); small and short.
#[test]
fn random_mutator_small() {
    let s = run(0xE9_4D49, 120, 5);
    assert!(
        s.gcs >= 5 && s.freed > 0,
        "{} GCs, {} freed",
        s.gcs,
        s.freed
    );
}

#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_generational() {
    let s = run_with(0xE9_5100, 1500, 1, true);
    assert!(s.gcs >= 400 && s.checks == s.gcs, "{} GCs", s.gcs);
    assert!(s.freed > 100, "{} freed", s.freed);
    // Every 64th stress GC is major; the run's explicit collects are major too.
    assert!(
        s.minor * 10 >= s.gcs * 8,
        "{} of {} GCs minor",
        s.minor,
        s.gcs
    );
    let s = run_with(0xE9_5107, 6000, 7, true);
    assert!(
        s.gcs >= 200 && s.freed > 500 && s.minor > 0,
        "{} GCs ({} minor), {} freed",
        s.gcs,
        s.minor,
        s.freed
    );
    let s = run_with(0xE9_5100, 20_000, 0, true);
    assert!(
        s.gcs >= 20 && s.freed > 1000 && s.minor > 0,
        "{} GCs ({} minor), {} freed",
        s.gcs,
        s.minor,
        s.freed
    );
}

/// The budget policy with no GC-stress and a small budget (one page): budget GCs are minor
/// until the old bytes pass twice the live bytes of the last major GC, then one is major.
#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_generational_budget_policy() {
    let s = run_budget(0xE9_5B00, 30_000, 0, true, 0);
    assert!(
        s.gcs >= 50 && s.freed > 1000 && s.minor > 0 && s.minor < s.gcs,
        "{} GCs ({} minor), {} freed",
        s.gcs,
        s.minor,
        s.freed
    );
}

/// Many seeds of the generational run (GC-stress 1-8: minor GCs, a major every 64th).
#[test]
#[cfg_attr(miri, ignore = "the Miri variant is below")]
fn random_mutator_generational_many_seeds() {
    for seed in 0..16u64 {
        let s = run_with(0x5E1D_0000 + seed, 1500, 1 + (seed % 8) as u32, true);
        assert!(s.gcs > 0 && s.minor > 0, "seed {seed}: {} GCs", s.gcs);
    }
}

/// The Miri variant of the generational run.
#[test]
fn random_mutator_generational_small() {
    let s = run_with(0xE9_4D51, 120, 5, true);
    assert!(
        s.gcs >= 5 && s.freed > 0,
        "{} GCs, {} freed",
        s.gcs,
        s.freed
    );
}

/// A wider sweep of the generational run (256 seeds, GC-stress 1-8, and the budget policy):
/// `cargo test -p ep9_heap --release -- --ignored seed_sweep`.
#[test]
#[ignore = "a few seconds in release; run by hand after a change to the S1 collector"]
fn random_mutator_generational_seed_sweep() {
    for seed in 0..256u64 {
        let stress = (seed % 9) as u32; // 0: the budget policy
        let s = run_budget(0x5E1D_5EED_0000 + seed, 2000, stress, true, 1 << 18);
        assert!(s.gcs > 0, "seed {seed}: no GC");
    }
}
