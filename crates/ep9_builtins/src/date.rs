//! Date (§21.4): module `date`, block 12. Makes `%Date%` and `%Date.prototype%`
//! (`IntrinsicId::Date`, `IntrinsicId::DatePrototype`).
//!
//! ```text
//! Date(...)            called: ToDateString(now); new: 0 args now, 1 arg a Date's value or
//!                      ToPrimitive then parse a String or ToNumber, 2-7 args local fields
//! Date.now/parse/UTC   the clock (Cx::now_epoch_ns), date/parse.rs, the date math
//! instance             ObjectClass::Date, slot 0 = [[DateValue]] (a Number); the prototype
//!                      is an ordinary object
//! %Date.prototype%     get*/getUTC* and set*/setUTC* (local fields: LocalTime, UTC through
//!                      Cx::local_tz_offset_ms), the string forms, toJSON, valueOf,
//!                      [@@toPrimitive]; Annex B getYear, setYear, toGMTString (= toUTCString)
//! date/math.rs         Day, MakeDay, MakeDate, MakeTime, TimeClip, ... over f64 (no Value)
//! date/format.rs       DateString, TimeString, TimeZoneString (empty tzName), ISO, UTC text
//! date/parse.rs        the ISO format and the toString/toUTCString forms; else NaN
//! ```
//!
//! `toLocaleString`, `toLocaleDateString` and `toLocaleTimeString` give the `toString` forms
//! (ECMA-402 replaces them later). Ids are append-only: definition `i` of [`DEFS`] is
//! `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-date.txt`.

mod format;
mod math;
mod parse;

use ep9_vm::abi::ObjectClass;
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    Local, ObjectProps, PropAttrs, PropDef, PropKey, PropValue, ToPrimitiveHint, WellKnownSymbol,
};

use self::math::{
    Fields, MS_PER_MINUTE, date_from_time, day, make_date, make_day, make_full_year, make_time,
    month_from_time, time_clip, time_within_day, year_from_time,
};
use self::parse::Parsed;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 12;

