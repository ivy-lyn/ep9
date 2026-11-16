//! The VM: isolate, realm, v0 objects and strings, the T0 interpreter, frames and calls,
//! dispatch slots and the pc-to-code registry (H14, H15).
//!
//! ```text
//! Isolate (isolate.rs) ── Runtime (runtime.rs): heap, VM stack, realm roots, units
//!    │                       ├─ object.rs / property.rs / conv.rs   v0 object model (no shapes)
//!    │                       │    kinds.rs (kind ids v1), string.rs, props.rs, key.rs
//!    │                       ├─ unit.rs, function_info.rs           loaded units, FunctionInfo
//!    │                       └─ realm.rs, natives.rs                N realms, intrinsics, globals,
//!    │                                                              host fns
//!    └─ Cx (interp/mod.rs): dispatch table @0 ── interp/dsl.rs + ops.rs + slow/
//!                              frames and calls: frame.rs, call.rs   errors: error.rs
//! ```
//!
//! Week-4 modules (stubs until their owners fill them): shape.rs, atoms.rs, symbol.rs,
//! wrapper.rs, bound.rs, accessor.rs, exotic.rs, ic.rs, feedback.rs. The builtin ABI v1 (H20)
//! is [`abi`]: `Cx`, `Local`, `Args`, ObjectOps. Builtin tables (H20): [`BuiltinTable`],
//! [`BuiltinModule`], [`BuiltinDef`], [`PropDef`], with the ids [`BuiltinId`] and
//! [`IntrinsicId`]; realm init walks the VM's table (`natives.rs`), then
//! [`IsolateConfig::builtins`]. Contract versions: [`versions`].
//!
//! Jobs and promises (P4-PROMISE): the microtask queue and the Promise records are in jobs.rs,
//! the promise operations are `Cx` methods in `abi/jobs.rs`, the tests are in `tests/jobs.rs`.
//!
//! Realms (M3-P1 REALMS, H22): an isolate holds N realms. Each is a REALM record (realm.rs);
//! the current realm's values are the roots, and a cross-realm call switches them
//! (`realm::switch`, V8-A10). The embedder API is in `abi/realm.rs` (`Cx::create_realm`,
//! `Cx::with_realm`, `Cx::evaluate_script_in`). Async functions (M3-P1 ASYNC): an async
//! function's state is a GENERATOR object in `MODE_ASYNC_FUNCTION`, and the runtime ids 32-35
//! are in `interp/slow/async_fn.rs`. Each resume after `await` is one promise reaction job
//! (`generator::resume_async_function`), so an `await` adds no native recursion.
//!
//! Month-3 modules (OBJECT_LAYOUT 3). Built in M3-P1: ordered_table.rs (the Map, Set, WeakMap
//! and WeakSet tables, B-COLLECTIONS), `abi/collections.rs`, `abi/time.rs` (the clock and
//! LocalTZA, B-DATE) and `abi/realm.rs`. M3-P0 stubs until their owners fill them:
//! arguments.rs (arguments objects, M3-P2 VM-ARGS), module.rs (module records and namespaces,
//! M3-P2 MODULE), facts_export.rs (the facts exporter, M3-P3 FACTS-EXPORT).
//!
//! Month-4 modules (M4-P0 stubs, OBJECT_LAYOUT 4 and IC_LAYOUT 2; plan v9 §2.5). Each is
//! empty or a hook that changes nothing until its owner fills it: validity.rs (validity cells,
//! M4-P1 VALIDITY), profile.rs (T0 recording and the IC_LAYOUT 2 slot table, M4-P1 PROFILE),
//! literal_site.rs and construct_ic.rs (allocation sites, M4-P1 OBJ-ALLOC), keyed_ic.rs (keyed
//! access, M4-P3 KEYED), instance_ic.rs and stub_cache.rs (M4-P3 IC2), regexp.rs and
//! `abi/regexp.rs` (M4-P2 RX0A, M4-P3 REGEXP1), proxy.rs (M4-P4 PROXY1), weakref.rs and
//! `abi/weakref.rs` (M4-P4 WEAKREF). `interp/ops.rs` calls the hooks (it has no owner after
//! M4-P0).
//!
//! Month-5 modules (M5-P0, plan v10 §2.4; OBJECT_LAYOUT 5, IC_LAYOUT 3, VM_LAYOUT 1):
//! [`layout`] (the layout facts native code bakes in, C-18; ungated since M5-P0) and exec.rs
//! ([`ExecOptions`], [`Isolate::new_with`] and [`build_info`], C-23). Filled in M5-P1:
//! deopt.rs (the deopt core and its side table, SPIKEC-5), fuse.rs (the fuse registry, FR0;
//! hook `shape::note_change`), elements.rs (elements kinds step 1, ELEM1; M5-P2 ELEM2 adds
//! DICTIONARY). Filled in M5-P2: slack.rs (the in-object capacity of `new F`, SLACK; hook
//! `call::ordinary_create_from_constructor` -> `slack::instance_capacity`), field_rep.rs (field
//! representation and const bits, CONSTREP; hooks in the `props` field stores and the store
//! IC hits). The month-5 stub left (marked on its first line) changes nothing until its owner
//! fills it: external_ref.rs (`ExternalRef<T>`, M5-P3 M1A-CODE).
//!
//! ADRs: H14 frames and dispatch (docs/adr/H14-frames-and-dispatch.md), H15 interpreter
//! dispatch, H10 rooting, H06 values.
#![cfg_attr(feature = "become", feature(explicit_tail_calls))]
#![cfg_attr(feature = "become", allow(incomplete_features))]

