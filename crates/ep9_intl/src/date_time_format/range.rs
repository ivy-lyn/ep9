//! Intl.DateTimeFormat.prototype.formatRange and formatRangeToParts (ECMA-402 §11.3.5,
//! §11.3.6; ids 45.5 and 45.6). INTL W5: the skeleton makes the two functions; W5 DTF-RANGE
//! writes their bodies (PartitionDateTimeRangePattern over the ICU4X range parts).

use ep9_vm::{Args, Cx, JsResult, Local};

/// Intl.DateTimeFormat.prototype.formatRange ( startDate, endDate ) (11.3.5). Skeleton: a
/// TypeError.
pub(super) fn format_range<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.DateTimeFormat.prototype.formatRange is not supported yet"))
}

/// Intl.DateTimeFormat.prototype.formatRangeToParts ( startDate, endDate ) (11.3.6).
/// Skeleton: a TypeError.
pub(super) fn format_range_to_parts<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx
        .throw_type_error("Intl.DateTimeFormat.prototype.formatRangeToParts is not supported yet"))
}
