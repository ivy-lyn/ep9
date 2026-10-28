//! The isolate's runtime state: heap, VM stack, realm roots, loaded units, host functions.
//!
//! ```text
//! Isolate ──owns──> Box<Cx>  (interpreter: dispatch table @0, hot fields)   ──cx.rt──┐
//!         ──owns──> Box<Runtime> <───────────────────────────────────────────────────┘
//!                     heap: Heap ──root source──> Rc<Shared> (also held by Runtime)
//!                     shared: Rc<Shared> { VM stack, pending exception, current realm roots }
//!                     units, host infos, registry, limits
//!                     shapes, atoms, symbols, builtins   (per-isolate state of shape.rs,
//!                                                         atoms.rs, symbol.rs, builtins.rs)
//!                     stub_cache, rng, kept_objects,     (M4-P0: stub_cache.rs, Math.random,
//!                     regexp_programs                     weakref.rs, regexp.rs)
//! ```
//!
//! `Runtime` is only ever shared (`&Runtime`); every mutable field is a `Cell` or `RefCell`.
//! The interpreter holds `&mut Cx`, which never contains GC-visible state: a GC reads only
//! `Shared` (through the root source) and heap cells. So an opaque runtime call cannot reorder
//! a store that the GC needs (the `noalias` rule of `&mut`).

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use ep9_heap::{AllocError, CellPtr, Heap, KindId, WeakHandle};
use ep9_value::Value;

use crate::atoms::AtomTable;
use crate::builtins::BuiltinState;
use crate::compiler::CompilerHooks;
use crate::dispatch::RangeRegistry;
use crate::frame::{UNDEFINED, VmStack, int32_bits, native_ptr, slot};
use crate::shape::ShapeState;
use crate::symbol::SymbolState;
use crate::unit::{FunctionInfo, LoadedUnit};

pub use crate::atoms::Atom;

/// An embedder callback that `InterruptHandle::request` queues: it runs once, on the isolate
/// thread, at an interrupt poll. It gets no `Cx`, so it runs no JS and allocates nothing on
/// the JS heap (H23-Q6).
pub(crate) type InterruptCallback = Box<dyn FnOnce() + Send>;

/// The queue of [`InterruptCallback`]s: other threads push, the isolate thread takes.
pub(crate) type InterruptQueue = Arc<Mutex<Vec<InterruptCallback>>>;

/// What an `InterruptHandle` holds of the queue. Weak: the `Runtime` is the only strong owner,
/// so a queued callback that holds a handle makes no cycle, and the queue and its callbacks
/// go when the isolate does.
pub(crate) type WeakInterruptQueue = Weak<Mutex<Vec<InterruptCallback>>>;

/// Realm intrinsics and other runtime roots. The realm roots (all but [`Root::RealmKey`]) hold
/// the values of the *current* realm: a cross-realm call writes the callee realm's values into
/// them from its REALM record, and puts the caller's back on return (`realm::switch`, H22-Q3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Root {
    GlobalObject,
    /// The global declarative record (script `let`/`const`/`class`): a null-prototype object.
    /// v0: an object, not a script context (H22/H13 decide the real layout).
    Lexical,
    ObjectPrototype,
    FunctionPrototype,
    ArrayPrototype,
    StringPrototype,
    NumberPrototype,
    BooleanPrototype,
    ErrorPrototype,
    TypeErrorPrototype,
    RangeErrorPrototype,
    ReferenceErrorPrototype,
    SyntaxErrorPrototype,
    EvalErrorPrototype,
    UriErrorPrototype,
    /// `%ArrayIteratorPrototype%`, the prototype of array iterators. String iterators have
    /// their own `%StringIteratorPrototype%` (`builtins::IntrinsicId::StringIteratorPrototype`,
    /// held in a builtin state slot).
    ArrayIteratorPrototype,
    // ---- week 4: the other intrinsics of `builtins::IntrinsicId` (H20 tables) ----
    /// `%Iterator.prototype%`.
    IteratorPrototype,
    ObjectConstructor,
    FunctionConstructor,
    ArrayConstructor,
    NumberConstructor,
    BooleanConstructor,
    StringConstructor,
    SymbolConstructor,
    ErrorConstructor,
    TypeErrorConstructor,
    RangeErrorConstructor,
    ReferenceErrorConstructor,
    SyntaxErrorConstructor,
    EvalErrorConstructor,
    UriErrorConstructor,
    /// `%Math%`.
    Math,
    // ---- M3-P0 (OBJECT_LAYOUT 3) ----
    /// The current realm: a REALM cell (`object::realmf`), made first by `realm::init`. Every
    /// new function stores it as its `[[Realm]]`; `Cx::realm` holds a copy. Its fields mirror
    /// the roots above (global object, lexical record, the intrinsics).
    Realm,
    // ---- M3-P1 REALMS: isolate-wide, not switched with the realm ----
    /// The VM-internal private name under which the global object of each realm holds its
    /// REALM record (`realm::of_global`; no JS code can name it). Made by the first
    /// `realm::init`.
    RealmKey,
}

