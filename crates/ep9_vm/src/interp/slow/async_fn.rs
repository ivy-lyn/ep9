#![allow(dead_code)] // m3-stub
//! Async functions (§27.7; owner M3-P1 ASYNC): the runtime ids 32-35 of ABI 5. M3-P0 stubs:
//! each throws today's "not implemented" error, with the id's name. `runtime.rs` routes the
//! ids here (since P0-VM3 step 2).
//!
//! | Id | Name | Args |
//! |---|---|---|
//! | 32 | `AsyncFunctionEnter` | closure, receiver |
//! | 33 | `AsyncFunctionAwait` | async object, value |
//! | 34 | `AsyncFunctionResolve` | async object, value |
//! | 35 | `AsyncFunctionReject` | async object, reason |

use crate::conv::Res;
use crate::error::not_implemented;
use crate::exotic::probe;
use crate::interp::Cx;

/// Id 32: the async function object (a GENERATOR in async-function mode). Stub.
pub(crate) fn async_function_enter(
    cx: &mut Cx,
    _fp: *mut u64,
    _closure: u64,
    _receiver: u64,
) -> Res<u64> {
    probe::note("async_fn AsyncFunctionEnter");
    Err(not_implemented(cx, "AsyncFunctionEnter"))
}

/// Id 33: Await (§27.7.5.3); subscribes the resume. Stub.
pub(crate) fn async_function_await(cx: &mut Cx, _object: u64, _value: u64) -> Res<u64> {
    probe::note("async_fn AsyncFunctionAwait");
    Err(not_implemented(cx, "AsyncFunctionAwait"))
}

/// Id 34: resolves the async function's promise; returns it. Stub.
pub(crate) fn async_function_resolve(cx: &mut Cx, _object: u64, _value: u64) -> Res<u64> {
    probe::note("async_fn AsyncFunctionResolve");
    Err(not_implemented(cx, "AsyncFunctionResolve"))
}

/// Id 35: rejects the async function's promise; returns it. Stub.
pub(crate) fn async_function_reject(cx: &mut Cx, _object: u64, _reason: u64) -> Res<u64> {
    probe::note("async_fn AsyncFunctionReject");
    Err(not_implemented(cx, "AsyncFunctionReject"))
}
