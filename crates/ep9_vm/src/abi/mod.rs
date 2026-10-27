//! The builtin ABI v1 (H20, docs/adr/H20-builtin-abi.md): what a builtin or an embedder's
//! host function sees.
//!
//! ```text
//! JS call ──> call::call_host: push a host frame, clear the scratch top
//!               h(&mut Cx<'s>, this: Local<'s>, args: Args<'s> (= the frame pointer), new.target)
//!                 Cx: ObjectOps, conversions, calls back into JS, throw_*
//!               pop; restore the caller's scratch top; debug: Err <=> pending exception
//! ```
//!
//! | Rule | Statement | Check |
//! |---|---|---|
//! | R1 | A `Local` in a Rust local stays live across every call (conservative stack scan, H10) | GC-stress tests |
//! | R2 | Never put a `Local`, `Args`, `Rest`, `PropertyDescriptor`, `ArgsBuilder` or `ScratchList` into Rust heap memory, a static or a thread_local | `'s` and `!Send` (compiler); `cargo xtask gc-lint` (P2) |
//! | R3 | Every `&mut Cx` method may allocate and may run JS. No view into heap memory lives across one | views borrow `&Cx` |
//! | R4 | Return `Err(Throw)` only after a `throw_*` (the exception is pending); pass a callee's `Err` on unchanged | debug assert in `call::call_host` |
//! | R5 | A builtin creates objects in its callee's realm | `call::call_host` switches to the callee's realm (H22-Q3); `ep9_builtins` `tests/realm.rs` |
//! | R6 | `[[GetOwnProperty]]` reports the virtual properties (array `length` and elements, function `length`/`name`/`prototype`, String wrapper `length` and indices) with their spec attributes | `tests/abi.rs` |
//!
//! Lists of values: `ScratchList` (precise, LIFO) or a JS array; call arguments built in Rust
//! go on the VM stack through `ArgsBuilder` (above the frame top, under the scratch top).
//!
//! The P3 surface (H20 "Spec helpers"): symbols (`symbol_for`, `key_for`,
//! `symbol_description`, `descriptive_string`, `new_symbol`), primitive wrappers
//! (`new_wrapper`, `to_object`, `this_primitive_value`), functions (`set_function_name`,
//! `set_function_length`, `new_bound_function`, `initial_name`) and
//! `get_prototype_from_constructor`.
//!
//! Month-2 submodules (C1), one owner each: `index.rs` (index access) and `iter.rs` (the
//! iterator protocol), `slots.rs` (internal slots), `jobs.rs` (jobs), `script.rs`
//! (`evaluate_script`). Each adds an `impl Cx` block.
//!
//! Month-3 submodules (M3-P0 stubs, one owner each): `collections.rs` (Map and Set storage,
//! M3-P1 B-COLLECTIONS), `realm.rs` (realms, M3-P1 REALMS), `time.rs` (the clock and the local
//! time zone, M3-P1 B-DATE). The facts exporter is public here as [`export_facts`]
//! (`facts_export.rs`, M3-P3 FACTS-EXPORT; V8-A13).
//!
//! Month-4 submodules (M4-P0 stubs, one owner each): `regexp.rs` (RegExp on the ABI and its
//! [`MatchRecord`], M4-P2 RX0A, then M4-P3 REGEXP1) and `weakref.rs` (WeakRef and
//! FinalizationRegistry, M4-P4 WEAKREF). `Cx::math_random` (Math.random over the isolate's
//! xorshift128+ state, V9-D16) is in this file.

mod collections;
mod index;
mod iter;
mod jobs;
mod local;
mod realm;
mod regexp;
mod script;
mod slots;
mod time;
mod weakref;

use core::marker::PhantomData;

use ep9_heap::ScratchRoots;
use ep9_value::{JsResult, Throw};

pub use self::jobs::{
    PromiseCapability, PromiseRejectionOperation, PromiseRejectionTracker, PromiseState,
};
pub use self::local::{
    Downcast, FlatStr, JsArray, JsFunction, JsKey, JsObject, JsString, JsSymbol, JsValue, Local,
};
pub use self::regexp::MatchRecord;
pub use self::{collections::CollectionCursor, iter::IteratorRecord, slots::ObjectClass};
use crate::builtins::{self, IntrinsicId};
use crate::conv::{self, Hint};
use crate::descriptor::Desc;
use crate::error;
pub use crate::facts_export::export_facts;
use crate::frame::{CONSTRUCT_BIT, NULL, UNDEFINED, int32_of, slot};
use crate::interp;
use crate::key;
use crate::kinds;
use crate::object;
use crate::property;
use crate::runtime::{Atom, ErrorKind, Runtime};
use crate::string::{self, MAX_STRING_LENGTH};
use crate::{atoms, bound, call, symbol, wrapper};

/// A host function: a builtin or an embedder function (H20 signature). `this`, the
/// arguments and `new_target` (undefined for a call) are [`Local`]s of the call `'s`.
pub type HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>;

// H20 sizes: one word each, and `JsResult<Local>` uses the `NonZeroU64` niche of `Value`.
const _: () = {
    assert!(size_of::<Local<'static>>() == 8);
    assert!(size_of::<Args<'static>>() == 8);
    assert!(size_of::<JsResult<Local<'static>>>() == 8);
    assert!(size_of::<Cx<'static>>() == 8);
};

// ---- Args ----------------------------------------------------------------------------------

/// The arguments of a host call: the host frame pointer (one word). Argument `i` sits at
/// `fp + PARAM0 - i` (frame.rs); reading past [`Args::len`] gives undefined.
#[derive(Clone, Copy)]
pub struct Args<'s> {
    fp: *const u64,
    _scope: PhantomData<&'s ()>,
}

impl<'s> Args<'s> {
    /// # Safety
    /// `fp` is a live host frame (`call::push_frame` wrote its header and arguments) that
    /// outlives `'s`.
    pub(crate) unsafe fn from_fp(fp: *const u64) -> Args<'s> {
        Args {
            fp,
            _scope: PhantomData,
        }
    }

    #[inline(always)]
    fn word(&self, s: isize) -> u64 {
        // SAFETY: header and argument slots of the live host frame (constructor contract);
        // callers pass a header slot or an argument index below `len()`.
        unsafe { *self.fp.offset(s) }
    }

    /// The argument count of the call.
    #[inline]
    pub fn len(&self) -> usize {
        (int32_of(self.word(slot::ARGC)) as u32 & !CONSTRUCT_BIT) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Argument `i`, or undefined if `i >= len()`.
    #[inline]
    pub fn get(&self, i: usize) -> Local<'s> {
        if i < self.len() {
            Local::from_bits(self.word(slot::PARAM0 - i as isize))
        } else {
            Local::undefined()
        }
    }

    /// The receiver the caller passed (not bound: host functions see it as it is).
    pub fn this(&self) -> Local<'s> {
        Local::from_bits(self.word(slot::RECEIVER))
    }

    /// True for a `[[Construct]]` call.
    pub fn is_construct_call(&self) -> bool {
        int32_of(self.word(slot::ARGC)) as u32 & CONSTRUCT_BIT != 0
    }

    /// `new.target`: the constructor of a `[[Construct]]` call, undefined for a call. The
    /// frame keeps it in `slot::NEW_TARGET` (fp - 3, H14-Q1, H20-Q1); the construct-bit gate
    /// stays, so a host frame never reports anything else there as new.target.
    pub fn new_target(&self) -> Local<'s> {
        if self.is_construct_call() {
            Local::from_bits(self.word(slot::NEW_TARGET))
        } else {
            Local::undefined()
        }
    }

    /// The function object being called (its realm, bound state and closure data, H20 R5).
    pub fn callee(&self) -> Local<'s, JsFunction> {
        Local::from_bits(self.word(slot::CALLEE))
    }

    /// The arguments from `from` on (empty if `from >= len()`).
    pub fn rest(&self, from: usize) -> Rest<'s> {
        let len = self.len().saturating_sub(from);
        Rest {
            a0: self.fp.wrapping_offset(slot::PARAM0 - from as isize),
            len: len as u32,
            _scope: PhantomData,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Local<'s>> + use<'s> {
        let a = *self;
        (0..a.len()).map(move |i| a.get(i))
    }
}

