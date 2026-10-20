//! A tiny non-moving heap of fake objects, a conservative scan of `[sp, stack top)` and a
//! precise trace of object fields.
//!
//! ```text
//! cage (ep9_platform::Cage, 4 GiB) [ heap: HEAP_BYTES, committed 1 MiB at a time | ... ]
//! object (8-byte words):
//!   w0      header = MAGIC << 48 | size_words << 32 | id
//!   w1      child count n (0..=MAX_CHILDREN)
//!   w2..    n child addresses (precise references, or 0)
//!   rest    payload: payload_word(id, i)
//! side tables (one entry per 8-byte granule):
//!   vo      bit set at each object start (interior-pointer lookup)
//!   flags   byte at each marked object start: how the scan found it (F_*)
//! ```

use std::ops::Range;

use ep9_platform::{Cage, CageConfig, load_word};

pub const GRANULE: usize = 8;
pub const MAX_OBJECT_BYTES: usize = 256;
pub const SIZE_CLASSES: [usize; 8] = [16, 24, 32, 48, 64, 96, 128, 256];
pub const MAX_CHILDREN: usize = 4;
pub const HEAP_BYTES: usize = 16 << 20;
const COMMIT_STEP: usize = 1 << 20;
const MAGIC: u64 = 0xE9C0;
/// Freed objects are filled with this word.
pub const ZAP: u64 = 0xDEAD_BEEF_DEAD_BEEF;

/// A word in the trampoline spill area pointed into the object (a register held it).
pub const F_SPILL: u8 = 1;
/// A word in a frame slot pointed into the object.
pub const F_STACK: u8 = 2;
/// Some word was the object base.
pub const F_EXACT: u8 = 4;
/// Some word pointed inside the object, not at its base.
pub const F_INTERIOR: u8 = 8;
/// Some word was base + size (one past the end).
pub const F_END: u8 = 16;
/// A precise root (the ScratchRoots model).
pub const F_PRECISE: u8 = 32;
/// Reached through a child field.
pub const F_TRACED: u8 = 64;
/// A live object the scan missed; the checker kept it alive so the run can go on.
pub const F_RESCUED: u8 = 128;

/// Which conservative rules the scan applies (03-gc section 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    /// Resolve interior pointers through the VO bitmap.
    pub interior: bool,
    /// Also treat a word `w` as `w - 1`, so a one-past-end pointer keeps its object.
    pub one_past_end: bool,
}

impl ScanOptions {
    pub const FULL: Self = Self {
        interior: true,
        one_past_end: true,
    };
}

pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Payload word `i` of object `id`.
pub fn payload_word(id: u32, i: usize) -> u64 {
    splitmix64(((id as u64) << 8) | i as u64)
}

/// Header fields of the object at `base`.
///
/// # Safety
/// `base` is the start of an allocated object.
pub unsafe fn header(base: usize) -> (u32, usize, bool) {
    // SAFETY: the caller guarantees `base` is an allocated object, so word 0 is initialized.
    let h = unsafe { (base as *const u64).read() };
    (h as u32, ((h >> 32) & 0xFFFF) as usize, h >> 48 == MAGIC)
}

/// Checks an object the mutator holds and returns a checksum. Panics if the object was freed
/// (zapped) or reused by another object: that would be a missed live object.
///
/// # Safety
/// `o` points to memory inside the committed heap.
pub unsafe fn verify(o: *const u64, id: u32) -> u64 {
    // SAFETY: the caller guarantees `o` is inside the committed heap.
    let (hid, words, ok) = unsafe { header(o as usize) };
    assert!(
        ok && hid == id,
        "object {id} at {o:p}: header lost (freed or reused, id {hid})"
    );
    // SAFETY: as above; word 1 and the payload are inside the object.
    let n = unsafe { o.add(1).read() } as usize;
    let mut sum = 0u64;
    for i in 2 + n..words {
        // SAFETY: `i < words`, inside the object.
        let w = unsafe { o.add(i).read() };
        assert_eq!(
            w,
            payload_word(id, i),
            "object {id} at {o:p}: payload word {i} changed"
        );
        sum = sum.wrapping_add(w);
    }
    sum
}

