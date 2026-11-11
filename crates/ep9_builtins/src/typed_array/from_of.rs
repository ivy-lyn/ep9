#![allow(dead_code)] // m8-stub
//! `%TypedArray%.from` (§23.2.2.1, `sec-%typedarray%.from`) and `%TypedArray%.of` (§23.2.2.2,
//! `sec-%typedarray%.of`). Owner: TypedArray TA-3 TA-CTORS R3. Until R3 lands, each function
//! is a stub that throws a TypeError (m8-stub, TypedArray TA-3 R1).

use ep9_vm::{Args, Cx, JsResult, Local};

/// The TypeError of a function that is not built yet.
fn stub<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s>> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-3)");
    Err(cx.throw_type_error(&msg))
}

/// `%TypedArray%.from ( source [ , mapper [ , thisArg ] ] )`.
pub(super) fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.from")
}

/// `%TypedArray%.of ( ...items )`.
pub(super) fn of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.of")
}
