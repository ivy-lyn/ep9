#![allow(dead_code)] // m4-stub
//! The megamorphic stub cache (owner M4-P3 IC2; OI-09): a per-isolate table from (receiver
//! shape, name) to a handler, probed by the megamorphic branch of the named-property hit paths
//! (`ic::load`, `ic::store`). It holds shapes weakly (H16 K7) and is flushed at every GC.
//!
//! The isolate's table is `Runtime::stub_cache` ([`StubCache`], made by [`StubCache::new`] in
//! `Runtime::new`; `runtime.rs` has no owner after M4-P0, so the owner changes only this
//! file). M4-P0 stubs: the table is empty, every probe misses, so a megamorphic site takes the
//! generic path as before.

use ep9_heap::Heap;

use crate::runtime::Runtime;
use crate::unit::FunctionInfo;

/// The isolate's stub cache. Holds no Value across a GC point (plan rule 8): the owner keeps
/// its entries weak or flushes them at each GC (for example from a weak source registered in
/// [`StubCache::new`]).
pub(crate) struct StubCache {
    _private: (),
}

impl StubCache {
    /// The empty cache of a new isolate. `heap` is the isolate's heap, before its first
    /// allocation (the place to register a GC hook).
    pub(crate) fn new(_heap: &Heap) -> StubCache {
        StubCache { _private: () }
    }
}

/// The megamorphic load `o.name` (`name` is constant index `name` of `fi`): the value, or `None`
/// on a miss. Called only from the megamorphic branch of `ic::load`; never allocates, never
/// calls.
#[inline(always)]
pub(crate) fn probe_load(_rt: &Runtime, _fi: &FunctionInfo, _o: u64, _name: u32) -> Option<u64> {
    None
}

/// The megamorphic store `o.name = v` (`define`: CreateDataProperty, 0x25); `true` if it
/// stored. Called only from the megamorphic branch of `ic::store`; never allocates, never
/// calls.
#[inline(always)]
pub(crate) fn probe_store(
    _rt: &Runtime,
    _fi: &FunctionInfo,
    _o: u64,
    _name: u32,
    _v: u64,
    _define: bool,
) -> bool {
    false
}
