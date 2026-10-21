//! The isolate's runtime state: heap, VM stack, realm roots, loaded units, host functions.
//!
//! ```text
//! Isolate ──owns──> Box<Cx>  (interpreter: dispatch table @0, hot fields)   ──cx.rt──┐
//!         ──owns──> Box<Runtime> <───────────────────────────────────────────────────┘
//!                     heap: Heap ──root source──> Rc<Shared> (also held by Runtime)
//!                     shared: Rc<Shared> { VM stack, pending exception, realm roots }
//!                     units, host infos, registry, limits
//!                     shapes, atoms, symbols, builtins   (per-isolate state of shape.rs,
//!                                                         atoms.rs, symbol.rs, builtins.rs)
//! ```
//!
//! `Runtime` is only ever shared (`&Runtime`); every mutable field is a `Cell` or `RefCell`.
//! The interpreter holds `&mut Cx`, which never contains GC-visible state: a GC reads only
//! `Shared` (through the root source) and heap cells. So an opaque runtime call cannot reorder
//! a store that the GC needs (the `noalias` rule of `&mut`).

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ep9_heap::{CellPtr, GlobalHandle, Heap, KindId};
use ep9_value::Value;

use crate::atoms::AtomTable;
use crate::builtins::BuiltinState;
use crate::dispatch::RangeRegistry;
use crate::frame::{UNDEFINED, VmStack, int32_bits, native_ptr, slot};
use crate::shape::ShapeState;
use crate::symbol::SymbolState;
use crate::unit::{FunctionInfo, LoadedUnit};

macro_rules! atoms {
    ($($name:ident = $text:literal,)*) => {
        /// Strings the VM needs often. Created once per isolate and kept as roots.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub(crate) enum Atom { $($name,)* }

        impl Atom {
            pub(crate) const ALL: &'static [Atom] = &[$(Atom::$name,)*];
            pub(crate) const fn text(self) -> &'static str {
                match self { $(Atom::$name => $text,)* }
            }
        }
    };
}

atoms! {
    Empty = "",
    Length = "length",
    Prototype = "prototype",
    Constructor = "constructor",
    Name = "name",
    Message = "message",
    Stack = "stack",
    ToStringKey = "toString",
    ValueOf = "valueOf",
    Raw = "raw",
    ValueKey = "value",
    Done = "done",
    Undefined = "undefined",
    Null = "null",
    True = "true",
    False = "false",
    Object = "object",
    Boolean = "boolean",
    Number = "number",
    String = "string",
    Function = "function",
    Symbol = "symbol",
    Bigint = "bigint",
    NaN = "NaN",
    Infinity = "Infinity",
    Comma = ",",
    GlobalThis = "globalThis",
}

/// Realm intrinsics and other runtime roots (v0: one realm per isolate; H22 allows N).
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
    /// v0: the prototype of array and string iterators (one object for both).
    ArrayIteratorPrototype,
}

pub(crate) const ROOT_COUNT: usize = Root::ArrayIteratorPrototype as usize + 1;

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
            roots: (0..ROOT_COUNT + Atom::ALL.len())
                .map(|_| Cell::new(UNDEFINED))
                .collect(),
        }
    }

    /// Visits every precise root the VM owns: the live VM stack up to the top frame or the
    /// scratch top (whichever is higher), the pending exception, the pending message and the
    /// realm roots and atoms.
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
    /// FunctionInfos of host functions. Boxed: frames and function cells point at them.
    #[allow(clippy::vec_box)]
    pub infos: RefCell<Vec<Box<FunctionInfo>>>,
    /// The frame info of `[[Construct]]` stub frames (`OP_CONSTRUCT_RETURN`).
    pub construct_stub: Box<FunctionInfo>,
    pub registry: RefCell<RangeRegistry>,
    /// Lowest native stack address the VM lets JS entries and host calls reach.
    pub native_limit: usize,
    pub stack_trace_limit: usize,
    /// Set by `TerminateHandle::terminate` (another thread); read at interrupt polls.
    pub terminate: Arc<AtomicBool>,
    /// True while a termination unwinds (no handler catches it).
    pub terminating: Cell<bool>,
    /// Template objects per (FunctionInfo, constant index) (§13.2.8.4 GetTemplateObject).
    pub templates: RefCell<HashMap<(usize, u32), GlobalHandle>>,
    /// Shapes (H17): the prototype epoch and the shape roots.
    pub shapes: ShapeState,
    /// The atom table (H19). Shared with the heap's weak source; never borrowed across an
    /// allocation.
    #[allow(dead_code)] // wk4-stub
    pub atoms: Rc<RefCell<AtomTable>>,
    /// Symbols: the registry and the identity-hash PRNG.
    #[allow(dead_code)] // wk4-stub
    pub symbols: SymbolState,
    /// Builtin tables (H20).
    #[allow(dead_code)] // wk4-stub
    pub builtins: BuiltinState,
}

impl Runtime {
    pub fn new(
        heap: Heap,
        stack_slots: usize,
        native_limit: usize,
        stack_trace_limit: usize,
    ) -> Box<Runtime> {
        crate::object::register_kinds(&heap);
        let shared = Rc::new(Shared::new(stack_slots));
        let sh = Rc::clone(&shared);
        heap.add_root_source(move |t| sh.trace(t));
        let rt = Box::new(Runtime {
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
            terminate: Arc::new(AtomicBool::new(false)),
            terminating: Cell::new(false),
            templates: RefCell::new(HashMap::new()),
            shapes: ShapeState::default(),
            atoms: Rc::new(RefCell::new(AtomTable::default())),
            symbols: SymbolState::default(),
            builtins: BuiltinState::default(),
        });
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

    /// Allocates a cell. GC point. v0: out of memory is fatal (V8 aborts too); H23 adds the
    /// heap-limit callback.
    #[inline]
    pub fn alloc(&self, kind: KindId, size: usize) -> CellPtr {
        match self.heap.alloc(kind, size) {
            Ok(c) => c,
            Err(e) => panic!("ep9: heap allocation failed: {e:?}"),
        }
    }

    pub fn root(&self, r: Root) -> u64 {
        self.shared.roots[r as usize].get()
    }

    pub fn set_root(&self, r: Root, v: u64) {
        self.shared.roots[r as usize].set(v);
    }

    pub fn atom(&self, a: Atom) -> u64 {
        self.shared.roots[ROOT_COUNT + a as usize].get()
    }

    pub fn set_atom(&self, a: Atom, v: u64) {
        self.shared.roots[ROOT_COUNT + a as usize].set(v);
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
        assert_eq!(ROOT_COUNT, 16);
        assert_eq!(ErrorKind::UriError.name(), "URIError");
        assert_eq!(crate::frame::HEADER, 8);
    }
}