pub mod abi;
mod accessor;
mod arguments;
mod atoms;
mod bound;
mod builtins;
mod call;
mod compiler;
mod construct_ic;
mod conv;
mod deopt;
mod descriptor;
mod dispatch;
mod elements;
mod error;
mod exec;
mod exotic;
mod external_ref;
mod facts_export;
mod feedback;
mod field_rep;
mod frame;
mod function_info;
mod fuse;
mod generator;
mod host;
mod ic;
mod instance_ic;
mod interp;
mod isolate;
mod jobs;
mod key;
mod keyed_ic;
mod kinds;
#[doc(hidden)]
pub mod layout;
mod literal_site;
mod module;
// The native entry (C-10): in every build; its spike API is behind `spike-native` (below).
mod native;
mod natives;
mod numconv;
mod object;
mod ordered_table;
mod profile;
mod property;
mod props;
mod proxy;
mod realm;
mod regexp;
mod runtime;
mod shape;
mod slack;
mod string;
mod stub_cache;
mod symbol;
mod unit;
mod unit_loader;
mod validity;
pub mod versions;
mod weakref;
mod wrapper;

// Month-6 modules (M6-P0 VM6, plan v11 §2.4 step 0; V11-A21), stubs that their owners fill:
// ic_ops.rs (`ops_of`, the CacheIR-like op lists of the T0 ICs; filled by M6-P1 CBC, read only
// by the tests until a compiled tier reads it), props_index.rs (Dictionary v1, the hash index of
// a large dictionary; hook `props::dict_find` -> `props_index::find`, built by M6-P1 DICT1) and
// select.rs (`select()`, doc 13 C-15, M6-P0 MODES6; first caller: unit_loader.rs, M7-P4).
mod ic_ops;
mod props_index;
mod select;

// BigInt values (spec-temporal B2; OBJECT_LAYOUT 7): the BIGINT cell (kind 9) and its operations.
mod bigint;

mod array_buffer; // TypedArray TA-1 (OBJECT_LAYOUT 9): ArrayBuffer and SAB data, BYTES, the BufferTable
mod typed_array; // TypedArray TA-1 (OBJECT_LAYOUT 9): the view cells and their exotic methods

#[cfg(test)]
mod tests;

pub use abi::{
    Args, ArgsBuilder, ArrayIterKind, CollectionCursor, Cx, Downcast, FlatStr, HostFn, IntoKey,
    IteratorRecord, JsArray, JsFunction, JsKey, JsObject, JsString, JsSymbol, JsValue, Local,
    MatchRecord, ObjectClass, PrimitiveType, PromiseCapability, PromiseRejectionOperation,
    PromiseRejectionTracker, PromiseState, PropertyDescriptor, Rest, ScratchList, SharedBlockRef,
    ToPrimitiveHint,
};
pub use atoms::Atom;
pub use builtins::{
    BuiltinDef, BuiltinId, BuiltinKind, BuiltinModule, BuiltinTable, ConstValue, IntrinsicDef,
    IntrinsicId, IntrinsicValue, ObjectProps, PropAttrs, PropDef, PropKey, PropValue,
    WellKnownSymbol, check_tables,
};
pub use compiler::{CompileFailure, CompileResult, CompilerHooks};
pub use dispatch::{
    CodeEntry, CodeRegistry, DispatchKind, DispatchSlot, NO_CODE, RangeRegistry, RegistryError,
};
pub use ep9_value::{JsResult, Throw, Value};
pub use exec::{BuildInfo, ExecOptions, Feedback, Preset, build_info};
pub use host::HostCx;
pub use interp::Dispatch;
pub use isolate::{
    Exception, InterruptHandle, Isolate, IsolateConfig, IsolateError, SourceLocation,
    TerminateHandle,
};
pub use runtime::ErrorKind;
pub use string::JsStr;
// Spike B (feature `spike-native` only): the native-entry API of `native.rs`.
#[cfg(feature = "spike-native")]
pub use native::{
    InstallError, NativeCx, NativeEntry, NativeTarget, RtCompareFn, RtFn0, RtFn1, RtFn2,
    RuntimeTable, call_from_rust, cmp, install_native, install_native_b, lookup_code,
    native_target, uninstall_native,
};

/// Facts for tests and tools (not a stable API).
#[doc(hidden)]
pub mod internals {
    /// FASTDIFF (route P0-FASTHITS): the hit count of each Switch human fast path on this
    /// thread since the last call (then 0), by its FASTDIFF row name. Feature `fastdiff-hits`.
    #[cfg(feature = "fastdiff-hits")]
    pub fn take_fastdiff_hits() -> Vec<(&'static str, u64)> {
        crate::call::fast_hits::take()
    }
    /// Opcodes with a cold block in the interpreter DSL.
    pub const COLD_OPCODES: usize = crate::interp::ops::COLD_COUNT;
    /// Handlers per threaded expansion: one per opcode, one per cold block, plus `Exit`,
    /// `BadOpcode` and `ConstructReturn` (the `cargo xtask disasm-check --expect` value).
    pub const THREADED_HANDLERS: usize = ep9_bytecode::OPCODE_COUNT + COLD_OPCODES + 3;
    /// The VM's own builtin table (block 0): for its id golden
    /// (`crates/ep9_vm/tests/golden/vm-builtin-ids.txt`) and table checks.
    pub static VM_BUILTINS: &crate::BuiltinTable = &crate::natives::VM_TABLE;
}
