//! Intl.DateTimeFormat (ECMA-402 §11; block 45): `%Intl.DateTimeFormat%` and
//! `%Intl.DateTimeFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.DateTimeFormat {w, c}     %Intl.DateTimeFormat%  prototype {}, supportedLocalesOf
//! Intl.DateTimeFormat.prototype  %Intl.DateTimeFormat.prototype%  constructor {w, c},
//!                    resolvedOptions, formatToParts {w, c}, get format {c},
//!                    [@@toStringTag] "Intl.DateTimeFormat" {c}
//! slots (ObjectClass::IntlDateTimeFormat, SLOTS = 7):
//!   0 [[Locale]]           a String ("ca", "hc", "nu" are its Unicode extension keys)
//!   1 [[Calendar]]         a String
//!   2 [[NumberingSystem]]  a String
//!   3 [[TimeZone]]         a String (an offset "+HH:MM" or an IANA name, as the user wrote
//!                          it, case-normalized)
//!   4 styles               an int32: bits 0-2 [[HourCycle]] (0 none, 1 + index in
//!                          HOUR_CYCLES), bits 3-5 [[DateStyle]], bits 6-8 [[TimeStyle]]
//!                          (0 none, 1 + index in STYLES)
//!   5 components           an int32: the Table 16 values (Components)
//!   6 [[BoundFormat]]      undefined or the bound format function
//! new Intl.DateTimeFormat(locales, options) ─ initialize (CreateDateTimeFormat, 11.1.2)
//!   ─> Resolved in the slots (no ICU4X object)
//! format(date) ─ TimeClip ─ cache::get_or_make(Service::DateTimeFormat, prefs tag, field
//!   set bits) ─> Rc<DateTimeFormatter<CompositeFieldSet>> ─ ICU4X format of the local time
//!   (tzdb::offset_at for a named zone) ─> the string and its UTS 35 pattern ─ pieces by the
//!   pattern ─ `pattern::adjust` (the ICU4C form) ─> the parts; format: their string
//! formatToParts(date) ─ the same parts
//! Date.prototype.toLocale{,Date,Time}String ─ to_locale_string (no globalThis.Intl read)
//! ```
//!
//! The legacy constructor path (ChainDateTimeFormat and UnwrapDateTimeFormat, normative
//! optional) uses `%Intl%.[[FallbackSymbol]]` (`intl::fallback_symbol`). The field set comes
//! from the ICU4X field bag (`fieldbag::DateTimeFieldBag`) or the styles; the per-field
//! width, the flexible day periods and the other pattern work are in the `pattern` module
//! (W4 DTF-PATTERN). No ICU4X object is in a slot (INTL-I1-D7, plan rule 8).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-date_time_format.txt`.

use std::rc::Rc;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use icu_calendar::Iso;
use icu_datetime::fieldbag::DateTimeFieldBag;
use icu_datetime::fieldbag::field;
use icu_datetime::fieldsets::YMD;
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
use icu_datetime::fieldsets::enums::{CompositeFieldSet, DateFieldSet};
use icu_datetime::options::{Length, TimePrecision};
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
use icu_locale_core::Locale;
use icu_time::zone::models::AtTime;
use icu_time::zone::{IanaParser, TimeZone, UtcOffset};
use icu_time::{DateTime, TimeZoneInfo, ZonedDateTime};

mod pattern;

use crate::available;
use crate::cache::{self, Service};
use crate::intl;
use crate::locale::is_unicode_type;
use crate::locale_info;
use crate::number_format;
use crate::options::{
    coerce_options_to_object, get_boolean_option, get_locale_matcher, get_number_option,
    get_string_option, index_of,
};
use crate::resolve;
use crate::tag;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 45;

/// The class of an Intl.DateTimeFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlDateTimeFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: `[[Calendar]]` (a String).
pub const SLOT_CALENDAR: usize = 1;
/// Slot 2: `[[NumberingSystem]]` (a String).
pub const SLOT_NUMBERING_SYSTEM: usize = 2;
/// Slot 3: `[[TimeZone]]` (a String).
pub const SLOT_TIME_ZONE: usize = 3;
/// Slot 4: the [`Styles`] (`[[HourCycle]]`, `[[DateStyle]]`, `[[TimeStyle]]`; an int32).
pub const SLOT_STYLES: usize = 4;
/// Slot 5: the [`Components`] (an int32).
pub const SLOT_COMPONENTS: usize = 5;
/// Slot 6: `[[BoundFormat]]` (undefined or a function).
pub const SLOT_BOUND_FORMAT: usize = 6;
/// The slot count of an Intl.DateTimeFormat object.
pub const SLOTS: usize = 7;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const GET_FORMAT: u16 = 2;
    pub const FORMAT_TO_PARTS: u16 = 3;
    pub const RESOLVED_OPTIONS: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DateTimeFormat", 0, constructor), // 0 (11.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (11.2.2)
    BuiltinDef::getter("format", get_format),                  // 2 (11.3.3)
    BuiltinDef::method("formatToParts", 1, format_to_parts),   // 3 (11.3.4)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 4 (11.3.7)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlDateTimeFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlDateTimeFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlDateTimeFormat,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlDateTimeFormatPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlDateTimeFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlDateTimeFormat,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, formatToParts, format.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("formatToParts"), def::FORMAT_TO_PARTS),
            PropDef::accessor(PropKey::Str("format"), Some(def::GET_FORMAT), None),
            // Intl.DateTimeFormat.prototype [ @@toStringTag ] ({c}, 11.3.8).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.DateTimeFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("DateTimeFormat"),
            IntrinsicId::IntlDateTimeFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "date_time_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[HourCycle]]` values, in their packed order.
pub const HOUR_CYCLES: [&str; 4] = ["h11", "h12", "h23", "h24"];
/// `[[DateStyle]]` and `[[TimeStyle]]` values, in their packed order.
pub const STYLES: [&str; 4] = ["full", "long", "medium", "short"];

/// The text values of weekday, era and dayPeriod.
const TEXT: &[&str] = &["narrow", "short", "long"];
/// The values of year, day, hour, minute and second.
const NUMERIC: &[&str] = &["2-digit", "numeric"];
/// The values of month.
const MONTH: &[&str] = &["2-digit", "numeric", "narrow", "short", "long"];
/// The values of timeZoneName.
const TIME_ZONE_NAME: &[&str] = &[
    "short",
    "long",
    "shortOffset",
    "longOffset",
    "shortGeneric",
    "longGeneric",
];
/// The fractionalSecondDigits "values" (1-3, read by GetNumberOption; packed as the count).
const DIGITS: &[&str] = &["1", "2", "3"];

