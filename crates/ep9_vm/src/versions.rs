//! Versions of the VM-internal contracts that native code (JIT, AOT units) bakes in. An AOT
//! unit records them in its header (H35); a unit whose versions differ from the running
//! VM's is not loaded as native code. Version 0 means "no native form yet". Bump a version
//! with every change to its contract; these are not the bytecode ABI (`ep9_bytecode`
//! `ABI_VERSION`).

/// Object layout: word0, the object header, fixed fields, slots (H8, H17). 2 (M2-P0): the
/// GENERATOR and SLOT_OBJECT kinds, the ITERATOR kind field, the CONTEXT scope word as a
/// native word (object.rs). 3 (M3-P0): the FUNCTION realm field (+48; the in-object slots
/// move to +56); the JS object kinds ARGUMENTS (25) and MODULE_NAMESPACE (26); the internal
/// cells MODULE (76), ORDERED_TABLE (77, with a weak-keys flag) and REALM (78).
pub const OBJECT_LAYOUT_VERSION: u32 = 3;
/// Inline-cache and feedback data layout (H16).
pub const IC_LAYOUT_VERSION: u32 = 1;
/// The frame layout: 8-slot header, arguments, registers (H14). 2 (M2-P0): fp-3 holds
/// new.target, or the generator object in a resumed frame (`frame::slot::NEW_TARGET`).
pub const FRAME_VERSION: u32 = 2;
/// The builtin (host function) ABI (H20).
pub const BUILTIN_ABI_VERSION: u32 = 1;
/// Deoptimization metadata (H29): none exists yet.
pub const DEOPT_META_VERSION: u32 = 0;
