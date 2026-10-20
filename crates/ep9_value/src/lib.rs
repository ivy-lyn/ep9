//! NaN-boxed `Value`, `JsResult`, `Throw`, tags and NaN purification (H6), the 8-byte object
//! header word `Word0` (H8), and exact ToInt32 / ToUint32 ([`to_int32`], [`to_uint32`]).
//! This crate has no dependencies.
//!
//! | Kind      | 64-bit pattern                 | Test                          |
//! |-----------|--------------------------------|-------------------------------|
//! | cell      | `0x0000_PPPP_PPPP_PPPP`, P < 2^47, 8-aligned | `v & NOT_CELL_MASK == 0` |
//! | int32     | `0xFFFE_0000_IIII_IIII`        | `v & NUMBER_TAG == NUMBER_TAG` |
//! | double    | f64 bits + 2^49 (top 16 bits 0x0002..=0xFFF2) | number, not int32 |
//! | immediate | null 0x02, false 0x06, true 0x07, undefined 0x0a, empty 0x22 | exact compare |
//!
//! The bit pattern 0 is never a `Value`. It is the niche that makes `Option<Value>` and
//! `JsResult<Value>` 8 bytes, so a `JsResult<Value>` returns in one register.
#![no_std]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("ep9_value needs a 64-bit target: Value stores raw cell pointers");
#[cfg(not(target_endian = "little"))]
compile_error!("ep9_value needs a little-endian target: Word0 byte offsets assume it");

#[cfg(test)]
extern crate std;

mod conv;
mod header;
mod value;

pub use conv::{to_int32, to_uint32};
pub use header::{ShapeId, Word0};
pub use value::{Unpacked, Value, encoding};

/// Marker for "an exception is pending in the isolate" (H20). The exception value itself lives
/// in the isolate. `Throw` is a 1-ZST, so core guarantees that `JsResult<Value>` has the size,
/// alignment and call ABI of `Value` (core::result, "Representation").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Throw;

/// Result of every operation that can throw. `JsResult<Value>` is 8 bytes and returns in one
/// register. rustc puts `Err(Throw)` in the zero niche; a test pins that choice.
pub type JsResult<T = Value> = Result<T, Throw>;

// Static asserts for SP-Value (09-milestones §4) and the 08-repo size checks. The Word0 field
// offsets are asserted in `header.rs`, where the fields are visible.
const _: () = {
    use core::mem::{align_of, size_of};
    assert!(size_of::<Value>() == 8);
    assert!(align_of::<Value>() == 8);
    assert!(size_of::<Option<Value>>() == 8);
    assert!(size_of::<JsResult<Value>>() == 8);
    assert!(size_of::<Throw>() == 0);
    assert!(align_of::<Throw>() == 1);
    assert!(size_of::<Word0>() == 8);
    assert!(align_of::<Word0>() == 8);
    assert!(size_of::<ShapeId>() == 4);
};

#[cfg(test)]
mod conv_node_cases;
#[cfg(test)]
mod tests;
