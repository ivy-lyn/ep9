//! The VM: isolate, realm, v0 objects and strings, the T0 interpreter, frames and calls,
//! dispatch slots and the pc-to-code registry (H14, H15).
//!
//! ```text
//! Isolate (isolate.rs) ── Runtime (runtime.rs): heap, VM stack, realm roots, units
//!    │                       ├─ object.rs / property.rs / conv.rs   v0 object model (no shapes)
//!    │                       ├─ unit.rs                             loaded units, FunctionInfo
//!    │                       └─ realm.rs                            intrinsics, globals
//!    └─ Cx (interp/mod.rs): dispatch table @0 ── interp/dsl.rs + ops.rs + slow.rs
//!                              frames and calls: frame.rs, call.rs   errors: error.rs
//! ```
//!
//! ADRs: H14 frames and dispatch (docs/adr/H14-frames-and-dispatch.md), H15 interpreter
//! dispatch, H10 rooting, H06 values.
#![cfg_attr(feature = "become", feature(explicit_tail_calls))]
#![cfg_attr(feature = "become", allow(incomplete_features))]

mod call;
mod conv;
mod dispatch;
mod error;
mod frame;
mod host;
mod interp;
mod isolate;
mod numconv;
mod object;
mod property;
mod realm;
mod runtime;
mod unit;

#[cfg(test)]
mod tests;

pub use dispatch::{
    CodeEntry, CodeRegistry, DispatchKind, DispatchSlot, NO_CODE, RangeRegistry, RegistryError,
};
pub use ep9_value::{JsResult, Throw, Value};
pub use host::{Args, HostCx, HostFn};
pub use interp::Dispatch;
pub use isolate::{
    Exception, Isolate, IsolateConfig, IsolateError, SourceLocation, TerminateHandle,
};
pub use runtime::ErrorKind;

/// Facts for tests and tools (not a stable API).
#[doc(hidden)]
pub mod internals {
    /// Opcodes with a cold block in the interpreter DSL.
    pub const COLD_OPCODES: usize = crate::interp::ops::COLD_COUNT;
    /// Handlers per threaded expansion: one per opcode, one per cold block, plus `Exit`,
    /// `BadOpcode` and `ConstructReturn` (the `cargo xtask disasm-check --expect` value).
    pub const THREADED_HANDLERS: usize = ep9_bytecode::OPCODE_COUNT + COLD_OPCODES + 3;
}