/// A tail of the arguments of a host call ([`Args::rest`]), for [`Cx::call_args`].
#[derive(Clone, Copy)]
pub struct Rest<'s> {
    /// The first argument of the tail; argument `k` of the tail is at `a0 - k`.
    a0: *const u64,
    len: u32,
    _scope: PhantomData<&'s ()>,
}

impl<'s> Rest<'s> {
    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    fn bits(&self, k: u32) -> u64 {
        debug_assert!(k < self.len);
        // SAFETY: `a0 - k` for `k < len` is an argument slot of the live host frame (Args).
        unsafe { *self.a0.sub(k as usize) }
    }

    /// Argument `k` of the tail, or undefined if `k >= len()`.
    pub fn get(&self, k: usize) -> Local<'s> {
        if k < self.len() {
            Local::from_bits(self.bits(k as u32))
        } else {
            Local::undefined()
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Local<'s>> + use<'s> {
        let r = *self;
        (0..r.len()).map(move |k| r.get(k))
    }
}

// ---- property descriptors and keys ----------------------------------------------------------

/// A property descriptor (§6.2.6): every field optional.
#[derive(Clone, Copy, Debug, Default)]
pub struct PropertyDescriptor<'s> {
    pub value: Option<Local<'s>>,
    pub get: Option<Local<'s>>,
    pub set: Option<Local<'s>>,
    pub writable: Option<bool>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl<'s> PropertyDescriptor<'s> {
    /// A complete data descriptor.
    pub fn data(
        value: Local<'s>,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> PropertyDescriptor<'s> {
        PropertyDescriptor {
            value: Some(value),
            writable: Some(writable),
            enumerable: Some(enumerable),
            configurable: Some(configurable),
            ..PropertyDescriptor::default()
        }
    }

    /// IsAccessorDescriptor (§6.2.6.1).
    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// IsDataDescriptor (§6.2.6.2).
    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    fn to_desc(self) -> Desc {
        Desc {
            value: self.value.map(|v| v.bits()),
            get: self.get.map(|v| v.bits()),
            set: self.set.map(|v| v.bits()),
            writable: self.writable,
            enumerable: self.enumerable,
            configurable: self.configurable,
        }
    }

    fn from_desc(d: Desc) -> PropertyDescriptor<'s> {
        PropertyDescriptor {
            value: d.value.map(Local::from_bits),
            get: d.get.map(Local::from_bits),
            set: d.set.map(Local::from_bits),
            writable: d.writable,
            enumerable: d.enumerable,
            configurable: d.configurable,
        }
    }
}

/// Something that names a property: Rust text (interned on use), a key, a symbol, or a JS
/// string (interned on use). Interning is a GC point, as every `&mut Cx` call is.
pub trait IntoKey<'s> {
    fn into_key(self, cx: &mut Cx<'s>) -> Local<'s, JsKey>;
}

impl<'s> IntoKey<'s> for &str {
    fn into_key(self, cx: &mut Cx<'s>) -> Local<'s, JsKey> {
        cx.key(self)
    }
}

impl<'s> IntoKey<'s> for Local<'s, JsKey> {
    fn into_key(self, _: &mut Cx<'s>) -> Local<'s, JsKey> {
        self
    }
}

impl<'s> IntoKey<'s> for Local<'s, JsSymbol> {
    fn into_key(self, _: &mut Cx<'s>) -> Local<'s, JsKey> {
        self.into()
    }
}

impl<'s> IntoKey<'s> for Local<'s, JsString> {
    fn into_key(self, cx: &mut Cx<'s>) -> Local<'s, JsKey> {
        cx.intern(self)
    }
}

/// The preferred type of ToPrimitive (§7.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToPrimitiveHint {
    Default,
    Number,
    String,
}

/// A primitive type that has a wrapper object (§10.4.3, §21.1.4, §20.3.4, §20.4.4), for
/// [`Cx::this_primitive_value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveType {
    Number,
    Boolean,
    String,
    Symbol,
}

impl PrimitiveType {
    /// The type of `bits`, if it is a primitive with a wrapper.
    fn of(bits: u64) -> Option<PrimitiveType> {
        let v = Local::<JsValue>::from_bits(bits);
        if v.is_number() {
            Some(PrimitiveType::Number)
        } else if v.is_bool() {
            Some(PrimitiveType::Boolean)
        } else if v.is_string() {
            Some(PrimitiveType::String)
        } else if v.is_symbol() {
            Some(PrimitiveType::Symbol)
        } else {
            None
        }
    }

    /// The constructor name (`Number`).
    pub const fn name(self) -> &'static str {
        match self {
            PrimitiveType::Number => "Number",
            PrimitiveType::Boolean => "Boolean",
            PrimitiveType::String => "String",
            PrimitiveType::Symbol => "Symbol",
        }
    }

    /// The prototype of its wrapper objects (the default of its constructor).
    pub const fn prototype(self) -> IntrinsicId {
        match self {
            PrimitiveType::Number => IntrinsicId::NumberPrototype,
            PrimitiveType::Boolean => IntrinsicId::BooleanPrototype,
            PrimitiveType::String => IntrinsicId::StringPrototype,
            PrimitiveType::Symbol => IntrinsicId::SymbolPrototype,
        }
    }
}

// ---- lists ---------------------------------------------------------------------------------

/// A LIFO list of Values that Rust code builds (03-gc §2 `ScratchRoots`): precise roots
/// until it drops. Lists drop in reverse order of creation; out of order panics.
pub struct ScratchList<'s> {
    roots: ScratchRoots<'s>,
}

impl<'s> ScratchList<'s> {
    /// Appends `v`; returns its index.
    pub fn push(&self, v: impl Into<Local<'s>>) -> usize {
        self.roots.push(crate::runtime::val(v.into().bits()))
    }

