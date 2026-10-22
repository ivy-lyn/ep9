//! Cells and the cell-kind table.
//!
//! ```text
//! word0.kind (byte 5) --> KindTable[kind] = CellKind { name, size, trace }
//!                                             |       |      +-- fn(CellPtr, &mut Tracer): visit every slot
//!                                             |       +--------- Fixed(bytes) | Dynamic(fn(CellPtr) -> bytes)
//!                                             +----------------- for errors and the verifier
//! ```
//!
//! - The object model lives in `ep9_vm`. It registers one [`CellKind`] per kind byte with
//!   [`crate::Heap::register_kind`] before it allocates a cell of that kind.
//! - Kind 0 is never registered, so zeroed memory is never a valid cell.
//! - Size comes from the header (H9 "heap always parsable"): a fixed size, or a function that
//!   reads only the cell itself (a length field). The size of a cell never changes after the
//!   first GC point that follows its allocation.
//! - Each kind lives in one [`Space`]: `Normal` cells come from [`crate::Heap::alloc`], `Shape`
//!   cells from [`crate::Heap::alloc_shape`] in the shape region (H08-Q1).
//! - No `Drop` (H12): the sweep frees a dead cell without running Rust code. A kind whose
//!   layout is a Rust type ([`HeapLayout`]) is registered with [`crate::Heap::register_typed`],
//!   which rejects at build time a type that needs `Drop`. Kinds laid out by byte offsets (all
//!   of `ep9_vm` today) keep [`crate::Heap::register_kind`].

use core::num::NonZeroU8;
use core::ptr::NonNull;

use ep9_value::{Value, Word0};

use crate::barrier::Slot;
use crate::heap::Heap;
use crate::layout::{GRANULE, MAX_SMALL_SIZE, cell_size_for};
use crate::trace::Tracer;

/// Kind byte of a cell (word0 byte 5). Never 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KindId(NonZeroU8);

impl KindId {
    /// `None` for 0, which is reserved.
    pub const fn new(kind: u8) -> Option<KindId> {
        match NonZeroU8::new(kind) {
            Some(k) => Some(KindId(k)),
            None => None,
        }
    }

    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

/// Visits every GC reference of a live cell of this kind. The heap calls it only with a live
/// cell of the registered kind.
pub type TraceFn = fn(cell: CellPtr, tracer: &mut Tracer<'_>);

/// Size in bytes of a live cell of this kind. It may read only the cell itself.
pub type SizeFn = fn(cell: CellPtr) -> usize;

#[derive(Clone, Copy, Debug)]
pub enum CellSize {
    Fixed(usize),
    Dynamic(SizeFn),
}

/// What the heap knows about one cell kind.
#[derive(Clone, Copy, Debug)]
pub struct CellKind {
    pub name: &'static str,
    pub size: CellSize,
    pub trace: TraceFn,
}

/// Where the cells of a kind are allocated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Space {
    /// Small pages, LOS. [`crate::Heap::alloc`].
    #[default]
    Normal,
    /// The shape region at the start of the cage, so a cell's ShapeId is its offset from the
    /// cage base (H08-Q1). [`crate::Heap::alloc_shape`]; at most [`crate::layout::MAX_SMALL_SIZE`]
    /// bytes.
    Shape,
}

/// Why `register_kind` failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindError {
    /// The kind byte already has a registration.
    Taken(KindId),
    /// A fixed size is 0 or does not fit `cell_size_for`, or a shape kind's fixed size is
    /// above [`crate::layout::MAX_SMALL_SIZE`].
    BadSize(KindId),
    /// A [`Space::Shape`] kind on a heap whose `shape_space_bytes` is 0.
    NoShapeSpace(KindId),
}

impl std::fmt::Display for KindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KindError::Taken(k) => write!(f, "cell kind {} is already registered", k.get()),
            KindError::BadSize(k) => write!(f, "cell kind {} has a bad fixed size", k.get()),
            KindError::NoShapeSpace(k) => {
                write!(
                    f,
                    "cell kind {} needs a shape space, and the heap has none",
                    k.get()
                )
            }
        }
    }
}

impl std::error::Error for KindError {}