pub(crate) const ROOT_COUNT: usize = Root::RealmKey as usize + 1;

/// The roots that belong to the current realm: every root before [`Root::RealmKey`].
pub(crate) const REALM_ROOT_COUNT: usize = Root::RealmKey as usize;

/// Root slots: realm roots, then the `atoms!` list, then the 256 single-character atoms
/// (undefined until first use, string.rs `single_char`).
const ATOM_ROOTS: usize = ROOT_COUNT;
const SINGLE_CHAR_ROOTS: usize = ATOM_ROOTS + Atom::ALL.len();
const ROOT_SLOTS: usize = SINGLE_CHAR_ROOTS + 256;

/// The ECMAScript native error types (§20.5.5) plus the plain `Error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Error,
    TypeError,
    RangeError,
    ReferenceError,
    SyntaxError,
    EvalError,
    UriError,
}

impl ErrorKind {
    pub const ALL: [ErrorKind; 7] = [
        ErrorKind::Error,
        ErrorKind::TypeError,
        ErrorKind::RangeError,
        ErrorKind::ReferenceError,
        ErrorKind::SyntaxError,
        ErrorKind::EvalError,
        ErrorKind::UriError,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            ErrorKind::Error => "Error",
            ErrorKind::TypeError => "TypeError",
            ErrorKind::RangeError => "RangeError",
            ErrorKind::ReferenceError => "ReferenceError",
            ErrorKind::SyntaxError => "SyntaxError",
            ErrorKind::EvalError => "EvalError",
            ErrorKind::UriError => "URIError",
        }
    }

    pub(crate) const fn prototype_root(self) -> Root {
        match self {
            ErrorKind::Error => Root::ErrorPrototype,
            ErrorKind::TypeError => Root::TypeErrorPrototype,
            ErrorKind::RangeError => Root::RangeErrorPrototype,
            ErrorKind::ReferenceError => Root::ReferenceErrorPrototype,
            ErrorKind::SyntaxError => Root::SyntaxErrorPrototype,
            ErrorKind::EvalError => Root::EvalErrorPrototype,
            ErrorKind::UriError => Root::UriErrorPrototype,
        }
    }
}

/// Where a throw happened: unit index and UTF-8 source offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub unit: u32,
    pub offset: u32,
}

/// State the GC root source reads. Shared by `Runtime` and the root closure.
pub(crate) struct Shared {
    pub stack: VmStack,
    /// Pending exception (Value bits), 0 if none.
    pub pending: Cell<u64>,
    /// The "pending message" register of `SetPendingMessage` (Value bits).
    pub pending_message: Cell<u64>,
    /// Location of the last throw from bytecode (not ReThrow), for uncaught non-Error values.
    pub throw_site: Cell<Option<Site>>,
    roots: Box<[Cell<u64>]>,
}

impl Shared {
    fn new(stack_slots: usize) -> Shared {
        Shared {
            stack: VmStack::new(stack_slots),
            pending: Cell::new(0),
            pending_message: Cell::new(UNDEFINED),
            throw_site: Cell::new(None),
            roots: (0..ROOT_SLOTS).map(|_| Cell::new(UNDEFINED)).collect(),
        }
    }