/// The internal slots of a Date object: « \[\[DateValue\]\] ».
const DATE_VALUE: usize = 0;
const DATE_SLOTS: usize = 1;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Date", 7, date),              // 0
    BuiltinDef::method("now", 0, now),                     // 1
    BuiltinDef::method("parse", 1, parse_fn),              // 2
    BuiltinDef::method("UTC", 7, utc_fn),                  // 3
    BuiltinDef::method("toString", 0, to_string),          // 4
    BuiltinDef::method("toDateString", 0, to_date_string), // 5
    BuiltinDef::method("toTimeString", 0, to_time_string), // 6
    BuiltinDef::method("toISOString", 0, to_iso_string),   // 7
    BuiltinDef::method("toUTCString", 0, to_utc_string),   // 8
    BuiltinDef::method("getDate", 0, get_date),            // 9
    BuiltinDef::method("setDate", 1, set_date),            // 10
    BuiltinDef::method("getDay", 0, get_day),              // 11
    BuiltinDef::method("getFullYear", 0, get_full_year),   // 12
    BuiltinDef::method("setFullYear", 3, set_full_year),   // 13
    BuiltinDef::method("getHours", 0, get_hours),          // 14
    BuiltinDef::method("setHours", 4, set_hours),          // 15
    BuiltinDef::method("getMilliseconds", 0, get_milliseconds), // 16
    BuiltinDef::method("setMilliseconds", 1, set_milliseconds), // 17
    BuiltinDef::method("getMinutes", 0, get_minutes),      // 18
    BuiltinDef::method("setMinutes", 3, set_minutes),      // 19
    BuiltinDef::method("getMonth", 0, get_month),          // 20
    BuiltinDef::method("setMonth", 2, set_month),          // 21
    BuiltinDef::method("getSeconds", 0, get_seconds),      // 22
    BuiltinDef::method("setSeconds", 2, set_seconds),      // 23
    BuiltinDef::method("getTime", 0, get_time),            // 24
    BuiltinDef::method("setTime", 1, set_time),            // 25
    BuiltinDef::method("getTimezoneOffset", 0, get_timezone_offset), // 26
    BuiltinDef::method("getUTCDate", 0, get_utc_date),     // 27
    BuiltinDef::method("setUTCDate", 1, set_utc_date),     // 28
    BuiltinDef::method("getUTCDay", 0, get_utc_day),       // 29
    BuiltinDef::method("getUTCFullYear", 0, get_utc_full_year), // 30
    BuiltinDef::method("setUTCFullYear", 3, set_utc_full_year), // 31
    BuiltinDef::method("getUTCHours", 0, get_utc_hours),   // 32
    BuiltinDef::method("setUTCHours", 4, set_utc_hours),   // 33
    BuiltinDef::method("getUTCMilliseconds", 0, get_utc_milliseconds), // 34
    BuiltinDef::method("setUTCMilliseconds", 1, set_utc_milliseconds), // 35
    BuiltinDef::method("getUTCMinutes", 0, get_utc_minutes), // 36
    BuiltinDef::method("setUTCMinutes", 3, set_utc_minutes), // 37
    BuiltinDef::method("getUTCMonth", 0, get_utc_month),   // 38
    BuiltinDef::method("setUTCMonth", 2, set_utc_month),   // 39
    BuiltinDef::method("getUTCSeconds", 0, get_utc_seconds), // 40
    BuiltinDef::method("setUTCSeconds", 2, set_utc_seconds), // 41
    BuiltinDef::method("valueOf", 0, get_time),            // 42
    BuiltinDef::method("getYear", 0, get_year),            // 43
    BuiltinDef::method("setYear", 1, set_year),            // 44
    BuiltinDef::method("toJSON", 1, to_json),              // 45
    BuiltinDef::method("toLocaleString", 0, to_string),    // 46
    BuiltinDef::method("toLocaleDateString", 0, to_date_string), // 47
    BuiltinDef::method("toLocaleTimeString", 0, to_time_string), // 48
    BuiltinDef::method("[Symbol.toPrimitive]", 1, to_primitive), // 49
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::Date,
        value: IntrinsicValue::Builtin(0),
    },
    // §21.4.4: %Date.prototype% is an ordinary object (not a Date instance).
    IntrinsicDef {
        id: IntrinsicId::DatePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// `key` = `DEFS[def]`, {w, c}, with a string key.
const fn m(key: &'static str, def: u16) -> PropDef {
    PropDef::method(PropKey::Str(key), def)
}

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Date,
        props: &[
            // §21.4.3.3 Date.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::DatePrototype,
                PropAttrs::NONE,
            ),
            m("now", 1),
            m("parse", 2),
            m("UTC", 3),
        ],
    },
    ObjectProps {
        target: IntrinsicId::DatePrototype,
        // The order of V8 (`Object.getOwnPropertyNames(Date.prototype)` in node).
        props: &[
            // §21.4.4.1 Date.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::Date,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 4),
            m("toDateString", 5),
            m("toTimeString", 6),
            m("toISOString", 7),
            m("toUTCString", 8),
            // §B.2.3.3 Date.prototype.toGMTString: the same function object as toUTCString.
            PropDef::alias(
                PropKey::Str("toGMTString"),
                PropKey::Str("toUTCString"),
                PropAttrs::METHOD,
            ),
            m("getDate", 9),
            m("setDate", 10),
            m("getDay", 11),
            m("getFullYear", 12),
            m("setFullYear", 13),
            m("getHours", 14),
            m("setHours", 15),
            m("getMilliseconds", 16),
            m("setMilliseconds", 17),
            m("getMinutes", 18),
            m("setMinutes", 19),
            m("getMonth", 20),
            m("setMonth", 21),
            m("getSeconds", 22),
            m("setSeconds", 23),
            m("getTime", 24),
            m("setTime", 25),
            m("getTimezoneOffset", 26),
            m("getUTCDate", 27),
            m("setUTCDate", 28),
            m("getUTCDay", 29),
            m("getUTCFullYear", 30),
            m("setUTCFullYear", 31),
            m("getUTCHours", 32),
            m("setUTCHours", 33),
            m("getUTCMilliseconds", 34),
            m("setUTCMilliseconds", 35),
            m("getUTCMinutes", 36),
            m("setUTCMinutes", 37),
            m("getUTCMonth", 38),
            m("setUTCMonth", 39),
            m("getUTCSeconds", 40),
            m("setUTCSeconds", 41),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 42),
            m("getYear", 43),
            m("setYear", 44),
            m("toJSON", 45),
            m("toLocaleString", 46),
            m("toLocaleDateString", 47),
            m("toLocaleTimeString", 48),
            // §21.4.4.45 Date.prototype [ %Symbol.toPrimitive% ] ({c}).
            PropDef {
                key: PropKey::Symbol(WellKnownSymbol::TO_PRIMITIVE),
                value: PropValue::Builtin(49),
                attrs: PropAttrs::CONFIGURABLE,
            },
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Date ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("Date"),
            IntrinsicId::Date,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "date",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the time zone and the clock (§21.4.1) ------------------------------------------------

/// LocalTime(t) (§21.4.1.25): t + the local offset at the UTC time t. `t` is finite.
fn local_time(cx: &mut Cx<'_>, t: f64) -> f64 {
    t + cx.local_tz_offset_ms(t, true)
}

/// UTC(t) (§21.4.1.26): the UTC time of the local time t.
fn utc(cx: &mut Cx<'_>, t: f64) -> f64 {
    // 1. If t is not finite, return NaN.
    if !t.is_finite() {
        return f64::NAN;
    }
    t - cx.local_tz_offset_ms(t, false)
}

/// The time value (UTC) of the current time (§21.4.2.1 step 1.a, §21.4.3.1): whole ms.
fn now_ms(cx: &mut Cx<'_>) -> f64 {
    cx.now_epoch_ns().div_euclid(1_000_000) as f64
}

/// The time value of a parsed string: TimeClip of the UTC time, NaN if it does not parse.
fn parse_time(cx: &mut Cx<'_>, s: &str) -> f64 {
    match parse::parse(s) {
        Some(Parsed::Utc(t)) => time_clip(t),
        Some(Parsed::Local(t)) => time_clip(utc(cx, t)),
        None => f64::NAN,
    }
}

/// ToDateString(tv) (§21.4.4.41.4).
fn to_date_string_text(cx: &mut Cx<'_>, tv: f64) -> String {
    // 1. If tv is NaN, return "Invalid Date".
    if tv.is_nan() {
        return "Invalid Date".to_string();
    }
    // 2. Let t be LocalTime(tv).
    let offset = cx.local_tz_offset_ms(tv, true);
    let t = tv + offset;
    // 3. Return DateString(t) + " " + TimeString(t) + TimeZoneString(tv).
    format!(
        "{} {}{}",
        format::date_string(t),
        format::time_string(t),
        format::time_zone_string(offset)
    )
}

/// A new string from Rust text, as a value.
fn string<'s>(cx: &mut Cx<'s>, s: &str) -> JsResult<Local<'s>> {
    Ok(cx.new_string(s)?.value())
}

