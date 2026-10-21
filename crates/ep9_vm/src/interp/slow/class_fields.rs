#![allow(dead_code)] // m2-stub
//! Class fields, static blocks and private methods in the VM (owner P2-CLS-VM): runtime ids
//! 24-25 (`SetClassFieldsInitializer`, `RunClassFieldsInitializer`, ABI 3), which
//! `slow/runtime.rs` routes here. Id 23 `CreatePrivateName` is in `slow/private.rs`. The field
//! initializer is stored on the constructor under a VM-internal private symbol (C8): the
//! FUNCTION layout has no free field.
//!
//! P0 stubs throw `not implemented in ep9 v0: <name>`, the text of the `CallRuntime`
//! catch-all arm before the ids were routed.

use crate::conv::Res;
use crate::error::not_implemented;
use crate::interp::Cx;

/// `SetClassFieldsInitializer(constructor, initializer)` (runtime id 24): stores the instance
/// members initializer on `constructor`. Stub.
pub(crate) fn set_class_fields_initializer(
    cx: &mut Cx,
    _constructor: u64,
    _initializer: u64,
) -> Res<u64> {
    Err(not_implemented(cx, "SetClassFieldsInitializer"))
}

/// `RunClassFieldsInitializer(constructor, receiver)` (runtime id 25): calls the initializer
/// stored on `constructor` with `this` = `receiver` (InitializeInstanceElements);
/// nothing if there is none. Stub.
pub(crate) fn run_class_fields_initializer(
    cx: &mut Cx,
    _constructor: u64,
    _receiver: u64,
) -> Res<u64> {
    Err(not_implemented(cx, "RunClassFieldsInitializer"))
}