/// A Rust type that is the layout of one cell kind: `#[repr(C)]`, a [`Word0`] first, every GC
/// reference in a [`Slot`]. Register it with [`Heap::register_typed`].
///
/// The heap frees a dead cell without running any Rust code: no `Drop`, no finalizer (H12).
/// So the type must not need `Drop`. A `String`, `Vec`, `Box` or `Rc` field would leak, and a
/// `Drop` impl would never run. [`Heap::register_typed`] checks this when it is built:
///
/// ```compile_fail,E0080
/// use ep9_heap::{CellPtr, Heap, HeapConfig, HeapLayout, KindId, Tracer};
/// use ep9_value::Word0;
///
/// #[repr(C)]
/// struct Named {
///     word0: Word0,
///     name: String, // needs Drop: register_typed does not build
/// }
///
/// impl HeapLayout for Named {
///     const KIND: KindId = KindId::new(70).unwrap();
///     const NAME: &'static str = "named";
///     fn trace(_: CellPtr, _: &mut Tracer<'_>) {}
/// }
///
/// let heap = Heap::new(HeapConfig::default()).unwrap();
/// heap.register_typed::<Named>().unwrap();
/// ```
///
/// The same type with a plain field builds and registers:
///
/// ```
/// use ep9_heap::{CageConfig, CellPtr, Heap, HeapConfig, HeapLayout, KindId, Slot, Tracer};
/// use ep9_value::Word0;
///
/// #[repr(C)]
/// struct Named {
///     word0: Word0,
///     name: Slot, // a string Value: traced, no Drop
/// }
///
/// impl HeapLayout for Named {
///     const KIND: KindId = KindId::new(70).unwrap();
///     const NAME: &'static str = "named";
///     fn trace(cell: CellPtr, tracer: &mut Tracer<'_>) {
///         // SAFETY: the heap calls trace only with a live `Named` cell; +8 is its slot.
///         tracer.visit(unsafe { cell.slot(8) });
///     }
/// }
///
/// let cage = CageConfig {
///     cage_size: 1 << 30,
///     code_range_size: 0,
///     ..CageConfig::default()
/// };
/// let heap = Heap::new(HeapConfig { cage, ..HeapConfig::default() }).unwrap();
/// heap.register_typed::<Named>().unwrap();
/// ```
///
/// The check runs where `register_typed::<T>` is instantiated, so it shows in `cargo build`
/// and `cargo test`, not in `cargo check` or clippy.
pub trait HeapLayout: Sized {
    /// The kind byte (word0 byte 5).
    const KIND: KindId;
    /// For errors and the verifier.
    const NAME: &'static str;
    /// Where its cells are allocated.
    const SPACE: Space = Space::Normal;
    /// The cell size: the type's size, or a [`SizeFn`] for a cell with a tail.
    const SIZE: CellSize = CellSize::Fixed(size_of::<Self>());
    /// Visits every GC reference of a live cell of this kind (a [`TraceFn`]).
    fn trace(cell: CellPtr, tracer: &mut Tracer<'_>);
}

impl Heap {
    /// Registers the cell kind of the Rust type `T`, as [`Heap::register_kind_in`] does with
    /// `T`'s constants. Does not build if `T` needs `Drop` (H12), is smaller than word0, or is
    /// aligned above the heap granule (8 bytes).
    pub fn register_typed<T: HeapLayout>(&self) -> Result<(), KindError> {
        const {
            assert!(
                !core::mem::needs_drop::<T>(),
                "a heap cell type must not need Drop: the GC frees cells without running Rust code (H12)"
            );
            assert!(
                size_of::<T>() >= size_of::<Word0>(),
                "a heap cell type starts with word0"
            );
            assert!(
                align_of::<T>() <= GRANULE,
                "a heap cell type is aligned to at most the heap granule (8 bytes)"
            );
        };
        self.register_kind_in(
            T::KIND,
            CellKind {
                name: T::NAME,
                size: T::SIZE,
                trace: T::trace,
            },
            T::SPACE,
        )
    }
}

pub(crate) struct KindTable {
    kinds: [Option<CellKind>; 256],
    spaces: [Space; 256],
}

impl KindTable {
    pub(crate) fn new() -> KindTable {
        KindTable {
            kinds: [None; 256],
            spaces: [Space::Normal; 256],
        }
    }

    /// [`KindTable::register_in`] with [`Space::Normal`] (the unit tests' entry).
    #[cfg(test)]
    pub(crate) fn register(&mut self, id: KindId, kind: CellKind) -> Result<(), KindError> {
        self.register_in(id, kind, Space::Normal)
    }

    /// Registers `kind` in `space`. The caller checks that a shape space exists.
    pub(crate) fn register_in(
        &mut self,
        id: KindId,
        kind: CellKind,
        space: Space,
    ) -> Result<(), KindError> {
        let slot = &mut self.kinds[id.get() as usize];
        if slot.is_some() {
            return Err(KindError::Taken(id));
        }
        if let CellSize::Fixed(n) = kind.size {
            match cell_size_for(n) {
                Some(s) if n != 0 && (space == Space::Normal || s <= MAX_SMALL_SIZE) => {}
                _ => return Err(KindError::BadSize(id)),
            }
        }
        *slot = Some(kind);
        self.spaces[id.get() as usize] = space;
        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self, kind: u8) -> Option<&CellKind> {
        self.kinds[kind as usize].as_ref()
    }

