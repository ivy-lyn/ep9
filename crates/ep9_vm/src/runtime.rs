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
    // ---- M6-P0 (OBJECT_LAYOUT 6; V11-D21, MC-w22-17: root slots move once, before any unit) ----
    /// The current realm's static fuse block: a FUSE cell (kind 83, `kinds::PROPS_INDEXED` has
    /// the layout table), made by `fuse::new_realm_block` at realm init (M6-P2 FUSE-PI). It
    /// mirrors the realm's intrinsic `IntrinsicId::RealmFuses` (`builtins::IntrinsicId::root`),
    /// so `realm::switch` writes it with the other intrinsics.
    RealmFuses,
    // ---- M3-P1 REALMS: isolate-wide, not switched with the realm ----
    /// The VM-internal private name under which the global object of each realm holds its
    /// REALM record (`realm::of_global`; no JS code can name it). Made by the first
    /// `realm::init`.
    RealmKey,
}

pub(crate) const ROOT_COUNT: usize = root_count();

/// The roots that belong to the current realm: every root before [`Root::RealmKey`].
pub(crate) const REALM_ROOT_COUNT: usize = Root::RealmKey as usize;

/// Root slots: realm roots, then the `atoms!` list, then the 256 single-character atoms
/// (undefined until first use, string.rs `single_char`); the slots at A2 start ([`APPENDED`]).
const ATOM_ROOTS: usize = A2_ROOT_COUNT;
const SINGLE_CHAR_ROOTS: usize = ATOM_ROOTS + A2_ATOM_COUNT;
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
            roots: (0..TABLE_SLOTS).map(|_| Cell::new(UNDEFINED)).collect(),
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
    /// before the realm exists. Stored in M5-P0; since M5-P3 (PROFILE6) `profile::records`
    /// reads the feedback mode.
    pub exec: Cell<crate::exec::ExecOptions>,
    /// The deopt side table (`deopt.rs`, V10-D32): code start -> deopt metadata and exit
    /// counters, filled by `NativeCx::install_b_with_deopt` (M5-P1 SPIKEC-5) and emptied of a
    /// code start by `uninstall_native`. Rust memory, no Value.
    #[cfg_attr(not(feature = "spike-native"), allow(dead_code))]
    // read by native.rs (spike-native)
    pub deopt: crate::deopt::SideTable,
    /// The fuse registry (`fuse.rs`, H31): fuse states and their weak dependents (M5-P1 FR0;
    /// `shape::note_change` calls its prototype hook). Rust memory.
    pub fuses: crate::fuse::Registry,
    /// The state of the sampled-call window jitter (`profile::budget_after_poll`, M5-P1
    /// PROFILE5): each poll advances it. 0 = not seeded yet (the next poll starts the fixed
    /// sequence); `profile::JITTER_OFF` = no jitter (the fixed reset to `BUDGET`).
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
            fuses: crate::fuse::Registry::new(&heap),
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
        self.shared.roots[root_slot(r)].get()
    }

    pub fn set_root(&self, r: Root, v: u64) {
        self.shared.roots[root_slot(r)].set(v);
    }

    /// Makes every current-realm root (the A2 ones before `Root::RealmKey`, and each appended
    /// realm root, [`APPENDED`]) undefined: the first step of a new realm's init.
    pub fn clear_realm_roots(&self) {
        for i in SLOT_MAP.realm_slots() {
            self.shared.roots[i].set(UNDEFINED);
        }
    }

    pub fn atom(&self, a: Atom) -> u64 {
        self.shared.roots[atom_slot(a)].get()
    }

    pub fn set_atom(&self, a: Atom, v: u64) {
        self.shared.roots[atom_slot(a)].set(v);
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

// ---- the root table form of A2 start (H13 "root slots", doc 13 C-18; M7-P1 A2-START) ----
//
// ```text
//  slot 0 .. 35          the `Root` values at A2 start (34 realm roots, then RealmKey)
//  slot 35 .. 71         the `atoms!` atoms at A2 start (atom id + 35)
//  slot 71 .. 327        the 256 single-character atoms (char code + 71)    = ROOT_SLOTS
//  slot 327 ..           APPENDED: each root or atom added after A2 start, in add order
// ```
//
// Unit code loads slot n at byte 8n of the table (H34 `RootIndex(n)`), so a slot number is
// the contract: from A2 start no slot moves, none is reused and none holds another item. A
// new `Root` value or a new atom does not take a slot inside a range above; it is appended to
// [`APPENDED`] and takes the next free slot at the end. The `Root` golden
// (`crates/ep9_unit/tests/golden/root-slots.txt`) records every slot, and is the same in every
// `ep9_vm` feature set: nothing here has a `cfg` (the build id names no feature, C-19).

/// The number of `Root` values at A2 start (`GlobalObject` ..= `RealmKey`). Root `r` below it
/// is in slot `r as usize`. Never change it.
const A2_ROOT_COUNT: usize = 35;
/// The realm roots at A2 start: the `Root` values below it (all but `RealmKey`).
const A2_REALM_ROOT_COUNT: usize = 34;
/// The number of `atoms!` atoms at A2 start (ids `0..A2_ATOM_COUNT`). Never change it.
const A2_ATOM_COUNT: usize = 36;

/// A root or an atom added after A2 start (an item of [`APPENDED`]), by its `Root` value or
/// atom id. Make one with [`Appended::realm_root`], [`Appended::root`] or [`Appended::atom`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // the list is empty until the first append
pub(crate) enum Appended {
    /// A root of the current realm: [`Runtime::clear_realm_roots`] clears it.
    RealmRoot(u8),
    /// An isolate-wide root (as `Root::RealmKey`).
    Root(u8),
    /// An atom of the `atoms!` list.
    Atom(u16),
}

#[cfg_attr(not(test), allow(dead_code))] // the list is empty until the first append
impl Appended {
    /// A realm root added after A2 start.
    pub(crate) const fn realm_root(r: Root) -> Appended {
        Appended::RealmRoot(r as u8)
    }

    /// An isolate-wide root added after A2 start.
    pub(crate) const fn root(r: Root) -> Appended {
        Appended::Root(r as u8)
    }

    /// An atom added after A2 start.
    pub(crate) const fn atom(a: Atom) -> Appended {
        Appended::Atom(a.id())
    }
}

/// The roots and atoms added after A2 start, in add order: item `i` is in slot
/// `ROOT_SLOTS + i`. Append only: never reorder, remove or reuse an item; a retired root keeps
/// its item (its slot then holds undefined). A new `Root` value goes after the last one in the
/// enum (and `ROOT_COUNT` names it); a new atom goes at the end of `atoms!`.
const APPENDED: &[Appended] = &[];

/// The slots of the whole table: the A2 ranges and [`APPENDED`].
const TABLE_SLOTS: usize = ROOT_SLOTS + APPENDED.len();

/// A root table layout: the A2 counts and the appended items. The isolate uses [`SLOT_MAP`];
/// the tests build other maps (a scratch append) to prove that an append moves no slot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlotMap<'a> {
    pub roots: usize,
    pub realm_roots: usize,
    pub atoms: usize,
    pub appended: &'a [Appended],
}