/// Like [`verify`] for an object whose id the caller does not know.
///
/// # Safety
/// As for [`verify`].
pub unsafe fn verify_any(o: *const u64) -> u64 {
    // SAFETY: forwarded caller contract.
    unsafe {
        let (id, _, _) = header(o as usize);
        verify(o, id)
    }
}

/// The id stored in an object header.
///
/// # Safety
/// `o` is an allocated object.
pub unsafe fn obj_id(o: *const u64) -> u32 {
    // SAFETY: forwarded caller contract.
    unsafe { header(o as usize).0 }
}

/// Size of an object in words.
///
/// # Safety
/// `o` is an allocated object.
pub unsafe fn obj_words(o: *const u64) -> usize {
    // SAFETY: forwarded caller contract.
    unsafe { header(o as usize).1 }
}

/// Stores child `k` of `parent`.
///
/// # Safety
/// `parent` is an allocated object with more than `k` child slots.
pub unsafe fn set_child(parent: *mut u64, k: usize, child: *mut u64) {
    // SAFETY: the caller guarantees slot `k` exists.
    unsafe { parent.add(2 + k).write(child as u64) }
}

/// Child `k` of `parent`.
///
/// # Safety
/// `parent` is an allocated object with more than `k` child slots.
pub unsafe fn child(parent: *const u64, k: usize) -> *mut u64 {
    // SAFETY: the caller guarantees slot `k` exists.
    unsafe { parent.add(2 + k).read() as *mut u64 }
}

pub struct Heap {
    cage: Cage,
    start: usize,
    committed: usize,
    bump: usize,
    vo: Vec<u64>,
    flags: Vec<u8>,
    marked: Vec<usize>,
    work: Vec<usize>,
    free: [Vec<usize>; SIZE_CLASSES.len()],
    addr_of: Vec<usize>,
    live: Vec<u32>,
    pub allocated: u64,
    pub freed: u64,
    pub sweeps: u64,
    pub scanned_words: u64,
    /// When true, `collect` logs every hit as (slot address, word, object base).
    pub log_hits: bool,
    pub hits: Vec<(usize, usize, usize)>,
}

impl Heap {
    pub fn new() -> Self {
        let config = CageConfig {
            cage_size: 4 << 30,
            ..CageConfig::default()
        };
        let cage = Cage::reserve(&config).expect("reserve the spike cage");
        let start = cage.heap().base();
        let granules = HEAP_BYTES / GRANULE;
        Self {
            cage,
            start,
            committed: start,
            bump: start,
            vo: vec![0; granules / 64],
            flags: vec![0; granules],
            marked: Vec::new(),
            work: Vec::new(),
            free: Default::default(),
            addr_of: Vec::new(),
            live: Vec::new(),
            allocated: 0,
            freed: 0,
            sweeps: 0,
            scanned_words: 0,
            log_hits: false,
            hits: Vec::new(),
        }
    }

    pub fn cage(&self) -> &Cage {
        &self.cage
    }

    /// `[start, bump)`: every object lies here.
    pub fn used(&self) -> Range<usize> {
        self.start..self.bump
    }

    pub fn live_objects(&self) -> usize {
        self.live.len()
    }

    fn granule(&self, addr: usize) -> usize {
        (addr - self.start) / GRANULE
    }

    fn vo_bit(&self, g: usize) -> bool {
        self.vo[g / 64] & (1 << (g % 64)) != 0
    }

    fn set_vo(&mut self, g: usize, on: bool) {
        if on {
            self.vo[g / 64] |= 1 << (g % 64);
        } else {
            self.vo[g / 64] &= !(1 << (g % 64));
        }
    }