/// Table 16 (the components of date and time formats), in table order: the property, its
/// values, and the bit width of its packed form (0 none, 1 + the index in the values).
pub const COMPONENTS: [(&str, &[&str], u32); 11] = [
    ("weekday", TEXT, 2),
    ("era", TEXT, 2),
    ("year", NUMERIC, 2),
    ("month", MONTH, 3),
    ("day", NUMERIC, 2),
    ("dayPeriod", TEXT, 2),
    ("hour", NUMERIC, 2),
    ("minute", NUMERIC, 2),
    ("second", NUMERIC, 2),
    ("fractionalSecondDigits", DIGITS, 2),
    ("timeZoneName", TIME_ZONE_NAME, 3),
];

/// The index in [`COMPONENTS`] of each component.
mod c {
    pub const WEEKDAY: usize = 0;
    pub const ERA: usize = 1;
    pub const YEAR: usize = 2;
    pub const MONTH: usize = 3;
    pub const DAY: usize = 4;
    pub const DAY_PERIOD: usize = 5;
    pub const HOUR: usize = 6;
    pub const MINUTE: usize = 7;
    pub const SECOND: usize = 8;
    pub const FRACTIONAL_SECOND_DIGITS: usize = 9;
    pub const TIME_ZONE_NAME: usize = 10;
}

/// The components of a date-time format (`formatOptions` of CreateDateTimeFormat after the
/// defaults): per row of [`COMPONENTS`], 0 for none, else 1 + the index of the value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Components(pub [u8; 11]);

impl Components {
    /// The packed form (24 bits; slot 5).
    pub fn pack(self) -> u32 {
        let mut bits = 0;
        let mut shift = 0;
        for (i, (_, _, width)) in COMPONENTS.iter().enumerate() {
            bits |= (u32::from(self.0[i]) & ((1 << width) - 1)) << shift;
            shift += width;
        }
        bits
    }

    /// The components of the packed form `bits` (a value out of range is none).
    pub fn unpack(bits: u32) -> Components {
        let mut out = [0u8; 11];
        let mut shift = 0;
        for (i, (_, values, width)) in COMPONENTS.iter().enumerate() {
            let v = (bits >> shift) & ((1 << width) - 1);
            out[i] = if v as usize <= values.len() {
                v as u8
            } else {
                0
            };
            shift += width;
        }
        Components(out)
    }

    /// The value of the component `i` (an index in [`COMPONENTS`]), if any.
    pub fn get(self, i: usize) -> Option<&'static str> {
        let v = self.0[i] as usize;
        (v > 0).then(|| COMPONENTS[i].1[v - 1])
    }

    /// Sets the component `i` to `value` (one of its values).
    fn set(&mut self, i: usize, value: &str) {
        self.0[i] = index_of(COMPONENTS[i].1, &value) as u8 + 1;
    }

    /// Whether any component is set.
    pub fn any(self) -> bool {
        self.0.iter().any(|v| *v != 0)
    }
}

/// `[[HourCycle]]`, `[[DateStyle]]` and `[[TimeStyle]]`, packed in one int32 (slot 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Styles {
    /// The index in [`HOUR_CYCLES`] (`None`: the format has no hour).
    pub hour_cycle: Option<usize>,
    /// The index in [`STYLES`] of `[[DateStyle]]`.
    pub date_style: Option<usize>,
    /// The index in [`STYLES`] of `[[TimeStyle]]`.
    pub time_style: Option<usize>,
}

impl Styles {
    /// The packed form (9 bits; slot 4).
    pub fn pack(self) -> u32 {
        let p = |v: Option<usize>| v.map_or(0, |v| (v as u32 & 3) + 1);
        p(self.hour_cycle) | p(self.date_style) << 3 | p(self.time_style) << 6
    }

    /// The styles of the packed form `bits`.
    pub fn unpack(bits: u32) -> Styles {
        let u = |v: u32| match v & 7 {
            0 => None,
            v => Some(((v - 1) as usize).min(3)),
        };
        Styles {
            hour_cycle: u(bits),
            date_style: u(bits >> 3),
            time_style: u(bits >> 6),
        }
    }
}

/// The internal slots of an Intl.DateTimeFormat (§11.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// `[[Calendar]]`.
    pub calendar: String,
    /// `[[NumberingSystem]]`.
    pub numbering_system: String,
    /// `[[TimeZone]]`.
    pub time_zone: String,
    /// `[[HourCycle]]`, `[[DateStyle]]`, `[[TimeStyle]]`.
    pub styles: Styles,
    /// The components of `[[DateTimeFormat]]` (none if a style is set).
    pub components: Components,
}

/// The `required` argument of CreateDateTimeFormat (11.1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Required {
    Date,
    Time,
    Any,
}

/// The `defaults` argument of CreateDateTimeFormat (11.1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Defaults {
    Date,
    Time,
    All,
}

// ---- the locale data --------------------------------------------------------------------------

/// The "ca" data of `locale` (default first): the first calendar of the locale preference
/// that ICU4X and Temporal have (`available::CALENDARS`), else "gregory"; then all of them.
fn ca_data(locale: &str) -> resolve::KeyData {
    let first = locale_info::calendars_of(locale)
        .iter()
        .copied()
        .find(|c| available::CALENDARS.contains(c))
        .unwrap_or("gregory");
    let mut v = vec![first.to_string()];
    v.extend(
        available::CALENDARS
            .iter()
            .filter(|c| **c != first)
            .map(|c| (*c).to_string()),
    );
    v
}

/// `[[hourCycle]]` of the locale data of `locale`: the preferred hour cycle of the locale.
pub fn default_hour_cycle(locale: &str) -> &'static str {
    match locale_info::hour_cycles_of(locale) {
        "h11" => "h11",
        "h12" => "h12",
        "h24" => "h24",
        _ => "h23",
    }
}

/// `[[hourCycle12]]` of the locale data of `locale` ("h11" for Japanese, as V8 has it; else
/// "h12").
pub fn hour_cycle_12(locale: &str) -> &'static str {
    if locale.split('-').next() == Some("ja") {
        "h11"
    } else {
        "h12"
    }
}

