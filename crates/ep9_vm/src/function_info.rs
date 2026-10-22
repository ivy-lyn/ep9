//! Function infos: the v0 SharedFunctionInfo (H14 "SFI").
//!
//! ```text
//! frame header slot 4 ──native word──> FunctionInfo
//! function cell +32   ──native word──> FunctionInfo
//! FunctionInfo.feedback ──> the function's slot in its unit's UnitFeedback (null: none yet)
//! ```
//!
//! Three constructors: [`FunctionInfo::bytecode`] (unit functions, unit.rs),
//! [`FunctionInfo::host`] (host functions, realm.rs) and [`FunctionInfo::stub`] (VM stub
//! frames).

use core::cell::Cell;

use ep9_bytecode::{Constant, FunctionFlags, FunctionKind, PositionEntry, position_at};
use ep9_heap::Slot;

use crate::dispatch::{DispatchKind, DispatchSlot};
use crate::feedback;
use crate::host::HostFn;
use crate::unit::LoadedUnit;

/// FunctionInfo flag bits.
pub(crate) mod fi_flags {
    pub const STRICT: u32 = 1;
    /// Has `[[Construct]]`.
    pub const CONSTRUCTOR: u32 = 2;
    /// Runs bytecode (dispatch slot kind Interp).
    pub const BYTECODE: u32 = 4;
    /// A host (Rust) function.
    pub const HOST: u32 = 8;
    /// A VM stub frame (`[[Construct]]` result check).
    pub const STUB: u32 = 16;
    /// An arrow function (lexical `this`).
    pub const ARROW: u32 = 32;
    /// A class constructor: `[[Call]]` throws a TypeError (§10.2.1 step 2).
    pub const CLASS_CONSTRUCTOR: u32 = 64;
    /// A derived class constructor: `[[Construct]]` binds no `this` (§10.2.2 step 5).
    pub const DERIVED: u32 = 128;
    /// A generator function (`function*`, a generator method): its calls make a generator
    /// object, its function object inherits from %GeneratorFunction.prototype% and its lazy
    /// `prototype` from %GeneratorPrototype% (M2-P1-GEN-FLAG). Not set for async generators
    /// (month 3 adds their own bit).
    pub const GENERATOR: u32 = 256;
    /// An async function (not an async generator): its function object inherits from
    /// %AsyncFunction.prototype% (§15.8.3, §27.7.3).
    pub const ASYNC: u32 = 512;
}

/// Per-function data the VM needs at run time. `repr(C)`: the call path reads the dispatch
/// slot, counts and flags at fixed offsets. New fields go at the end.
#[repr(C)]
pub(crate) struct FunctionInfo {
    pub slot: DispatchSlot,
    pub register_count: u32,
    pub parameter_count: u32,
    pub flags: u32,
    pub function_id: u32,
    pub code: *const u8,
    pub code_len: u32,
    /// The JS `length` property.
    pub length: u32,
    /// Constant pool (Values, see unit.rs). Null for host functions and stubs.
    pub consts: *const Slot,
    /// Owning unit. Null for host functions and stubs.
    pub unit: *const LoadedUnit,
    pub host: Option<HostFn>,
    pub name: String,
    pub kind: Option<FunctionKind>,
    /// This function's slot in its unit's feedback (H16 lazy FeedbackVector): null until the
    /// vector is made, then the unit slot that holds the FEEDBACK cell (`feedback.rs`). T0
    /// reads it through the frame's FunctionInfo word (fp-4).
    pub feedback: Cell<*const feedback::Slot>,
    /// Calls counted while the function has no feedback vector (`feedback::count_call`):
    /// below the threshold until the call that makes the vector, then fixed.
    pub invocations: Cell<u32>,
}

/// The FunctionInfo flags of a unit function.
fn bytecode_flags(f: &ep9_bytecode::Function) -> u32 {
    let strict = f.flags.contains(FunctionFlags::STRICT);
    let constructor = matches!(
        f.kind,
        FunctionKind::Normal | FunctionKind::ClassConstructor | FunctionKind::DerivedConstructor
    ) && !f.flags.contains(FunctionFlags::GENERATOR)
        && !f.flags.contains(FunctionFlags::ASYNC);
    let mut flags = fi_flags::BYTECODE;
    if strict {
        flags |= fi_flags::STRICT;
    }
    if constructor {
        flags |= fi_flags::CONSTRUCTOR;
    }
    if f.kind == FunctionKind::Arrow {
        flags |= fi_flags::ARROW;
    }
    if matches!(
        f.kind,
        FunctionKind::ClassConstructor | FunctionKind::DerivedConstructor
    ) {
        flags |= fi_flags::CLASS_CONSTRUCTOR;
    }
    if f.kind == FunctionKind::DerivedConstructor {
        flags |= fi_flags::DERIVED;
    }
    if f.flags.contains(FunctionFlags::GENERATOR) && !f.flags.contains(FunctionFlags::ASYNC) {
        flags |= fi_flags::GENERATOR;
    }
    if f.flags.contains(FunctionFlags::ASYNC) && !f.flags.contains(FunctionFlags::GENERATOR) {
        flags |= fi_flags::ASYNC;
    }
    flags
}