    /// Visits every precise root the VM owns: the live VM stack up to the top frame or the
    /// scratch top (whichever is higher), the pending exception, the pending message, the
    /// realm roots, the `atoms!` list and the single-character atoms.
    fn trace(&self, t: &mut ep9_heap::Tracer<'_>) {
        let fp = self.stack.fp();
        let base = self.stack.base();
        let top = if fp.is_null() { base } else { frame_top(fp) };
        let scratch = self.stack.scratch_top();
        let end = if scratch.addr() > top.addr() {
            scratch
        } else {
            top
        };
        debug_assert!(end.addr() >= base.addr() && end.addr() <= self.stack.end().addr());
        let n = (end.addr() - base.addr()) / 8;
        for i in 0..n {
            // SAFETY: [base, end) lies inside the stack allocation; every slot below the top
            // frame was written with a valid Value at frame push (frame.rs), and Rust code
            // writes valid Values before it raises the scratch top past them.
            t.visit_bits(unsafe { *base.add(i) });
        }
        let p = self.pending.get();
        if p != 0 {
            t.visit_bits(p);
        }
        t.visit_bits(self.pending_message.get());
        for r in self.roots.iter() {
            t.visit_bits(r.get());
        }
    }
}

/// First slot above the frame at `fp`: `fp + register_count` of its function info.
pub(crate) fn frame_top(fp: *mut u64) -> *mut u64 {
    // SAFETY: `fp` is a live frame pointer (caller contract): its header slot holds the native
    // word of a FunctionInfo that outlives every frame of it.
    let fi = unsafe { &*native_ptr::<FunctionInfo>(*fp.offset(slot::FUNCTION_INFO)) };
    fp.wrapping_add(fi.register_count as usize)
}