/// thisTimeValue: RequireInternalSlot(this, \[\[DateValue\]\]), then the value.
fn this_time_value<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<f64> {
    let v = cx.slot(this, ObjectClass::Date, DATE_VALUE)?;
    Ok(v.as_number().unwrap_or(f64::NAN))
}

// ---- the constructor and its functions (§21.4.2, §21.4.3) ------------------------------------

/// Date ( ...values ) (§21.4.2.1).
fn date<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, return ToDateString(the current time).
    let Some(nt) = new_target.as_object() else {
        let now = now_ms(cx);
        let s = to_date_string_text(cx, now);
        return string(cx, &s);
    };
    // 2. Let numberOfArgs be the number of elements in values.
    let dv = match args.len() {
        // 3. numberOfArgs = 0: dv is the current time.
        0 => now_ms(cx),
        // 4. numberOfArgs = 1.
        1 => {
            let value = args.get(0);
            // b. A Date object: its [[DateValue]].
            let tv = if cx.object_class(value) == Some(ObjectClass::Date) {
                this_time_value(cx, value)?
            } else {
                // c.i. Let v be ? ToPrimitive(value).
                let v = cx.to_primitive(value, ToPrimitiveHint::Default)?;
                if v.is_string() {
                    // c.ii. Parse a String as Date.parse does.
                    let s = cx.to_rust_string(v)?;
                    parse_time(cx, &s)
                } else {
                    // c.iii. Let tv be ? ToNumber(v).
                    cx.to_number(v)?
                }
            };
            // d. Let dv be TimeClip(tv).
            time_clip(tv)
        }
        // 5. numberOfArgs ≥ 2: the local time of the fields.
        _ => {
            let f = to_fields(cx, args)?;
            let final_date = make_date(
                make_day(f[0], f[1], f[2]),
                make_time(f[3], f[4], f[5], f[6]),
            );
            // k. Let dv be TimeClip(UTC(finalDate)).
            time_clip(utc(cx, final_date))
        }
    };
    // 6. Let O be ? OrdinaryCreateFromConstructor(NewTarget, "%Date.prototype%",
    //    « [[DateValue]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::DatePrototype)?;
    let o = cx.new_object_with_slots(proto, ObjectClass::Date, DATE_SLOTS);
    // 7. Set O.[[DateValue]] to dv.
    cx.set_slot(o, ObjectClass::Date, DATE_VALUE, Local::number(dv))?;
    // 8. Return O.
    Ok(o.value())
}

