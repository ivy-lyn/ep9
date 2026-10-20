//! Dispatch slots and the pc→code registry (H14).
//!
//! ```text
//! call f --> FunctionInfo(f).slot = { kind, entry }     kind: interp | baseline | opt | aot-baseline | aot-opt
//!            tier-up, unit load, deopt-all, PIN ==> write one slot. Never patch a call site (H14, H16).
//!
//! pc --> CodeRegistry::lookup(pc) --> CodeEntry { kind, range, function }
//!        users: unwinding, Error.stack, profiler, debugger, deopt (04 §9)
//! ```
//!
//! v0 uses only [`DispatchKind::Interp`]: the slot entry is the bytecode start. The other kinds
//! exist so that tiers and the unit loader write the same slot later.

use core::cell::Cell;
use core::ops::Range;
use std::collections::BTreeMap;

/// What a dispatch slot runs (H14). The byte values are part of the AOT unit contract (H35).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DispatchKind {
    /// T0 interpreter; entry = bytecode start.
    Interp = 0,
    /// T1 baseline JIT code.
    Baseline = 1,
    /// T2/T3 optimized JIT code.
    Opt = 2,
    /// Baseline-AOT code from a loaded unit (A3).
    AotBaseline = 3,
    /// Optimized-AOT code from a loaded unit (A4).
    AotOpt = 4,
}

impl DispatchKind {
    pub const ALL: [DispatchKind; 5] = [
        DispatchKind::Interp,
        DispatchKind::Baseline,
        DispatchKind::Opt,
        DispatchKind::AotBaseline,
        DispatchKind::AotOpt,
    ];

    pub const fn from_u8(b: u8) -> Option<DispatchKind> {
        match b {
            0 => Some(DispatchKind::Interp),
            1 => Some(DispatchKind::Baseline),
            2 => Some(DispatchKind::Opt),
            3 => Some(DispatchKind::AotBaseline),
            4 => Some(DispatchKind::AotOpt),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            DispatchKind::Interp => "interp",
            DispatchKind::Baseline => "baseline",
            DispatchKind::Opt => "opt",
            DispatchKind::AotBaseline => "aot-baseline",
            DispatchKind::AotOpt => "aot-opt",
        }
    }

    /// True for code that a unit loader installs (H35).
    pub const fn is_aot(self) -> bool {
        matches!(self, DispatchKind::AotBaseline | DispatchKind::AotOpt)
    }
}

/// Kind byte of a slot that has no code (host functions, VM stubs). Calls take the slow path.
pub const NO_CODE: u8 = 0xFF;

/// One dispatch slot: `{ kind, entry }`. A call reads it; a tier change writes it. `repr(C)`:
/// the interpreter call path reads `kind` at offset 0 and `entry` at offset 8.
#[derive(Debug)]
#[repr(C)]
pub struct DispatchSlot {
    kind: Cell<u8>,
    entry: Cell<usize>,
}

impl DispatchSlot {
    pub const KIND_OFFSET: usize = 0;
    pub const ENTRY_OFFSET: usize = 8;

    pub const fn new(kind: DispatchKind, entry: usize) -> DispatchSlot {
        DispatchSlot {
            kind: Cell::new(kind as u8),
            entry: Cell::new(entry),
        }
    }

    /// A slot with no code ([`NO_CODE`]).
    pub const fn empty() -> DispatchSlot {
        DispatchSlot {
            kind: Cell::new(NO_CODE),
            entry: Cell::new(0),
        }
    }

    pub fn kind(&self) -> Option<DispatchKind> {
        DispatchKind::from_u8(self.kind.get())
    }

    #[inline(always)]
    pub fn kind_byte(&self) -> u8 {
        self.kind.get()
    }

    #[inline(always)]
    pub fn entry(&self) -> usize {
        self.entry.get()
    }

    /// Installs new code: one slot write, no call-site patch (H14).
    pub fn install(&self, kind: DispatchKind, entry: usize) {
        self.entry.set(entry);
        self.kind.set(kind as u8);
    }
}

const _: () = {
    assert!(core::mem::offset_of!(DispatchSlot, kind) == DispatchSlot::KIND_OFFSET);
    assert!(core::mem::offset_of!(DispatchSlot, entry) == DispatchSlot::ENTRY_OFFSET);
};

/// What the registry knows about one code range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeEntry {
    /// The code kind of the range.
    pub kind: DispatchKind,
    /// First byte.
    pub start: usize,
    /// Length in bytes (at least 1).
    pub len: usize,
    /// Address of the owning `FunctionInfo` (an opaque id outside `ep9_vm`).
    pub function: usize,
}

