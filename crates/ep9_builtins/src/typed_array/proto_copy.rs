#![allow(dead_code)] // m8-stub
//! The copy, species and sort methods of `%TypedArray.prototype%`
//! (`sec-properties-of-the-%typedarray%-prototype-object`). Owner: TypedArray TA-4 R4 PROTO-B
//! (this file and `species.rs`). Until R4 lands, each method is a stub that throws a TypeError
//! (m8-stub, TypedArray TA-4 R1).

use ep9_vm::{Args, Cx, JsResult, Local};

/// The TypeError of a function that is not built yet.
fn stub<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s>> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-4)");
    Err(cx.throw_type_error(&msg))
}

/// `%TypedArray%.prototype.copyWithin ( target, start [ , end ] )`
/// (`sec-%typedarray%.prototype.copywithin`).
pub(super) fn copy_within<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.copyWithin")
}

/// `%TypedArray%.prototype.fill ( value [ , start [ , end ] ] )`
/// (`sec-%typedarray%.prototype.fill`).
pub(super) fn fill<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.fill")
}

/// `%TypedArray%.prototype.filter ( callback [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.filter`).
pub(super) fn filter<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.filter")
}

/// `%TypedArray%.prototype.map ( callback [ , thisArg ] )` (`sec-%typedarray%.prototype.map`).
pub(super) fn map<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.map")
}

/// `%TypedArray%.prototype.reverse ( )` (`sec-%typedarray%.prototype.reverse`).
pub(super) fn reverse<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.reverse")
}

/// `%TypedArray%.prototype.set ( source [ , offset ] )` (`sec-%typedarray%.prototype.set`).
pub(super) fn set<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.set")
}

/// `%TypedArray%.prototype.slice ( start, end )` (`sec-%typedarray%.prototype.slice`).
pub(super) fn slice<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.slice")
}

/// `%TypedArray%.prototype.sort ( comparator )` (`sec-%typedarray%.prototype.sort`).
pub(super) fn sort<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.sort")
}

/// `%TypedArray%.prototype.subarray ( start, end )` (`sec-%typedarray%.prototype.subarray`).
pub(super) fn subarray<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.subarray")
}

/// `%TypedArray%.prototype.toReversed ( )` (`sec-%typedarray%.prototype.toreversed`).
pub(super) fn to_reversed<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.toReversed")
}

/// `%TypedArray%.prototype.toSorted ( comparator )` (`sec-%typedarray%.prototype.tosorted`).
pub(super) fn to_sorted<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.toSorted")
}

/// `%TypedArray%.prototype.with ( index, value )` (`sec-%typedarray%.prototype.with`).
pub(super) fn with<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.with")
}