    /// Allocates an object of at least `words` words with `children` child slots.
    /// Returns `None` when the heap is full.
    pub fn alloc_raw(&mut self, words: usize, children: usize) -> Option<usize> {
        assert!(children <= MAX_CHILDREN && words >= 2 + children);
        let class = SIZE_CLASSES.iter().position(|&c| c >= words * GRANULE)?;
        let size = SIZE_CLASSES[class];
        let addr = match self.free[class].pop() {
            Some(a) => a,
            None => {
                if self.bump + size > self.start + HEAP_BYTES {
                    return None;
                }
                while self.bump + size > self.committed {
                    self.cage
                        .heap()
                        .commit(self.committed - self.start, COMMIT_STEP)
                        .ok()?;
                    self.committed += COMMIT_STEP;
                }
                let a = self.bump;
                self.bump += size;
                a
            }
        };
        let id = u32::try_from(self.addr_of.len()).expect("more than 2^32 objects");
        self.addr_of.push(addr);
        self.live.push(id);
        let w = size / GRANULE;
        let p = addr as *mut u64;
        // SAFETY: `[addr, addr + size)` is committed heap memory that no live object uses
        // (fresh bump space or a swept cell).
        unsafe {
            p.write((MAGIC << 48) | ((w as u64) << 32) | id as u64);
            p.add(1).write(children as u64);
            for k in 0..children {
                p.add(2 + k).write(0);
            }
            for i in 2 + children..w {
                p.add(i).write(payload_word(id, i));
            }
        }
        let g = self.granule(addr);
        self.set_vo(g, true);
        self.allocated += 1;
        Some(addr)
    }

    pub fn addr_of(&self, id: u32) -> usize {
        self.addr_of[id as usize]
    }

    /// The object that contains `addr`, if any.
    pub fn find(&self, addr: usize, interior: bool) -> Option<usize> {
        if addr < self.start || addr >= self.bump {
            return None;
        }
        let g = self.granule(addr);
        if !interior {
            return (addr.is_multiple_of(GRANULE) && self.vo_bit(g)).then_some(addr);
        }
        // The nearest object start at or below `addr`, at most one object size back.
        let lo = g.saturating_sub(MAX_OBJECT_BYTES / GRANULE - 1);
        let s = (lo..=g).rev().find(|&s| self.vo_bit(s))?;
        let base = self.start + s * GRANULE;
        // SAFETY: the VO bit says an allocated object starts at `base`.
        let (_, words, _) = unsafe { header(base) };
        (addr < base + words * GRANULE).then_some(base)
    }

    /// Mark flags of the object starting at `base`; 0 if not marked or not an object start.
    pub fn flags_of(&self, base: usize) -> u8 {
        if base < self.start || base >= self.bump || !base.is_multiple_of(GRANULE) {
            return 0;
        }
        let g = self.granule(base);
        if self.vo_bit(g) { self.flags[g] } else { 0 }
    }

    fn mark(&mut self, base: usize, f: u8) {
        let g = self.granule(base);
        if self.flags[g] == 0 {
            self.marked.push(base);
            self.work.push(base);
        }
        self.flags[g] |= f;
    }

    /// Keeps a missed live object alive (no trace: the checker walks children itself).
    pub fn rescue(&mut self, base: usize) {
        let g = self.granule(base);
        if self.flags[g] == 0 {
            self.marked.push(base);
        }
        self.flags[g] |= F_RESCUED;
    }

    fn scan_word(&mut self, w: usize, src: u8, opt: ScanOptions) {
        if let Some(b) = self.find(w, opt.interior) {
            let how = if b == w { F_EXACT } else { F_INTERIOR };
            self.mark(b, src | how);
        }
        if opt.one_past_end
            && let Some(b) = self.find(w.wrapping_sub(1), true)
        {
            // SAFETY: `find` returned an allocated object start.
            let (_, words, _) = unsafe { header(b) };
            if b + words * GRANULE == w {
                self.mark(b, src | F_END);
            }
        }
    }