/// The layout of this build.
pub(crate) const SLOT_MAP: SlotMap<'static> = SlotMap {
    roots: A2_ROOT_COUNT,
    realm_roots: A2_REALM_ROOT_COUNT,
    atoms: A2_ATOM_COUNT,
    appended: APPENDED,
};

impl SlotMap<'_> {
    /// The first slot of the single-character atoms.
    pub(crate) const fn char_base(&self) -> usize {
        self.roots + self.atoms
    }

    /// The first slot of the appended range.
    pub(crate) const fn appended_base(&self) -> usize {
        self.char_base() + 256
    }

    /// The number of slots.
    pub(crate) const fn len(&self) -> usize {
        self.appended_base() + self.appended.len()
    }

    /// The slot of the root with value `r`, or None if the map has no such root.
    pub(crate) const fn root_slot(&self, r: usize) -> Option<usize> {
        if r < self.roots {
            return Some(r);
        }
        let mut i = 0;
        while i < self.appended.len() {
            if let Appended::RealmRoot(x) | Appended::Root(x) = self.appended[i]
                && x as usize == r
            {
                return Some(self.appended_base() + i);
            }
            i += 1;
        }
        None
    }

    /// The slot of the atom with id `a`, or None if the map has no such atom.
    pub(crate) const fn atom_slot(&self, a: usize) -> Option<usize> {
        if a < self.atoms {
            return Some(self.roots + a);
        }
        let mut i = 0;
        while i < self.appended.len() {
            if let Appended::Atom(x) = self.appended[i]
                && x as usize == a
            {
                return Some(self.appended_base() + i);
            }
            i += 1;
        }
        None
    }

    /// The slots of the realm roots, in slot order: the A2 range, then each appended realm
    /// root.
    pub(crate) fn realm_slots(&self) -> impl Iterator<Item = usize> + '_ {
        let base = self.appended_base();
        (0..self.realm_roots).chain(
            self.appended
                .iter()
                .enumerate()
                .filter(|(_, x)| matches!(x, Appended::RealmRoot(_)))
                .map(move |(i, _)| base + i),
        )
    }
}

