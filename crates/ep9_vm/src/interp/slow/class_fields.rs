#![allow(dead_code)] // m2-stub
//! Class fields, static blocks and private methods in the VM (owner P2-CLS-VM): runtime ids
//! 23-25 (`CreatePrivateName`, `SetClassFieldsInitializer`, `RunClassFieldsInitializer`,
//! ABI 3), which `slow/runtime.rs` routes here after the P0-LANG merge. The field
//! initializer is stored on the constructor under a VM-internal private symbol (C8): the
//! FUNCTION layout has no free field.
//!
//! P0: no entry yet (the ids arrive with ABI 3).