    /// The space of a kind byte. `Normal` for a kind with no registration.
    #[inline]
    pub(crate) fn space(&self, kind: u8) -> Space {
        self.spaces[kind as usize]
    }

    /// Allocated size of a live cell: its kind's size, rounded as the allocator rounds.
    /// `None` if the kind byte has no registration (a corrupt or dead cell).
    #[inline]
    pub(crate) fn cell_size(&self, cell: CellPtr) -> Option<usize> {
        // SAFETY: callers pass a cell start found through a VO bit or a LOS header, so word0
        // was written by the allocator and the memory is committed.
        let kind = unsafe { cell.header() }.kind();
        let k = self.get(kind)?;
        let raw = match k.size {
            CellSize::Fixed(n) => n,
            CellSize::Dynamic(f) => f(cell),
        };
        cell_size_for(raw)
    }
}

/// A pointer to a heap cell (word0 first). `Copy`, 8 bytes, no lifetime.
///
/// A `CellPtr` keeps its cell alive only where the GC can see it: on the native stack or in a
/// register (conservative scan), or in a precise root (handles, [`crate::ScratchRoots`], a root
/// source). A `CellPtr` stored only in malloc memory (a `Vec`) is invisible (probe case S7).
/// So every method that reads the cell is `unsafe`: the caller vouches that the cell is live.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct CellPtr(NonNull<Word0>);

const _: () = assert!(size_of::<CellPtr>() == 8 && size_of::<Option<CellPtr>>() == 8);

impl CellPtr {
    /// # Safety
    /// `p` points to the start of a live cell of a [`crate::Heap`], with provenance for the
    /// whole cell.
    #[inline(always)]
    pub const unsafe fn from_raw(p: NonNull<Word0>) -> CellPtr {
        CellPtr(p)
    }

    /// The cell of a cell-kind `Value`. The pointer gets the provenance that
    /// `Value::from_cell_ptr` exposed.
    #[inline(always)]
    pub fn from_value(v: Value) -> Option<CellPtr> {
        v.as_cell_ptr().map(|p| CellPtr(p.cast()))
    }

    #[inline(always)]
    pub fn addr(self) -> usize {
        self.0.addr().get()
    }

    #[inline(always)]
    pub const fn as_ptr(self) -> *mut Word0 {
        self.0.as_ptr()
    }

    #[inline(always)]
    pub const fn cast<T>(self) -> NonNull<T> {
        self.0.cast()
    }

    /// The cell as a `Value`. Building the Value reads nothing.
    #[inline(always)]
    pub fn to_value(self) -> Value {
        // SAFETY: a CellPtr is 8-aligned and below 2^47 by construction (the heap hands out
        // only cells inside its reservation, H7). Liveness is the rooting rule above.
        unsafe { Value::from_cell_ptr(self.0) }
    }

    /// The header.
    ///
    /// # Safety
    /// The cell is live for `'a`.
    #[inline(always)]
    pub unsafe fn header<'a>(self) -> &'a Word0 {
        // SAFETY: caller contract; word0 is written at allocation and only accessed as atomics.
        unsafe { self.0.as_ref() }
    }

    /// The `Value` slot at `byte_offset`.
    ///
    /// # Safety
    /// The cell is live for `'a`, and `[byte_offset, byte_offset + 8)` is a `Value` slot of
    /// this cell (8-aligned, inside the cell, after word0).
    #[inline(always)]
    pub unsafe fn slot<'a>(self, byte_offset: usize) -> &'a Slot {
        debug_assert!(byte_offset >= 8 && byte_offset.is_multiple_of(8));
        // SAFETY: caller contract. Slots are atomics, so shared references are fine.
        unsafe { self.0.cast::<u8>().add(byte_offset).cast::<Slot>().as_ref() }
    }

    /// `n` consecutive `Value` slots from `byte_offset`.
    ///
    /// # Safety
    /// As for [`CellPtr::slot`], for every slot of the range.
    #[inline(always)]
    pub unsafe fn slots<'a>(self, byte_offset: usize, n: usize) -> &'a [Slot] {
        debug_assert!(byte_offset >= 8 && byte_offset.is_multiple_of(8));
        // SAFETY: caller contract.
        unsafe {
            core::slice::from_raw_parts(
                self.0.cast::<u8>().add(byte_offset).cast::<Slot>().as_ptr(),
                n,
            )
        }
    }