/// The fields of Date(y, m, ...) and Date.UTC (§21.4.2.1 step 5, §21.4.3.4 steps 1-8):
/// ToNumber of each present argument in order; defaults date 1 and +0 for the time; the
/// year through MakeFullYear.
fn to_fields<'s>(cx: &mut Cx<'s>, args: Args<'s>) -> JsResult<[f64; 7]> {
    let mut f = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    // The year is always converted (undefined gives NaN); the rest only when present.
    let n = args.len().clamp(1, 7);
    for (i, slot) in f.iter_mut().enumerate().take(n) {
        *slot = cx.to_number(args.get(i))?;
    }
    f[0] = make_full_year(f[0]);
    Ok(f)
}

/// Date.now ( ) (§21.4.3.1).
fn now<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::number(now_ms(cx)))
}

/// Date.parse ( string ) (§21.4.3.2).
fn parse_fn<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let string be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    let s = cx.to_rust_string(s)?;
    Ok(Local::number(parse_time(cx, &s)))
}

/// Date.UTC ( year [ , month [ , date [ , hours [ , minutes [ , seconds [ , ms ] ] ] ] ] ] )
/// (§21.4.3.4).
fn utc_fn<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-8. The fields; yr = MakeFullYear(y).
    let f = to_fields(cx, args)?;
    // 9. Return TimeClip(MakeDate(MakeDay(yr, m, dt), MakeTime(h, min, s, milli))).
    let tv = make_date(
        make_day(f[0], f[1], f[2]),
        make_time(f[3], f[4], f[5], f[6]),
    );
    Ok(Local::number(time_clip(tv)))
}

// ---- getters (§21.4.4.2-21, Annex B getYear) --------------------------------------------------

