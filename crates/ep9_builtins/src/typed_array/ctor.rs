#![allow(dead_code)] // m8-stub
//! `%TypedArray%` (§23.2.1, `sec-%typedarray%`), the 12 TypedArray constructors (§23.2.5,
//! `sec-typedarray`), `get %TypedArray%[@@species]` (§23.2.2.4) and the getters `buffer`,
//! `byteLength`, `byteOffset` and `length` of `%TypedArray.prototype%` (§23.2.3; decision O3).
//! Owner: TypedArray TA-3 TA-CTORS R2. Until R2 lands, each function is a stub that throws a
//! TypeError (m8-stub, TypedArray TA-3 R1).

use ep9_vm::{Args, Cx, JsResult, Local};

use super::VIEWS;

/// The TypeError of a function that is not built yet.
fn stub<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s>> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-3)");
    Err(cx.throw_type_error(&msg))
}

/// `%TypedArray%` ( ).
pub(super) fn typed_array<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray")
}

/// `get %TypedArray%[@@species]`.
pub(super) fn species<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "get [Symbol.species]")
}

/// The getters of `%TypedArray.prototype%`.
macro_rules! getters {
    ($($f:ident $name:literal),*) => {
        $(
            pub(super) fn $f<'s>(
                cx: &mut Cx<'s>,
                _this: Local<'s>,
                _args: Args<'s>,
                _new_target: Local<'s>,
            ) -> JsResult<Local<'s>> {
                stub(cx, $name)
            }
        )*
    };
}

getters!(
    buffer "get buffer",
    byte_length "get byteLength",
    byte_offset "get byteOffset",
    length "get length"
);

/// The constructor of row `r` of [`VIEWS`].
macro_rules! views {
    ($($f:ident $r:literal),*) => {
        $(
            pub(super) fn $f<'s>(
                cx: &mut Cx<'s>,
                _this: Local<'s>,
                _args: Args<'s>,
                _new_target: Local<'s>,
            ) -> JsResult<Local<'s>> {
                stub(cx, VIEWS[$r].2)
            }
        )*
    };
}

views!(
    int8_array 0,
    uint8_array 1,
    uint8_clamped_array 2,
    int16_array 3,
    uint16_array 4,
    int32_array 5,
    uint32_array 6,
    big_int64_array 7,
    big_uint64_array 8,
    float16_array 9,
    float32_array 10,
    float64_array 11
);