impl CodeEntry {
    pub fn range(&self) -> Range<usize> {
        self.start..self.start + self.len
    }

    pub fn contains(&self, pc: usize) -> bool {
        self.range().contains(&pc)
    }
}

/// Why a registration failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// The range is empty.
    Empty,
    /// The range overlaps an entry that starts here.
    Overlap(usize),
}

/// The pc→code registry (H14): maps a pc to the code range that holds it. JIT code and unit
/// ranges register here; bytecode ranges do too (T0 frames). Users: the unwinder, Error.stack,
/// the profiler, the debugger and the deoptimizer.
pub trait CodeRegistry {
    /// Adds `entry`. Ranges never overlap.
    fn register(&mut self, entry: CodeEntry) -> Result<(), RegistryError>;
    /// Removes the entry that starts at `start`.
    fn unregister(&mut self, start: usize) -> Option<CodeEntry>;
    /// The entry whose range holds `pc`.
    fn lookup(&self, pc: usize) -> Option<CodeEntry>;
    /// Number of entries.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A [`CodeRegistry`] over a sorted map (O(log n) lookup). Enough for v0 and tests; a lock-free
/// reader for the sampling profiler is H24 work.
#[derive(Debug, Default)]
pub struct RangeRegistry {
    by_start: BTreeMap<usize, CodeEntry>,
}

impl RangeRegistry {
    pub fn new() -> RangeRegistry {
        RangeRegistry::default()
    }
}

impl CodeRegistry for RangeRegistry {
    fn register(&mut self, entry: CodeEntry) -> Result<(), RegistryError> {
        if entry.len == 0 {
            return Err(RegistryError::Empty);
        }
        let end = entry.start + entry.len;
        if let Some((&s, e)) = self.by_start.range(..end).next_back()
            && s + e.len > entry.start
        {
            return Err(RegistryError::Overlap(s));
        }
        self.by_start.insert(entry.start, entry);
        Ok(())
    }

    fn unregister(&mut self, start: usize) -> Option<CodeEntry> {
        self.by_start.remove(&start)
    }

    fn lookup(&self, pc: usize) -> Option<CodeEntry> {
        let (_, e) = self.by_start.range(..=pc).next_back()?;
        e.contains(pc).then_some(*e)
    }

    fn len(&self) -> usize {
        self.by_start.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(start: usize, len: usize, f: usize) -> CodeEntry {
        CodeEntry {
            kind: DispatchKind::Interp,
            start,
            len,
            function: f,
        }
    }

    #[test]
    fn kinds_round_trip() {
        for k in DispatchKind::ALL {
            assert_eq!(DispatchKind::from_u8(k as u8), Some(k));
        }
        assert_eq!(DispatchKind::from_u8(NO_CODE), None);
        assert!(DispatchKind::AotOpt.is_aot() && !DispatchKind::Opt.is_aot());
        assert_eq!(DispatchKind::AotBaseline.name(), "aot-baseline");
    }

    #[test]
    fn slot_install_writes_one_slot() {
        let s = DispatchSlot::new(DispatchKind::Interp, 0x1000);
        assert_eq!(s.kind(), Some(DispatchKind::Interp));
        s.install(DispatchKind::Baseline, 0x2000);
        assert_eq!(
            (s.kind(), s.entry()),
            (Some(DispatchKind::Baseline), 0x2000)
        );
        assert_eq!(DispatchSlot::empty().kind(), None);
    }

    #[test]
    fn registry_lookup_and_overlap() {
        let mut r = RangeRegistry::new();
        assert!(r.is_empty());
        r.register(e(100, 10, 1)).unwrap();
        r.register(e(110, 5, 2)).unwrap();
        r.register(e(200, 1, 3)).unwrap();
        assert_eq!(r.register(e(105, 2, 4)), Err(RegistryError::Overlap(100)));
        assert_eq!(r.register(e(90, 11, 4)), Err(RegistryError::Overlap(100)));
        assert_eq!(r.register(e(300, 0, 4)), Err(RegistryError::Empty));
        assert_eq!(r.lookup(99), None);
        assert_eq!(r.lookup(100).map(|e| e.function), Some(1));
        assert_eq!(r.lookup(109).map(|e| e.function), Some(1));
        assert_eq!(r.lookup(110).map(|e| e.function), Some(2));
        assert_eq!(r.lookup(115), None);
        assert_eq!(r.lookup(200).map(|e| e.function), Some(3));
        assert_eq!(r.len(), 3);
        assert_eq!(r.unregister(110).map(|e| e.function), Some(2));
        assert_eq!(r.lookup(112), None);
    }
}
