//! The Temporal namespace (block 19): `%Temporal%` and `%Temporal.Now%`, both ordinary
//! objects (Temporal §1, §2). Each class module adds its constructor to `%Temporal%`; the
//! `Temporal.Now` functions are here (WP T9).
//!
//! ```text
//! globalThis.Temporal {w, c}  %Temporal%      [@@toStringTag] "Temporal" {c}, Now {w, c}
//! Temporal.Now               %Temporal.Now%  [@@toStringTag] "Temporal.Now" {c},
//!                                            timeZoneId, instant, plainDateTimeISO,
//!                                            zonedDateTimeISO, plainDateISO, plainTimeISO
//! clock: Cx::now_epoch_ns; host zone: Cx::system_time_zone_id, checked with the tz data and
//!        mapped to its primary identifier (None or an unknown name: "UTC", design SPEC-T-D16)
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-temporal.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};
use temporal_rs::{Instant, TimeZone, ZonedDateTime};

use crate::error::OrThrow;
use crate::fields::to_temporal_time_zone_identifier;
use crate::tzdb::provider;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 19;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("timeZoneId", 0, now_time_zone_id), // 0 (T9)
    BuiltinDef::method("instant", 0, now_instant),         // 1
    BuiltinDef::method("plainDateTimeISO", 0, now_plain_date_time_iso), // 2
    BuiltinDef::method("zonedDateTimeISO", 0, now_zoned_date_time_iso), // 3
    BuiltinDef::method("plainDateISO", 0, now_plain_date_iso), // 4
    BuiltinDef::method("plainTimeISO", 0, now_plain_time_iso), // 5
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::Temporal,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalNow,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[
            // Temporal [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal"),
                PropAttrs::CONFIGURABLE,
            ),
            // Temporal.Now ({w, c}).
            PropDef::intrinsic(
                PropKey::Str("Now"),
                IntrinsicId::TemporalNow,
                PropAttrs::METHOD,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalNow,
        props: &[
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.Now"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::method(PropKey::Str("timeZoneId"), 0),
            PropDef::method(PropKey::Str("instant"), 1),
            PropDef::method(PropKey::Str("plainDateTimeISO"), 2),
            PropDef::method(PropKey::Str("zonedDateTimeISO"), 3),
            PropDef::method(PropKey::Str("plainDateISO"), 4),
            PropDef::method(PropKey::Str("plainTimeISO"), 5),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[PropDef::intrinsic(
            PropKey::Str("Temporal"),
            IntrinsicId::Temporal,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "temporal",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- Temporal.Now (§2) ----------------------------------------------------------------------

/// SystemTimeZoneIdentifier: the primary identifier of the host zone name if the tz data knows
/// it (a link such as "US/Pacific" gives "America/Los_Angeles"), else "UTC".
fn system_time_zone(cx: &mut Cx<'_>) -> TimeZone {
    cx.system_time_zone_id()
        .and_then(|id| TimeZone::try_from_identifier_str_with_provider(&id, provider()).ok())
        .and_then(|tz| tz.primary_identifier_with_provider(provider()).ok())
        .unwrap_or_else(|| TimeZone::utc_with_provider(provider()))
}

/// SystemUTCEpochNanoseconds: the clock, clamped to the Instant range.
fn system_instant(cx: &mut Cx<'_>) -> JsResult<Instant> {
    const LIMIT: i128 = 8_640_000_000_000_000_000_000;
    let ns = cx.now_epoch_ns().clamp(-LIMIT, LIMIT);
    Instant::try_new(ns).or_throw(cx)
}

/// SystemDateTime(temporalTimeZoneLike) as a ZonedDateTime: undefined is the system
/// zone, else ToTemporalTimeZoneIdentifier; the time is the clock's.
fn system_zoned<'s>(cx: &mut Cx<'s>, tz_like: Local<'s>) -> JsResult<ZonedDateTime> {
    let tz = if tz_like.is_undefined() {
        system_time_zone(cx)
    } else {
        to_temporal_time_zone_identifier(cx, tz_like)?
    };
    let i = system_instant(cx)?;
    ZonedDateTime::try_new_iso_from_instant_with_provider(i, tz, provider()).or_throw(cx)
}

/// Temporal.Now.timeZoneId ( ) (§2.2.1).
fn now_time_zone_id<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let tz = system_time_zone(cx);
    let id = tz.identifier_with_provider(provider()).or_throw(cx)?;
    cx.new_string(&id).map(|s| s.value())
}

/// Temporal.Now.instant ( ) (§2.2.2).
fn now_instant<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let i = system_instant(cx)?;
    crate::instant::create(cx, &i, None)
}

/// Temporal.Now.plainDateTimeISO ( [ temporalTimeZoneLike ] ) (§2.2.3).
fn now_plain_date_time_iso<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = system_zoned(cx, args.get(0))?;
    crate::plain_date_time::create(cx, &z.to_plain_date_time(), None)
}

/// Temporal.Now.zonedDateTimeISO ( [ temporalTimeZoneLike ] ) (§2.2.4).
fn now_zoned_date_time_iso<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = system_zoned(cx, args.get(0))?;
    crate::zoned_date_time::create(cx, &z, None)
}

/// Temporal.Now.plainDateISO ( [ temporalTimeZoneLike ] ) (§2.2.5).
fn now_plain_date_iso<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = system_zoned(cx, args.get(0))?;
    crate::plain_date::create(cx, &z.to_plain_date(), None)
}

/// Temporal.Now.plainTimeISO ( [ temporalTimeZoneLike ] ) (§2.2.6).
fn now_plain_time_iso<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = system_zoned(cx, args.get(0))?;
    crate::plain_time::create(cx, &z.to_plain_time(), None)
}
