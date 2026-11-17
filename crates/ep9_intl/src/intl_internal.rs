//! The ep9-internal Intl functions (block 47; INTL W5, design INTL-I1 §1.4 and §4.6):
//! `%IntlTemporalToLocaleString%`, the function that the 8 Temporal `toLocaleString` methods
//! call (ECMA-402 with Temporal: CreateDateTimeFormat and FormatDateTime,
//! `date_time_format::temporal_to_locale_string`, or DurationFormat for a Temporal.Duration,
//! `duration_format::to_locale_string`).
//!
//! ```text
//! %IntlTemporalToLocaleString%   ep9-internal: no property names it, so no script reaches it
//!   ep9_temporal <type>.prototype.toLocaleString(locales, options)
//!     -> cx.has_intrinsic(IntrinsicId::IntlTemporalToLocaleString)
//!     -> cx.call(cx.intrinsic(..), this, [locales, options])
//! ```
//!
//! The module has no props, so the group rule (INTL-I1-D8) leaves it out of the lazy group
//! "Intl": realm init makes the function object, and `Cx::has_intrinsic` is true without an
//! install. `ep9_builtins::MODULES` lists it after `duration_format`; `crate::MODULES` (the
//! group "Intl") does not.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-intl_internal.txt`.

use ep9_temporal::intl_view::{self, Kind};
use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    Local, ObjectProps,
};

use crate::{date_time_format, duration_format};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 47;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("toLocaleString", 0, temporal_to_locale_string), // 0
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::IntlTemporalToLocaleString,
    value: IntrinsicValue::Builtin(0),
}];

/// The properties this module adds: none (the module stays out of every lazy group).
pub const PROPS: &[ObjectProps] = &[];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "intl_internal",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// `%IntlTemporalToLocaleString%` ( locales, options ) with the Temporal object as the this
/// value (the caller, the toLocaleString method of its type, checked the internal slot): a
/// Duration through DurationFormat, the other kinds through DateTimeFormat.
fn temporal_to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let (locales, options) = (args.get(0), args.get(1));
    match intl_view::kind(cx, this) {
        Some(Kind::Duration) => {
            let fields = intl_view::to_temporal_duration(cx, this)?;
            duration_format::to_locale_string(cx, fields, locales, options)
        }
        Some(kind) => date_time_format::temporal_to_locale_string(cx, kind, this, locales, options),
        None => Err(cx.throw_type_error("toLocaleString: the this value is not a Temporal object")),
    }
}
