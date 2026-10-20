//! The random mutator and the 10^6-safepoint stress.
//!
//! **Live, exactly.** At a safepoint, an object is live if some active mutator frame reads it
//! after the safepoint returns. By construction:
//! - a frame registers each object it holds in the truth log right after it gets it, and
//!   removes it just before it stops using it;
//! - after every safepoint (and every deeper call) the frame reads every object it holds and
//!   checks it (`verify`);
//! - child objects are read only through a held node.
//!
//! So the live set = truth log roots + every object reachable from them through child fields.
//!
//! **How the test knows the truth.** The truth log holds object ids (`u32`), never addresses,
//! and it lives in malloc memory inside the boxed `Mutator`. The scanner reads only the stack,
//! so the log cannot help it. After each scan, the checker walks the live set with its own
//! code and counts every live object that the collection did not mark as missed. A missed
//! object is then kept alive (`F_RESCUED`) so the run can go on and count more.
//!
//! **Second line of defense.** Sweeps free and zap every unmarked object. If the checker were
//! wrong and a live object got freed, the frame's next `verify` would panic.

use std::collections::HashSet;
use std::hint::black_box;
use std::ops::Range;

use ep9_platform::{SpillAbi, current_thread_stack_bounds, with_spilled_registers_abi};

use crate::heap::*;
use crate::value;

/// How a safepoint exposes registers to the scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spill {
    /// The ep9_platform trampoline of this ABI (the real thing).
    Trampoline(SpillAbi),
    /// The negative control: scan from the caller's sp with no register spill.
    NoSpill(SpillAbi),
}

#[derive(Clone, Debug)]
pub struct Config {
    pub spill: Spill,
    pub scan: ScanOptions,
    pub seed: u64,
    pub max_depth: u32,
    pub check_truth: bool,
    /// Sweep after every n-th collection; 0 = never.
    pub sweep_every: u64,
    /// Chance (per mille) that an allocation runs a collection first.
    pub alloc_gc_per_mille: u32,
}

impl Config {
    pub fn stress(spill: Spill, scan: ScanOptions, seed: u64) -> Self {
        Self {
            spill,
            scan,
            seed,
            max_depth: 16,
            check_truth: true,
            sweep_every: 16,
            alloc_gc_per_mille: 250,
        }
    }

    /// Probes: host trampoline, every rule on, no randomness in the GC.
    pub fn probe() -> Self {
        Self {
            spill: Spill::Trampoline(SpillAbi::HOST),
            scan: ScanOptions::FULL,
            seed: 1,
            max_depth: 0,
            check_truth: false,
            sweep_every: 0,
            alloc_gc_per_mille: 0,
        }
    }
}

/// Frame shapes of the stress mutator. Each misses for a different reason if a rule is off.
pub const SHAPES: [&str; 9] = [
    "base", "interior", "loop", "many", "node", "end", "values", "scratch", "garbage",
];
const SH_BASE: u8 = 0;
const SH_INTERIOR: u8 = 1;
const SH_LOOP: u8 = 2;
const SH_MANY: u8 = 3;
const SH_NODE: u8 = 4;
const SH_END: u8 = 5;
const SH_VALUES: u8 = 6;
const SH_SCRATCH: u8 = 7;

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub safepoints: u64,
    pub frames: u64,
    pub max_depth: u32,
    /// Live objects checked, summed over all safepoints.
    pub checked: u64,
    pub max_live: usize,
    pub missed: u64,
    pub missed_by_shape: [u64; SHAPES.len()],
    /// Truth roots by how the scan found them.
    pub roots_spill_only: u64,
    pub roots_stack_only: u64,
    pub roots_spill_and_stack: u64,
    pub roots_precise: u64,
    pub roots_traced_only: u64,
    /// Truth roots by the kind of word that found them.
    pub roots_exact: u64,
    pub roots_interior: u64,
    pub roots_end: u64,
    /// Safepoints by the depth of the frame that last stepped (0..=16).
    pub depth_hist: [u64; 17],
    pub first_misses: Vec<String>,
}