/// The slot of each root added after A2 start, by `Root` value - `A2_ROOT_COUNT`.
const APPENDED_ROOT_SLOTS: [usize; ROOT_COUNT - A2_ROOT_COUNT] = {
    let mut out = [0; ROOT_COUNT - A2_ROOT_COUNT];
    let mut r = A2_ROOT_COUNT;
    while r < ROOT_COUNT {
        out[r - A2_ROOT_COUNT] = match SLOT_MAP.root_slot(r) {
            Some(s) => s,
            None => panic!("a Root added after A2 start is not in APPENDED"),
        };
        r += 1;
    }
    out
};

/// The slot of each atom added after A2 start, by atom id - `A2_ATOM_COUNT`.
const APPENDED_ATOM_SLOTS: [usize; Atom::ALL.len() - A2_ATOM_COUNT] = {
    let mut out = [0; Atom::ALL.len() - A2_ATOM_COUNT];
    let mut a = A2_ATOM_COUNT;
    while a < Atom::ALL.len() {
        out[a - A2_ATOM_COUNT] = match SLOT_MAP.atom_slot(a) {
            Some(s) => s,
            None => panic!("an atom added after A2 start is not in APPENDED"),
        };
        a += 1;
    }
    out
};

// Each appended item names a new root or atom, once (the root range is empty until one is).
#[allow(clippy::impossible_comparisons)]
const _: () = {
    assert!(ROOT_SLOTS == SLOT_MAP.appended_base() && TABLE_SLOTS == SLOT_MAP.len());
    assert!(A2_REALM_ROOT_COUNT == REALM_ROOT_COUNT && A2_ROOT_COUNT <= ROOT_COUNT);
    assert!(A2_ATOM_COUNT <= Atom::ALL.len());
    let mut i = 0;
    while i < APPENDED.len() {
        let ok = match APPENDED[i] {
            Appended::RealmRoot(r) | Appended::Root(r) => {
                let r = r as usize;
                r >= A2_ROOT_COUNT
                    && r < ROOT_COUNT
                    && APPENDED_ROOT_SLOTS[r - A2_ROOT_COUNT] == ROOT_SLOTS + i
            }
            Appended::Atom(a) => {
                let a = a as usize;
                a >= A2_ATOM_COUNT
                    && a < Atom::ALL.len()
                    && APPENDED_ATOM_SLOTS[a - A2_ATOM_COUNT] == ROOT_SLOTS + i
            }
        };
        assert!(
            ok,
            "an APPENDED item names no new root or atom, or names one twice"
        );
        i += 1;
    }
};