/// `[[hourCycle24]]` of the locale data: "h23" for every locale.
pub const HOUR_CYCLE_24: &str = "h23";

/// The "hc" data of `locale` (default first).
fn hc_data(locale: &str) -> resolve::KeyData {
    let mut v = vec![default_hour_cycle(locale).to_string()];
    v.extend(HOUR_CYCLES.iter().map(|h| (*h).to_string()));
    v
}

// ---- time zones -------------------------------------------------------------------------------

/// The offset in minutes of the UTC offset string `s` of the UTCOffset grammar without
/// seconds (ECMA-262 §21.4.1.33): an ASCII sign, then `HH`, `HHMM` or `HH:MM` (hours 00-23,
/// minutes 00-59). `None` for any other string.
pub fn parse_offset_minutes(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    let sign = match b.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = |x: &[u8]| -> Option<i32> {
        if x.len() == 2 && x.iter().all(u8::is_ascii_digit) {
            Some(i32::from(x[0] - b'0') * 10 + i32::from(x[1] - b'0'))
        } else {
            None
        }
    };
    let rest = &b[1..];
    let (h, m) = match rest.len() {
        2 => (digits(rest)?, 0),
        4 => (digits(&rest[..2])?, digits(&rest[2..])?),
        5 if rest[2] == b':' => (digits(&rest[..2])?, digits(&rest[3..])?),
        _ => return None,
    };
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h * 60 + m))
}

/// FormatOffsetTimeZoneIdentifier ( offsetMinutes ) (ECMA-262 §21.4.1.32 with the colon):
/// "+HH:MM" (the sign "+" for 0).
pub fn format_offset_minutes(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let a = minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", a / 60, a % 60)
}

/// The `[[TimeZone]]` of the timeZone option `tz` (11.1.2 steps 29-30): the offset form
/// "+HH:MM", or the available named zone as the user wrote it, case-normalized
/// (`available::named_time_zone`). `None`: a RangeError.
pub fn time_zone_identifier(tz: &str) -> Option<String> {
    if tz.starts_with(['+', '-']) {
        return parse_offset_minutes(tz).map(format_offset_minutes);
    }
    available::named_time_zone(tz).map(|(id, _)| id)
}

/// SystemTimeZoneIdentifier ( ) (ECMA-262 §21.4.1.24): the primary id of the host zone,
/// "UTC" if the host gives none or one the tz data does not have.
fn system_time_zone(cx: &mut Cx<'_>) -> String {
    cx.system_time_zone_id()
        .and_then(|id| available::named_time_zone(&id))
        .map_or_else(|| "UTC".to_string(), |(_, primary)| primary.to_string())
}

/// The ICU4X zone and the offset in seconds at the epoch milliseconds `ms` of the
/// `[[TimeZone]]` `tz` (an unknown zone: UTC).
fn zone_at(tz: &str, ms: i64) -> (TimeZone, i32) {
    if let Some(m) = parse_offset_minutes(tz) {
        return (TimeZone::UNKNOWN, m * 60);
    }
    let Some((_, primary)) = available::named_time_zone(tz) else {
        return (TimeZone::UNKNOWN, 0);
    };
    let offset = ep9_temporal::tzdb::offset_at(primary, i128::from(ms) * 1_000_000)
        .map_or(0, |(secs, _)| secs);
    (IanaParser::new().parse(primary), offset)
}

/// The ICU4X input of the time value `ms` (an integer in the TimeClip range) in the zone `tz`.
fn zoned(tz: &str, ms: i64) -> ZonedDateTime<Iso, TimeZoneInfo<AtTime>> {
    let (zone, secs) = zone_at(tz, ms);
    let offset = UtcOffset::try_from_seconds(secs).unwrap_or(UtcOffset::zero());
    let local = ZonedDateTime::from_epoch_milliseconds_and_utc_offset(ms, offset);
    let info = zone.with_offset(Some(offset)).at_date_time(DateTime {
        date: local.date,
        time: local.time,
    });
    ZonedDateTime {
        date: local.date,
        time: local.time,
        zone: info,
    }
}

// ---- CreateDateTimeFormat (11.1.2) ------------------------------------------------------------

