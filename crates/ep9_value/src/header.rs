//! The 8-byte object header word `word0` (H8, 06-object-model §2).
//!
//! ```text
//! bytes 0-3  shape_id  u32  offset into the shape space; IC guards compare it
//! byte  4    gc_state  u8   cell state for the barrier: BLACK=1, WHITE=2, GREY=3 (H11);
//!                           the mark bit is in side metadata (H9); the barrier reads only this byte
//! byte  5    kind      u8   GC visitor and size without loading the shape
//! bytes 6-7  flags     u16  hash-present, pinned, is-prototype, ...
//! ```
//!
//! | gc_state | Value | Meaning (03-gc §3) |
//! |---|---|---|
//! | BLACK | 1 | Old (mark bit stuck) and not remembered. In S2: scanned |
//! | WHITE | 2 | Young: allocated since the last GC. Every allocation writes WHITE |
//! | GREY | 3 | Remembered (S1) or queued for a revisit (S2) |
//!
//! `ep9_heap` owns these values and the barrier threshold (`ep9_heap::barrier`).
//!
//! Each field is its own Relaxed atomic of the field's size. Rust forbids racing atomic
//! accesses of different sizes to overlapping memory (core::sync::atomic, "mixed-size
//! accesses"), so a byte load of `gc_state` must not race with a 64-bit access to the word.
//! Whole-word writes (S3 forwarding) may happen only while no mutator or marker runs.

use core::fmt;
use core::mem::offset_of;
use core::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, Ordering::Relaxed};

/// Offset into the shape space (H8). Stable across runs only for snapshot root shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ShapeId(u32);

impl ShapeId {
    #[inline(always)]
    pub const fn new(offset: u32) -> ShapeId {
        ShapeId(offset)
    }

    #[inline(always)]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Header word at offset 0 of every heap cell. 8 bytes, 8-byte aligned.
#[repr(C, align(8))]
pub struct Word0 {
    shape_id: AtomicU32,
    gc_state: AtomicU8,
    kind: AtomicU8,
    flags: AtomicU16,
}

const _: () = {
    assert!(offset_of!(Word0, shape_id) == Word0::SHAPE_ID_OFFSET);
    assert!(offset_of!(Word0, gc_state) == Word0::GC_STATE_OFFSET);
    assert!(offset_of!(Word0, kind) == Word0::KIND_OFFSET);
    assert!(offset_of!(Word0, flags) == Word0::FLAGS_OFFSET);
};

impl Word0 {
    /// Byte offsets inside the cell. JIT and AOT code use them (`ldrb w, [cell, #4]`).
    pub const SHAPE_ID_OFFSET: usize = 0;
    pub const GC_STATE_OFFSET: usize = 4;
    pub const KIND_OFFSET: usize = 5;
    pub const FLAGS_OFFSET: usize = 6;

    #[inline(always)]
    pub const fn new(shape_id: ShapeId, gc_state: u8, kind: u8, flags: u16) -> Word0 {
        Word0 {
            shape_id: AtomicU32::new(shape_id.0),
            gc_state: AtomicU8::new(gc_state),
            kind: AtomicU8::new(kind),
            flags: AtomicU16::new(flags),
        }
    }

    /// The little-endian u64 that the 8 header bytes form. Snapshot and golden tests use it.
    pub const fn pack(shape_id: ShapeId, gc_state: u8, kind: u8, flags: u16) -> u64 {
        shape_id.0 as u64 | (gc_state as u64) << 32 | (kind as u64) << 40 | (flags as u64) << 48
    }

    #[inline(always)]
    pub fn shape_id(&self) -> ShapeId {
        ShapeId(self.shape_id.load(Relaxed))
    }

    #[inline(always)]
    pub fn set_shape_id(&self, id: ShapeId) {
        self.shape_id.store(id.0, Relaxed);
    }

    #[inline(always)]
    pub fn gc_state(&self) -> u8 {
        self.gc_state.load(Relaxed)
    }

    #[inline(always)]
    pub fn set_gc_state(&self, state: u8) {
        self.gc_state.store(state, Relaxed);
    }

    #[inline(always)]
    pub fn kind(&self) -> u8 {
        self.kind.load(Relaxed)
    }

    #[inline(always)]
    pub fn set_kind(&self, kind: u8) {
        self.kind.store(kind, Relaxed);
    }

    #[inline(always)]
    pub fn flags(&self) -> u16 {
        self.flags.load(Relaxed)
    }

    #[inline(always)]
    pub fn set_flags(&self, flags: u16) {
        self.flags.store(flags, Relaxed);
    }
}

impl fmt::Debug for Word0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Word0")
            .field("shape_id", &self.shape_id().0)
            .field("gc_state", &self.gc_state())
            .field("kind", &self.kind())
            .field("flags", &self.flags())
            .finish()
    }
}