    pub fn get(&self, i: usize) -> Local<'s> {
        Local::from_bits(self.roots.get(i).to_bits())
    }

    pub fn set(&self, i: usize, v: impl Into<Local<'s>>) {
        self.roots.set(i, crate::runtime::val(v.into().bits()));
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

/// Call arguments built in Rust, on the VM stack above the top frame (H20 ABI-3, \[I40\]): no
/// heap list. Each push writes the next slot and raises the stack's scratch top past it, so
/// the GC scans the arguments; a call from Rust starts its frame above them and restores
/// the scratch top on return. Builders nest LIFO: a push or a drop out of order panics.
pub struct ArgsBuilder<'s> {
    rt: &'s Runtime,
    /// The first argument slot.
    start: *mut u64,
    /// The scratch top before this builder (null: none).
    prev: *mut u64,
    len: u32,
}

impl<'s> ArgsBuilder<'s> {
    fn new(cx: &Cx<'s>) -> ArgsBuilder<'s> {
        let rt: &'s Runtime = cx.cx.rt();
        let start = call::top_of(cx.cx, rt.stack().fp());
        ArgsBuilder {
            rt,
            start,
            prev: rt.stack().scratch_top(),
            len: 0,
        }
    }

    /// The scratch top this builder expects: its end, or the previous top while empty.
    fn end(&self) -> *mut u64 {
        if self.len == 0 {
            self.prev
        } else {
            self.start.wrapping_add(self.len as usize)
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Appends `v`. Throws RangeError if the VM stack is full. Panics if another builder
    /// pushed since this one's last push (LIFO).
    pub fn push(&mut self, cx: &mut Cx<'s>, v: impl Into<Local<'s>>) -> JsResult<()> {
        let stack = self.rt.stack();
        assert!(
            stack.scratch_top() == self.end(),
            "ArgsBuilder used out of LIFO order"
        );
        let slot = self.start.wrapping_add(self.len as usize);
        if slot.addr() >= cx.cx.stack_limit.addr() {
            return Err(cx.throw_range_error("Maximum call stack size exceeded"));
        }
        // SAFETY: `slot` is below the VM stack end (checked) and above the top frame and
        // every scratch Value below this builder; nothing else uses it while the builder is
        // the innermost (asserted above). It holds a valid Value before the scratch top
        // covers it.
        unsafe { *slot = v.into().bits() };
        stack.set_scratch_top(slot.wrapping_add(1));
        self.len += 1;
        Ok(())
    }

    /// Argument `i` (undefined if `i >= len()`).
    pub fn get(&self, i: usize) -> Local<'s> {
        if i < self.len() {
            Local::from_bits(self.bits(i as u32))
        } else {
            Local::undefined()
        }
    }

    #[inline]
    fn bits(&self, i: u32) -> u64 {
        debug_assert!(i < self.len);
        // SAFETY: slot `start + i` (`i < len`) was written by `push` and is still under the
        // scratch top (LIFO).
        unsafe { *self.start.add(i as usize) }
    }
}

impl Drop for ArgsBuilder<'_> {
    fn drop(&mut self) {
        let stack = self.rt.stack();
        if stack.scratch_top() != self.end() {
            if !std::thread::panicking() {
                panic!("ArgsBuilder dropped out of LIFO order");
            }
            return;
        }
        if self.prev.is_null() {
            stack.clear_scratch();
        } else {
            stack.set_scratch_top(self.prev);
        }
    }
}

/// HasOwnProperty(O, P) (§7.3.13) on a routed kind: its `[[GetOwnProperty]]` (the hook may
/// throw) is not undefined. Out of line, so [`Cx::has_own_property`] keeps one kind test.
/// GC point.
#[inline(never)]
fn routed_has_own_property(cx: &mut interp::Cx, o: u64, k: u64) -> JsResult<bool> {
    Ok(property::get_own_property(cx, o, k)?.is_some())
}

// ---- Cx ------------------------------------------------------------------------------------

/// What a host function can do with the isolate (H20). It exposes no `&Runtime`: builtins
/// reach the VM only through these methods. Every `&mut self` method may allocate and may run
/// JS (R3).
pub struct Cx<'s> {
    cx: &'s mut interp::Cx,
}

impl<'s> Cx<'s> {
    pub(crate) fn new(cx: &'s mut interp::Cx) -> Cx<'s> {
        Cx { cx }
    }

    /// The interpreter context, for builtins inside ep9_vm.
    pub(crate) fn inner(&mut self) -> &mut interp::Cx {
        self.cx
    }

    #[inline(always)]
    fn rt(&self) -> &'s Runtime {
        self.cx.rt()
    }