/// CreateDateTimeFormat ( newTarget, locales, options, required, defaults ) steps 2-44
/// (11.1.2; the object creation of step 1 is the caller's): the internal slots for
/// `locales` and `options`, in the spec order of the observable steps. No ICU4X object is
/// made.
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
    required: Required,
    defaults: Defaults,
) -> JsResult<Resolved> {
    // 2. Let hour12 be undefined (it is read below).
    // 3. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    // 4. Set options to ? CoerceOptionsToObject(options).
    let options = coerce_options_to_object(cx, options)?;
    // 5-6. Let matcher be ? GetOption(options, "localeMatcher", ...).
    get_locale_matcher(cx, options)?;
    // 7-9. Let calendar be ? GetOption(options, "calendar", string, empty, undefined); a
    //      RangeError if it does not match the type nonterminal.
    let ca = get_string_option(cx, options, "calendar", &[], None)?;
    if let Some(v) = &ca
        && !is_unicode_type(v)
    {
        return Err(cx.throw_range_error(&format!("Invalid calendar : {v}")));
    }
    // 10-12. numberingSystem, the same way.
    let nu = get_string_option(cx, options, "numberingSystem", &[], None)?;
    if let Some(v) = &nu
        && !is_unicode_type(v)
    {
        return Err(cx.throw_range_error(&format!("Invalid numberingSystem : {v}")));
    }
    // 13. Let hour12 be ? GetOption(options, "hour12", boolean, empty, undefined).
    let hour12 = get_boolean_option(cx, options, "hour12", None)?;
    // 14. Let hourCycle be ? GetOption(options, "hourCycle", string, « "h11", "h12", "h23",
    //     "h24" », undefined).
    let hc = get_string_option(cx, options, "hourCycle", &HOUR_CYCLES, None)?;
    // 15-16. If hour12 is not undefined, set hourCycle to null.
    let hc = if hour12.is_some() { None } else { hc };
    // 17. Let r be ResolveLocale(%DateTimeFormat%.[[AvailableLocales]], requestedLocales, opt,
    //     « "ca", "hc", "nu" », %DateTimeFormat%.[[LocaleData]]).
    let r = resolve::resolve_locale(
        cx,
        &requested,
        &[("ca", ca), ("hc", hc), ("nu", nu)],
        &["ca", "hc", "nu"],
        &|l, key| match key {
            "ca" => ca_data(l),
            "hc" => hc_data(l),
            _ => number_format::nu_data(l),
        },
    )?;
    let keyword = |key: &str| {
        r.keywords
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    // 18. [[Locale]]: with hour12 set, opt.[[hc]] is null, so the "hc" keyword goes.
    let mut locale = r.locale.clone();
    if hour12.is_some() {
        let kept: Vec<(String, String)> = tag::unicode_keywords(&r.locale)
            .into_iter()
            .filter(|(k, _)| k != "hc")
            .collect();
        locale = if kept.is_empty() {
            r.data_locale.clone()
        } else {
            match resolve::insert_unicode_extension(&r.data_locale, &kept) {
                Ok(l) => l,
                Err(e) => return Err(cx.throw_range_error(&e.message(&r.data_locale))),
            }
        };
    }
    // 19-21. [[Calendar]] and [[NumberingSystem]].
    let calendar = keyword("ca");
    let numbering_system = keyword("nu");
    // 23-25. hc: hourCycle12 / hourCycle24 of the locale data for hour12, else r.[[hc]].
    let hc = match hour12 {
        Some(true) => hour_cycle_12(&r.data_locale),
        Some(false) => HOUR_CYCLE_24,
        None => {
            let v = keyword("hc");
            HOUR_CYCLES
                .iter()
                .copied()
                .find(|h| *h == v)
                .unwrap_or_else(|| default_hour_cycle(&r.data_locale))
        }
    };
    // 26-28. Let timeZone be ? Get(options, "timeZone"); undefined: SystemTimeZoneIdentifier(),
    //        else ? ToString(timeZone).
    let tz = cx.get(options, "timeZone")?;
    let time_zone = if tz.is_undefined() {
        system_time_zone(cx)
    } else {
        let tz = cx.to_rust_string(tz)?;
        // 29-30. The offset form, or GetAvailableNamedTimeZoneIdentifier (a RangeError if
        //        empty).
        match time_zone_identifier(&tz) {
            Some(id) => id,
            None => return Err(cx.throw_range_error(&format!("Invalid time zone specified: {tz}"))),
        }
    };
    // 32-35. For each row of Table 16, in table order: GetNumberOption for
    //        fractionalSecondDigits (1-3), else GetOption(options, prop, string, values).
    let mut components = Components::default();
    for (i, (prop, values, _)) in COMPONENTS.iter().enumerate() {
        if i == c::FRACTIONAL_SECOND_DIGITS {
            if let Some(n) = get_number_option(cx, options, prop, 1, 3, None)? {
                components.0[i] = n.clamp(1, 3) as u8;
            }
        } else if let Some(v) = get_string_option(cx, options, prop, values, None)? {
            components.set(i, &v);
        }
    }
    let has_explicit_components = components.any();
    // 36. Let formatMatcher be ? GetOption(options, "formatMatcher", string, « "basic",
    //     "best fit" », "best fit").
    get_string_option(
        cx,
        options,
        "formatMatcher",
        &["basic", "best fit"],
        Some("best fit"),
    )?;
    // 37-40. dateStyle and timeStyle.
    let date_style = get_string_option(cx, options, "dateStyle", &STYLES, None)?;
    let time_style = get_string_option(cx, options, "timeStyle", &STYLES, None)?;
    let date_style = date_style.map(|s| index_of(&STYLES, &s.as_str()) as usize);
    let time_style = time_style.map(|s| index_of(&STYLES, &s.as_str()) as usize);
    let has_hour;
    // 41. If dateStyle is not undefined or timeStyle is not undefined, then
    if date_style.is_some() || time_style.is_some() {
        // a. If hasExplicitFormatComponents is true, throw a TypeError exception.
        if has_explicit_components {
            return Err(cx.throw_type_error(
                "Can't set option dateStyle or timeStyle together with a date-time component",
            ));
        }
        // b. If required is date and timeStyle is not undefined, throw a TypeError exception.
        if required == Required::Date && time_style.is_some() {
            return Err(cx.throw_type_error("Invalid option : timeStyle"));
        }
        // c. If required is time and dateStyle is not undefined, throw a TypeError exception.
        if required == Required::Time && date_style.is_some() {
            return Err(cx.throw_type_error("Invalid option : dateStyle"));
        }
        // d-e. bestFormat is DateTimeStyleFormat(dateStyle, timeStyle, styles): it has an
        //      hour if timeStyle is set.
        has_hour = time_style.is_some();
        components = Components::default();
    } else {
        // 42. a. Let needDefaults be true.
        let mut need_defaults = true;
        // b. If required is date or any: weekday, year, month, day.
        if matches!(required, Required::Date | Required::Any)
            && [c::WEEKDAY, c::YEAR, c::MONTH, c::DAY]
                .iter()
                .any(|i| components.0[*i] != 0)
        {
            need_defaults = false;
        }
        // c. If required is time or any: dayPeriod, hour, minute, second,
        //    fractionalSecondDigits.
        if matches!(required, Required::Time | Required::Any)
            && [
                c::DAY_PERIOD,
                c::HOUR,
                c::MINUTE,
                c::SECOND,
                c::FRACTIONAL_SECOND_DIGITS,
            ]
            .iter()
            .any(|i| components.0[*i] != 0)
        {
            need_defaults = false;
        }
        // d. needDefaults and defaults date or all: year, month, day "numeric".
        if need_defaults && matches!(defaults, Defaults::Date | Defaults::All) {
            for i in [c::YEAR, c::MONTH, c::DAY] {
                components.set(i, "numeric");
            }
        }
        // e. needDefaults and defaults time or all: hour, minute, second "numeric".
        if need_defaults && matches!(defaults, Defaults::Time | Defaults::All) {
            for i in [c::HOUR, c::MINUTE, c::SECOND] {
                components.set(i, "numeric");
            }
        }
        // f. bestFormat: the ICU4X field set of the components (`field_set`).
        has_hour = components.0[c::HOUR] != 0;
    }
    // 44. If bestFormat has a field [[hour]], set [[HourCycle]] to hc.
    let hour_cycle = has_hour.then(|| index_of(&HOUR_CYCLES, &hc) as usize);
    Ok(Resolved {
        locale,
        calendar,
        numbering_system,
        time_zone,
        styles: Styles {
            hour_cycle,
            date_style,
            time_style,
        },
        components,
    })
}

/// Stores `r` in the slots of the new Intl.DateTimeFormat `dtf`.
fn store<'s>(cx: &mut Cx<'s>, dtf: Local<'s, JsObject>, r: &Resolved) -> JsResult<()> {
    for (slot, text) in [
        (SLOT_LOCALE, &r.locale),
        (SLOT_CALENDAR, &r.calendar),
        (SLOT_NUMBERING_SYSTEM, &r.numbering_system),
        (SLOT_TIME_ZONE, &r.time_zone),
    ] {
        let s = cx.new_string(text)?;
        cx.set_slot(dtf, CLASS, slot, s)?;
    }
    cx.set_slot(
        dtf,
        CLASS,
        SLOT_STYLES,
        Local::int32(r.styles.pack() as i32),
    )?;
    let comps = Local::int32(r.components.pack() as i32);
    cx.set_slot(dtf, CLASS, SLOT_COMPONENTS, comps)?;
    Ok(())
}

