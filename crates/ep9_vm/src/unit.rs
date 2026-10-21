//! Loaded units. Function infos (the v0 SharedFunctionInfo) are in `function_info.rs`.
//!
//! ```text
//! VerifiedUnit --load--> LoadedUnit (boxed, never freed while the isolate lives)
//!                          strings:  [Slot]       unit string table as atoms         (root range)
//!                          consts:   [[Slot]]     constant pool per function          (root ranges)
//!                          infos:    [FunctionInfo] { dispatch slot, code, counts, consts, unit }
//!                          feedback: UnitFeedback  per-function feedback (H16, lazy)
//! ```
//!
//! Constant pool entries become Values: Number → number, String → its atom, Oddball → its
//! value. The other kinds keep their index as an int32 marker, and handlers read the
//! `Constant` from the unit. BigInt is the hole marker: `LdaConstant` of a BigInt throws "not
//! implemented" (v0).

use core::ptr::NonNull;

use ep9_bytecode::{Constant, Oddball, SourceLines, VerifiedUnit};
use ep9_heap::{RootId, RootStrength, Slot};
use ep9_value::Value;

use crate::atoms;
use crate::dispatch::{CodeEntry, CodeRegistry, DispatchKind};
use crate::feedback::UnitFeedback;
use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_bits};
use crate::runtime::{Runtime, val};

pub(crate) use crate::function_info::FunctionInfo;

/// A unit the isolate loaded. Boxed and kept until the isolate drops: frames, function cells
/// and dispatch slots point into it.
pub(crate) struct LoadedUnit {
    pub index: u32,
    pub file: String,
    pub unit: VerifiedUnit,
    pub infos: Box<[FunctionInfo]>,
    /// Per-function feedback (H16; `feedback.rs`).
    pub feedback: UnitFeedback,
    strings: Box<[Slot]>,
    consts: Box<[Box<[Slot]>]>,
    roots: Vec<RootId>,
}

impl LoadedUnit {
    /// Loads `unit`: materializes strings and constants (GC points; everything is rooted
    /// before the first allocation), builds function infos and registers bytecode ranges.
    pub fn load(rt: &Runtime, unit: VerifiedUnit, file: String, index: u32) -> Box<LoadedUnit> {
        let undefined = || Slot::new(Value::UNDEFINED);
        let feedback = UnitFeedback::new(rt, unit.functions.len());
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
            feedback,
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
        // Every unit string is an atom (H19): one cell per text in the isolate, so a name
        // from two units, or from a unit and ToPropertyKey, is one key (bits compare).
        for (i, s) in lu.unit.strings.iter().enumerate() {
            let b = match s {
                ep9_bytecode::UnitString::Latin1(b) => atoms::intern_latin1(rt, b),
                ep9_bytecode::UnitString::Utf16(u) => atoms::intern_utf16(rt, u),
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
                let name = f
                    .name
                    .map(|id| match &lu.unit.strings[id.index()] {
                        ep9_bytecode::UnitString::Latin1(b) => {
                            b.iter().map(|&c| char::from(c)).collect()
                        }
                        ep9_bytecode::UnitString::Utf16(u) => String::from_utf16_lossy(u),
                    })
                    .unwrap_or_default();
                FunctionInfo::bytecode(f, i as u32, lu.consts[i].as_ptr(), unit_ptr, name)
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