/// Defines a getter: thisTimeValue; NaN gives NaN; else `$f` of the local (`local`) or UTC
/// time.
macro_rules! getter {
    ($(#[$doc:meta])* $name:ident, $local:expr, $f:expr) => {
        $(#[$doc])*
        fn $name<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            _args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            // 1-3. Let t be dateObject.[[DateValue]] (RequireInternalSlot first).
            let t = this_time_value(cx, this)?;
            // 4. If t is NaN, return NaN.
            if t.is_nan() {
                return Ok(Local::number(f64::NAN));
            }
            // 5. Return the field of LocalTime(t) (local getters) or of t (UTC getters).
            let t = if $local { local_time(cx, t) } else { t };
            let f: fn(f64) -> f64 = $f;
            Ok(Local::number(f(t)))
        }
    };
}

getter!(/// Date.prototype.getDate ( ) (§21.4.4.2).
    get_date, true, date_from_time);
getter!(/// Date.prototype.getDay ( ) (§21.4.4.3).
    get_day, true, math::week_day);
getter!(/// Date.prototype.getFullYear ( ) (§21.4.4.4).
    get_full_year, true, year_from_time);
getter!(/// Date.prototype.getHours ( ) (§21.4.4.5).
    get_hours, true, math::hour_from_time);
getter!(/// Date.prototype.getMilliseconds ( ) (§21.4.4.6).
    get_milliseconds, true, math::ms_from_time);
getter!(/// Date.prototype.getMinutes ( ) (§21.4.4.7).
    get_minutes, true, math::min_from_time);
getter!(/// Date.prototype.getMonth ( ) (§21.4.4.8).
    get_month, true, month_from_time);
getter!(/// Date.prototype.getSeconds ( ) (§21.4.4.9).
    get_seconds, true, math::sec_from_time);
getter!(/// Date.prototype.getUTCDate ( ) (§21.4.4.12).
    get_utc_date, false, date_from_time);
getter!(/// Date.prototype.getUTCDay ( ) (§21.4.4.13).
    get_utc_day, false, math::week_day);
getter!(/// Date.prototype.getUTCFullYear ( ) (§21.4.4.14).
    get_utc_full_year, false, year_from_time);
getter!(/// Date.prototype.getUTCHours ( ) (§21.4.4.15).
    get_utc_hours, false, math::hour_from_time);
getter!(/// Date.prototype.getUTCMilliseconds ( ) (§21.4.4.16).
    get_utc_milliseconds, false, math::ms_from_time);
getter!(/// Date.prototype.getUTCMinutes ( ) (§21.4.4.17).
    get_utc_minutes, false, math::min_from_time);
getter!(/// Date.prototype.getUTCMonth ( ) (§21.4.4.18).
    get_utc_month, false, month_from_time);
getter!(/// Date.prototype.getUTCSeconds ( ) (§21.4.4.19).
    get_utc_seconds, false, math::sec_from_time);
getter!(/// Date.prototype.getYear ( ) (§B.2.3.1): YearFromTime(LocalTime(t)) - 1900.
    get_year, true, |t| year_from_time(t) - 1900.0);

/// Date.prototype.getTime ( ) (§21.4.4.10) and Date.prototype.valueOf ( ) (§21.4.4.44).
fn get_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Return dateObject.[[DateValue]].
    Ok(Local::number(this_time_value(cx, this)?))
}

/// Date.prototype.getTimezoneOffset ( ) (§21.4.4.11).
fn get_timezone_offset<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let t be dateObject.[[DateValue]].
    let t = this_time_value(cx, this)?;
    // 4. If t is NaN, return NaN.
    if t.is_nan() {
        return Ok(Local::number(f64::NAN));
    }
    // 5. Return (t - LocalTime(t)) / msPerMinute.
    let local = local_time(cx, t);
    Ok(Local::number((t - local) / MS_PER_MINUTE))
}

// ---- setters (§21.4.4.20-31, Annex B setYear) -------------------------------------------------

