#![allow(dead_code)] // m5-stub
//! The fuse registry v0 (owner M5-P1 FR0, CONSTREP appends descriptor kind 3; H31, MC-w16-15,
//! MC-w15-9, V10-D12): named facts the compiled code depends on. A fuse is intact until a
//! change pops it; a pop invalidates every dependent code handle.
//!
//! ```text
//!  FuseRef::Static(i)       a builtin fuse, by stable index (H31:19)
//!  FuseRef::Dynamic(d)      a fuse named by a descriptor:
//!    kind 1 PROTO_CHAIN       prototype-chain validity by ProtoRef (refuses a proxy chain)
//!    kind 2 CONSTRUCTOR_CHAIN constructor chain by FunctionRef
//!    kind 3                   field constness and representation (CONSTREP appends it)
//!  Registry (Runtime::fuses): per isolate, the state of each fuse and its weak dependents
//!  hook: shape::note_change on a prototype -> on_proto_change (FR0 does not edit shape.rs)
//! ```
//!
//! The FUSE kind id 83 is reserved (OBJECT_LAYOUT 5) in case FR0 needs a heap cell. M5-P0 stub:
//! the registry is empty, every fuse reads intact, and the hook does nothing.

/// A descriptor kind (H31 Dynamic descriptor kinds; append-only).
pub(crate) mod descriptor {
    /// Prototype-chain validity by ProtoRef (MC-w16-15).
    pub const PROTO_CHAIN: u8 = 1;
    /// The constructor chain by FunctionRef (MC-w15-9, V10-D12).
    pub const CONSTRUCTOR_CHAIN: u8 = 2;
}

/// A dynamic fuse name: a descriptor kind and its subject (a ProtoRef or FunctionRef word).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Descriptor {
    pub kind: u8,
    pub subject: u64,
}

/// The name of a fuse (H31).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FuseRef {
    /// A builtin fuse by stable index.
    Static(u32),
    /// A fuse named by a descriptor.
    Dynamic(Descriptor),
}

/// The per-isolate fuse registry (`Runtime::fuses`): fuse states and the weak dependents
/// lists. Rust memory; FR0 decides how dependents are held (weak code handles).
pub(crate) struct Registry {
    _private: (),
}

impl Registry {
    /// The registry of a new isolate.
    pub(crate) fn new() -> Registry {
        Registry { _private: () }
    }

    /// True if no fuse has a dependent (M5-P0: always).
    pub(crate) fn is_empty(&self) -> bool {
        true
    }

    /// True if `f` is intact (the GuardFuse read). M5-P0: every fuse is intact.
    pub(crate) fn intact(&self, _f: FuseRef) -> bool {
        true
    }

    /// Pops `f`: marks it broken and invalidates every dependent. M5-P0: nothing depends on a
    /// fuse.
    pub(crate) fn pop(&self, _f: FuseRef) {}
}

/// The hook of a structural change of the prototype object `o` (`shape::note_change`, after
/// the validity cells are invalidated). M5-P0 stub: nothing to pop. Not a GC point.
#[inline(always)]
pub(crate) fn on_proto_change(_rt: &crate::runtime::Runtime, _o: u64) {}
