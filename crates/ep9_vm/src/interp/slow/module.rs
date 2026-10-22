#![allow(dead_code)] // m3-stub
//! Modules in the interpreter (§16.2; owner M3-P2 MODULE, then M3-P3 IMPORT-TLA): the cold
//! bodies of `LdaModuleVariable` and `StaModuleVariable` (`ops.rs`), and the runtime ids
//! 42-43 of ABI 5 (`runtime.rs` routes them here since P0-VM3 step 2). M3-P0 stubs: each throws
//! today's "not implemented" error ("module variables" for the two opcodes, the id's name
//! for the ids).
//!
//! | Entry | Args |
//! |---|---|
//! | `LdaModuleVariable` | cell index, depth |
//! | `StaModuleVariable` | cell index, depth, the accumulator |
//! | 42 `GetImportMeta` | none |
//! | 43 `DynamicImport` | specifier, options (undefined when absent) |

use crate::conv::Res;
use crate::error::not_implemented;
use crate::exotic::probe;
use crate::interp::Cx;

use super::not_impl;

/// `LdaModuleVariable cell, depth`: the accumulator, or 0 if an exception is pending. Stub.
#[cold]
#[inline(never)]
pub(crate) fn lda_module_variable(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _cell: i32,
    _depth: u32,
) -> u64 {
    probe::note("module lda_module_variable");
    not_impl(cx, fp, pc, "module variables")
}

/// `StaModuleVariable cell, depth` with the value `v`: the accumulator, or 0 if an exception
/// is pending. Stub.
#[cold]
#[inline(never)]
pub(crate) fn sta_module_variable(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _cell: i32,
    _depth: u32,
    _v: u64,
) -> u64 {
    probe::note("module sta_module_variable");
    not_impl(cx, fp, pc, "module variables")
}

/// Id 42: the `import.meta` object (§13.3.12.1). Stub.
pub(crate) fn get_import_meta(cx: &mut Cx, _fp: *mut u64) -> Res<u64> {
    probe::note("module GetImportMeta");
    Err(not_implemented(cx, "GetImportMeta"))
}

/// Id 43: `import(specifier, options)` (§13.3.10.1): a promise. Stub.
pub(crate) fn dynamic_import(
    cx: &mut Cx,
    _fp: *mut u64,
    _specifier: u64,
    _options: u64,
) -> Res<u64> {
    probe::note("module DynamicImport");
    Err(not_implemented(cx, "DynamicImport"))
}
