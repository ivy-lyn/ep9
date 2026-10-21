#![allow(dead_code)] // m2-stub
//! Dynamic scope lookup: sloppy direct eval and `with` (owner P2-LOOKUP).
//!
//! ```text
//! LdaLookupSlot name               lda_lookup_slot(.., false)   walk the contexts by name
//! LdaLookupSlotInsideTypeof name   lda_lookup_slot(.., true)    (object.rs `context_scope`
//! StaLookupSlot name, flags        sta_lookup_slot               names each context's slots),
//! CreateWithContext object, scope  create_with_context           `with` objects, `.eval_ext`
//! CallRuntime DeleteLookupSlot     delete_lookup_slot            (runtime id 8)
//! ```
//!
//! The opcode entries use the result protocol of `slow` (the new accumulator, or 0 with an
//! exception pending). `name` and `scope` are constant indices of the frame's function.
//! P0 stubs throw the v0 errors ("dynamic scope lookup (eval, with)", "with statements",
//! "DeleteLookupSlot"). `LoadLookupSlotForCall` (runtime id 26, ABI 3) comes with the P0-LANG
//! merge.

use crate::conv::Res;
use crate::error::not_implemented;
use crate::interp::Cx;

use super::not_impl;

/// `LdaLookupSlot(name)` / `LdaLookupSlotInsideTypeof(name)` (`inside_typeof`). Stub.
#[cold]
#[inline(never)]
pub(crate) fn lda_lookup_slot(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _name: u32,
    _inside_typeof: bool,
) -> u64 {
    not_impl(cx, fp, pc, "dynamic scope lookup (eval, with)")
}

/// `StaLookupSlot(name, flags)` with the value `v` (the accumulator, which it keeps). Stub.
#[cold]
#[inline(never)]
pub(crate) fn sta_lookup_slot(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _name: u32,
    _flags: u8,
    _v: u64,
) -> u64 {
    not_impl(cx, fp, pc, "dynamic scope lookup (eval, with)")
}

/// `CreateWithContext(object, scope)`: the new context. Stub.
#[cold]
#[inline(never)]
pub(crate) fn create_with_context(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _object: u64,
    _scope: u32,
) -> u64 {
    not_impl(cx, fp, pc, "with statements")
}

/// `DeleteLookupSlot(name)` (runtime id 8): sloppy `delete name` of a name that is not
/// statically bound. Stub: the v0 error.
pub(crate) fn delete_lookup_slot(cx: &mut Cx, _fp: *mut u64, _name: u64) -> Res<u64> {
    Err(not_implemented(cx, "DeleteLookupSlot"))
}