/// Everything the interpreter and the runtime share. Only `&Runtime` exists.
pub(crate) struct Runtime {
    pub heap: Heap,
    pub shared: Rc<Shared>,
    /// Boxed: frames, function cells and dispatch slots point into them.
    #[allow(clippy::vec_box)]
    pub units: RefCell<Vec<Box<LoadedUnit>>>,
    /// FunctionInfos of embedder host functions (`Isolate::register_host_function`). Boxed:
    /// frames and function cells point at them. Table builtins use the dense arrays of
    /// `builtins`.
    #[allow(clippy::vec_box)]
    pub infos: RefCell<Vec<Box<FunctionInfo>>>,
    /// The frame info of `[[Construct]]` stub frames (`OP_CONSTRUCT_RETURN`).
    pub construct_stub: Box<FunctionInfo>,
    pub registry: RefCell<RangeRegistry>,
    /// Lowest native stack address the VM lets JS entries and host calls reach.
    pub native_limit: usize,
    pub stack_trace_limit: usize,
    /// The compiler hooks (`IsolateConfig::compiler`, C2), for eval and `Function()`.
    pub compiler: Option<&'static CompilerHooks>,
    /// The interrupt word (`interp::interrupt` bits: `TERMINATE`, `REQUEST`): set by
    /// `TerminateHandle::terminate` and `InterruptHandle::request` (other threads), read at
    /// interrupt polls through `Cx::interrupt`.
    pub terminate: Arc<AtomicU32>,
    /// The callbacks of `InterruptHandle::request`. The `REQUEST` bit of `terminate` says the
    /// queue may be non-empty; the poll clears the bit, then takes the queue
    /// ([`Runtime::run_interrupt_requests`]). Holds no Value.
    pub interrupt_requests: InterruptQueue,
    /// True while a termination unwinds (no handler catches it).
    pub terminating: Cell<bool>,
    /// Template objects per (FunctionInfo, constant index), weak (`realm::add_template`).
    pub templates: RefCell<HashMap<(usize, u32), WeakHandle>>,
    /// Shapes (H17): the prototype epoch and the shape roots.
    pub shapes: ShapeState,
    /// The atom table (H19). Shared with the heap's weak source; never borrowed across an
    /// allocation.
    pub atoms: Rc<RefCell<AtomTable>>,
    /// Symbols: the registry and the identity-hash PRNG.
    pub symbols: SymbolState,
    /// Builtin tables (H20): the dense FunctionInfo arrays and function roots per module.
    pub builtins: BuiltinState,
    /// The microtask job queue, the rejected-promise list and the built-in closure infos
    /// (`jobs.rs`, P4-PROMISE): heap records behind a root source.
    pub jobs: crate::jobs::JobState,
    /// A number that no other runtime of the process gets ([`NEXT_ID`]): the key of the
    /// isolate's module hosts (`module::host_key`). An address can come again after a drop;
    /// this number does not.
    pub id: u64,
    // ---- M4-P0 (plan v9 §2.5 task 4): `runtime.rs` has no owner after M4-P0, so each field's
    // type lives in its owner's module and has its own interior mutability ----
    /// The megamorphic stub cache (`stub_cache.rs`, M4-P3 IC2): empty until IC2 fills it.
    #[allow(dead_code)] // read by its owner module from M4-P1 on
    pub stub_cache: crate::stub_cache::StubCache,
    /// The xorshift128+ state of `Math.random` (`Cx::math_random`; M4-P1 MATH1, V9-D16),
    /// seeded per isolate from the process hash key and [`Runtime::id`] ([`rng_seed`]). Never
    /// both words 0.
    pub rng: Cell<[u64; 2]>,
    /// The WeakRef state (§9.10; `weakref.rs`, M4-P4 WEAKREF): the `[[KeptAlive]]` set (a
    /// strong root), the chain of FinalizationRegistry objects (each held weakly) and the
    /// cleanup flag that the GC's weak source sets.
    pub kept_objects: crate::weakref::KeptObjects,
    /// The compiled RegExp programs (`regexp.rs`, M4-P2 RX0A; V9-A7): Rust memory with no
    /// Value; a RegExp object holds its int32 index, and the table's weak source frees the
    /// entry of a dead RegExp.
    #[allow(dead_code)] // read by its owner module from M4-P1 on
    pub regexp_programs: crate::regexp::ProgramTable,
    // ---- M5-P0 (plan v10 §2.4, P0-VM5): the month-5 Runtime fields; each type lives in its
    // owner's module (as M4-P0) ----
    /// The execution options of the isolate (`exec.rs`, C-23): set by `Isolate::new_with`
    /// before the realm exists. Stored in M5-P0; M5-P3 PROFILE6 reads the feedback mode.
    #[allow(dead_code)] // read from M5-P3 on
    pub exec: Cell<crate::exec::ExecOptions>,
    /// The deopt side table (`deopt.rs`, V10-D32): code start -> deopt metadata and exit
    /// counters, filled at install. Rust memory, no Value. Empty until M5-P1 SPIKEC-5.
    #[allow(dead_code)] // filled by M5-P1 SPIKEC-5
    pub deopt: crate::deopt::SideTable,
    /// The fuse registry (`fuse.rs`, H31): fuse states and their weak dependents. Rust
    /// memory. Empty until M5-P1 FR0.
    #[allow(dead_code)] // filled by M5-P1 FR0
    pub fuses: crate::fuse::Registry,
    /// The state of the sampled-call window jitter (`profile.rs`, `CALL_WINDOW`; doc 13:705):
    /// M5-P1 PROFILE5 seeds and advances it if the window start gets a jitter. 0 = no jitter
    /// (today).
    #[allow(dead_code)] // M5-P1 PROFILE5
    pub call_window_jitter: Cell<u64>,
}