/// The internal slots of the date-time format `o` (TypeError if `o` is not an
/// Intl.DateTimeFormat: RequireInternalSlot(o, `[[InitializedDateTimeFormat]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<Resolved> {
    let mut text = [String::new(), String::new(), String::new(), String::new()];
    for (i, slot) in [
        SLOT_LOCALE,
        SLOT_CALENDAR,
        SLOT_NUMBERING_SYSTEM,
        SLOT_TIME_ZONE,
    ]
    .into_iter()
    .enumerate()
    {
        let v = cx.slot(o, CLASS, slot)?;
        text[i] = cx.to_rust_string(v)?;
    }
    let styles = cx.slot(o, CLASS, SLOT_STYLES)?.as_int32().unwrap_or(0) as u32;
    let comps = cx.slot(o, CLASS, SLOT_COMPONENTS)?.as_int32().unwrap_or(0) as u32;
    let [locale, calendar, numbering_system, time_zone] = text;
    Ok(Resolved {
        locale,
        calendar,
        numbering_system,
        time_zone,
        styles: Styles::unpack(styles),
        components: Components::unpack(comps),
    })
}

// ---- the ICU4X formatter (crate::cache) -------------------------------------------------------

/// The ICU4X field bag of the components `cs`.
fn field_bag(cs: Components) -> DateTimeFieldBag {
    let text = |i: usize| cs.0[i];
    let mut bag = DateTimeFieldBag::default();
    bag.weekday = match text(c::WEEKDAY) {
        1 => Some(field::Weekday::Narrow),
        2 => Some(field::Weekday::Short),
        3 => Some(field::Weekday::Long),
        _ => None,
    };
    bag.era = match text(c::ERA) {
        1 => Some(field::Era::Narrow),
        2 => Some(field::Era::Short),
        3 => Some(field::Era::Long),
        _ => None,
    };
    bag.year = match text(c::YEAR) {
        1 => Some(field::Year::TwoDigit),
        2 => Some(field::Year::Numeric),
        _ => None,
    };
    bag.month = match text(c::MONTH) {
        1 => Some(field::Month::TwoDigit),
        2 => Some(field::Month::Numeric),
        3 => Some(field::Month::Narrow),
        4 => Some(field::Month::Short),
        5 => Some(field::Month::Long),
        _ => None,
    };
    bag.day = match text(c::DAY) {
        1 => Some(field::Day::TwoDigit),
        2 => Some(field::Day::Numeric),
        _ => None,
    };
    bag.day_period = match text(c::DAY_PERIOD) {
        1 => Some(field::DayPeriod::FlexibleNarrow),
        2 => Some(field::DayPeriod::FlexibleShort),
        3 => Some(field::DayPeriod::FlexibleLong),
        _ => None,
    };
    bag.hour = match text(c::HOUR) {
        1 => Some(field::Hour::TwoDigit),
        2 => Some(field::Hour::Numeric),
        _ => None,
    };
    bag.minute = match text(c::MINUTE) {
        1 => Some(field::Minute::TwoDigit),
        2 => Some(field::Minute::Numeric),
        _ => None,
    };
    bag.second = match text(c::SECOND) {
        1 => Some(field::Second::TwoDigit),
        2 => Some(field::Second::Numeric),
        _ => None,
    };
    bag.fractional_second_digits = match text(c::FRACTIONAL_SECOND_DIGITS) {
        1 => Some(field::FractionalSecondDigits::F1),
        2 => Some(field::FractionalSecondDigits::F2),
        3 => Some(field::FractionalSecondDigits::F3),
        _ => None,
    };
    bag.time_zone_name = match text(c::TIME_ZONE_NAME) {
        1 => Some(field::TimeZoneName::ShortSpecific),
        2 => Some(field::TimeZoneName::LongSpecific),
        3 => Some(field::TimeZoneName::ShortOffset),
        4 => Some(field::TimeZoneName::LongOffset),
        5 => Some(field::TimeZoneName::ShortGeneric),
        6 => Some(field::TimeZoneName::LongGeneric),
        _ => None,
    };
    bag
}

/// The ICU4X builder of the styles `s` (DateTimeStyleFormat, 11.5.4): the date fields YMDE
/// for "full", else YMD; the time precision minute for "short", else second; the zone name
/// long for "full", short for "long". ICU4X has one length for all fields: the date style's
/// wins.
fn style_builder(s: Styles) -> FieldSetBuilder {
    let length = |i: usize| match STYLES[i] {
        "full" | "long" => Length::Long,
        "medium" => Length::Medium,
        _ => Length::Short,
    };
    let mut b = FieldSetBuilder::default();
    if let Some(d) = s.date_style {
        b.date_fields = Some(if STYLES[d] == "full" {
            DateFields::YMDE
        } else {
            DateFields::YMD
        });
    }
    if let Some(t) = s.time_style {
        b.time_precision = Some(if STYLES[t] == "short" {
            TimePrecision::Minute
        } else {
            TimePrecision::Second
        });
        b.zone_style = match STYLES[t] {
            "full" => Some(ZoneStyle::SpecificLong),
            "long" => Some(ZoneStyle::SpecificShort),
            _ => None,
        };
    }
    b.length = s.date_style.or(s.time_style).map(length);
    b
}

