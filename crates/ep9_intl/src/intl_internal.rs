//! The ep9-internal Intl functions (block 47; INTL W5, design INTL-I1 §1.4 and §4.6):
//! `%IntlTemporalToLocaleString%`, the function that the 8 Temporal `toLocaleString` methods
//! call (ECMA-402 with Temporal: CreateDateTimeFormat and FormatDateTime, or DurationFormat
//! for a Temporal.Duration).
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

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    Local, ObjectProps,
};

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
/// value. Skeleton: the toString text of the this value (the Temporal methods do not call it
/// yet).
fn temporal_to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.to_string(this)?.into())
}