    #[inline(always)]
    fn local<T>(b: u64) -> Local<'s, T> {
        Local::from_bits(b)
    }

    /// The key of a static atom (no lookup).
    fn atom(&self, a: Atom) -> Local<'s, JsKey> {
        Self::local(self.rt().atom(a))
    }

    /// The result of a `[[Construct]]` from Rust. `call::construct_with` gives undefined only
    /// for a derived constructor that returned without initializing `this`: the ReferenceError
    /// of GetThisBinding (§9.1.1.3.4).
    fn constructed(&mut self, r: JsResult<u64>) -> JsResult<Local<'s, JsObject>> {
        let r = r?;
        if kinds::is_object(r) {
            return Ok(Self::local(r));
        }
        Err(self.throw_error(
            ErrorKind::ReferenceError,
            "Must call super constructor in derived class before accessing 'this' or \
             returning from derived constructor",
        ))
    }

    // ---- exceptions (R4) ----

    /// Makes `v` the pending exception.
    pub fn throw(&mut self, v: impl Into<Local<'s>>) -> Throw {
        self.rt().throw_value(v.into().bits());
        Throw
    }

    /// Creates an error of `kind` and makes it the pending exception.
    pub fn throw_error(&mut self, kind: ErrorKind, message: &str) -> Throw {
        error::throw_error(self.cx, kind, message)
    }

    pub fn throw_type_error(&mut self, message: &str) -> Throw {
        self.throw_error(ErrorKind::TypeError, message)
    }

    pub fn throw_range_error(&mut self, message: &str) -> Throw {
        self.throw_error(ErrorKind::RangeError, message)
    }

    /// The catchable `"not implemented in ep9 v0: <what>"` Error.
    pub fn not_implemented(&mut self, what: &str) -> Throw {
        error::not_implemented(self.cx, what)
    }

    // ---- calls ----

    /// Call(F, thisArg, args) (§7.3.14).
    pub fn call(
        &mut self,
        f: impl Into<Local<'s>>,
        this: impl Into<Local<'s>>,
        args: &[Local<'s>],
    ) -> JsResult<Local<'s>> {
        let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
        let (f, this) = (f.into().bits(), this.into().bits());
        call::call_with(
            self.cx,
            f,
            this,
            argc,
            |i| args[i as usize].bits(),
            UNDEFINED,
        )
        .map(Self::local)
    }

    /// Call(F, thisArg, rest) with a tail of this call's arguments: one copy, into the callee
    /// frame (H20-Q3: a copy-free forward needs a tail-call host path, M1).
    pub fn call_args(
        &mut self,
        f: impl Into<Local<'s>>,
        this: impl Into<Local<'s>>,
        args: Rest<'s>,
    ) -> JsResult<Local<'s>> {
        let (f, this) = (f.into().bits(), this.into().bits());
        call::call_with(self.cx, f, this, args.len, |k| args.bits(k), UNDEFINED).map(Self::local)
    }

    /// Call(F, thisArg, list) with arguments from an [`ArgsBuilder`] (consumed).
    pub fn call_built(
        &mut self,
        f: impl Into<Local<'s>>,
        this: impl Into<Local<'s>>,
        args: ArgsBuilder<'s>,
    ) -> JsResult<Local<'s>> {
        let (f, this) = (f.into().bits(), this.into().bits());
        let r = call::call_with(self.cx, f, this, args.len, |i| args.bits(i), UNDEFINED);
        drop(args);
        r.map(Self::local)
    }

    /// Construct(F, args, newTarget) (§7.3.15). `new_target` is a constructor; pass `f`
    /// for a plain `new f(...args)`.
    pub fn construct(
        &mut self,
        f: impl Into<Local<'s>>,
        args: &[Local<'s>],
        new_target: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
        let (f, nt) = (f.into().bits(), new_target.into().bits());
        let r = call::construct_with(self.cx, f, argc, |i| args[i as usize].bits(), nt);
        self.constructed(r)
    }

    /// Construct(F, rest, newTarget) with a tail of this call's arguments.
    pub fn construct_args(
        &mut self,
        f: impl Into<Local<'s>>,
        args: Rest<'s>,
        new_target: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let (f, nt) = (f.into().bits(), new_target.into().bits());
        let r = call::construct_with(self.cx, f, args.len, |k| args.bits(k), nt);
        self.constructed(r)
    }

    /// Construct(F, list, newTarget) with arguments from an [`ArgsBuilder`] (consumed).
    pub fn construct_built(
        &mut self,
        f: impl Into<Local<'s>>,
        args: ArgsBuilder<'s>,
        new_target: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let (f, nt) = (f.into().bits(), new_target.into().bits());
        let r = call::construct_with(self.cx, f, args.len, |i| args.bits(i), nt);
        drop(args);
        self.constructed(r)
    }

    /// A new empty [`ArgsBuilder`] above the current top frame and scratch Values.
    pub fn args_builder(&self) -> ArgsBuilder<'s> {
        ArgsBuilder::new(self)
    }

    /// A new empty [`ScratchList`].
    pub fn scratch_list(&self) -> ScratchList<'s> {
        ScratchList {
            roots: self.rt().heap.scratch_roots(),
        }
    }

    // ---- keys and strings ----

    /// The property key for the text `s` (an atom, H19).
    pub fn key(&mut self, s: &str) -> Local<'s, JsKey> {
        Self::local(atoms::intern_str(self.rt(), s))
    }

    /// The property key (atom) for the string `s`. A rope or slice is flattened or copied
    /// first, so a builtin never makes a rope or slice atom (`atoms::atomize`, H19-Q1, Q4). A
    /// GC point, as every `&mut Cx` call is.
    pub fn intern(&mut self, s: Local<'s, JsString>) -> Local<'s, JsKey> {
        Self::local(atoms::atomize(self.rt(), s.bits()))
    }

    /// A new string from Rust text. RangeError past the string length limit (2^29 - 24).
    pub fn new_string(&mut self, s: &str) -> JsResult<Local<'s, JsString>> {
        // UTF-16 length <= UTF-8 length; count exactly only near the limit.
        if s.len() > MAX_STRING_LENGTH && s.encode_utf16().count() > MAX_STRING_LENGTH {
            return Err(self.throw_range_error("Invalid string length"));
        }
        Ok(Self::local(object::new_string(self.rt(), s)))
    }

    /// A new string from UTF-16 code units (lone surrogates allowed).
    pub fn new_string_utf16(&mut self, units: &[u16]) -> JsResult<Local<'s, JsString>> {
        if units.len() > MAX_STRING_LENGTH {
            return Err(self.throw_range_error("Invalid string length"));
        }
        Ok(Self::local(object::new_string_utf16(self.rt(), units)))
    }

    /// The length of `s` in UTF-16 code units.
    pub fn string_len(&self, s: impl Into<Local<'s, JsString>>) -> usize {
        object::str_len(s.into().bits())
    }

    /// The flat form of `s`, whose code units can be read (H19). A GC point once strings
    /// can be ropes.
    pub fn flatten(&mut self, s: impl Into<Local<'s, JsString>>) -> JsResult<Local<'s, FlatStr>> {
        Ok(Self::local(string::flatten(self.cx, s.into().bits())))
    }

    /// Code unit `i` of the flat string `s`, or `None` past its end.
    pub fn code_unit_at(&self, s: Local<'s, FlatStr>, i: usize) -> Option<u16> {
        let v = string::view(self.cx, s.bits());
        (i < v.len()).then(|| v.at(i))
    }

    /// ToString(v) as Rust text (lone surrogates become U+FFFD).
    pub fn to_rust_string(&mut self, v: impl Into<Local<'s>>) -> JsResult<String> {
        conv::to_rust_string(self.cx, v.into().bits())
    }

    // ---- creation ----

    /// The global object of the current realm (in a host function: the function's own realm,
    /// H20 R5).
    pub fn global(&self) -> Local<'s, JsObject> {
        Self::local(self.rt().global())
    }

    /// An intrinsic object of the current realm (§6.1.7.4). Panics if no table made it (a
    /// table names only intrinsics that exist: `builtins::check_tables`).
    pub fn intrinsic(&self, id: IntrinsicId) -> Local<'s, JsObject> {
        let v = builtins::intrinsic(self.rt(), id);
        assert!(kinds::is_object(v), "{} is not installed", id.name());
        Self::local(v)
    }

    /// Does the realm have the intrinsic `id`? (The core ones always; the others when a
    /// table made them.)
    pub fn has_intrinsic(&self, id: IntrinsicId) -> bool {
        kinds::is_object(builtins::intrinsic(self.rt(), id))
    }

    /// OrdinaryObjectCreate(%Object.prototype%).
    pub fn new_object(&mut self) -> Local<'s, JsObject> {
        Self::local(object::new_plain_object(self.rt()))
    }

    /// OrdinaryObjectCreate(proto); `proto` is an object or null.
    pub fn new_object_with_proto(&mut self, proto: impl Into<Local<'s>>) -> Local<'s, JsObject> {
        let p = proto.into().bits();
        assert!(
            p == NULL || kinds::is_object(p),
            "a prototype is an object or null"
        );
        Self::local(object::new_object(self.rt(), p))
    }

    /// ArrayCreate(0) (§10.4.2.2).
    pub fn new_array(&mut self) -> Local<'s, JsArray> {
        Self::local(object::new_array(self.rt(), 0))
    }

    /// CreateArrayFromList (§7.3.18).
    pub fn create_array_from_list(&mut self, items: &[Local<'s>]) -> JsResult<Local<'s, JsArray>> {
        let rt = self.rt();
        if items.len() > object::arr::MAX_LENGTH as usize {
            return Err(self.throw_range_error("Invalid array length"));
        }
        let a = object::new_array(rt, items.len());
        for (i, v) in items.iter().enumerate() {
            object::array_set(rt, a, i as u32, v.bits());
        }
        Ok(Self::local(a))
    }

    /// A new primitive wrapper object (§10.4.3 String exotic, or a Number, Boolean or Symbol
    /// object) of the primitive `value` with prototype `proto` (`wrapper::new_wrapper`).
    /// TypeError if `value` has no wrapper type (undefined, null, an object). The wrapper must
    /// hold `value` as its `[[...Data]]`: while `wrapper.rs` is the P0 stub (a plain object),
    /// this throws "not implemented" rather than return an object without the primitive.
    pub fn new_wrapper(
        &mut self,
        proto: impl Into<Local<'s, JsObject>>,
        value: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let (p, v) = (proto.into().bits(), value.into().bits());
        if PrimitiveType::of(v).is_none() {
            let msg = format!("Cannot wrap {} in an object", conv::describe(v));
            return Err(self.throw_type_error(&msg));
        }
        let w = wrapper::new_wrapper(self.rt(), p, v);
        if wrapper::wrapper_value(w).is_none_or(|d| !property::same_value(d, v)) {
            return Err(self.not_implemented("wrapper objects for primitives"));
        }
        Ok(Self::local(w))
    }

    /// A new symbol with the description `description` (a string or undefined): `Symbol()`
    /// after its ToString step (§20.4.1.1).
    pub fn new_symbol(
        &mut self,
        description: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsSymbol>> {
        let d = description.into();
        if !d.is_undefined() && !d.is_string() {
            return Err(self.throw_type_error("a symbol description is a string or undefined"));
        }
        Ok(Self::local(symbol::new_symbol(self.rt(), d.bits())))
    }

    // ---- symbols (§20.4) over symbol.rs ----

    /// `Symbol.for(key)` steps 2-6 (§20.4.2.2): the registry's symbol for the string `key`,
    /// or a new registered symbol whose description is `key`.
    pub fn symbol_for(
        &mut self,
        key: impl Into<Local<'s, JsString>>,
    ) -> JsResult<Local<'s, JsSymbol>> {
        let key = key.into().bits();
        Ok(Self::local(symbol::symbol_for(self.cx, key)))
    }

    /// KeyForSymbol (§20.4.5.1): the registry key of `sym`, or `None` if `sym` is not
    /// registered (`Symbol.keyFor`).
    pub fn key_for(&self, sym: Local<'s, JsSymbol>) -> Option<Local<'s, JsString>> {
        let k = symbol::key_for(sym.bits());
        (k != UNDEFINED).then(|| Self::local(k))
    }

    /// `[[Description]]` of `sym` (§20.4.3.2): a string or undefined.
    pub fn symbol_description(&self, sym: Local<'s, JsSymbol>) -> Local<'s> {
        Self::local(symbol::description(sym.bits()))
    }

    /// SymbolDescriptiveString (§20.4.3.3.1): "Symbol(" + description + ")". RangeError past
    /// the string length limit.
    pub fn descriptive_string(
        &mut self,
        sym: Local<'s, JsSymbol>,
    ) -> JsResult<Local<'s, JsString>> {
        symbol::descriptive_string(self.cx, sym.bits()).map(Self::local)
    }

    // ---- functions ----

    /// SetFunctionName(F, name, prefix) (§10.2.9): defines `F.name` {w: false, e: false,
    /// c: true}. A symbol `name` becomes `"[description]"` (`""` without one); a `prefix`
    /// ("get", "set", "bound") goes in front with a space. TypeError if `name` is not a string
    /// or a symbol, or if `F.name` cannot be defined.
    pub fn set_function_name(
        &mut self,
        f: impl Into<Local<'s, JsObject>>,
        name: impl Into<Local<'s>>,
        prefix: Option<&str>,
    ) -> JsResult<()> {
        let (f, name) = (f.into(), name.into());
        let mut text = if let Some(sym) = name.as_symbol() {
            let d = self.symbol_description(sym);
            match d.as_string() {
                None => Self::local(self.rt().atom(Atom::Empty)),
                Some(d) => {
                    let open = self.new_string("[")?;
                    let close = self.new_string("]")?;
                    let t = self.concat(open, d)?;
                    self.concat(t, close)?
                }
            }
        } else if let Some(s) = name.as_string() {
            s
        } else {
            return Err(self.throw_type_error("a function name is a string or a symbol"));
        };
        if let Some(p) = prefix {
            let p = self.new_string(&format!("{p} "))?;
            text = self.concat(p, text)?;
        }
        let key = self.atom(Atom::Name);
        let desc = PropertyDescriptor::data(text.value(), false, false, true);
        self.define_property_or_throw(f, key, desc)
    }

    /// SetFunctionLength(F, length) (§10.2.10): defines `F.length` = `length` (an integer
    /// or +∞) {w: false, e: false, c: true}.
    pub fn set_function_length(
        &mut self,
        f: impl Into<Local<'s, JsObject>>,
        length: f64,
    ) -> JsResult<()> {
        let key = self.atom(Atom::Length);
        let desc = PropertyDescriptor::data(Local::number(length), false, false, true);
        self.define_property_or_throw(f, key, desc)
    }

    /// BoundFunctionCreate(target, boundThis, boundArgs) (§10.4.1.3), with `args` (a tail of
    /// this call's arguments, as `Function.prototype.bind` passes them) as the bound
    /// arguments (`bound::new_bound_function`; P3-BLTA fills it).
    pub fn new_bound_function(
        &mut self,
        target: impl Into<Local<'s, JsFunction>>,
        bound_this: impl Into<Local<'s>>,
        args: Rest<'s>,
    ) -> JsResult<Local<'s, JsFunction>> {
        let (target, this) = (target.into().bits(), bound_this.into().bits());
        // One copy onto the VM stack (precise roots, in order), never a heap list (R2).
        let mut list = self.args_builder();
        for v in args.iter() {
            list.push(self, v)?;
        }
        let n = list.len();
        // SAFETY: `list` wrote slots `start..start + n` with valid Values (`push`); they stay
        // under the scratch top, unchanged, while `list` lives, which is past this call (the
        // heap does not move cells, so the GC never writes them).
        let bound_args = unsafe { core::slice::from_raw_parts(list.start, n) };
        let r = bound::new_bound_function(self.cx, target, this, bound_args);
        drop(list);
        r.map(Self::local)
    }

    /// `[[InitialName]]` of the built-in function object `f` (§10.3): the name it was made
    /// with (CreateBuiltinFunction §10.3.4, then SetFunctionName §10.2.9 steps 4-5, with the
    /// optional step 5.b: "get x" / "set x" for an accessor's functions, as V8). It stays the
    /// same when `f.name` is redefined or deleted. `None` for a function with no
    /// `[[InitialName]]`: a bytecode function or a bound function. A GC point (the atom).
    pub fn initial_name(
        &mut self,
        f: impl Into<Local<'s, JsFunction>>,
    ) -> Option<Local<'s, JsString>> {
        let f = f.into().bits();
        if !kinds::is_function(f) {
            return None;
        }
        // A host function is a built-in one: a table builtin or an embedder's function. Its
        // FunctionInfo keeps the name its `name` property was made from (`object::new_function`).
        let (fi, rt) = (object::function_info(f), self.rt());
        fi.host
            .is_some()
            .then(|| Self::local(atoms::intern_str(rt, &fi.name)))
    }

    /// The concatenation of two strings. RangeError past the length limit.
    fn concat(
        &mut self,
        a: Local<'s, JsString>,
        b: Local<'s, JsString>,
    ) -> JsResult<Local<'s, JsString>> {
        match object::concat(self.rt(), a.bits(), b.bits()) {
            Some(s) => Ok(Self::local(s)),
            None => Err(self.throw_range_error("Invalid string length")),
        }
    }

    // ---- ObjectOps v0: internal methods and their spec wrappers (§7.3) ----

    /// Get(O, P) (§7.3.2).
    pub fn get(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
    ) -> JsResult<Local<'s>> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        property::get(self.cx, o, k).map(Self::local)
    }

    /// O.`[[Get]]`(P, Receiver) (§10.1.8.1 OrdinaryGet): a getter found on the chain from `o`
    /// runs with `receiver` (any value) as `this` (`Reflect.get`, §28.1.5).
    pub fn get_with_receiver(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        receiver: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s>> {
        let (o, r) = (o.into().bits(), receiver.into().bits());
        let k = key.into_key(self).bits();
        property::get_with_receiver(self.cx, o, k, r).map(Self::local)
    }

    /// GetV(V, P) (§7.3.3): a property of any value; TypeError for undefined and null.
    pub fn get_v(&mut self, v: impl Into<Local<'s>>, key: impl IntoKey<'s>) -> JsResult<Local<'s>> {
        let v = v.into().bits();
        let k = key.into_key(self).bits();
        property::get(self.cx, v, k).map(Self::local)
    }

    /// Set(O, P, V, Throw) (§7.3.4). `Ok(false)` if the assignment failed and `throw` is
    /// false; a TypeError if it failed and `throw` is true.
    pub fn set(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        v: impl Into<Local<'s>>,
        throw: bool,
    ) -> JsResult<bool> {
        let (o, v) = (o.into().bits(), v.into().bits());
        let k = key.into_key(self).bits();
        if property::ordinary_set(self.cx, o, k, v)? {
            return Ok(true);
        }
        if throw {
            let msg = format!(
                "Cannot assign to read only property '{}' of object",
                key::key_text(self.rt(), k)
            );
            return Err(self.throw_type_error(&msg));
        }
        Ok(false)
    }

    /// O.`[[Set]]`(P, V, Receiver) (§10.1.9.1 OrdinarySet): the first property on the chain
    /// from `o` decides; a setter runs with `receiver` as `this`, and a new or writable data
    /// property lands on `receiver` (`Reflect.set`, §28.1.12) through its
    /// `[[DefineOwnProperty]]`, so an array receiver's `length` goes through ArraySetLength
    /// (a RangeError for an invalid length). `Ok(false)` when the assignment fails, also for a
    /// data store on a non-object receiver; a setter on the chain runs with any receiver.
    pub fn set_with_receiver(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        v: impl Into<Local<'s>>,
        receiver: impl Into<Local<'s>>,
    ) -> JsResult<bool> {
        let (o, v, r) = (o.into().bits(), v.into().bits(), receiver.into().bits());
        let k = key.into_key(self).bits();
        property::set_with_receiver(self.cx, o, k, v, r)
    }

    /// HasProperty(O, P) (§7.3.12).
    pub fn has_property(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
    ) -> JsResult<bool> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        Ok(property::has_property(self.cx, o, k))
    }

    /// HasOwnProperty(O, P) (§7.3.13). A routed kind (ARGUMENTS, MODULE_NAMESPACE) asks its
    /// `[[GetOwnProperty]]`, which may throw (a namespace binding in its TDZ, §10.4.6.5).
    pub fn has_own_property(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
    ) -> JsResult<bool> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        if crate::exotic::is_routed(kinds::cell_kind(o)) {
            return routed_has_own_property(self.cx, o, k);
        }
        Ok(property::has_own_property(self.cx, o, k))
    }

    /// O.`[[GetOwnProperty]]`(P) (§10.1.5), with the virtual properties (R6).
    pub fn get_own_property(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
    ) -> JsResult<Option<PropertyDescriptor<'s>>> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        Ok(property::get_own_property(self.cx, o, k)?.map(PropertyDescriptor::from_desc))
    }

    /// O.`[[DefineOwnProperty]]`(P, Desc) (§10.1.6, ValidateAndApplyPropertyDescriptor). v0:
    /// an accessor descriptor throws "not implemented".
    pub fn define_own_property(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        desc: PropertyDescriptor<'s>,
    ) -> JsResult<bool> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        property::define_own_property(self.cx, o, k, desc.to_desc())
    }

    /// DefinePropertyOrThrow(O, P, Desc) (§7.3.8).
    pub fn define_property_or_throw(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        desc: PropertyDescriptor<'s>,
    ) -> JsResult<()> {
        let o = o.into().bits();
        let k = key.into_key(self);
        if property::define_own_property(self.cx, o, k.bits(), desc.to_desc())? {
            return Ok(());
        }
        let msg = format!(
            "Cannot redefine property: {}",
            key::key_text(self.rt(), k.bits())
        );
        Err(self.throw_type_error(&msg))
    }

    /// CreateDataProperty(O, P, V) (§7.3.5).
    pub fn create_data_property(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        v: impl Into<Local<'s>>,
    ) -> JsResult<bool> {
        let desc = PropertyDescriptor::data(v.into(), true, true, true);
        self.define_own_property(o, key, desc)
    }

    /// CreateDataPropertyOrThrow(O, P, V) (§7.3.7).
    pub fn create_data_property_or_throw(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
        v: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let desc = PropertyDescriptor::data(v.into(), true, true, true);
        self.define_property_or_throw(o, key, desc)
    }

    /// O.`[[Delete]]`(P) (§10.1.10).
    pub fn delete(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
    ) -> JsResult<bool> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        property::delete(self.cx, o, k, false)
    }

    /// DeletePropertyOrThrow(O, P) (§7.3.10).
    pub fn delete_property_or_throw(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        key: impl IntoKey<'s>,
    ) -> JsResult<()> {
        let o = o.into().bits();
        let k = key.into_key(self).bits();
        property::delete(self.cx, o, k, true).map(|_| ())
    }

    /// O.`[[OwnPropertyKeys]]`() (§10.1.11) as a new array: array indices ascending, then
    /// strings in creation order, then symbols.
    pub fn own_property_keys(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
    ) -> JsResult<Local<'s, JsArray>> {
        let o = o.into().bits();
        Ok(Self::local(property::own_property_keys(self.cx, o)))
    }

    /// O.`[[GetPrototypeOf]]`() (§10.1.1): an object or null.
    pub fn get_prototype_of(&mut self, o: impl Into<Local<'s, JsObject>>) -> JsResult<Local<'s>> {
        Ok(Self::local(object::proto_of(o.into().bits())))
    }

    /// O.`[[SetPrototypeOf]]`(V) (§10.1.2); `proto` is an object or null.
    pub fn set_prototype_of(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        proto: impl Into<Local<'s>>,
    ) -> JsResult<bool> {
        let (o, p) = (o.into().bits(), proto.into().bits());
        if p != NULL && !kinds::is_object(p) {
            return Err(self.throw_type_error("Object prototype may only be an Object or null"));
        }
        Ok(property::set_prototype_of(self.rt(), o, p))
    }

    /// O.`[[IsExtensible]]`() (§10.1.3).
    pub fn is_extensible(&mut self, o: impl Into<Local<'s, JsObject>>) -> JsResult<bool> {
        Ok(property::is_extensible(o.into().bits()))
    }

    /// O.`[[PreventExtensions]]`() (§10.1.4).
    pub fn prevent_extensions(&mut self, o: impl Into<Local<'s, JsObject>>) -> JsResult<bool> {
        Ok(property::prevent_extensions(self.rt(), o.into().bits()))
    }

    /// IsCallable (§7.2.3).
    pub fn is_callable(&self, v: impl Into<Local<'s>>) -> bool {
        kinds::is_callable(v.into().bits())
    }

    /// IsConstructor (§7.2.4).
    pub fn is_constructor(&self, v: impl Into<Local<'s>>) -> bool {
        let b = v.into().bits();
        kinds::is_callable(b) && object::is_constructor(b)
    }

    /// IsArray (§7.2.2). v0: no proxies, so it never throws.
    pub fn is_array(&self, v: impl Into<Local<'s>>) -> bool {
        kinds::is_array(v.into().bits())
    }

    /// Whether `v` has an `[[ErrorData]]` internal slot (an Error instance, §20.5): the test
    /// of IsError (§20.5.8.2), for `Error.isError`. Any value; never throws (v0: no proxies).
    pub fn is_error(&self, v: impl Into<Local<'s>>) -> bool {
        kinds::is_error(v.into().bits())
    }

    // ---- conversions (§7.1) and comparisons (§7.2) ----

    /// ToBoolean (§7.1.2).
    pub fn to_boolean(&self, v: impl Into<Local<'s>>) -> bool {
        conv::to_boolean(v.into().bits())
    }

    /// ToNumber (§7.1.4).
    pub fn to_number(&mut self, v: impl Into<Local<'s>>) -> JsResult<f64> {
        conv::to_number(self.cx, v.into().bits())
    }

    /// ToIntegerOrInfinity (§7.1.5).
    pub fn to_integer_or_infinity(&mut self, v: impl Into<Local<'s>>) -> JsResult<f64> {
        let d = self.to_number(v)?;
        Ok(if d.is_nan() || d == 0.0 {
            0.0
        } else if d.is_infinite() {
            d
        } else {
            d.trunc()
        })
    }

    /// ToInt32 (§7.1.6).
    pub fn to_int32(&mut self, v: impl Into<Local<'s>>) -> JsResult<i32> {
        conv::to_int32_of(self.cx, v.into().bits())
    }

    /// ToUint32 (§7.1.7).
    pub fn to_uint32(&mut self, v: impl Into<Local<'s>>) -> JsResult<u32> {
        conv::to_uint32_of(self.cx, v.into().bits())
    }

    /// ToLength (§7.1.20): an integer in [0, 2^53 - 1].
    pub fn to_length(&mut self, v: impl Into<Local<'s>>) -> JsResult<u64> {
        let n = self.to_integer_or_infinity(v)?;
        const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
        Ok(if n <= 0.0 { 0 } else { n.min(MAX_SAFE) as u64 })
    }

    /// ToString (§7.1.17).
    pub fn to_string(&mut self, v: impl Into<Local<'s>>) -> JsResult<Local<'s, JsString>> {
        conv::to_string(self.cx, v.into().bits()).map(Self::local)
    }

    /// ToPropertyKey (§7.1.19): an atom string (or, from P2, a symbol).
    pub fn to_property_key(&mut self, v: impl Into<Local<'s>>) -> JsResult<Local<'s, JsKey>> {
        let k = key::to_property_key(self.cx, v.into().bits())?;
        if kinds::is_string(k) {
            return Ok(Self::local(atoms::intern(self.rt(), k)));
        }
        Ok(Self::local(k))
    }

    /// ToObject (§7.1.18): an object as it is; a wrapper of a primitive with the realm's
    /// prototype for its type ([`Cx::new_wrapper`]); TypeError for undefined and null.
    pub fn to_object(&mut self, v: impl Into<Local<'s>>) -> JsResult<Local<'s, JsObject>> {
        let v = v.into();
        if let Some(o) = v.as_object() {
            return Ok(o);
        }
        let Some(ty) = PrimitiveType::of(v.bits()) else {
            return Err(self.throw_type_error("Cannot convert undefined or null to object"));
        };
        if !self.has_intrinsic(ty.prototype()) {
            return Err(self.not_implemented("wrapper objects for primitives"));
        }
        let proto = self.intrinsic(ty.prototype());
        self.new_wrapper(proto, v)
    }

    /// thisNumberValue, thisBooleanValue, thisStringValue and thisSymbolValue (§21.1.3.7.1,
    /// §20.3.3.3.1, §22.1.3.35.1, §20.4.3.4.1): `v` if it is a primitive of type `ty`, the
    /// primitive inside `v` if it is a wrapper of that type, else a TypeError.
    pub fn this_primitive_value(
        &mut self,
        v: impl Into<Local<'s>>,
        ty: PrimitiveType,
    ) -> JsResult<Local<'s>> {
        let v = v.into().bits();
        if PrimitiveType::of(v) == Some(ty) {
            return Ok(Self::local(v));
        }
        if kinds::is_object(v)
            && let Some(p) = wrapper::wrapper_value(v)
            && PrimitiveType::of(p) == Some(ty)
        {
            return Ok(Self::local(p));
        }
        let msg = format!(
            "{}.prototype method called on an incompatible receiver {}",
            ty.name(),
            conv::describe(v)
        );
        Err(self.throw_type_error(&msg))
    }

    /// ToPrimitive (§7.1.1).
    pub fn to_primitive(
        &mut self,
        v: impl Into<Local<'s>>,
        hint: ToPrimitiveHint,
    ) -> JsResult<Local<'s>> {
        let hint = match hint {
            ToPrimitiveHint::Default => Hint::Default,
            ToPrimitiveHint::Number => Hint::Number,
            ToPrimitiveHint::String => Hint::String,
        };
        conv::to_primitive(self.cx, v.into().bits(), hint).map(Self::local)
    }

    /// SameValue (§7.2.10).
    pub fn same_value(&self, a: impl Into<Local<'s>>, b: impl Into<Local<'s>>) -> bool {
        property::same_value(a.into().bits(), b.into().bits())
    }

    /// IsStrictlyEqual (§7.2.16).
    pub fn is_strictly_equal(&self, a: impl Into<Local<'s>>, b: impl Into<Local<'s>>) -> bool {
        conv::strict_equals(a.into().bits(), b.into().bits())
    }

    /// `typeof v` (§13.5.3).
    pub fn type_of(&self, v: impl Into<Local<'s>>) -> &'static str {
        conv::type_of(v.into().bits()).text()
    }

    // ---- spec helpers ----

    /// ToPropertyDescriptor (§6.2.6.5).
    pub fn to_property_descriptor(
        &mut self,
        v: impl Into<Local<'s>>,
    ) -> JsResult<PropertyDescriptor<'s>> {
        let v = v.into();
        let Some(o) = v.as_object() else {
            let msg = format!(
                "Property description must be an object: {}",
                conv::describe(v.bits())
            );
            return Err(self.throw_type_error(&msg));
        };
        let mut d = PropertyDescriptor::default();
        let field = |cx: &mut Self, name: &str| -> JsResult<Option<Local<'s>>> {
            let k = cx.key(name);
            if cx.has_property(o, k)? {
                cx.get(o, k).map(Some)
            } else {
                Ok(None)
            }
        };
        if let Some(e) = field(self, "enumerable")? {
            d.enumerable = Some(self.to_boolean(e));
        }
        if let Some(c) = field(self, "configurable")? {
            d.configurable = Some(self.to_boolean(c));
        }
        d.value = field(self, "value")?;
        if let Some(w) = field(self, "writable")? {
            d.writable = Some(self.to_boolean(w));
        }
        for (name, slot) in [("get", 0), ("set", 1)] {
            if let Some(f) = field(self, name)? {
                if !f.is_undefined() && !f.is_callable() {
                    let kind = if slot == 0 { "Getter" } else { "Setter" };
                    let msg = format!("{kind} must be a function: {}", conv::describe(f.bits()));
                    return Err(self.throw_type_error(&msg));
                }
                if slot == 0 {
                    d.get = Some(f);
                } else {
                    d.set = Some(f);
                }
            }
        }
        if d.is_accessor() && d.is_data() {
            return Err(self.throw_type_error(
                "Invalid property descriptor. Cannot both specify accessors and a value or \
                 writable attribute",
            ));
        }
        Ok(d)
    }

    /// FromPropertyDescriptor (§6.2.6.4): undefined for `None`, else a new object with the
    /// present fields in the order value, writable, get, set, enumerable, configurable.
    pub fn from_property_descriptor(
        &mut self,
        desc: Option<PropertyDescriptor<'s>>,
    ) -> JsResult<Local<'s>> {
        let Some(d) = desc else {
            return Ok(Local::undefined());
        };
        let o = self.new_object();
        let fields: [(&str, Option<Local<'s>>); 6] = [
            ("value", d.value),
            ("writable", d.writable.map(Local::boolean)),
            ("get", d.get),
            ("set", d.set),
            ("enumerable", d.enumerable.map(Local::boolean)),
            ("configurable", d.configurable.map(Local::boolean)),
        ];
        for (name, v) in fields {
            if let Some(v) = v {
                self.create_data_property_or_throw(o, name, v)?;
            }
        }
        Ok(o.value())
    }

    /// CreateListFromArrayLike (§7.3.19) into an [`ArgsBuilder`], for a call.
    pub fn create_list_from_array_like(
        &mut self,
        v: impl Into<Local<'s>>,
    ) -> JsResult<ArgsBuilder<'s>> {
        let v = v.into();
        let Some(o) = v.as_object() else {
            return Err(self.throw_type_error("CreateListFromArrayLike called on non-object"));
        };
        let len = self.get(o, self.atom(Atom::Length))?;
        let len = self.to_length(len)?;
        let mut list = self.args_builder();
        // Get(O, ! ToString(𝔽(i))) without a string key (M4-P3 KEYED, OI-04): a present
        // array element is read in place, any other array index through `get_int` (an
        // arguments element, a hole, the chain); each read sees the state after the last
        // push and the last getter. An index past 2^32 - 2 takes the key.
        for i in 0..len {
            let ob = o.bits();
            let e = match u32::try_from(i) {
                Ok(n) if n != u32::MAX => {
                    match kinds::is_array(ob).then(|| object::array_get(ob, n)) {
                        Some(Some(v)) => v,
                        _ => property::get_int(self.cx, ob, n)?,
                    }
                }
                _ => property::get_keyed(self.cx, ob, conv::num(i as f64))?,
            };
            list.push(self, Self::local::<JsValue>(e))?;
        }
        Ok(list)
    }

    /// GetPrototypeFromConstructor (§10.1.14): `constructor.prototype` if it is an object,
    /// else the `default` intrinsic of GetFunctionRealm(constructor) (step 4; the realm of a
    /// bound function's target, else the current realm).
    pub fn get_prototype_from_constructor(
        &mut self,
        constructor: impl Into<Local<'s, JsObject>>,
        default: IntrinsicId,
    ) -> JsResult<Local<'s, JsObject>> {
        let constructor = constructor.into();
        let key = self.atom(Atom::Prototype);
        let p = self.get(constructor, key)?;
        Ok(match p.as_object() {
            Some(p) => p,
            None => {
                let rt = self.rt();
                let realm = crate::realm::get_function_realm(rt, constructor.bits());
                let v = crate::realm::intrinsic_in(rt, realm, default);
                assert!(kinds::is_object(v), "{} is not installed", default.name());
                Self::local(v)
            }
        })
    }

    /// OrdinaryCreateFromConstructor (§10.1.13): an ordinary object.
    pub fn ordinary_create_from_constructor(
        &mut self,
        constructor: impl Into<Local<'s, JsObject>>,
        default: IntrinsicId,
    ) -> JsResult<Local<'s, JsObject>> {
        let p = self.get_prototype_from_constructor(constructor, default)?;
        Ok(self.new_object_with_proto(p))
    }

    // ---- other ----

    /// Runs a full garbage collection now (test hooks such as `gc()`).
    pub fn collect_garbage(&mut self) {
        self.rt().heap.collect();
    }

    // ---- Math.random (M4-P0, part 2; V9-D16) ----

    /// The next value of `Math.random` (§21.3.2.27): a number in [0, 1) from the isolate's
    /// xorshift128+ state (`Runtime::rng`, the V8 generator). The state is seeded per isolate
    /// from the process hash key (random from the OS, or fixed by `EP9_HASH_SEED`) and the
    /// isolate's id (`runtime::rng_seed`): each isolate has its own sequence, and one
    /// `EP9_HASH_SEED` repeats it run to run. Never throws, never allocates; not for
    /// cryptography.
    pub fn math_random(&mut self) -> f64 {
        let rng = &self.rt().rng;
        let [mut s1, s0] = rng.get();
        s1 ^= s1 << 23;
        s1 ^= s1 >> 17;
        s1 ^= s0;
        s1 ^= s0 >> 26;
        rng.set([s0, s1]);
        // The top 52 bits of the first word as the mantissa of a number in [1, 2), minus 1:
        // every result is a multiple of 2^-52 below 1 (exact operations only, H36).
        f64::from_bits((s0 >> 12) | 0x3FF0_0000_0000_0000) - 1.0
    }
}
