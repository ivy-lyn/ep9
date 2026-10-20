//! Precise root sources (H10, 03-gc §2).
//!
//! | Source | API | Strength | Who uses it |
//! |---|---|---|---|
//! | Handle table | [`crate::Heap::new_global`], [`crate::Heap::new_weak`] | strong / weak | `Global<T>` in ep9_vm, embedders |
//! | Scratch roots | [`crate::Heap::scratch_roots`] -> [`ScratchRoots`] | strong, scoped (LIFO) | Rust temporary lists (probe S7) |
//! | Root sources | [`crate::Heap::add_root_source`] | strong | VM register stack, realm tables, embedder roots |
//! | Root ranges | [`crate::Heap::add_root_range`] | strong or weak | unit RW data: link cells strong, IC cells weak (H22) |
//! | Weak sources | [`crate::Heap::add_weak_source`] | weak | weak tables outside the heap |
//!
//! - Every source can be visited more than once per GC (the S2 fixpoint rescans all of them).
//! - Stores into roots take no barrier (elision E3).

use core::ptr::NonNull;

use ep9_value::Value;

use crate::barrier::Slot;
use crate::trace::{Context, Tracer, WeakVisitor};

/// A strong handle-table entry. Copy; release it with [`crate::Heap::drop_global`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobalHandle(pub(crate) u32);

/// A weak handle-table entry. Its value reads as `None` once its cell died.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WeakHandle(pub(crate) u32);

/// The id of a registered root source, range or weak source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RootId(pub(crate) u32);

/// Strength of a root range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootStrength {
    Strong,
    /// Cleared to `undefined` at GC end when the cell dies (unit IC cells).
    Weak,
}

#[derive(Clone, Copy, Debug)]
enum HandleSlot {
    Free,
    Strong(u64),
    /// Bits 0 once cleared (0 is never a `Value`).
    Weak(u64),
}

#[derive(Default)]
pub(crate) struct Handles {
    slots: Vec<HandleSlot>,
    free: Vec<u32>,
}

impl Handles {
    fn alloc(&mut self, s: HandleSlot) -> u32 {
        match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = s;
                i
            }
            None => {
                self.slots.push(s);
                u32::try_from(self.slots.len() - 1).expect("more than 2^32 handles")
            }
        }
    }

    pub(crate) fn new_global(&mut self, v: Value) -> GlobalHandle {
        GlobalHandle(self.alloc(HandleSlot::Strong(v.to_bits())))
    }

    pub(crate) fn new_weak(&mut self, v: Value) -> WeakHandle {
        WeakHandle(self.alloc(HandleSlot::Weak(v.to_bits())))
    }

    pub(crate) fn global(&self, h: GlobalHandle) -> Value {
        match self.slots.get(h.0 as usize) {
            // SAFETY: strong handle slots hold bits from `Value::to_bits`, and the GC keeps a
            // cell held by a strong handle alive.
            Some(HandleSlot::Strong(b)) => unsafe { Value::from_bits_unchecked(*b) },
            _ => panic!("{h:?} is not a live global handle"),
        }
    }

    pub(crate) fn set_global(&mut self, h: GlobalHandle, v: Value) {
        match self.slots.get_mut(h.0 as usize) {
            Some(HandleSlot::Strong(b)) => *b = v.to_bits(),
            _ => panic!("{h:?} is not a live global handle"),
        }
    }

    pub(crate) fn weak(&self, h: WeakHandle) -> Option<Value> {
        match self.slots.get(h.0 as usize) {
            Some(HandleSlot::Weak(0)) => None,
            // SAFETY: weak slots hold bits from `Value::to_bits` until the GC clears them to 0
            // when their cell dies, so a non-zero cell value is live.
            Some(HandleSlot::Weak(b)) => Some(unsafe { Value::from_bits_unchecked(*b) }),
            _ => panic!("{h:?} is not a live weak handle"),
        }
    }

    pub(crate) fn release(&mut self, i: u32, weak: bool) {
        let ok = matches!(
            (self.slots.get(i as usize), weak),
            (Some(HandleSlot::Strong(_)), false) | (Some(HandleSlot::Weak(_)), true)
        );
        assert!(ok, "handle {i} is not live (double release?)");
        self.slots[i as usize] = HandleSlot::Free;
        self.free.push(i);
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    pub(crate) fn visit_strong(&self, t: &mut Tracer<'_>) {
        t.context = Context::Root("global handle");
        for s in &self.slots {
            if let HandleSlot::Strong(b) = s {
                t.visit_bits(*b);
            }
        }
    }

    /// Verifier: weak handles must hold valid values too.
    pub(crate) fn verify_weak(&self, t: &mut Tracer<'_>) {
        t.context = Context::Root("weak handle");
        for s in &self.slots {
            if let HandleSlot::Weak(b) = s
                && *b != 0
            {
                t.visit_bits(*b);
            }
        }
    }

    pub(crate) fn clear_dead(&mut self, w: &mut WeakVisitor<'_>) {
        for s in &mut self.slots {
            if let HandleSlot::Weak(b) = s
                && *b != 0
            {
                // SAFETY: a non-zero weak slot holds bits from `Value::to_bits`.
                let v = unsafe { Value::from_bits_unchecked(*b) };
                if !w.is_live(v) {
                    *b = 0;
                    w.cleared += 1;
                }
            }
        }
    }
}