pub struct Mutator {
    pub heap: Heap,
    pub cfg: Config,
    pub stats: Stats,
    rng: u64,
    p_recurse: u32,
    target: u64,
    cur_depth: u32,
    truth: Vec<(u32, u8)>,
    scratch: Vec<usize>,
    stack_high: usize,
    seen: HashSet<usize>,
    todo: Vec<(usize, u8)>,
    asm_spill: Option<SpillAbi>,
    watch: usize,
    /// Mark flags of the watched object at each collection.
    pub watch_log: Vec<u8>,
}

impl Mutator {
    /// Boxed, so heap addresses kept in the mutator state (bump pointer, cage base) never sit
    /// on the scanned stack. Create it on the thread whose stack it scans.
    pub fn new(cfg: Config) -> Box<Self> {
        let stack_high = current_thread_stack_bounds().expect("stack bounds").high;
        let rng = splitmix64(cfg.seed) | 1;
        Box::new(Self {
            heap: Heap::new(),
            cfg,
            stats: Stats::default(),
            rng,
            p_recurse: 25,
            target: u64::MAX,
            cur_depth: 0,
            truth: Vec::new(),
            scratch: Vec::new(),
            stack_high,
            seen: HashSet::new(),
            todo: Vec::new(),
            asm_spill: None,
            watch: 0,
            watch_log: Vec::new(),
        })
    }

    fn next(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u32) -> u32 {
        (((self.next() >> 32) * n as u64) >> 32) as u32
    }

    /// A random object size in words: 2..=32 (16 to 256 bytes).
    fn words(&mut self) -> usize {
        2 + self.below(31) as usize
    }

    /// Records the mark flags of `base` at every following collection.
    pub fn watch(&mut self, base: usize) {
        self.watch = base;
        self.watch_log.clear();
    }

    #[inline(always)]
    pub fn set_asm_spill(&mut self, abi: Option<SpillAbi>) {
        self.asm_spill = abi;
    }

    /// Registers a precise root (the `ScratchRoots` model). Roots pop in LIFO order.
    pub fn push_scratch(&mut self, addr: usize) {
        self.scratch.push(addr);
    }

    pub fn pop_scratch(&mut self) {
        self.scratch.pop();
    }

    /// A safepoint: a call that may collect. Never inlined, so it is a real call.
    #[inline(never)]
    pub fn safepoint(&mut self) {
        match self.cfg.spill {
            Spill::Trampoline(abi) => {
                with_spilled_registers_abi(abi, |r| self.gc(r.sp(), r.spill_area()))
            }
            Spill::NoSpill(abi) => crate::asm::collect_without_spill(self, abi),
        }
    }

    /// Entry from the asm helpers.
    pub fn gc_from_asm(&mut self, sp: usize) {
        let spill = match self.asm_spill {
            Some(abi) => sp..sp + abi.spill_area_bytes(),
            None => sp..sp,
        };
        self.gc(sp, spill);
    }

    fn gc(&mut self, sp: usize, spill: Range<usize>) {
        self.stats.safepoints += 1;
        self.stats.depth_hist[self.cur_depth.min(16) as usize] += 1;
        // SAFETY: `sp` comes from a trampoline or helper on this thread, so `[sp, stack_high)`
        // is the live, 8-aligned part of this thread's stack.
        unsafe {
            self.heap
                .collect(sp, self.stack_high, spill, &self.scratch, self.cfg.scan)
        };
        if self.watch != 0 {
            let f = self.heap.flags_of(self.watch);
            self.watch_log.push(f);
        }
        if self.cfg.check_truth {
            self.check_truth();
        }
        if self.cfg.sweep_every != 0 && self.stats.safepoints.is_multiple_of(self.cfg.sweep_every) {
            self.heap.sweep();
        }
    }

