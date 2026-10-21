#![allow(dead_code)] // m2-stub
//! Tests of the native entry (feature `spike-native` only; owners P1-NATIVE-ENTRY, then
//! P2-SPIKE-B): calls into hand-written native stubs, GC stress across a native frame, and an
//! exception that crosses one. P0 checks only the two `Cx` fields that native code reads.

use crate::{Isolate, IsolateConfig};

#[test]
fn the_native_cx_fields_are_set_at_isolate_creation() {
    let iso = Isolate::new(IsolateConfig::default()).unwrap();
    assert_eq!(iso.cx.native_limit, iso.rt.native_limit);
    assert_ne!(iso.cx.native_limit, 0);
    assert!(core::ptr::eq(
        iso.cx.runtime_table,
        &crate::native::RUNTIME_TABLE
    ));
}