/// The next [`Runtime::id`].
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The `Math.random` seed of the isolate with [`Runtime::id`] `id` (V9-D16): two words from
/// the process hash key (random from the OS, or fixed by `EP9_HASH_SEED`), read through the
/// keyed hash of fixed texts as the symbol hash seed is (`symbol.rs`), with `id` mixed in by
/// splitmix64. So each isolate of a process has its own sequence (§21.3.2.27: distinct realms
/// give distinct sequences), and two runs with one `EP9_HASH_SEED` give the n-th isolate of
/// the run the same sequence. Never both words 0 (xorshift128+ would stay at 0). Never an
/// address.
pub(crate) fn rng_seed(id: u64) -> [u64; 2] {
    fn splitmix64(x: u64) -> u64 {
        let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    let word = |text: &[u8]| {
        let (a, b) = crate::string::hash_text(crate::string::JsStr::Latin1(text));
        splitmix64((u64::from(a) << 32) | u64::from(b))
    };
    let s0 = splitmix64(word(b"ep9 Math.random seed 0") ^ splitmix64(id));
    let s1 = splitmix64(word(b"ep9 Math.random seed 1") ^ s0);
    if s0 | s1 == 0 { [1, 2] } else { [s0, s1] }
}

impl Runtime {
    pub fn new(
        heap: Heap,
        stack_slots: usize,
        native_limit: usize,
        stack_trace_limit: usize,
        compiler: Option<&'static CompilerHooks>,
    ) -> Box<Runtime> {
        crate::object::register_kinds(&heap);
        let shared = Rc::new(Shared::new(stack_slots));
        let sh = Rc::clone(&shared);
        heap.add_root_source(move |t| sh.trace(t));
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let rt = Box::new(Runtime {
            // The fields that register a heap source come before `heap` moves in.
            jobs: crate::jobs::JobState::new(&heap),
            stub_cache: crate::stub_cache::StubCache::new(&heap),
            rng: Cell::new(rng_seed(id)),
            kept_objects: crate::weakref::KeptObjects::new(&heap),
            regexp_programs: crate::regexp::ProgramTable::new(&heap),
            exec: Cell::new(crate::exec::ExecOptions::build_default()),
            deopt: crate::deopt::SideTable::new(),
            fuses: crate::fuse::Registry::new(),
            call_window_jitter: Cell::new(0),
            heap,
            shared,
            units: RefCell::new(Vec::new()),
            infos: RefCell::new(Vec::new()),
            construct_stub: Box::new(FunctionInfo::stub(
                crate::interp::CONSTRUCT_RETURN_STUB.as_ptr(),
            )),
            registry: RefCell::new(RangeRegistry::new()),
            native_limit,
            stack_trace_limit,
            compiler,
            terminate: Arc::new(AtomicU32::new(0)),
            interrupt_requests: Arc::new(Mutex::new(Vec::new())),
            terminating: Cell::new(false),
            templates: RefCell::new(HashMap::new()),
            shapes: ShapeState::default(),
            atoms: Rc::new(RefCell::new(AtomTable::default())),
            symbols: SymbolState::default(),
            builtins: BuiltinState::default(),
            id,
        });
        crate::atoms::init(&rt);
        crate::shape::init(&rt);
        rt
    }

    #[inline]
    pub fn stack(&self) -> &VmStack {
        &self.shared.stack
    }

    /// Records the interpreter state before a GC point or a host call: the topmost frame, and
    /// its bytecode offset for stack traces (H14 header slot 7).
    #[inline]
    pub fn enter(&self, fp: *mut u64, pc: *const u8) {
        self.shared.stack.set_fp(fp);
        if fp.is_null() {
            return;
        }
        // SAFETY: `fp` is the live top frame of the caller; its function info outlives it.
        let fi = unsafe { &*native_ptr::<FunctionInfo>(*fp.offset(slot::FUNCTION_INFO)) };
        if let Some(off) = fi.offset_of(pc) {
            // SAFETY: header slot 7 of a live frame.
            unsafe { *fp.offset(slot::BYTECODE_OFFSET) = int32_bits(off as i32) };
        }
    }

    /// Allocates a cell. GC point. Out of memory is fatal, never a JS exception (H23, as V8):
    /// the heap first calls the embedder's near-limit callback (`HeapConfig::near_limit`),
    /// which may raise the limit; if the allocation still fails, this panics with
    /// "ep9: heap limit reached", and a release build aborts (`panic = "abort"`).
    #[inline]
    pub fn alloc(&self, kind: KindId, size: usize) -> CellPtr {
        match self.heap.alloc(kind, size) {
            Ok(c) => c,
            Err(e) => alloc_failed(e),
        }
    }

    /// Runs the callbacks that `InterruptHandle::request` queued, in request order, on this
    /// (the isolate) thread. The caller has cleared the `REQUEST` bit first, so a request that
    /// lands meanwhile runs now or at the next poll, never not at all. The lock is not held
    /// while a callback runs: a callback may request again (it then runs at a later poll).
    #[cold]
    #[inline(never)]
    pub fn run_interrupt_requests(&self) {
        let queued = core::mem::take(
            &mut *self
                .interrupt_requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for callback in queued {
            callback();
        }
    }

    pub fn root(&self, r: Root) -> u64 {
        self.shared.roots[r as usize].get()
    }

    pub fn set_root(&self, r: Root, v: u64) {
        self.shared.roots[r as usize].set(v);
    }

    /// Makes every current-realm root (all before `Root::RealmKey`) undefined: the first step
    /// of a new realm's init (`realm::create`).
    pub fn clear_realm_roots(&self) {
        for r in &self.shared.roots[..REALM_ROOT_COUNT] {
            r.set(UNDEFINED);
        }
    }

    pub fn atom(&self, a: Atom) -> u64 {
        self.shared.roots[ATOM_ROOTS + usize::from(a.id())].get()
    }

    pub fn set_atom(&self, a: Atom, v: u64) {
        self.shared.roots[ATOM_ROOTS + usize::from(a.id())].set(v);
    }

    /// The single-character atom of `c`, or undefined before its first use.
    pub fn single_char(&self, c: u8) -> u64 {
        self.shared.roots[SINGLE_CHAR_ROOTS + c as usize].get()
    }

    pub fn set_single_char(&self, c: u8, v: u64) {
        self.shared.roots[SINGLE_CHAR_ROOTS + c as usize].set(v);
    }

    pub fn global(&self) -> u64 {
        self.root(Root::GlobalObject)
    }

    /// Makes `v` the pending exception. Returns 0 (the throw marker of runtime entries).
    pub fn throw_value(&self, v: u64) -> u64 {
        debug_assert!(v != 0);
        self.shared.pending.set(v);
        0
    }

    pub fn pending(&self) -> u64 {
        self.shared.pending.get()
    }

    pub fn take_pending(&self) -> u64 {
        self.shared.pending.replace(0)
    }

    pub fn unit(&self, index: u32) -> *const LoadedUnit {
        &*self.units.borrow()[index as usize]
    }

    /// Native stack check for JS entries, host calls and recursive runtime routines.
    #[inline]
    pub fn native_stack_ok(&self) -> bool {
        ep9_platform::stack_pointer() > self.native_limit
    }
}

/// The fatal path of [`Runtime::alloc`] (H23 "Out of memory"): out of its own function, so the
/// inlined allocation stays small.
#[cold]
#[inline(never)]
fn alloc_failed(e: AllocError) -> ! {
    match e {
        AllocError::OutOfMemory => panic!("ep9: heap limit reached"),
        e => panic!("ep9: heap allocation failed: {e:?}"),
    }
}

/// Debug builds: `atoms::check_at_drop` (the atom table check, and the W4-KEY-EQ counter
/// check with `EP9_ASSERT_ATOM_KEYS` set). Release builds have no `Drop`.
#[cfg(debug_assertions)]
impl Drop for Runtime {
    fn drop(&mut self) {
        crate::atoms::check_at_drop(self);
    }
}

/// The `Value` of bits that the VM holds (stack slots, accumulator, cell slots, roots).
#[inline(always)]
pub(crate) fn val(b: u64) -> Value {
    debug_assert!(Value::is_valid_bits(b), "invalid Value bits {b:#x}");
    // SAFETY: the VM only stores valid Value bits in these places (frame.rs and object.rs
    // write only Value bits or the two non-cell encodings), and cells stay live while held.
    unsafe { Value::from_bits_unchecked(b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoms_and_roots_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for a in Atom::ALL {
            assert!(seen.insert(a.text()), "duplicate atom {:?}", a.text());
        }
        assert_eq!(ROOT_COUNT, 34);
        assert_eq!(REALM_ROOT_COUNT, 33);
        assert_eq!(ROOT_SLOTS, 34 + Atom::ALL.len() + 256);
        assert_eq!(ErrorKind::UriError.name(), "URIError");
        assert_eq!(crate::frame::HEADER, 8);
    }
}