    fn check_truth(&mut self) {
        let s = &mut self.stats;
        self.seen.clear();
        self.todo.clear();
        for &(id, shape) in &self.truth {
            let a = self.heap.addr_of(id);
            let f = self.heap.flags_of(a);
            match (f & F_SPILL != 0, f & F_STACK != 0) {
                _ if f & F_PRECISE != 0 => s.roots_precise += 1,
                (true, false) => s.roots_spill_only += 1,
                (false, true) => s.roots_stack_only += 1,
                (true, true) => s.roots_spill_and_stack += 1,
                (false, false) if f & F_TRACED != 0 => s.roots_traced_only += 1,
                (false, false) => {}
            }
            s.roots_exact += u64::from(f & F_EXACT != 0);
            s.roots_interior += u64::from(f & F_INTERIOR != 0);
            s.roots_end += u64::from(f & F_END != 0);
            self.todo.push((a, shape));
        }
        let mut live = 0;
        while let Some((a, shape)) = self.todo.pop() {
            if !self.seen.insert(a) {
                continue;
            }
            live += 1;
            if self.heap.flags_of(a) == 0 {
                self.stats.missed += 1;
                self.stats.missed_by_shape[shape as usize] += 1;
                if self.stats.first_misses.len() < 8 {
                    self.stats.first_misses.push(format!(
                        "safepoint {}: object at {a:#x} (shape {}) not marked",
                        self.stats.safepoints, SHAPES[shape as usize]
                    ));
                }
                self.heap.rescue(a);
            }
            for c in self.heap.children(a) {
                self.todo.push((c, shape));
            }
        }
        self.stats.checked += live as u64;
        self.stats.max_live = self.stats.max_live.max(live);
    }

    /// Allocation for the mutator. It may collect first, before it touches any heap address.
    #[inline(never)]
    pub fn new_obj(&mut self, words: usize, children: usize) -> *mut u64 {
        if self.below(1000) < self.cfg.alloc_gc_per_mille {
            self.safepoint();
        }
        if let Some(a) = self.heap.alloc_raw(words, children) {
            return a as *mut u64;
        }
        // Heap full: collect and sweep, then retry once.
        let every = std::mem::replace(&mut self.cfg.sweep_every, 1);
        self.safepoint();
        self.cfg.sweep_every = every;
        self.heap
            .alloc_raw(words, children)
            .expect("heap exhausted after a sweep") as *mut u64
    }

    /// Registers `o` in the truth log and returns its id.
    fn hold(&mut self, o: *mut u64, shape: u8) -> u32 {
        // SAFETY: `o` was just allocated.
        let id = unsafe { obj_id(o) };
        self.truth.push((id, shape));
        id
    }

    /// Between two uses: a safepoint, or a deeper random frame with chance
    /// `p_recurse / divisor` percent. No new frames once the target is reached.
    #[inline(never)]
    fn step_with(&mut self, depth: u32, divisor: u32) -> u64 {
        if depth < self.cfg.max_depth
            && self.stats.safepoints < self.target
            && self.below(100 * divisor) < self.p_recurse
        {
            self.frame(depth + 1)
        } else {
            self.cur_depth = depth;
            self.safepoint();
            1
        }
    }

    fn step(&mut self, depth: u32) -> u64 {
        self.step_with(depth, 1)
    }

    /// Runs random frames until `n` safepoints have happened.
    pub fn run(&mut self, n: u64) -> u64 {
        let mut sum = 0u64;
        self.target = n;
        while self.stats.safepoints < n {
            // Vary the tree shape: sometimes shallow, sometimes deep. The mean number of
            // recursive steps per frame stays below 1, so trees stay finite.
            self.p_recurse = 10 + self.below(36);
            sum = sum.wrapping_add(self.frame(0));
            debug_assert!(self.truth.is_empty() && self.scratch.is_empty());
        }
        sum
    }

    #[inline(never)]
    fn frame(&mut self, depth: u32) -> u64 {
        self.stats.frames += 1;
        self.stats.max_depth = self.stats.max_depth.max(depth);
        match self.below(SHAPES.len() as u32) as u8 {
            SH_BASE => self.f_base(depth),
            SH_INTERIOR => self.f_interior(depth),
            SH_LOOP => self.f_loop(depth),
            SH_MANY => self.f_many(depth),
            SH_NODE => self.f_node(depth),
            SH_END => self.f_end(depth),
            SH_VALUES => self.f_values(depth),
            SH_SCRATCH => self.f_scratch(depth),
            _ => self.f_garbage(depth),
        }
    }

