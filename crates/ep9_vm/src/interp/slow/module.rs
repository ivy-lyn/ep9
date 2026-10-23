//! Modules in the interpreter (§16.2; owner M3-P2 MODULE, then M3-P3 IMPORT-TLA): the cold
//! bodies of `LdaModuleVariable` and `StaModuleVariable` (`ops.rs`), and the runtime ids
//! 42-43 of ABI 5 (`runtime.rs` routes them here). The module of the running code is the
//! MODULE record at the end of its context chain (`module::module_of_context`); its cells
//! are boxes in the module environment (`module.rs`).
//!
//! | Entry | Args | Result |
//! |---|---|---|
//! | `LdaModuleVariable` | cell index, depth | the cell's value (the hole in its TDZ; see below) |
//! | `StaModuleVariable` | cell index, depth, the accumulator | the accumulator; an import cell throws a TypeError |
//! | 42 `GetImportMeta` | none | the module's `import.meta` object |
//! | 43 `DynamicImport` | specifier, options (undefined when absent) | not built (M3-P3 IMPORT-TLA) |
//!
//! A load gives the hole only to an instruction that checks it: the front end puts a
//! `ThrowReferenceErrorIfHole` right after each load of a binding that may be in its TDZ.
//! Any other load of a cell in the hole is a `var` binding read before its declaration ran
//! (the module's `var` cells are made at Evaluate, M3-P2 step 1): undefined.

use ep9_bytecode::Opcode;

use crate::conv::Res;
use crate::error::{not_implemented, throw_error};
use crate::exotic::probe;
use crate::frame::{HOLE, UNDEFINED};
use crate::interp::Cx;
use crate::module;
use crate::runtime::ErrorKind;

use super::{context, enter, info, not_impl};

/// True if the instruction after the one at `pc` (in the frame `fp`) is
/// `ThrowReferenceErrorIfHole`.
fn hole_checked_next(fp: *mut u64, pc: *const u8) -> bool {
    let fi = info(fp);
    let Some(off) = fi.offset_of(pc) else {
        return false;
    };
    // SAFETY: the code of a bytecode FunctionInfo is `code_len` bytes of verified bytecode.
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let Ok(insn) = ep9_bytecode::decode_at(code, off) else {
        return false;
    };
    let next = off + insn.bytes.len() as u32;
    ep9_bytecode::decode_at(code, next).is_ok_and(|n| n.opcode == Opcode::ThrowReferenceErrorIfHole)
}

/// The internal error of a module cell that the module does not have (a BytecodeGen bug).
fn bad_cell(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    enter(cx, fp, pc);
    throw_error(
        cx,
        ErrorKind::Error,
        "ep9 internal error: module cell out of range",
    );
    0
}

/// `LdaModuleVariable cell, depth`: the accumulator, or 0 if an exception is pending. Code
/// that is not module code has no module: the v0 "module variables" error.
#[cold]
#[inline(never)]
pub(crate) fn lda_module_variable(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    cell: i32,
    depth: u32,
) -> u64 {
    probe::note("module lda_module_variable");
    let Some(m) = module::module_of_context(context(fp), depth) else {
        return not_impl(cx, fp, pc, "module variables");
    };
    match module::cell_get(m, cell) {
        Some(HOLE) if !hole_checked_next(fp, pc) => UNDEFINED,
        Some(v) => v,
        None => bad_cell(cx, fp, pc),
    }
}

/// `StaModuleVariable cell, depth` with the value `v`: the accumulator, or 0 if an exception
/// is pending. An import cell is immutable (§9.1.1.5, CreateImportBinding): a TypeError.
#[cold]
#[inline(never)]
pub(crate) fn sta_module_variable(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    cell: i32,
    depth: u32,
    v: u64,
) -> u64 {
    probe::note("module sta_module_variable");
    let Some(m) = module::module_of_context(context(fp), depth) else {
        return not_impl(cx, fp, pc, "module variables");
    };
    if cell < 0 {
        enter(cx, fp, pc);
        throw_error(cx, ErrorKind::TypeError, "Assignment to constant variable.");
        return 0;
    }
    if module::cell_set(cx.rt(), m, cell, v) {
        v
    } else {
        bad_cell(cx, fp, pc)
    }
}

/// Id 42: the `import.meta` object of the running module (§13.3.12.1).
pub(crate) fn get_import_meta(cx: &mut Cx, fp: *mut u64) -> Res<u64> {
    match module::module_of_context(context(fp), 0) {
        Some(m) => Ok(module::import_meta(cx.rt(), m)),
        None => Err(not_implemented(cx, "GetImportMeta")),
    }
}

/// Id 43: `import(specifier, options)` (§13.3.10.1): a promise. Stub (M3-P3 IMPORT-TLA).
pub(crate) fn dynamic_import(
    cx: &mut Cx,
    _fp: *mut u64,
    _specifier: u64,
    _options: u64,
) -> Res<u64> {
    probe::note("module DynamicImport");
    Err(not_implemented(cx, "DynamicImport"))
}