/// The ICU4X composite field set of `r` (bestFormat). An option that ICU4X does not take
/// for the fields is dropped (alignment, year style, then length); YMD medium if none works.
pub fn field_set(r: &Resolved) -> CompositeFieldSet {
    let mut b = if r.styles.date_style.is_some() || r.styles.time_style.is_some() {
        style_builder(r.styles)
    } else {
        let mut bag = field_bag(r.components);
        // With an hour, the day period is the AM/PM field that `pattern::adjust` writes as
        // the flexible day period: it must not set the length of the text fields (ICU4X's
        // field bag takes the longest text width, the day period's too).
        if bag.hour.is_some() {
            bag.day_period = None;
        }
        bag.to_field_set_builder()
    };
    for step in 0..4 {
        if let Ok(fs) = b.clone().build_composite() {
            return fs;
        }
        match step {
            0 => b.alignment = None,
            1 => b.year_style = None,
            _ => b.length = None,
        }
    }
    CompositeFieldSet::Date(DateFieldSet::YMD(YMD::medium()))
}

/// The ICU4X preferences tag of `r`: the data locale of `[[Locale]]` with the resolved "ca",
/// "hc" (h24 as h23: ICU4X has no h24; `pattern::adjust` writes 24 for hour 0) and "nu".
fn prefs_tag(r: &Resolved) -> String {
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let mut t = format!("{data_locale}-u-ca-{}", r.calendar);
    if let Some(h) = r.styles.hour_cycle {
        let h = if HOUR_CYCLES[h] == "h24" {
            "h23"
        } else {
            HOUR_CYCLES[h]
        };
        t.push_str("-hc-");
        t.push_str(h);
    }
    t.push_str("-nu-");
    t.push_str(&r.numbering_system);
    t
}

/// The cached ICU4X formatter of `r` (`None` if ICU4X has no data, not the case with the
/// compiled data).
fn formatter_for(r: &Resolved) -> Option<Rc<DateTimeFormatter<CompositeFieldSet>>> {
    let key = prefs_tag(r);
    let bits = u64::from(r.components.pack()) | u64::from(r.styles.pack()) << 24;
    cache::get_or_make(Service::DateTimeFormat, &key, bits, || {
        let fs = field_set(r);
        let locale = Locale::try_from_str(&key).unwrap_or(Locale::UNKNOWN);
        DateTimeFormatter::try_new(DateTimeFormatterPreferences::from(&locale), fs)
            .or_else(|_| {
                // A numbering system with no ICU4X digits: the locale's own.
                let data_locale = tag::remove_unicode_extension(&r.locale);
                let base = format!("{data_locale}-u-ca-{}", r.calendar);
                let locale = Locale::try_from_str(&base).unwrap_or(Locale::UNKNOWN);
                DateTimeFormatter::try_new(DateTimeFormatterPreferences::from(&locale), fs)
            })
            .ok()
    })
}

/// TimeClip ( time ) (ECMA-262 §21.4.1.31) as an integer: `None` for NaN, an infinity or a
/// value past 8.64 × 10^15 (no float library call, H36: the cast truncates toward zero).
pub fn time_clip(t: f64) -> Option<i64> {
    if !t.is_finite() || t.abs() > 8.64e15 {
        return None;
    }
    Some(t as i64)
}

/// The time value of 2000-01-01T12:00:00Z: the reference date of a style format's year width
/// (`format_ms`).
const REFERENCE_MS: i64 = 946_728_000_000;

/// The parts (Table 18 type, value) of the formatted time value `ms` (a TimeClip result) of
/// `r` (PartitionDateTimePattern, 11.5.5): the ICU4X output cut by its pattern, with the
/// changes of `pattern::adjust` (W4 DTF-PATTERN).
pub fn format_ms(r: &Resolved, ms: i64) -> Option<Vec<(&'static str, String)>> {
    format_pieces(r, ms).map(|pieces| pattern::parts(&pieces))
}

/// The pieces (field letter and count, text) of the formatted time value `ms` of `r`: the
/// ICU4X output cut by its pattern, after `pattern::adjust`.
fn format_pieces(r: &Resolved, ms: i64) -> Option<Vec<pattern::Piece>> {
    let f = formatter_for(r)?;
    let input = zoned(&r.time_zone, ms);
    let formatted = f.format(&input);
    let p = formatted.pattern().to_string();
    let pieces = pattern::pieces(&formatted.to_string(), &p);
    // A date style writes the year with the width of the locale pattern ("M/d/yy" in en
    // short); ICU4X writes the full year for a year far from the present: read the width at
    // the reference date.
    let two_digit_year =
        r.styles.date_style.is_some() && pattern::field_count(&p, 'y').is_some_and(|n| n != 2) && {
            let reference = f.format(&zoned(&r.time_zone, REFERENCE_MS));
            pattern::field_count(&reference.pattern().to_string(), 'y') == Some(2)
        };
    let clock = pattern::Clock {
        hour: input.time.hour.number(),
        minute: input.time.minute.number(),
        second: input.time.second.number(),
    };
    Some(pattern::adjust(r, &input, &clock, two_digit_year, pieces))
}

/// The components that resolvedOptions reports (11.3.7 step 5 reads `[[DateTimeFormat]]`, the
/// format record that CreateDateTimeFormat selected, not the request): the numeric hour,
/// minute, second, month and day widths of the pattern that format uses (count 2: "2-digit",
/// count 1: "numeric"; V8 maps its ICU pattern back the same way). A text field, the year,
/// fractionalSecondDigits, timeZoneName and a field that the request did not have stay as
/// they are; so do the styles (no components are reported then).
fn selected_components(r: &Resolved) -> Components {
    let mut out = r.components;
    if r.styles.date_style.is_some() || r.styles.time_style.is_some() {
        return out;
    }
    let Some(pieces) = format_pieces(r, REFERENCE_MS) else {
        return out;
    };
    for p in pieces {
        let Some((ch, n)) = p.field else {
            continue;
        };
        let i = match ch {
            'M' | 'L' if n <= 2 => c::MONTH,
            'd' => c::DAY,
            'h' | 'H' | 'K' | 'k' => c::HOUR,
            'm' => c::MINUTE,
            's' => c::SECOND,
            _ => continue,
        };
        // 1 "2-digit", 2 "numeric" (NUMERIC and the first two MONTH values).
        if matches!(out.0[i], 1 | 2) {
            out.0[i] = if n == 2 { 1 } else { 2 };
        }
    }
    out
}

