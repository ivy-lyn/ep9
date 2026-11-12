#![allow(dead_code)] // m8-stub
//! The abstract operations of the `%TypedArray.prototype%` methods that make a new TypedArray:
//! TypedArraySpeciesCreate (`sec-typedarray-species-create`), TypedArrayCreateSameType
//! (`sec-typedarray-create-same-type`) and SpeciesConstructor (`sec-speciesconstructor`, a
//! private copy, the crate form). Owner: TypedArray TA-4 R4 PROTO-B. Until R4 lands, each one is
//! a stub that throws a TypeError and has no caller (m8-stub, TypedArray TA-4 R1); R4 sets the
//! final signatures.

use ep9_vm::abi::JsObject;
use ep9_vm::{Cx, JsResult, Local};

/// The TypeError of an operation that is not built yet.
fn stub<'s, T>(cx: &mut Cx<'s>, name: &str) -> JsResult<T> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-4)");
    Err(cx.throw_type_error(&msg))
}

/// TypedArraySpeciesCreate ( exemplar, argumentList ) (`sec-typedarray-species-create`).
pub(super) fn typed_array_species_create<'s>(
    cx: &mut Cx<'s>,
    _exemplar: Local<'s, JsObject>,
    _args: &[Local<'s>],
) -> JsResult<Local<'s, JsObject>> {
    stub(cx, "TypedArraySpeciesCreate")
}

/// TypedArrayCreateSameType ( exemplar, argumentList ) (`sec-typedarray-create-same-type`).
pub(super) fn typed_array_create_same_type<'s>(
    cx: &mut Cx<'s>,
    _exemplar: Local<'s, JsObject>,
    _args: &[Local<'s>],
) -> JsResult<Local<'s, JsObject>> {
    stub(cx, "TypedArrayCreateSameType")
}

/// SpeciesConstructor ( O, defaultConstructor ) (`sec-speciesconstructor`).
pub(super) fn species_constructor<'s>(
    cx: &mut Cx<'s>,
    _o: Local<'s, JsObject>,
    _default_constructor: Local<'s, JsObject>,
) -> JsResult<Local<'s>> {
    stub(cx, "SpeciesConstructor")
}
