//! Tests of `ExternalRef<T>` on live heap cells (C-26, H25; M5-P3 M1A-CODE): each H25
//! inventory field holds the `ExternalRef` word of its pointer, the word is a non-cell Value
//! (so the GC never traces it), and a GC leaves it as it was. The type's own round trip is in
//! `external_ref.rs`; the FEEDBACK Call key half is compared with the callee's FunctionInfo
//! word in `tests::profile`; `cargo xtask extref-lint` checks the sources.

use ep9_value::Value;

use super::{config, isolate};
use crate::Dispatch;
use crate::builtins::{self, IntrinsicId};
use crate::external_ref::ExternalRef;
use crate::frame::UNDEFINED;
use crate::kinds;
use crate::object::{self, cell, ctxf, fun, rd};
use crate::unit::{ContextScope, FunctionInfo};

/// The word is a valid double Value that is not a cell: the GC skips it (H14 native word).
fn is_native_word(w: u64) -> bool {
    if !Value::is_valid_bits(w) {
        return false;
    }
    // SAFETY: the bits are valid (checked above).
    let v = unsafe { Value::from_bits_unchecked(w) };
    !v.is_cell() && v.is_double()
}

/// FUNCTION +24 holds `ExternalRef<FunctionInfo>`; `object::function_info` reads it back, and
/// a GC changes neither the word nor the pointer.
#[test]
fn the_function_info_word_is_an_external_ref() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let read = |iso: &crate::Isolate| {
        let f = builtins::intrinsic(&iso.rt, IntrinsicId::Function);
        assert!(kinds::is_function(f));
        let w = rd(cell(f), fun::INFO);
        let fi: *const FunctionInfo = object::function_info(f);
        (w, fi)
    };
    let (w, fi) = read(&iso);
    assert!(is_native_word(w), "{w:#x}");
    assert_eq!(w, ExternalRef::new(fi).to_word());
    assert_eq!(ExternalRef::<FunctionInfo>::from_word(w).get(), fi);
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    assert_eq!(read(&iso), (w, fi));
}

/// CONTEXT `ctxf::SCOPE` holds `ExternalRef<ContextScope>` of the pointer `new_context` got.
#[test]
fn the_context_scope_word_is_an_external_ref() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let scope = ContextScope {
        unit: core::ptr::null(),
        scope: core::ptr::null(),
    };
    let p: *const ContextScope = &scope;
    let c = object::new_context(&iso.rt, UNDEFINED, p, 2);
    let w = rd(cell(c), ctxf::SCOPE);
    assert!(is_native_word(w), "{w:#x}");
    assert_eq!(w, ExternalRef::new(p).to_word());
    assert_eq!(ExternalRef::<ContextScope>::from_word(w).get(), p);
    std::hint::black_box(c);
}

/// The null pointer (a CONTEXT made with no scope, as `tests::layout5` makes one) is a native
/// word too: 0 + 2^49, a subnormal double.
#[test]
fn a_null_external_ref_is_a_native_word() {
    let r = ExternalRef::<u8>::new(core::ptr::null());
    assert!(is_native_word(r.to_word()));
    assert!(r.get().is_null());
}