    /// S1: three base pointers in locals; the compiler keeps them in callee-saved registers.
    #[inline(never)]
    fn f_base(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let (wa, wb, wc) = (self.words(), self.words(), self.words());
        let a = self.new_obj(wa, 0);
        let ia = self.hold(a, SH_BASE);
        let b = self.new_obj(wb, 0);
        let ib = self.hold(b, SH_BASE);
        let c = self.new_obj(wc, 0);
        let ic = self.hold(c, SH_BASE);
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            // SAFETY: a, b, c are held (in the truth log), so the GC must keep them.
            sum ^= unsafe { verify(a, ia) ^ verify(b, ib) ^ verify(c, ic) };
        }
        self.truth.truncate(mark);
        sum
    }

    /// S2: only an interior pointer to the payload stays live.
    #[inline(never)]
    fn f_interior(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let w = 4 + self.below(29) as usize;
        let o = self.new_obj(w, 0);
        let id = self.hold(o, SH_INTERIOR);
        // SAFETY: `o` is allocated.
        let words = unsafe { obj_words(o) };
        let k = 2 + self.below(words as u32 - 2) as usize;
        // black_box hides `p = o + k` from the optimizer, so `o` is dead from here on and
        // only the interior pointer stays live.
        // SAFETY: `k < words`, inside the object.
        let p = black_box(unsafe { o.add(k) });
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            // SAFETY: the object is held; `p` points to its word `k`.
            let v = unsafe { p.read() };
            assert_eq!(v, payload_word(id, k), "interior word changed");
            sum ^= v;
        }
        // SAFETY: `p - k` is the object base.
        sum ^= unsafe { verify(p.sub(k), id) };
        self.truth.truncate(mark);
        sum
    }

    /// S3: a loop over the payload with a safepoint in every iteration (strength reduction).
    #[inline(never)]
    fn f_loop(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let w = 8 + self.below(25) as usize;
        let o = self.new_obj(w, 0);
        let id = self.hold(o, SH_LOOP);
        // SAFETY: `o` is allocated.
        let words = unsafe { obj_words(o) };
        // A moving pointer, as after strength reduction [R05 S3]. black_box hides its link
        // to `o`, so only the moving (interior) pointer stays live.
        // SAFETY: `o` has 0 children, so the payload starts at word 2.
        let mut p = black_box(unsafe { o.add(2) } as *const u64);
        let mut sum = 0u64;
        for i in 0..words - 2 {
            // Up to 30 steps here, so recurse 8x less often than other frames.
            sum = sum.wrapping_add(self.step_with(depth, 8));
            // SAFETY: `p` = word `i + 2 < words` of the held object.
            let v = unsafe { p.read() };
            assert_eq!(v, payload_word(id, i + 2), "loop word changed");
            sum ^= v;
            // SAFETY: at most one past the end after the last iteration.
            p = unsafe { p.add(1) };
        }
        // SAFETY: after the loop `p` is one past the end; `p - words` is the object.
        sum ^= unsafe { verify(black_box(p).sub(words), id) };
        self.truth.truncate(mark);
        sum
    }

    /// More references than callee-saved registers: most live in stack slots.
    #[inline(never)]
    fn f_many(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let n = 8 + self.below(9) as usize;
        let mut objs = [(std::ptr::null_mut::<u64>(), 0u32); 16];
        for slot in objs.iter_mut().take(n) {
            let w = self.words();
            let o = self.new_obj(w, 0);
            *slot = (o, self.hold(o, SH_MANY));
        }
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            for &(o, id) in objs.iter().take(n) {
                // SAFETY: every object in `objs[..n]` is held.
                sum ^= unsafe { verify(o, id) };
            }
        }
        self.truth.truncate(mark);
        sum
    }

    /// Children reachable only through a held node (precise trace).
    #[inline(never)]
    fn f_node(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let nc = 1 + self.below(MAX_CHILDREN as u32) as usize;
        let extra = self.below(4) as usize;
        let node = self.new_obj(2 + nc + extra, nc);
        let id = self.hold(node, SH_NODE);
        for k in 0..nc {
            let w = self.words();
            let c = self.new_obj(w, 0);
            // SAFETY: `node` has `nc` child slots.
            unsafe { set_child(node, k, c) };
        }
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            // SAFETY: `node` is held; its children are reachable through it.
            unsafe {
                sum ^= verify(node, id);
                for k in 0..nc {
                    sum ^= verify_any(child(node, k));
                }
            }
        }
        self.truth.truncate(mark);
        sum
    }

    /// Only a one-past-end pointer stays live (needs the end rule).
    #[inline(never)]
    fn f_end(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let w = self.words();
        let o = self.new_obj(w, 0);
        let id = self.hold(o, SH_END);
        // SAFETY: `o` is allocated.
        let words = unsafe { obj_words(o) };
        // SAFETY: one past the end of the object is a valid pointer value.
        let end = black_box(unsafe { o.add(words) });
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            // black_box inside the loop stops the compiler from hoisting `end - words`.
            // SAFETY: `end - words` is the held object.
            sum ^= unsafe { verify(black_box(end).sub(words), id) };
        }
        self.truth.truncate(mark);
        sum
    }

    /// NaN-boxed values (S6): cells are raw pointers, numbers never fall in the cage.
    #[inline(never)]
    fn f_values(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let (wa, wb) = (self.words(), self.words());
        let a = self.new_obj(wa, 0);
        let ia = self.hold(a, SH_VALUES);
        let b = self.new_obj(wb, 0);
        let ib = self.hold(b, SH_VALUES);
        let iv = self.next() as i32;
        let dv = f64::from_bits(self.next());
        let vals = [
            value::cell(a as usize),
            value::int32(iv),
            value::double(dv),
            value::cell(b as usize),
        ];
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            let v = black_box(vals);
            assert!(value::is_cell(v[0]) && value::is_cell(v[3]));
            assert_eq!(value::as_int32(v[1]), Some(iv));
            let d = value::as_double(v[2]).unwrap();
            assert!(d.to_bits() == dv.to_bits() || (d.is_nan() && dv.is_nan()));
            // SAFETY: both cells are held objects.
            sum ^= unsafe { verify(v[0] as *const u64, ia) ^ verify(v[3] as *const u64, ib) };
        }
        self.truth.truncate(mark);
        sum
    }

    /// S7 rule: an object held only in a Rust `Vec`, registered as a precise root.
    #[inline(never)]
    fn f_scratch(&mut self, depth: u32) -> u64 {
        let mark = self.truth.len();
        let w = self.words();
        let o = self.new_obj(w, 0);
        let id = self.hold(o, SH_SCRATCH);
        let v: Vec<*mut u64> = vec![o];
        self.scratch.push(o as usize);
        let mut sum = 0u64;
        for _ in 0..1 + self.below(3) {
            sum = sum.wrapping_add(self.step(depth));
            // SAFETY: the object is held and a precise root.
            sum ^= unsafe { verify(v[0], id) };
        }
        self.scratch.pop();
        self.truth.truncate(mark);
        sum
    }

    /// Garbage: objects nobody holds. Sweeps free them.
    #[inline(never)]
    fn f_garbage(&mut self, depth: u32) -> u64 {
        let mut sum = 0u64;
        for _ in 0..1 + self.below(6) {
            let w = self.words();
            let o = self.new_obj(w, 0);
            // SAFETY: just allocated; no safepoint since.
            sum ^= unsafe { verify_any(o) };
        }
        sum.wrapping_add(self.step(depth))
    }
}

/// Runs the stress on a fresh 16 MiB thread and returns the stats and the heap counters.
pub fn stress(cfg: Config, safepoints: u64) -> (Stats, HeapCounters) {
    std::thread::Builder::new()
        .name("sp-gc-stress".into())
        .stack_size(16 << 20)
        .spawn(move || {
            let mut m = Mutator::new(cfg);
            let sum = m.run(safepoints);
            black_box(sum);
            let h = &m.heap;
            let counters = HeapCounters {
                allocated: h.allocated,
                freed: h.freed,
                sweeps: h.sweeps,
                live_at_end: h.live_objects(),
                heap_used: h.used().len(),
                scanned_words: h.scanned_words,
            };
            (m.stats.clone(), counters)
        })
        .expect("spawn the stress thread")
        .join()
        .expect("the stress thread panicked")
}

#[derive(Clone, Debug, Default)]
pub struct HeapCounters {
    pub allocated: u64,
    pub freed: u64,
    pub sweeps: u64,
    pub live_at_end: usize,
    pub heap_used: usize,
    pub scanned_words: u64,
}