// ---- formatToParts: the pattern field types ---------------------------------------------------

/// The part type of the UTS 35 field letter `ch` (Table 18 names; "literal" for a letter
/// with no ECMA-402 type).
fn part_type(ch: char) -> &'static str {
    match ch {
        'G' => "era",
        'y' | 'Y' | 'u' => "year",
        'r' => "relatedYear",
        'U' => "yearName",
        'M' | 'L' => "month",
        'd' => "day",
        'E' | 'c' | 'e' => "weekday",
        'a' | 'b' | 'B' => "dayPeriod",
        'h' | 'H' | 'K' | 'k' => "hour",
        'm' => "minute",
        's' => "second",
        'S' => "fractionalSecond",
        'z' | 'Z' | 'O' | 'v' | 'V' | 'X' | 'x' => "timeZoneName",
        _ => "literal",
    }
}

/// The length in bytes of the digit run at the start of `s` (any Unicode digit), or of the
/// non-digit run if `s` starts with a non-digit.
fn run_len(s: &str) -> usize {
    let first_is_digit = s.chars().next().is_some_and(char::is_numeric);
    s.char_indices()
        .find(|(_, ch)| ch.is_numeric() != first_is_digit)
        .map_or(s.len(), |(i, _)| i)
}

// ---- the crate functions (Date.prototype.toLocale*) -------------------------------------------

/// Date.prototype.toLocaleString, toLocaleDateString and toLocaleTimeString (ECMA-402 §20.4)
/// of the time value `tv`: "Invalid Date" for NaN, else FormatDateTime(CreateDateTimeFormat(
/// %Intl.DateTimeFormat%, locales, options, `required`, `defaults`), tv). No object is made
/// and `globalThis.Intl` is not read.
pub fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    tv: f64,
    locales: Local<'s>,
    options: Local<'s>,
    required: Required,
    defaults: Defaults,
) -> JsResult<Local<'s>> {
    // 2. If x is NaN, return "Invalid Date".
    if tv.is_nan() {
        return Ok(cx.new_string("Invalid Date")?.into());
    }
    // 3. Let dateFormat be ? CreateDateTimeFormat(%Intl.DateTimeFormat%, locales, options,
    //    required, defaults).
    let r = initialize(cx, locales, options, required, defaults)?;
    // 4. Return ! FormatDateTime(dateFormat, x).
    let s = format_date_time(cx, &r, tv)?;
    Ok(cx.new_string(&join(&s))?.into())
}

/// The string of `parts`.
fn join(parts: &[(&'static str, String)]) -> String {
    parts.iter().map(|(_, v)| v.as_str()).collect()
}

/// FormatDateTimeToParts ( dateTimeFormat, x ) (11.5.7) of `r`: the parts (a RangeError for
/// a time value that TimeClip makes NaN); FormatDateTime (11.5.6) is their string.
fn format_date_time(
    cx: &mut Cx<'_>,
    r: &Resolved,
    x: f64,
) -> JsResult<Vec<(&'static str, String)>> {
    // PartitionDateTimePattern 1-2. Let x be TimeClip(x); NaN: a RangeError.
    let Some(ms) = time_clip(x) else {
        return Err(cx.throw_range_error("Invalid time value"));
    };
    match format_ms(r, ms) {
        Some(sp) => Ok(sp),
        None => Err(cx.throw_type_error("Intl.DateTimeFormat: no date-time data")),
    }
}

/// The time value of the `date` argument of a format function: Date.now() for undefined,
/// else ? ToNumber(date).
fn date_value<'s>(cx: &mut Cx<'s>, date: Local<'s>) -> JsResult<f64> {
    if date.is_undefined() {
        // ! Call(%Date.now%, undefined): the current time in whole milliseconds.
        return Ok((cx.now_epoch_ns().div_euclid(1_000_000)) as f64);
    }
    cx.to_number(date)
}

// ---- the builtins -----------------------------------------------------------------------------

/// Intl.DateTimeFormat ( [ locales [ , options ] ] ) (11.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, let newTarget be the active function object, else let
    //    newTarget be NewTarget.
    let nt = match new_target.as_object() {
        Some(nt) => nt,
        None => args.callee().into(),
    };
    // 2. Let dateTimeFormat be ? CreateDateTimeFormat(newTarget, locales, options, any, date).
    //    Step 1 of it: OrdinaryCreateFromConstructor(newTarget,
    //    "%Intl.DateTimeFormat.prototype%", ...).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlDateTimeFormatPrototype)?;
    let dtf = cx.new_object_with_slots(proto, CLASS, SLOTS);
    let r = initialize(cx, args.get(0), args.get(1), Required::Any, Defaults::Date)?;
    store(cx, dtf, &r)?;
    // 3. If the implementation supports the normative optional constructor mode of 4.3
    //    Note 1, return ? ChainDateTimeFormat(dateTimeFormat, NewTarget, this).
    intl::chain_legacy_constructed(cx, IntrinsicId::IntlDateTimeFormat, dtf, new_target, this)
}

/// UnwrapDateTimeFormat ( dtf ) (11.5.2): the object that holds the slots (the legacy
/// `%Intl%.[[FallbackSymbol]]` property of an instance with no slots).
fn unwrap<'s>(cx: &mut Cx<'s>, dtf: Local<'s>) -> JsResult<Local<'s>> {
    let has_slots = cx.object_class(dtf) == Some(CLASS);
    intl::unwrap_legacy_constructed(cx, IntrinsicId::IntlDateTimeFormat, dtf, has_slots)
}

/// Intl.DateTimeFormat.supportedLocalesOf ( locales [ , options ] ) (11.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.DateTimeFormat%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// get Intl.DateTimeFormat.prototype.format (11.3.3).
fn get_format<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let dtf be ? UnwrapDateTimeFormat(this value) (then RequireInternalSlot).
    let dtf = unwrap(cx, this)?;
    let bound = cx.slot(dtf, CLASS, SLOT_BOUND_FORMAT)?;
    // 4. If dtf.[[BoundFormat]] is undefined, then
    if bound.is_undefined() {
        // a-b. Let F be a new built-in function object from the DateTime Format Functions
        //      closure (11.5.1; length 1, name ""), with F.[[DateTimeFormat]] dtf.
        let f = cx.new_builtin_closure(date_time_format_function, "", 1, dtf);
        // c. Set dtf.[[BoundFormat]] to F.
        cx.set_slot(dtf, CLASS, SLOT_BOUND_FORMAT, f)?;
        return Ok(f.into());
    }
    // 5. Return dtf.[[BoundFormat]].
    Ok(bound)
}

