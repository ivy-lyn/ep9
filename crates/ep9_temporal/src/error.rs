//! Error mapping (design §3.5): a `temporal_rs` error becomes a JS exception.
//!
//! | `temporal_rs::error::ErrorKind` | ep9 throws |
//! |---|---|
//! | `Range` | RangeError |
//! | `Syntax` | RangeError (the spec makes a bad Temporal string a RangeError) |
//! | `Type` | TypeError |
//! | `Generic`, `Assert` | Error (`Assert` is a defect: debug builds panic) |

use ep9_vm::{Cx, ErrorKind, JsResult, Throw};
use temporal_rs::TemporalError;
use temporal_rs::error::ErrorKind as TemporalKind;

/// Throws the JS error for `e`.
pub(crate) fn throw(cx: &mut Cx<'_>, e: TemporalError) -> Throw {
    let kind = e.kind();
    let msg = e.into_message();
    match kind {
        TemporalKind::Range | TemporalKind::Syntax => cx.throw_range_error(msg),
        TemporalKind::Type => cx.throw_type_error(msg),
        TemporalKind::Generic => cx.throw_error(ErrorKind::Error, msg),
        TemporalKind::Assert => {
            debug_assert!(false, "temporal_rs assertion: {msg}");
            cx.throw_error(ErrorKind::Error, msg)
        }
    }
}

/// `result.or_throw(cx)?`: a `temporal_rs` result as a `JsResult`.
pub(crate) trait OrThrow<T> {
    fn or_throw(self, cx: &mut Cx<'_>) -> JsResult<T>;
}

impl<T> OrThrow<T> for Result<T, TemporalError> {
    fn or_throw(self, cx: &mut Cx<'_>) -> JsResult<T> {
        self.map_err(|e| throw(cx, e))
    }
}