/// The number of `Root` values ([`ROOT_COUNT`]). The `count!` list names each `Root` once, in
/// value order. It makes a match with no wildcard arm, so a `Root` that is not in the list fails
/// to build; the count is the list length, and a name out of order or named twice fails the
/// const assert. A root appended after A2 start goes last in this list; then it needs its
/// [`APPENDED`] item and its line in the `Root` golden. (M7-P1 JIT reviewer F2 and MC-w31-1: the
/// count read one arm, so a new root in another arm built and panicked at its first use.)
const fn root_count() -> usize {
    macro_rules! count {
        ($(Root::$r:ident)|+) => {{
            const fn _every(r: Root) {
                match r {
                    $(Root::$r)|+ => {}
                }
            }
            let all = [$(Root::$r),+];
            let mut i = 0;
            while i < all.len() {
                assert!(all[i] as usize == i, "count!: not each Root once, in value order");
                i += 1;
            }
            all.len()
        }};
    }
    count! {
        // The 35 roots of A2 start (`A2_ROOT_COUNT`), in value order.
        Root::GlobalObject | Root::Lexical
        | Root::ObjectPrototype | Root::FunctionPrototype | Root::ArrayPrototype
        | Root::StringPrototype | Root::NumberPrototype | Root::BooleanPrototype
        | Root::ErrorPrototype | Root::TypeErrorPrototype | Root::RangeErrorPrototype
        | Root::ReferenceErrorPrototype | Root::SyntaxErrorPrototype
        | Root::EvalErrorPrototype | Root::UriErrorPrototype
        | Root::ArrayIteratorPrototype
        // week 4
        | Root::IteratorPrototype
        | Root::ObjectConstructor | Root::FunctionConstructor | Root::ArrayConstructor
        | Root::NumberConstructor | Root::BooleanConstructor | Root::StringConstructor
        | Root::SymbolConstructor | Root::ErrorConstructor | Root::TypeErrorConstructor
        | Root::RangeErrorConstructor | Root::ReferenceErrorConstructor
        | Root::SyntaxErrorConstructor | Root::EvalErrorConstructor
        | Root::UriErrorConstructor
        | Root::Math
        // M3-P0, M6-P0
        | Root::Realm | Root::RealmFuses
        // M3-P1: isolate-wide
        | Root::RealmKey
        // A root appended after A2 start goes here, with its `APPENDED` item.
    }
}

/// The slot of root `r` ([`SLOT_MAP`]). A root of A2 start is its own slot; the compare folds
/// away while no root is appended (a `Root` value is below `ROOT_COUNT`).
#[inline(always)]
fn root_slot(r: Root) -> usize {
    let v = r as usize;
    if v < A2_ROOT_COUNT {
        v
    } else {
        APPENDED_ROOT_SLOTS[v - A2_ROOT_COUNT]
    }
}