    /// Raw pointer to the byte at `byte_offset`, for non-Value fields.
    ///
    /// # Safety
    /// `byte_offset` is inside the cell.
    #[inline(always)]
    pub unsafe fn field<T>(self, byte_offset: usize) -> *mut T {
        // SAFETY: caller contract: the offset stays inside the cell's allocation.
        unsafe { self.0.cast::<u8>().add(byte_offset).cast::<T>().as_ptr() }
    }
}

impl std::fmt::Debug for CellPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CellPtr({:#x})", self.addr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace_none(_: CellPtr, _: &mut Tracer<'_>) {}
    fn size_16(_: CellPtr) -> usize {
        16
    }

    #[test]
    fn kind_ids() {
        assert_eq!(KindId::new(0), None);
        assert_eq!(KindId::new(9).map(KindId::get), Some(9));
        assert_eq!(size_of::<Option<KindId>>(), 1);
    }

    #[test]
    fn register_rules() {
        let mut t = KindTable::new();
        let k = KindId::new(3).unwrap();
        let kind = CellKind {
            name: "leaf",
            size: CellSize::Fixed(24),
            trace: trace_none,
        };
        assert_eq!(t.register(k, kind), Ok(()));
        assert_eq!(t.register(k, kind), Err(KindError::Taken(k)));
        let k2 = KindId::new(4).unwrap();
        let bad = CellKind {
            size: CellSize::Fixed(0),
            ..kind
        };
        assert_eq!(t.register(k2, bad), Err(KindError::BadSize(k2)));
        let dynamic = CellKind {
            size: CellSize::Dynamic(size_16),
            ..kind
        };
        assert_eq!(t.register(k2, dynamic), Ok(()));
        assert_eq!(t.get(3).map(|k| k.name), Some("leaf"));
        assert!(t.get(0).is_none() && t.get(5).is_none());
    }

    /// A typed kind: word0, one strong slot, one raw word.
    #[repr(C)]
    struct Pair {
        word0: Word0,
        first: Slot,
        raw: u64,
    }

    impl HeapLayout for Pair {
        const KIND: KindId = KindId::new(40).unwrap();
        const NAME: &'static str = "test-pair";
        fn trace(cell: CellPtr, tracer: &mut Tracer<'_>) {
            // SAFETY: the heap calls trace only with a live Pair cell; `first` is its slot.
            tracer.visit(unsafe { cell.slot(core::mem::offset_of!(Pair, first)) });
        }
    }

    /// A typed kind of the shape space.
    #[repr(C)]
    struct ShapeLike {
        word0: Word0,
        raw: u64,
    }

    impl HeapLayout for ShapeLike {
        const KIND: KindId = KindId::new(41).unwrap();
        const NAME: &'static str = "test-shape-like";
        const SPACE: Space = Space::Shape;
        fn trace(_: CellPtr, _: &mut Tracer<'_>) {}
    }

    #[test]
    fn register_typed_uses_the_type() {
        use crate::tests::{LEAF, test_heap};

        let heap = test_heap(|_| {});
        assert_eq!(heap.register_typed::<Pair>(), Ok(()));
        assert_eq!(
            heap.register_typed::<Pair>(),
            Err(KindError::Taken(Pair::KIND))
        );
        assert_eq!(
            heap.register_typed::<ShapeLike>(),
            Err(KindError::NoShapeSpace(ShapeLike::KIND))
        );
        {
            let inner = heap.inner.borrow();
            let k = inner.kinds.get(Pair::KIND.get()).expect("registered");
            assert_eq!(k.name, "test-pair");
            assert!(matches!(k.size, CellSize::Fixed(24)));
        }
        // The size is the type's: another size is refused.
        assert!(heap.alloc(Pair::KIND, 32).is_err());
        let pair = heap
            .alloc(Pair::KIND, size_of::<Pair>())
            .expect("alloc pair");
        let leaf = heap.alloc(LEAF, 24).expect("alloc leaf");
        // SAFETY: `pair` is a new Pair cell, written before the next GC point (elision E2).
        unsafe {
            (*pair.cast::<Pair>().as_ptr()).first.set(
                pair.header(),
                leaf.to_value(),
                heap.barrier(),
            );
            (*pair.cast::<Pair>().as_ptr()).raw = 7;
        }
        let g = heap.new_global(pair.to_value());
        let leaf_addr = leaf.addr();
        // Only the global and `Pair::trace` keep the leaf: the scan reads no stack words.
        heap.collect_with_words(&[]);
        assert!(heap.is_cell(pair.addr()));
        assert!(
            heap.is_cell(leaf_addr),
            "Pair::trace did not visit its slot"
        );
        heap.drop_global(g);
        heap.collect_with_words(&[]);
        assert!(!heap.is_cell(pair.addr()) && !heap.is_cell(leaf_addr));
    }
}
