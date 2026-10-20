//! Loaded units and function infos (the v0 SharedFunctionInfo, H14 "SFI").
//!
//! ```text
//! VerifiedUnit --load--> LoadedUnit (boxed, never freed while the isolate lives)
//!                          strings: [Slot]       unit string table as heap strings  (root range)
//!                          consts:  [[Slot]]     constant pool per function          (root ranges)
//!                          infos:   [FunctionInfo] { dispatch slot, code, counts, consts, unit }
//! frame header slot 4 ──native word──> FunctionInfo
//! function cell +32   ──native word──> FunctionInfo
//! ```
//!
//! Constant pool entries become Values: Number → number, String → heap string, Oddball → its
//! value. The other kinds keep their index as an int32 marker, and handlers read the
//! `Constant` from the unit. BigInt is the hole marker: `LdaConstant` of a BigInt throws "not
//! implemented" (v0).

use core::ptr::NonNull;

use ep9_bytecode::{
    Constant, FunctionFlags, FunctionKind, Oddball, PositionEntry, SourceLines, VerifiedUnit,
    position_at,
};
use ep9_heap::{RootId, RootStrength, Slot};
use ep9_value::Value;

use crate::dispatch::{CodeEntry, CodeRegistry, DispatchKind, DispatchSlot};
use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_bits};
use crate::host::HostFn;
use crate::object;
use crate::runtime::{Runtime, val};

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
}

/// Per-function data the VM needs at run time. `repr(C)`: the call path reads the dispatch
/// slot, counts and flags at fixed offsets.
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
    /// Constant pool (Values, see module docs). Null for host functions and stubs.
    pub consts: *const Slot,
    /// Owning unit. Null for host functions and stubs.
    pub unit: *const LoadedUnit,
    pub host: Option<HostFn>,
    pub name: String,
    pub kind: Option<FunctionKind>,
}

impl FunctionInfo {
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

    /// Bytecode offset of `pc`, if `pc` is inside this function's code.
    pub fn offset_of(&self, pc: *const u8) -> Option<u32> {
        if !self.is_bytecode() {
            return None;
        }
        let off = pc.addr().wrapping_sub(self.code.addr());
        (off < self.code_len as usize).then_some(off as u32)
    }

    /// Constant `k` as Value bits (see module docs for the non-Value kinds).
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

/// A unit the isolate loaded. Boxed and kept until the isolate drops: frames, function cells
/// and dispatch slots point into it.
pub(crate) struct LoadedUnit {
    pub index: u32,
    pub file: String,
    pub unit: VerifiedUnit,
    pub infos: Box<[FunctionInfo]>,
    strings: Box<[Slot]>,
    consts: Box<[Box<[Slot]>]>,
    roots: Vec<RootId>,
}

impl LoadedUnit {
    /// Loads `unit`: materializes strings and constants (GC points; everything is rooted
    /// before the first allocation), builds function infos and registers bytecode ranges.
    pub fn load(rt: &Runtime, unit: VerifiedUnit, file: String, index: u32) -> Box<LoadedUnit> {
        let undefined = || Slot::new(Value::UNDEFINED);
        let mut lu = Box::new(LoadedUnit {
            index,
            file,
            strings: (0..unit.strings.len()).map(|_| undefined()).collect(),
            consts: unit
                .functions
                .iter()
                .map(|f| (0..f.constants.len()).map(|_| undefined()).collect())
                .collect(),
            unit,
            infos: Box::new([]),
            roots: Vec::new(),
        });
        // SAFETY: the slices live in boxes that the LoadedUnit owns until Drop, which removes
        // the ranges first. They hold `undefined` now and Value bits later (never 0).
        unsafe {
            let id = rt
                .heap
                .add_root_range(NonNull::from(&lu.strings[..]), RootStrength::Strong);
            lu.roots.push(id);
            for c in lu.consts.iter() {
                let id = rt
                    .heap
                    .add_root_range(NonNull::from(&c[..]), RootStrength::Strong);
                lu.roots.push(id);
            }
        }
        for (i, s) in lu.unit.strings.iter().enumerate() {
            let b = match s {
                ep9_bytecode::UnitString::Latin1(b) => object::new_string_latin1(rt, b),
                ep9_bytecode::UnitString::Utf16(u) => object::new_string_utf16(rt, u),
            };
            lu.strings[i].init(val(b));
        }
        for (fi, f) in lu.unit.functions.iter().enumerate() {
            for (k, c) in f.constants.iter().enumerate() {
                let b = match c {
                    Constant::Number(n) => Value::number(n.value()).to_bits(),
                    Constant::String(id) => lu.strings[id.index()].bits(),
                    // v0: no BigInt values; LdaConstant sees the hole and throws "not
                    // implemented".
                    Constant::BigInt(_) => HOLE,
                    Constant::Oddball(o) => match o {
                        Oddball::Undefined => UNDEFINED,
                        Oddball::Null => NULL,
                        Oddball::True => TRUE,
                        Oddball::False => FALSE,
                        Oddball::Hole => HOLE,
                    },
                    _ => int32_bits(k as i32),
                };
                lu.consts[fi][k].init(val(b));
            }
        }
        let unit_ptr: *const LoadedUnit = &*lu;
        let infos: Box<[FunctionInfo]> = lu
            .unit
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let strict = f.flags.contains(FunctionFlags::STRICT);
                let constructor = matches!(
                    f.kind,
                    FunctionKind::Normal
                        | FunctionKind::ClassConstructor
                        | FunctionKind::DerivedConstructor
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
                FunctionInfo {
                    slot: DispatchSlot::new(
                        DispatchKind::Interp,
                        f.bytecode.as_ptr().expose_provenance(),
                    ),
                    register_count: f.register_count,
                    parameter_count: f.parameter_count,
                    flags,
                    function_id: i as u32,
                    code: f.bytecode.as_ptr(),
                    code_len: f.bytecode.len() as u32,
                    length: f.length,
                    consts: lu.consts[i].as_ptr(),
                    unit: unit_ptr,
                    host: None,
                    name: f
                        .name
                        .map(|id| match &lu.unit.strings[id.index()] {
                            ep9_bytecode::UnitString::Latin1(b) => {
                                b.iter().map(|&c| char::from(c)).collect()
                            }
                            ep9_bytecode::UnitString::Utf16(u) => String::from_utf16_lossy(u),
                        })
                        .unwrap_or_default(),
                    kind: Some(f.kind),
                }
            })
            .collect();
        lu.infos = infos;
        let mut reg = rt.registry.borrow_mut();
        for info in lu.infos.iter() {
            // Empty bytecode never verifies, so len >= 1.
            let _ = reg.register(CodeEntry {
                kind: DispatchKind::Interp,
                start: info.code.addr(),
                len: info.code_len as usize,
                function: core::ptr::from_ref(info).addr(),
            });
        }
        lu
    }

    /// The heap string for unit string `id`.
    pub fn string(&self, id: usize) -> u64 {
        self.strings[id].bits()
    }

    /// 1-based line and UTF-16 column of a UTF-8 source offset, if the unit embeds source.
    pub fn line_column(&self, offset: u32) -> Option<(u32, u32)> {
        let src = self.unit.source.as_deref()?;
        let lc = SourceLines::new(src).line_column(offset);
        Some((lc.line, lc.column))
    }
}