/// The field group a setter writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Set {
    /// setMilliseconds ( ms ).
    Ms,
    /// setSeconds ( sec [ , ms ] ).
    Seconds,
    /// setMinutes ( min [ , sec [ , ms ] ] ).
    Minutes,
    /// setHours ( hour [ , min [ , sec [ , ms ] ] ] ).
    Hours,
    /// setDate ( date ).
    Date,
    /// setMonth ( month [ , date ] ).
    Month,
    /// setFullYear ( year [ , month [ , date ] ] ).
    FullYear,
}

impl Set {
    /// The number of parameters.
    fn arity(self) -> usize {
        match self {
            Set::Ms | Set::Date => 1,
            Set::Seconds | Set::Month => 2,
            Set::Minutes | Set::FullYear => 3,
            Set::Hours => 4,
        }
    }
}

/// The set*/setUTC* steps (§21.4.4.20-31): RequireInternalSlot; t = \[\[DateValue\]\]; ToNumber
/// of the first argument and of each other present one, in order; NaN t gives NaN (setFullYear:
/// t = +0); then the new value from the given fields and the fields of t (LocalTime(t) for
/// the local setters), TimeClip(UTC(...)) (local) or TimeClip(...) (UTC), stored and returned.
fn set_fields<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    which: Set,
    local: bool,
) -> JsResult<Local<'s>> {
    // 1-3. Let t be dateObject.[[DateValue]] (after RequireInternalSlot).
    let mut t = this_time_value(cx, this)?;
    // ToNumber of the first argument (always) and of the present others, in order.
    let n = args.len().clamp(1, which.arity());
    let mut v = [f64::NAN; 4];
    for (i, slot) in v.iter_mut().enumerate().take(n) {
        *slot = cx.to_number(args.get(i))?;
    }
    let arg = |i: usize, default: f64| if i < n { v[i] } else { default };
    if t.is_nan() {
        // setFullYear: "If t is NaN, set t to +0𝔽"; the others return NaN.
        if which != Set::FullYear {
            return Ok(Local::number(f64::NAN));
        }
        t = 0.0;
    } else if local {
        t = local_time(cx, t);
    }
    let f = Fields::of(t);
    let new_date = match which {
        Set::Ms => make_date(day(t), make_time(f.hour, f.min, f.sec, v[0])),
        Set::Seconds => make_date(day(t), make_time(f.hour, f.min, v[0], arg(1, f.ms))),
        Set::Minutes => make_date(day(t), make_time(f.hour, v[0], arg(1, f.sec), arg(2, f.ms))),
        Set::Hours => make_date(
            day(t),
            make_time(v[0], arg(1, f.min), arg(2, f.sec), arg(3, f.ms)),
        ),
        Set::Date => make_date(make_day(f.year, f.month, v[0]), time_within_day(t)),
        Set::Month => make_date(make_day(f.year, v[0], arg(1, f.date)), time_within_day(t)),
        Set::FullYear => make_date(
            make_day(v[0], arg(1, f.month), arg(2, f.date)),
            time_within_day(t),
        ),
    };
    let u = time_clip(if local { utc(cx, new_date) } else { new_date });
    cx.set_slot(this, ObjectClass::Date, DATE_VALUE, Local::number(u))?;
    Ok(Local::number(u))
}