/// The slot of atom `a` ([`SLOT_MAP`]).
#[inline(always)]
fn atom_slot(a: Atom) -> usize {
    let v = usize::from(a.id());
    if v < A2_ATOM_COUNT {
        ATOM_ROOTS + v
    } else {
        APPENDED_ATOM_SLOTS[v - A2_ATOM_COUNT]
    }
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
        assert_eq!(ROOT_COUNT, 35); // 34 before M6-P0 (Root::RealmFuses)
        assert_eq!(REALM_ROOT_COUNT, 34);
        assert_eq!(ROOT_SLOTS, 35 + A2_ATOM_COUNT + 256);
        assert_eq!(TABLE_SLOTS, ROOT_SLOTS + APPENDED.len());
        assert_eq!(ErrorKind::UriError.name(), "URIError");
        assert_eq!(crate::frame::HEADER, 8);
    }

    // ---- the `Root` golden (H13 "root slots" item (5); doc 13 C-18; M7-P1 A2-START) ----

    /// Every `Root`, in value order (a new `Root` fails to build in `root_count` until it has
    /// an arm there, then makes this list one item too short until it is named here).
    const ROOTS: [Root; ROOT_COUNT] = [
        Root::GlobalObject,
        Root::Lexical,
        Root::ObjectPrototype,
        Root::FunctionPrototype,
        Root::ArrayPrototype,
        Root::StringPrototype,
        Root::NumberPrototype,
        Root::BooleanPrototype,
        Root::ErrorPrototype,
        Root::TypeErrorPrototype,
        Root::RangeErrorPrototype,
        Root::ReferenceErrorPrototype,
        Root::SyntaxErrorPrototype,
        Root::EvalErrorPrototype,
        Root::UriErrorPrototype,
        Root::ArrayIteratorPrototype,
        Root::IteratorPrototype,
        Root::ObjectConstructor,
        Root::FunctionConstructor,
        Root::ArrayConstructor,
        Root::NumberConstructor,
        Root::BooleanConstructor,
        Root::StringConstructor,
        Root::SymbolConstructor,
        Root::ErrorConstructor,
        Root::TypeErrorConstructor,
        Root::RangeErrorConstructor,
        Root::ReferenceErrorConstructor,
        Root::SyntaxErrorConstructor,
        Root::EvalErrorConstructor,
        Root::UriErrorConstructor,
        Root::Math,
        Root::Realm,
        Root::RealmFuses,
        Root::RealmKey,
    ];

    /// The golden file. It is in `ep9_unit` (the unit container is the reader of root slots);
    /// `EP9_BLESS=1` rewrites it.
    fn golden_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ep9_unit/tests/golden/root-slots.txt")
    }

    /// The name of root value `r` (a value no `Root` has is a scratch root of a test map).
    fn root_name(r: usize) -> String {
        ROOTS
            .get(r)
            .map_or_else(|| format!("scratch-root-{r}"), |x| format!("{x:?}"))
    }

    /// The name of atom id `a`.
    fn atom_name(a: usize) -> String {
        Atom::ALL.get(a).map_or_else(
            || format!("scratch-atom-{a} \"\""),
            |x| format!("{x:?} {:?}", x.text()),
        )
    }

    /// The golden text of `map`: one line per slot, `<slot> <kind> <id> <name>`.
    fn slot_text(map: &SlotMap<'_>) -> String {
        use std::fmt::Write as _;
        let mut s = String::from(
            "# ep9 isolate root table: one line per slot, <slot> <kind> <id> <name>. H13 \"root slots\"\n\
             # (doc 13 C-18): from A2 start a slot never moves; a new root or atom is appended at\n\
             # the end (runtime.rs APPENDED). Kinds: realm-root, root, atom (id + text), char (code).\n\
             # Generated by ep9_vm runtime::tests (EP9_BLESS=1); the same in every feature set.\n",
        );
        for r in 0..map.roots {
            let kind = if r < map.realm_roots {
                "realm-root"
            } else {
                "root"
            };
            let _ = writeln!(s, "{r} {kind} {r} {}", root_name(r));
        }
        for a in 0..map.atoms {
            let _ = writeln!(s, "{} atom {a} {}", map.roots + a, atom_name(a));
        }
        for c in 0..256 {
            let _ = writeln!(s, "{} char {c} {c:#04x}", map.char_base() + c);
        }
        for (i, x) in map.appended.iter().enumerate() {
            let slot = map.appended_base() + i;
            let _ = match *x {
                Appended::RealmRoot(r) => {
                    writeln!(s, "{slot} realm-root {r} {}", root_name(r.into()))
                }
                Appended::Root(r) => writeln!(s, "{slot} root {r} {}", root_name(r.into())),
                Appended::Atom(a) => writeln!(s, "{slot} atom {a} {}", atom_name(a.into())),
            };
        }
        s
    }

    /// The `Root` golden: every slot of this build's table, the same in the jit and the
    /// no-jit builds (run it in both; H13 "root slots" item (5)).
    #[test]
    fn the_root_golden_names_every_slot() {
        let actual = slot_text(&SLOT_MAP);
        let path = golden_path();
        if std::env::var_os("EP9_BLESS").is_some() {
            std::fs::write(&path, &actual).unwrap();
            return;
        }
        let golden = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", path.display()))
            .replace("\r\n", "\n");
        assert_eq!(
            golden,
            actual,
            "the root slots differ from {}",
            path.display()
        );
        assert_eq!(
            actual.lines().filter(|l| !l.starts_with('#')).count(),
            TABLE_SLOTS
        );
    }

    /// Each accessor reads the slot the golden names, and the realm roots are the ones that
    /// `clear_realm_roots` clears.
    #[test]
    fn the_accessors_use_the_slot_map() {
        for r in ROOTS {
            assert_eq!(Some(root_slot(r)), SLOT_MAP.root_slot(r as usize), "{r:?}");
        }
        for &a in Atom::ALL {
            assert_eq!(
                Some(atom_slot(a)),
                SLOT_MAP.atom_slot(a.id().into()),
                "{a:?}"
            );
        }
        assert_eq!(SINGLE_CHAR_ROOTS, SLOT_MAP.char_base());
        let realm: Vec<usize> = SLOT_MAP.realm_slots().collect();
        let want: Vec<usize> = ROOTS
            .iter()
            .filter(|&&r| r != Root::RealmKey)
            .map(|&r| root_slot(r))
            .collect();
        assert_eq!(realm, want);
        // Every slot holds one item.
        let mut seen = std::collections::HashSet::new();
        for r in ROOTS {
            assert!(seen.insert(root_slot(r)));
        }
        for &a in Atom::ALL {
            assert!(seen.insert(atom_slot(a)));
        }
        for c in 0..256 {
            assert!(seen.insert(SINGLE_CHAR_ROOTS + c));
        }
        assert_eq!(seen.len(), TABLE_SLOTS);
        assert!(seen.iter().all(|&s| s < TABLE_SLOTS));
    }

    /// C-18 item (5): a scratch root appended at the end adds one golden line and changes
    /// none; so does a scratch atom (the case that moved the 256 character slots before A2
    /// start), and a realm root appended later is cleared with the A2 realm roots.
    #[test]
    fn an_append_adds_one_golden_line_and_moves_no_slot() {
        let base = slot_text(&SLOT_MAP);
        let mut appended = APPENDED.to_vec();
        let scratch_root = ROOT_COUNT as u8;
        let scratch_atom = Atom::ALL.len() as u16;
        let steps = [
            Appended::RealmRoot(scratch_root),
            Appended::Atom(scratch_atom),
            Appended::Root(scratch_root + 1),
        ];
        // The constructors name an item by its `Root` value or atom id.
        assert_eq!(Appended::realm_root(Root::Math), Appended::RealmRoot(31));
        assert_eq!(Appended::root(Root::RealmKey), Appended::Root(34));
        assert_eq!(Appended::atom(Atom::Callee), Appended::Atom(35));
        let mut before = base;
        for step in steps {
            appended.push(step);
            let map = SlotMap {
                appended: &appended,
                ..SLOT_MAP
            };
            let after = slot_text(&map);
            let new_line = after
                .strip_prefix(before.as_str())
                .unwrap_or_else(|| panic!("an append changed an existing golden line ({step:?})"));
            assert_eq!(new_line.lines().count(), 1, "{step:?}: {new_line:?}");
            assert!(
                new_line.starts_with(&format!("{} ", map.len() - 1)),
                "{new_line:?}"
            );
            // No item of A2 start moves.
            for r in 0..ROOT_COUNT {
                assert_eq!(map.root_slot(r), SLOT_MAP.root_slot(r));
            }
            for a in 0..Atom::ALL.len() {
                assert_eq!(map.atom_slot(a), SLOT_MAP.atom_slot(a));
            }
            assert_eq!(map.char_base(), SINGLE_CHAR_ROOTS);
            before = after;
        }
        let map = SlotMap {
            appended: &appended,
            ..SLOT_MAP
        };
        let base = TABLE_SLOTS;
        assert_eq!(map.root_slot(scratch_root.into()), Some(base));
        assert_eq!(map.atom_slot(scratch_atom.into()), Some(base + 1));
        assert_eq!(map.root_slot(usize::from(scratch_root) + 1), Some(base + 2));
        let realm: Vec<usize> = map.realm_slots().collect();
        let mut want: Vec<usize> = SLOT_MAP.realm_slots().collect();
        want.push(base);
        assert_eq!(
            realm, want,
            "the appended realm root is cleared with the others"
        );
    }

    /// MC-w31-1: `ROOT_COUNT` is the number of `Root` values. The `enum Root` source (one
    /// `Name,` line per value) and the `count!` list of `root_count` name the values of `ROOTS`,
    /// in order. A `Root` that is not in the `count!` list fails to build first: error E0004,
    /// "pattern `Root::Foo` not covered"; one named twice fails the `count!` const assert.
    #[test]
    fn root_count_is_the_number_of_root_values() {
        let src = include_str!("runtime.rs");
        let part = |start: &str, end: &str| {
            let (_, rest) = src.split_once(start).expect(start);
            rest.split_once(end).expect(end).0
        };
        let names = |text: &str, sep: char| -> Vec<String> {
            let code = text.lines().filter(|l| !l.trim_start().starts_with("//"));
            let code: String = code.collect::<Vec<_>>().join("\n");
            code.split(sep)
                .map(|n| n.trim().trim_start_matches("Root::").to_string())
                .filter(|n| !n.is_empty())
                .collect()
        };
        let in_enum = names(part("pub(crate) enum Root {\n", "\n}\n"), ',');
        let in_count = names(part("    count! {\n", "\n    }\n"), '|');
        let golden: Vec<String> = ROOTS.iter().map(|r| format!("{r:?}")).collect();
        assert_eq!(in_enum.len(), ROOT_COUNT, "{in_enum:?}");
        assert_eq!(in_enum, golden, "enum Root and ROOTS");
        assert_eq!(in_count, golden, "the count! list and ROOTS");
    }
}
