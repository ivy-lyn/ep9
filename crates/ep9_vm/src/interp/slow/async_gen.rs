#![allow(dead_code)] // m3-stub
//! Async generators (§27.6; owner M3-P2 ASYNCGEN): the runtime ids 36-41 of ABI 5. M3-P0
//! stubs: each throws today's "not implemented" error, with the id's name. `runtime.rs`
//! routes the ids here (P0-VM3 step 2, after the ABI 5 merge).
//!
//! | Id | Name | Args |
//! |---|---|---|
//! | 36 | `AsyncGeneratorEnter` | closure, receiver |
//! | 37 | `AsyncGeneratorAwait` | generator, value |
//! | 38 | `AsyncGeneratorYield` | generator, value |
//! | 39 | `AsyncGeneratorResolve` | generator, value, done |
//! | 40 | `AsyncGeneratorReject` | generator, reason |
//! | 41 | `CreateAsyncFromSyncIterator` | iterator, next method |

use crate::conv::Res;
use crate::error::not_implemented;
use crate::interp::Cx;

/// Id 36: the async generator object. Stub.
pub(crate) fn async_generator_enter(
    cx: &mut Cx,
    _fp: *mut u64,
    _closure: u64,
    _receiver: u64,
) -> Res<u64> {
    Err(not_implemented(cx, "AsyncGeneratorEnter"))
}

/// Id 37: Await inside an async generator. Stub.
pub(crate) fn async_generator_await(cx: &mut Cx, _generator: u64, _value: u64) -> Res<u64> {
    Err(not_implemented(cx, "AsyncGeneratorAwait"))
}

/// Id 38: AsyncGeneratorYield (§27.6.3.8). Stub.
pub(crate) fn async_generator_yield(cx: &mut Cx, _generator: u64, _value: u64) -> Res<u64> {
    Err(not_implemented(cx, "AsyncGeneratorYield"))
}

/// Id 39: AsyncGeneratorCompleteStep with a normal completion (§27.6.3.4). Stub.
pub(crate) fn async_generator_resolve(
    cx: &mut Cx,
    _generator: u64,
    _value: u64,
    _done: u64,
) -> Res<u64> {
    Err(not_implemented(cx, "AsyncGeneratorResolve"))
}

/// Id 40: AsyncGeneratorCompleteStep with a throw completion. Stub.
pub(crate) fn async_generator_reject(cx: &mut Cx, _generator: u64, _reason: u64) -> Res<u64> {
    Err(not_implemented(cx, "AsyncGeneratorReject"))
}

/// Id 41: CreateAsyncFromSyncIterator (§27.1.6.1) over the iterator and its `next`. Stub.
pub(crate) fn create_async_from_sync_iterator(cx: &mut Cx, _iterator: u64, _next: u64) -> Res<u64> {
    Err(not_implemented(cx, "CreateAsyncFromSyncIterator"))
}