/// Defines a setter over [`set_fields`].
macro_rules! setter {
    ($(#[$doc:meta])* $name:ident, $which:expr, $local:expr) => {
        $(#[$doc])*
        fn $name<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            set_fields(cx, this, args, $which, $local)
        }
    };
}

setter!(/// Date.prototype.setDate ( date ) (§21.4.4.20).
    set_date, Set::Date, true);
setter!(/// Date.prototype.setFullYear ( year [ , month [ , date ] ] ) (§21.4.4.21).
    set_full_year, Set::FullYear, true);
setter!(/// Date.prototype.setHours ( hour [ , min [ , sec [ , ms ] ] ] ) (§21.4.4.22).
    set_hours, Set::Hours, true);
setter!(/// Date.prototype.setMilliseconds ( ms ) (§21.4.4.23).
    set_milliseconds, Set::Ms, true);
setter!(/// Date.prototype.setMinutes ( min [ , sec [ , ms ] ] ) (§21.4.4.24).
    set_minutes, Set::Minutes, true);
setter!(/// Date.prototype.setMonth ( month [ , date ] ) (§21.4.4.25).
    set_month, Set::Month, true);
setter!(/// Date.prototype.setSeconds ( sec [ , ms ] ) (§21.4.4.26).
    set_seconds, Set::Seconds, true);
setter!(/// Date.prototype.setUTCDate ( date ) (§21.4.4.28).
    set_utc_date, Set::Date, false);
setter!(/// Date.prototype.setUTCFullYear ( year [ , month [ , date ] ] ) (§21.4.4.29).
    set_utc_full_year, Set::FullYear, false);
setter!(/// Date.prototype.setUTCHours ( hour [ , min [ , sec [ , ms ] ] ] ) (§21.4.4.30).
    set_utc_hours, Set::Hours, false);
setter!(/// Date.prototype.setUTCMilliseconds ( ms ) (§21.4.4.31).
    set_utc_milliseconds, Set::Ms, false);
setter!(/// Date.prototype.setUTCMinutes ( min [ , sec [ , ms ] ] ) (§21.4.4.32).
    set_utc_minutes, Set::Minutes, false);
setter!(/// Date.prototype.setUTCMonth ( month [ , date ] ) (§21.4.4.33).
    set_utc_month, Set::Month, false);
setter!(/// Date.prototype.setUTCSeconds ( sec [ , ms ] ) (§21.4.4.34).
    set_utc_seconds, Set::Seconds, false);

/// Date.prototype.setTime ( time ) (§21.4.4.27).
fn set_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Perform ? RequireInternalSlot(dateObject, [[DateValue]]) (before step 3, which can
    //      run JS).
    this_time_value(cx, this)?;
    // 3. Let t be ? ToNumber(time).
    let t = cx.to_number(args.get(0))?;
    // 4. Let v be TimeClip(t).
    let v = time_clip(t);
    // 5. Set dateObject.[[DateValue]] to v.
    cx.set_slot(this, ObjectClass::Date, DATE_VALUE, Local::number(v))?;
    // 6. Return v.
    Ok(Local::number(v))
}

/// Date.prototype.setYear ( year ) (§B.2.3.2).
fn set_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let t be dateObject.[[DateValue]].
    let t = this_time_value(cx, this)?;
    // 4. Let y be ? ToNumber(year).
    let y = cx.to_number(args.get(0))?;
    // 5. If t is NaN, set t to +0𝔽; otherwise, set t to LocalTime(t).
    let t = if t.is_nan() { 0.0 } else { local_time(cx, t) };
    // 6. Let yyyy be MakeFullYear(y).
    let yyyy = make_full_year(y);
    // 7. Let d be MakeDay(yyyy, MonthFromTime(t), DateFromTime(t)).
    let d = make_day(yyyy, month_from_time(t), date_from_time(t));
    // 8. Let date be MakeDate(d, TimeWithinDay(t)).
    let date = make_date(d, time_within_day(t));
    // 9-11. Set dateObject.[[DateValue]] to TimeClip(UTC(date)) and return it.
    let u = time_clip(utc(cx, date));
    cx.set_slot(this, ObjectClass::Date, DATE_VALUE, Local::number(u))?;
    Ok(Local::number(u))
}

// ---- strings (§21.4.4.35-43) ----------------------------------------------------------------

/// Date.prototype.toString ( ) (§21.4.4.41); toLocaleString in v0.
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let tv be dateObject.[[DateValue]].
    let tv = this_time_value(cx, this)?;
    // 4. Return ToDateString(tv).
    let s = to_date_string_text(cx, tv);
    string(cx, &s)
}