    /// One collection without the sweep: clear marks, scan `[sp, high)` conservatively, mark
    /// the precise roots, trace children.
    ///
    /// # Safety
    /// `[sp, high)` is readable stack memory of the current thread and `sp` is 8-aligned.
    pub unsafe fn collect(
        &mut self,
        sp: usize,
        high: usize,
        spill: Range<usize>,
        precise: &[usize],
        opt: ScanOptions,
    ) {
        self.hits.clear();

        for b in self.marked.drain(..) {
            let g = (b - self.start) / GRANULE;
            self.flags[g] = 0;
        }
        let mut p = sp;
        while p < high {
            // SAFETY: the caller guarantees `[sp, high)` is readable and aligned.
            let w = unsafe { load_word(p) };
            if self.cage.contains(w) {
                let src = if spill.contains(&p) { F_SPILL } else { F_STACK };
                self.scan_word(w, src, opt);
                if self.log_hits
                    && let Some(b) = self.find(w.wrapping_sub(1), true).or(self.find(w, true))
                {
                    self.hits.push((p, w, b));
                }
            }
            p += 8;
        }
        self.scanned_words += ((high - sp) / 8) as u64;
        for &r in precise {
            if let Some(b) = self.find(r, false) {
                self.mark(b, F_PRECISE | F_EXACT);
            }
        }
        while let Some(b) = self.work.pop() {
            // SAFETY: only allocated object starts are pushed to `work`.
            let n = unsafe { (b as *const u64).add(1).read() } as usize;
            for k in 0..n {
                // SAFETY: `k < n`, the child count of this object.
                let c = unsafe { child(b as *const u64, k) } as usize;
                if c != 0 {
                    self.mark(c, F_TRACED);
                }
            }
        }
    }

    /// Children of an allocated object.
    pub fn children(&self, base: usize) -> impl Iterator<Item = usize> {
        // SAFETY: callers pass allocated object starts (truth ids or traced children).
        let n = unsafe { (base as *const u64).add(1).read() } as usize;
        (0..n)
            // SAFETY: `k < n`, the child count.
            .map(move |k| unsafe { child(base as *const u64, k) } as usize)
            .filter(|&c| c != 0)
    }

    /// Frees every object the last `collect` did not mark: zap, clear its VO bit, reuse later.
    pub fn sweep(&mut self) {
        self.sweeps += 1;
        let mut i = 0;
        while i < self.live.len() {
            let id = self.live[i];
            let a = self.addr_of[id as usize];
            let g = self.granule(a);
            if self.flags[g] != 0 {
                i += 1;
                continue;
            }
            // SAFETY: `a` is an allocated object that no root reaches.
            let (_, words, _) = unsafe { header(a) };
            for k in 0..words {
                // SAFETY: inside the dead object.
                unsafe { (a as *mut u64).add(k).write(ZAP) };
            }
            self.set_vo(g, false);
            let class = SIZE_CLASSES
                .iter()
                .position(|&c| c == words * GRANULE)
                .unwrap();
            self.free[class].push(a);
            self.addr_of[id as usize] = 0;
            self.live.swap_remove(i);
            self.freed += 1;
        }
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interior_lookup_and_sweep() {
        let mut h = Heap::new();
        let a = h.alloc_raw(4, 0).unwrap(); // 32 B
        let b = h.alloc_raw(2, 0).unwrap(); // 16 B
        assert_eq!(b, a + 32);
        assert_eq!(h.find(a, false), Some(a));
        assert_eq!(h.find(a + 8, false), None);
        assert_eq!(h.find(a + 31, true), Some(a));
        assert_eq!(h.find(b, true), Some(b));
        assert_eq!(h.find(b + 16, true), None, "past bump");
        // A one-past-end word of `a` is the base of `b`: with the end rule both are marked.
        let words = [b];
        // SAFETY: `words` is a readable, aligned local array.
        unsafe {
            let p = words.as_ptr() as usize;
            h.collect(p, p + 8, 0..0, &[], ScanOptions::FULL)
        };
        assert_eq!(h.flags_of(b) & (F_EXACT | F_STACK), F_EXACT | F_STACK);
        assert_eq!(h.flags_of(a) & F_END, F_END);
        // Without the end rule `a` is not marked; sweep frees and zaps it, then reuses it.
        // SAFETY: as above.
        unsafe {
            let p = words.as_ptr() as usize;
            h.collect(
                p,
                p + 8,
                0..0,
                &[],
                ScanOptions {
                    interior: true,
                    one_past_end: false,
                },
            )
        };
        assert_eq!(h.flags_of(a), 0);
        h.sweep();
        // SAFETY: `a` is inside the committed heap.
        assert_eq!(unsafe { (a as *const u64).read() }, ZAP);
        assert_eq!(h.find(a + 8, true), None, "freed memory is not an object");
        assert_eq!(
            h.alloc_raw(4, 0),
            Some(a),
            "a freed cell of the same class is reused"
        );
    }
}
