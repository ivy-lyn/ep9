#![allow(dead_code)] // m4-stub
//! WeakRef and FinalizationRegistry on the builtin ABI (owner M4-P4 WEAKREF; GFF-WEAKREF,
//! V9-D23): the part-2 `Cx` methods that `ep9_builtins` weakref.rs uses (WeakRef creation and
//! deref with AddToKeptObjects, FinalizationRegistry register, unregister and cleanup), over
//! `weakref.rs` (the WEAK_CELL kind and `Runtime::kept_objects`).
//!
//! M4-P0 declares the module and its `impl Cx` block (empty), because `abi/mod.rs` is frozen
//! after M4-P0. A new method is a part-2 addition: a new `h20-core.txt` line (EP9_BLESS) and an
//! H20 Change-log row (V9-A1, V9-A14).

use super::Cx;

impl<'s> Cx<'s> {}