/// Date.prototype.toDateString ( ) (§21.4.4.35); toLocaleDateString in v0.
fn to_date_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let tv be dateObject.[[DateValue]].
    let tv = this_time_value(cx, this)?;
    // 4. If tv is NaN, return "Invalid Date".
    if tv.is_nan() {
        return string(cx, "Invalid Date");
    }
    // 5. Let t be LocalTime(tv).
    let t = local_time(cx, tv);
    // 6. Return DateString(t).
    string(cx, &format::date_string(t))
}

/// Date.prototype.toTimeString ( ) (§21.4.4.42); toLocaleTimeString in v0.
fn to_time_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let tv be dateObject.[[DateValue]].
    let tv = this_time_value(cx, this)?;
    // 4. If tv is NaN, return "Invalid Date".
    if tv.is_nan() {
        return string(cx, "Invalid Date");
    }
    // 5. Let t be LocalTime(tv).
    let offset = cx.local_tz_offset_ms(tv, true);
    let t = tv + offset;
    // 6. Return the string-concatenation of TimeString(t) and TimeZoneString(tv).
    let s = format!(
        "{}{}",
        format::time_string(t),
        format::time_zone_string(offset)
    );
    string(cx, &s)
}

/// Date.prototype.toISOString ( ) (§21.4.4.36).
fn to_iso_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let tv be dateObject.[[DateValue]].
    let tv = this_time_value(cx, this)?;
    // 4. If tv is not finite, throw a RangeError exception.
    if !tv.is_finite() {
        return Err(cx.throw_range_error("Invalid time value"));
    }
    // 5-6. The Date Time String Format, in UTC.
    string(cx, &format::iso_string(tv))
}

/// Date.prototype.toUTCString ( ) (§21.4.4.43); also toGMTString (§B.2.3.3).
fn to_utc_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let tv be dateObject.[[DateValue]].
    let tv = this_time_value(cx, this)?;
    // 4. If tv is NaN, return "Invalid Date".
    if tv.is_nan() {
        return string(cx, "Invalid Date");
    }
    // 5-10. weekday, day, month, year and TimeString(tv).
    string(cx, &format::utc_string(tv))
}

/// Date.prototype.toJSON ( key ) (§21.4.4.37).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let tv be ? ToPrimitive(O, number).
    let tv = cx.to_primitive(o, ToPrimitiveHint::Number)?;
    // 3. If tv is a Number and tv is not finite, return null.
    if tv.as_number().is_some_and(|n| !n.is_finite()) {
        return Ok(Local::null());
    }
    // 4. Return ? Invoke(O, "toISOString").
    let f = cx.get(o, "toISOString")?;
    cx.call(f, o, &[])
}

/// Date.prototype [ %Symbol.toPrimitive% ] ( hint ) (§21.4.4.45).
fn to_primitive<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let O be the this value; if it is not an Object, throw a TypeError.
    let Some(o) = this.as_object() else {
        return Err(
            cx.throw_type_error("Date.prototype[Symbol.toPrimitive] called on a non-object")
        );
    };
    // 3-5. tryFirst: string for "string" and "default", number for "number"; else a
    //      TypeError.
    let hint = args.get(0);
    let hint = if hint.is_string() {
        cx.to_rust_string(hint)?
    } else {
        String::new()
    };
    let names = match hint.as_str() {
        "string" | "default" => ["toString", "valueOf"],
        "number" => ["valueOf", "toString"],
        _ => return Err(cx.throw_type_error("Invalid hint")),
    };
    // 6. Return ? OrdinaryToPrimitive(O, tryFirst) (§7.1.1.1).
    for name in names {
        let method = cx.get(o, name)?;
        if cx.is_callable(method) {
            let result = cx.call(method, o, &[])?;
            if !result.is_object() {
                return Ok(result);
            }
        }
    }
    Err(cx.throw_type_error("Cannot convert object to primitive value"))
}