type SourceFn = Box<dyn FnMut(&mut Tracer<'_>)>;
type WeakSourceFn = Box<dyn FnMut(&mut WeakVisitor<'_>)>;

enum Entry {
    Source(SourceFn),
    WeakSource(WeakSourceFn),
    Range {
        slots: NonNull<[Slot]>,
        strength: RootStrength,
    },
}

#[derive(Default)]
pub(crate) struct Roots {
    entries: Vec<(u32, Entry)>,
    next: u32,
}

impl Roots {
    fn add(&mut self, e: Entry) -> RootId {
        let id = self.next;
        self.next = self.next.checked_add(1).expect("root id overflow");
        self.entries.push((id, e));
        RootId(id)
    }

    pub(crate) fn add_source(&mut self, f: SourceFn) -> RootId {
        self.add(Entry::Source(f))
    }

    pub(crate) fn add_weak_source(&mut self, f: WeakSourceFn) -> RootId {
        self.add(Entry::WeakSource(f))
    }

    pub(crate) fn add_range(&mut self, slots: NonNull<[Slot]>, strength: RootStrength) -> RootId {
        self.add(Entry::Range { slots, strength })
    }

    pub(crate) fn remove(&mut self, id: RootId) -> bool {
        match self.entries.iter().position(|(i, _)| *i == id.0) {
            Some(p) => {
                self.entries.remove(p);
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Strong ranges and strong sources. In verify mode, weak ranges too.
    pub(crate) fn visit_strong(&mut self, t: &mut Tracer<'_>, verify: bool) {
        for (_, e) in &mut self.entries {
            match e {
                Entry::Source(f) => {
                    t.context = Context::Root("root source");
                    f(t);
                }
                Entry::Range { slots, strength } => {
                    if *strength == RootStrength::Strong || verify {
                        t.context = Context::Root("root range");
                        // SAFETY: `add_root_range`'s contract keeps the slots valid and
                        // initialized until the range is removed. Slots are atomics, so a
                        // shared reference is fine while the owner also writes them.
                        t.visit_slots(unsafe { slots.as_ref() });
                    }
                }
                Entry::WeakSource(_) => {}
            }
        }
    }

    pub(crate) fn visit_weak(&mut self, w: &mut WeakVisitor<'_>) {
        for (_, e) in &mut self.entries {
            match e {
                Entry::WeakSource(f) => f(w),
                Entry::Range {
                    slots,
                    strength: RootStrength::Weak,
                } => {
                    // SAFETY: as in `visit_strong`.
                    for s in unsafe { slots.as_ref() } {
                        w.visit(s);
                    }
                }
                _ => {}
            }
        }
    }
}

/// A scope of precise roots for Rust temporaries (03-gc §2 `ScratchRoots`).
///
/// Values pushed here stay alive until the scope drops. Scopes nest LIFO: only the innermost
/// live scope may push, and scopes drop in reverse order of creation.
pub struct ScratchRoots<'h> {
    pub(crate) heap: &'h crate::Heap,
    pub(crate) base: usize,
    pub(crate) depth: u32,
}

impl ScratchRoots<'_> {
    /// Roots `v`; returns its index in this scope.
    pub fn push(&self, v: Value) -> usize {
        assert_eq!(
            self.heap.scratch_depth.get(),
            self.depth,
            "push into a ScratchRoots scope that is not the innermost"
        );
        let mut s = self.heap.scratch.borrow_mut();
        s.push(v.to_bits());
        s.len() - 1 - self.base
    }

    pub fn get(&self, i: usize) -> Value {
        let s = self.heap.scratch.borrow();
        assert!(self.base + i < s.len(), "scratch index {i} out of range");
        let b = s[self.base + i];
        // SAFETY: scratch entries hold bits from `Value::to_bits` and are precise roots.
        unsafe { Value::from_bits_unchecked(b) }
    }

    pub fn set(&self, i: usize, v: Value) {
        let mut s = self.heap.scratch.borrow_mut();
        let len = s.len();
        assert!(self.base + i < len, "scratch index {i} out of range");
        s[self.base + i] = v.to_bits();
    }

    pub fn len(&self) -> usize {
        self.heap.scratch.borrow().len() - self.base
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for ScratchRoots<'_> {
    fn drop(&mut self) {
        if self.heap.scratch_depth.get() != self.depth && !std::thread::panicking() {
            panic!("ScratchRoots scopes dropped out of LIFO order");
        }
        self.heap.scratch.borrow_mut().truncate(self.base);
        self.heap.scratch_depth.set(self.depth - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_slots_are_reused() {
        let mut h = Handles::default();
        let a = h.new_global(Value::int32(1));
        let b = h.new_weak(Value::int32(2));
        assert_eq!(h.len(), 2);
        assert_eq!(h.global(a).as_int32(), Some(1));
        assert_eq!(h.weak(b).and_then(Value::as_int32), Some(2));
        h.set_global(a, Value::TRUE);
        assert_eq!(h.global(a).as_bool(), Some(true));
        h.release(a.0, false);
        let c = h.new_global(Value::NULL);
        assert_eq!(c.0, a.0, "free slot reused");
        assert_eq!(h.len(), 2);
    }

    #[test]
    #[should_panic(expected = "not live")]
    fn double_release_panics() {
        let mut h = Handles::default();
        let a = h.new_global(Value::int32(1));
        h.release(a.0, false);
        h.release(a.0, false);
    }

    #[test]
    #[should_panic(expected = "not a live global")]
    fn weak_is_not_global() {
        let mut h = Handles::default();
        let w = h.new_weak(Value::int32(1));
        h.global(GlobalHandle(w.0));
    }

    #[test]
    fn root_registry_ids() {
        let mut r = Roots::default();
        let a = r.add_source(Box::new(|_| {}));
        let b = r.add_weak_source(Box::new(|_| {}));
        assert_ne!(a, b);
        assert_eq!(r.len(), 2);
        assert!(r.remove(a));
        assert!(!r.remove(a));
        assert_eq!(r.len(), 1);
    }
}