impl FunctionInfo {
    /// The info of unit function `function_id` (`f`), whose constant pool is `consts` and
    /// whose unit is `unit`. `f.bytecode` must stay at its address while the info lives (the
    /// unit owns both).
    pub fn bytecode(
        f: &ep9_bytecode::Function,
        function_id: u32,
        consts: *const Slot,
        unit: *const LoadedUnit,
        name: String,
    ) -> FunctionInfo {
        FunctionInfo {
            slot: DispatchSlot::new(
                DispatchKind::Interp,
                f.bytecode.as_ptr().expose_provenance(),
            ),
            register_count: f.register_count,
            parameter_count: f.parameter_count,
            flags: bytecode_flags(f),
            function_id,
            code: f.bytecode.as_ptr(),
            code_len: f.bytecode.len() as u32,
            length: f.length,
            consts,
            unit,
            host: None,
            name,
            kind: Some(f.kind),
            feedback: Cell::new(core::ptr::null()),
            invocations: Cell::new(0),
        }
    }

    pub fn host(f: HostFn, name: &str, length: u32, constructor: bool) -> FunctionInfo {
        FunctionInfo {
            slot: DispatchSlot::empty(),
            register_count: 0,
            parameter_count: 0,
            flags: fi_flags::HOST
                | fi_flags::STRICT
                | if constructor {
                    fi_flags::CONSTRUCTOR
                } else {
                    0
                },
            function_id: u32::MAX,
            code: core::ptr::null(),
            code_len: 0,
            length,
            consts: core::ptr::null(),
            unit: core::ptr::null(),
            host: Some(f),
            name: name.into(),
            kind: None,
            feedback: Cell::new(core::ptr::null()),
            invocations: Cell::new(0),
        }
    }

    /// The frame info of a VM stub whose code is `code` (one internal opcode).
    pub fn stub(code: *const u8) -> FunctionInfo {
        FunctionInfo {
            slot: DispatchSlot::empty(),
            register_count: 0,
            parameter_count: 0,
            flags: fi_flags::STUB,
            function_id: u32::MAX,
            code,
            code_len: 1,
            length: 0,
            consts: core::ptr::null(),
            unit: core::ptr::null(),
            host: None,
            name: String::new(),
            kind: None,
            feedback: Cell::new(core::ptr::null()),
            invocations: Cell::new(0),
        }
    }

    #[inline(always)]
    pub fn is_strict(&self) -> bool {
        self.flags & fi_flags::STRICT != 0
    }

    #[inline(always)]
    pub fn is_constructor(&self) -> bool {
        self.flags & fi_flags::CONSTRUCTOR != 0
    }

    #[inline(always)]
    pub fn is_bytecode(&self) -> bool {
        self.flags & fi_flags::BYTECODE != 0
    }

    #[inline(always)]
    pub fn is_class_constructor(&self) -> bool {
        self.flags & fi_flags::CLASS_CONSTRUCTOR != 0
    }

    #[inline(always)]
    pub fn is_derived(&self) -> bool {
        self.flags & fi_flags::DERIVED != 0
    }

    /// A (sync) generator function ([`fi_flags::GENERATOR`]).
    #[inline(always)]
    pub fn is_generator(&self) -> bool {
        self.flags & fi_flags::GENERATOR != 0
    }

    /// An async function, not an async generator ([`fi_flags::ASYNC`]).
    #[inline(always)]
    pub fn is_async_function(&self) -> bool {
        self.flags & fi_flags::ASYNC != 0
    }

    /// Bytecode offset of `pc`, if `pc` is inside this function's code.
    pub fn offset_of(&self, pc: *const u8) -> Option<u32> {
        if !self.is_bytecode() {
            return None;
        }
        let off = pc.addr().wrapping_sub(self.code.addr());
        (off < self.code_len as usize).then_some(off as u32)
    }

    /// Constant `k` as Value bits (see unit.rs for the non-Value kinds).
    #[inline(always)]
    pub fn constant(&self, k: u32) -> u64 {
        // SAFETY: `consts` points at the pool of this function, and the verifier checked
        // every constant index against the pool length.
        unsafe { (*self.consts.add(k as usize)).bits() }
    }

    pub fn loaded_unit(&self) -> Option<&LoadedUnit> {
        // SAFETY: a non-null `unit` points at a LoadedUnit that the isolate keeps until it
        // drops.
        (!self.unit.is_null()).then(|| unsafe { &*self.unit })
    }

    /// The `Constant` entry `k` of this bytecode function.
    pub fn constant_entry(&self, k: u32) -> &Constant {
        let u = self.loaded_unit().expect("a bytecode function");
        &u.unit.functions[self.function_id as usize].constants[k as usize]
    }

    /// Source position (UTF-8 offset) of the instruction at bytecode `offset`.
    pub fn source_offset(&self, offset: u32) -> Option<u32> {
        let u = self.loaded_unit()?;
        let f = &u.unit.functions[self.function_id as usize];
        position_at(&f.positions, offset)
            .map(|p: PositionEntry| p.source_offset)
            .or(Some(f.source_start))
    }
}