/// A DateTime Format function (11.5.1).
fn date_time_format_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let dtf be F.[[DateTimeFormat]].
    let dtf = cx.closure_data(args.callee());
    let r = read(cx, dtf)?;
    // 3-4. x: Date.now() for undefined, else ? ToNumber(date).
    let x = date_value(cx, args.get(0))?;
    // 5. Return ? FormatDateTime(dtf, x).
    let s = join(&format_date_time(cx, &r, x)?);
    Ok(cx.new_string(&s)?.into())
}

/// Intl.DateTimeFormat.prototype.formatToParts ( date ) (11.3.4).
fn format_to_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let dtf be the this value; ? RequireInternalSlot(dtf, [[InitializedDateTimeFormat]]).
    let r = read(cx, this)?;
    // 3-4. x: Date.now() for undefined, else ? ToNumber(date).
    let x = date_value(cx, args.get(0))?;
    // 5. Return ? FormatDateTimeToParts(dtf, x) (11.5.7).
    let parts = format_date_time(cx, &r, x)?;
    let array = cx.new_array();
    for (i, (t, v)) in parts.iter().enumerate() {
        cx.poll_interrupt()?;
        let o = cx.new_object();
        let t = cx.new_string(t)?;
        cx.create_data_property_or_throw(o, "type", t)?;
        let v = cx.new_string(v)?;
        cx.create_data_property_or_throw(o, "value", v)?;
        cx.create_data_property_index_or_throw(array, i as u64, o)?;
    }
    Ok(array.into())
}

/// Intl.DateTimeFormat.prototype.resolvedOptions ( ) (11.3.7).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let dtf be ? UnwrapDateTimeFormat(this value) (then RequireInternalSlot).
    let dtf = unwrap(cx, this)?;
    let r = read(cx, dtf)?;
    // 4. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 5. For each row of Table 17, in table order: locale, calendar, numberingSystem,
    //    timeZone, hourCycle, hour12, the components (no style set), dateStyle, timeStyle.
    for (key, value) in [
        ("locale", r.locale.as_str()),
        ("calendar", r.calendar.as_str()),
        ("numberingSystem", r.numbering_system.as_str()),
        ("timeZone", r.time_zone.as_str()),
    ] {
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, key, v)?;
    }
    if let Some(h) = r.styles.hour_cycle {
        let hc = HOUR_CYCLES[h];
        let v = cx.new_string(hc)?;
        cx.create_data_property_or_throw(o, "hourCycle", v)?;
        // hour12: true for h11 and h12, else false.
        let h12 = Local::boolean(matches!(hc, "h11" | "h12"));
        cx.create_data_property_or_throw(o, "hour12", h12)?;
    }
    if r.styles.date_style.is_none() && r.styles.time_style.is_none() {
        // The widths of the selected pattern (W4 Codex r1 F5).
        let shown = selected_components(&r);
        for (i, (prop, _, _)) in COMPONENTS.iter().enumerate() {
            let Some(v) = shown.get(i) else {
                continue;
            };
            if i == c::FRACTIONAL_SECOND_DIGITS {
                let n = Local::int32(i32::from(r.components.0[i]));
                cx.create_data_property_or_throw(o, *prop, n)?;
            } else {
                let v = cx.new_string(v)?;
                cx.create_data_property_or_throw(o, *prop, v)?;
            }
        }
    }
    for (key, style) in [
        ("dateStyle", r.styles.date_style),
        ("timeStyle", r.styles.time_style),
    ] {
        if let Some(s) = style {
            let v = cx.new_string(STYLES[s])?;
            cx.create_data_property_or_throw(o, key, v)?;
        }
    }
    // 6. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_forms_round_trip() {
        let mut cs = Components::default();
        for (i, (_, values, _)) in COMPONENTS.iter().enumerate() {
            cs.0[i] = values.len() as u8;
        }
        assert_eq!(Components::unpack(cs.pack()), cs);
        assert!(cs.pack() < 1 << 24);
        for hc in [None, Some(0), Some(3)] {
            for ds in [None, Some(0), Some(3)] {
                for ts in [None, Some(1), Some(2)] {
                    let s = Styles {
                        hour_cycle: hc,
                        date_style: ds,
                        time_style: ts,
                    };
                    assert_eq!(Styles::unpack(s.pack()), s);
                }
            }
        }
    }

    #[test]
    fn offset_time_zones() {
        for (s, want) in [
            ("+03", Some("+03:00")),
            ("-00", Some("+00:00")),
            ("-00:00", Some("+00:00")),
            ("+0103", Some("+01:03")),
            ("-22:23", Some("-22:23")),
            ("+23:59", Some("+23:59")),
        ] {
            assert_eq!(time_zone_identifier(s).as_deref(), want, "{s}");
        }
        for s in [
            "+3",
            "+24",
            "+23:0",
            "+130",
            "+13234",
            "-10.50",
            "-2400",
            "-1:10",
            "+15:59:00",
            "+222700",
            "\u{2212}05",
            "+1:00",
            "+ab",
        ] {
            assert_eq!(time_zone_identifier(s), None, "{s}");
        }
    }

    #[test]
    fn named_time_zones_keep_the_case_normalized_id() {
        for (s, want) in [
            ("utc", Some("UTC")),
            ("Etc/UTC", Some("Etc/UTC")),
            ("GMT", Some("GMT")),
            ("asia/calcutta", Some("Asia/Calcutta")),
            ("Europe/Bratislava", Some("Europe/Bratislava")),
            ("PST", None),
            ("America/San_Francisco", None),
        ] {
            assert_eq!(time_zone_identifier(s).as_deref(), want, "{s}");
        }
    }

    #[test]
    fn time_clip_is_an_integer() {
        assert_eq!(time_clip(1.9), Some(1));
        assert_eq!(time_clip(-1.9), Some(-1));
        assert_eq!(time_clip(8.64e15), Some(8_640_000_000_000_000));
        assert_eq!(time_clip(8.64e15 + 1.0), None);
        assert_eq!(time_clip(f64::NAN), None);
        assert_eq!(time_clip(f64::NEG_INFINITY), None);
    }
}
