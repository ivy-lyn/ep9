//! Locale info (Intl Locale Info proposal, the ECMA-402 §14.3 additions): the
//! Intl.Locale.prototype methods `getCalendars`, `getCollations`, `getHourCycles`,
//! `getNumberingSystems`, `getTimeZones`, `getTextInfo` and `getWeekInfo`. Their ids are in the
//! locale module (block 30, ids 16-22).
//!
//! ```text
//! RegionPreference(loc)   "rg" override > region subtag > "sd" subdivision
//!                         > the region of Add Likely Subtags > "001"
//! getCalendars            [ca] | CALENDAR_PREFERENCES[region] (CLDR 48 calendarPreferenceData,
//!                         only the ids of crate::available::CALENDARS)
//! getCollations           [co] | the crate::available::COLLATIONS that the icu_collator data
//!                         has for the lookup locale (root "und" if none), sorted
//! getHourCycles           [hc] | CLDR 48 timeData "preferred" (HOUR_CYCLE_*), language-region
//!                         rows first
//! getNumberingSystems     [nu] | the icu_decimal default of the lookup locale | ["latn"]
//! getTimeZones            ep9_temporal::tzdb::zones_in_region(region subtag) | undefined
//! getTextInfo             { direction }: the script (or the likely script) in the icu_locale
//!                         script direction data; else undefined
//! getWeekInfo             { firstDay, weekend }: [[FirstDayOfWeek]] or icu_calendar week data
//!                         for "und-<region>" (the H36 via of INTL-I1-D2 covers icu_calendar)
//! ```
//!
//! No ICU4X object is kept: each call loads `'static` compiled data (rule 8).

use ep9_vm::{Args, Cx, JsResult, Local};
use icu_calendar::types::Weekday;
use icu_calendar::week::WeekInformation;
use icu_collator::provider::{Baked as CollatorData, CollationMetadataV1};
use icu_decimal::provider::{Baked as DecimalData, DecimalSymbolsV1};
use icu_locale::{Direction, LocaleDirectionality};
use icu_locale_core::{LanguageIdentifier, Locale};
use icu_provider::prelude::*;

use crate::available;
use crate::intl::string_array;
use crate::locale::{
    self, BaseName, CLASS, SLOT_CALENDAR, SLOT_COLLATION, SLOT_FIRST_DAY_OF_WEEK, SLOT_HOUR_CYCLE,
    SLOT_NUMBERING_SYSTEM,
};
use crate::resolve;
use crate::tag;

// ---- data -----------------------------------------------------------------------------------

/// CLDR 48 `calendarPreferenceData` (supplemental/calendarPreferenceData.json; the copy in
/// the icu_provider_source 2.3.0 test data, which node v26.13.0 (ICU 78.3, CLDR 48) gives for
/// each of its regions), as (regions, calendars in preference order). The ids are CLDR BCP 47
/// ids ("gregorian" is "gregory"); "islamic" and "islamic-rgsa" are left out, because they are
/// not in AvailableCalendars (INTL-I1-D2). Every other region: "001".
const CALENDAR_PREFERENCES: &[(&[&str], &[&str])] = &[
    (&["001"], &["gregory"]),
    (
        &["AE", "BH", "KW", "QA"],
        &[
            "gregory",
            "islamic-umalqura",
            "islamic-civil",
            "islamic-tbla",
        ],
    ),
    (
        &["AF", "IR"],
        &["persian", "gregory", "islamic-civil", "islamic-tbla"],
    ),
    (
        &[
            "AL", "AZ", "BD", "DJ", "DZ", "EH", "ER", "ID", "IQ", "JO", "KM", "LB", "LY", "MA",
            "MR", "MV", "MY", "NE", "OM", "PK", "PS", "SD", "SY", "TD", "TJ", "TM", "TN", "TR",
            "UZ", "XK", "YE",
        ],
        &["gregory", "islamic-civil", "islamic-tbla"],
    ),
    (&["CN", "CX", "HK", "MO", "SG"], &["gregory", "chinese"]),
    (
        &["EG"],
        &["gregory", "coptic", "islamic-civil", "islamic-tbla"],
    ),
    (&["ET"], &["gregory", "ethiopic"]),
    (
        &["IL"],
        &["gregory", "hebrew", "islamic-civil", "islamic-tbla"],
    ),
    (&["IN"], &["gregory", "indian"]),
    (&["JP"], &["gregory", "japanese"]),
    (&["KR"], &["gregory", "dangi"]),
    (&["SA"], &["gregory", "islamic-umalqura"]),
    (&["TH"], &["buddhist", "gregory"]),
    (&["TW"], &["gregory", "roc", "chinese"]),
];

/// CLDR 48 `timeData`: the regions whose preferred hour format is "h" (h12), sorted. Every
/// other region (and "001") prefers "H" (h23); CLDR 48 has no region that prefers "K" or "k".
/// Made from node v26.13.0 `new Intl.Locale("und-" + region).getHourCycles()` over every
/// region code it names (the deprecated codes NH and YD left out).
const HOUR_CYCLE_H12_REGIONS: &[&str] = &[
    "419", "AE", "AG", "AL", "AR", "AS", "AU", "BB", "BD", "BH", "BM", "BN", "BO", "BS", "BT",
    "CA", "CL", "CO", "CR", "CU", "CY", "DJ", "DM", "DO", "DZ", "EC", "EG", "EH", "ER", "ET", "FJ",
    "FM", "GD", "GH", "GM", "GR", "GT", "GU", "GY", "HK", "HN", "IN", "IQ", "JM", "JO", "KH", "KI",
    "KN", "KP", "KR", "KW", "KY", "LB", "LC", "LR", "LS", "LY", "MH", "MO", "MP", "MR", "MW", "MX",
    "MY", "NA", "NI", "NZ", "OM", "PA", "PE", "PG", "PH", "PK", "PR", "PS", "PW", "PY", "QA", "SA",
    "SB", "SD", "SG", "SL", "SO", "SS", "SV", "SY", "SZ", "TC", "TD", "TN", "TO", "TT", "TW", "UM",
    "US", "UY", "VC", "VE", "VG", "VI", "VU", "WS", "YE", "ZM",
];

/// CLDR 48 `timeData` rows for a language and a region, as (language, region, hour cycle):
/// they come before the region rows. The rows where node v26.13.0 gives a language-region
/// result other than the region result, over every language of
/// [`available::CLDR_LOCALES`].
const HOUR_CYCLE_LANGUAGE_REGIONS: &[(&str, &str, &str)] = &[
    ("ar", "001", "h12"),
    ("en", "001", "h12"),
    ("fr", "CA", "h23"),
    ("ku", "SY", "h23"),
];

// ---- RegionPreference -----------------------------------------------------------------------

/// The value of the Unicode extension keyword `key` of the canonical tag `tag`: `None` if
/// there is none, `Some("")` for a keyword with no value (or "true").
fn keyword(tag: &str, key: &str) -> Option<String> {
    tag::unicode_keywords(tag)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// CanonicalUnicodeSubdivision ( locale, key ): the region of the `unicode_subdivision_id`
/// value of the keyword `key` ("rg" or "sd"; "gbzzzz" -> "GB", "inka" -> "IN"), upper case.
/// `None` if there is no keyword, the value is no subdivision id, or the region is a private
/// use code (AA, QM-QZ, XA-XZ, ZZ).
fn subdivision_region(tag: &str, key: &str) -> Option<String> {
    let v = keyword(tag, key)?;
    let b = v.as_bytes();
    let n = if b.len() >= 2 && b[..2].iter().all(u8::is_ascii_alphabetic) {
        2
    } else if b.len() >= 3 && b[..3].iter().all(u8::is_ascii_digit) {
        3
    } else {
        return None;
    };
    let suffix = &v[n..];
    if !(1..=4).contains(&suffix.len()) || !suffix.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let region = v[..n].to_ascii_uppercase();
    let private = region == "AA"
        || region == "ZZ"
        || (region.starts_with('Q') && region.as_bytes()[1] >= b'M')
        || region.starts_with('X');
    (!private).then_some(region)
}

/// RegionPreference ( locale ): the region whose data the Locale info methods use (the "rg"
/// override, the region subtag, the "sd" subdivision, the region of Add Likely Subtags, then
/// "001"), as in SpiderMonkey `Locale.cpp` and ECMA-402 PR 1059.
pub fn region_preference(tag: &str) -> String {
    if let Some(r) = subdivision_region(tag, "rg") {
        return r;
    }
    let (base, _) = BaseName::split(tag);
    if let Some(r) = &base.region {
        return r.clone();
    }
    if let Some(r) = subdivision_region(tag, "sd") {
        return r;
    }
    let likely = locale::add_likely_subtags(&BaseName {
        variants: Vec::new(),
        region: None,
        ..base
    });
    likely.region.unwrap_or_else(|| "001".to_string())
}

// ---- the lists (no Cx) ----------------------------------------------------------------------

/// CalendarsOfLocale ( loc ) for the canonical tag `tag` with no "ca" keyword.
pub fn calendars_of(tag: &str) -> &'static [&'static str] {
    let region = region_preference(tag);
    CALENDAR_PREFERENCES
        .iter()
        .find(|(regions, _)| regions.contains(&region.as_str()))
        .map_or(CALENDAR_PREFERENCES[0].1, |(_, c)| c)
}

/// Whether the icu_collator data has the collation `collation` for `locale` (with locale
/// fallback, as `icu_collator::Collator` loads it).
fn has_collation(locale: &Locale, collation: &str) -> bool {
    let Ok(attr) = DataMarkerAttributes::try_from_str(collation) else {
        return false;
    };
    let dl = DataLocale::from(locale);
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    let r: Result<DataResponse<CollationMetadataV1>, DataError> = CollatorData.load(DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attr, &dl),
        metadata,
    });
    r.is_ok()
}

/// CollationsOfLocale ( loc ) for the canonical tag `tag` with no "co" keyword: the
/// AvailableCollations that the collation data has for the lookup locale of the base name
/// (LookupMatchingLocaleByPrefix over the available locales), sorted. A tag whose language is
/// "und", or that has no available prefix, uses the root collations (ECMA-402 PR 1072).
/// INTL W1 COLLATOR can use this list for `Intl.Collator` too.
pub fn collations_of(tag: &str) -> Vec<&'static str> {
    let (base, _) = BaseName::split(tag);
    let found = if base.language == "und" {
        None
    } else {
        resolve::lookup_matching_locale(&[base.text()]).map(|m| m.locale)
    };
    let locale = found
        .and_then(|l| Locale::try_from_str(&l).ok())
        .unwrap_or(Locale::UNKNOWN);
    available::COLLATIONS
        .iter()
        .copied()
        .filter(|c| has_collation(&locale, c))
        .collect()
}

/// HourCyclesOfLocale ( loc ) for the canonical tag `tag` with no "hc" keyword: the preferred
/// hour cycle of the language and the region of [`region_preference`].
pub fn hour_cycles_of(tag: &str) -> &'static str {
    let region = region_preference(tag);
    let (base, _) = BaseName::split(tag);
    if let Some((_, _, hc)) = HOUR_CYCLE_LANGUAGE_REGIONS
        .iter()
        .find(|(l, r, _)| *l == base.language && *r == region)
    {
        return hc;
    }
    if HOUR_CYCLE_H12_REGIONS
        .binary_search(&region.as_str())
        .is_ok()
    {
        "h12"
    } else {
        "h23"
    }
}

/// NumberingSystemsOfLocale ( loc ) for the canonical tag `tag` with no "nu" keyword: the
/// default numbering system (the icu_decimal symbols data) of the lookup locale over the
/// available locales; "latn" if none is found.
pub fn numbering_system_of(tag: &str) -> String {
    let (base, _) = BaseName::split(tag);
    let Some(found) = resolve::lookup_matching_locale(&[base.text()]) else {
        return "latn".to_string();
    };
    let Ok(locale) = Locale::try_from_str(&found.locale) else {
        return "latn".to_string();
    };
    let dl = DataLocale::from(&locale);
    let r: Result<DataResponse<DecimalSymbolsV1>, DataError> = DecimalData.load(DataRequest {
        id: DataIdentifierBorrowed::for_locale(&dl),
        ..Default::default()
    });
    r.map_or_else(
        |_| "latn".to_string(),
        |r| r.payload.get().numsys().to_string(),
    )
}

/// TextDirectionOfLocale ( loc ): the direction of the script of the base name, or of its
/// likely script; `None` if there is no script or the script direction data has none for it.
pub fn text_direction_of(tag: &str) -> Option<&'static str> {
    let (base, _) = BaseName::split(tag);
    let script = match base.script.clone() {
        Some(s) => s,
        None => locale::add_likely_subtags(&base).script?,
    };
    let id = LanguageIdentifier::try_from_str(&format!("und-{script}")).ok()?;
    match LocaleDirectionality::new_extended().get(&id)? {
        Direction::LeftToRight => Some("ltr"),
        Direction::RightToLeft => Some("rtl"),
        _ => None,
    }
}

/// WeekdayToNumber of a `[[FirstDayOfWeek]]` value ("mon" -> 1 .. "sun" -> 7).
fn weekday_number(fw: &str) -> Option<i32> {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
        .iter()
        .position(|d| *d == fw)
        .map(|i| i as i32 + 1)
}

/// The day number of `d` (Monday 1 .. Sunday 7).
fn day_number(d: Weekday) -> i32 {
    match d {
        Weekday::Monday => 1,
        Weekday::Tuesday => 2,
        Weekday::Wednesday => 3,
        Weekday::Thursday => 4,
        Weekday::Friday => 5,
        Weekday::Saturday => 6,
        Weekday::Sunday => 7,
    }
}

/// WeekInfoOfLocale ( loc ): the first day (`first_day` if it is a weekday id, else the week
/// data of the region of [`region_preference`]) and the weekend days in ascending order.
pub fn week_info_of(tag: &str, first_day: Option<&str>) -> (i32, Vec<i32>) {
    let region = region_preference(tag);
    let data = Locale::try_from_str(&format!("und-{region}"))
        .ok()
        .and_then(|l| WeekInformation::try_new((&l).into()).ok())
        .or_else(|| WeekInformation::try_new((&Locale::UNKNOWN).into()).ok());
    let (first, weekend) = match data {
        Some(w) => {
            let days = [
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
                Weekday::Saturday,
                Weekday::Sunday,
            ];
            let weekend = days
                .iter()
                .filter(|d| w.weekend.contains(**d))
                .map(|d| day_number(*d))
                .collect();
            (day_number(w.first_weekday), weekend)
        }
        None => (1, vec![6, 7]),
    };
    (first_day.and_then(weekday_number).unwrap_or(first), weekend)
}

// ---- the methods ----------------------------------------------------------------------------

/// The slot `slot` of `this` as a Rust string, or `None` for undefined.
fn slot_text<'s>(cx: &mut Cx<'s>, this: Local<'s>, slot: usize) -> JsResult<Option<String>> {
    let v = cx.slot(this, CLASS, slot)?;
    if v.is_undefined() {
        return Ok(None);
    }
    Ok(Some(cx.to_rust_string(v)?))
}

/// The list of a keyword method: `[value]` if the keyword slot `slot` is set, else
/// `default(tag)`.
fn keyword_list<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    slot: usize,
    default: fn(&str) -> Vec<String>,
) -> JsResult<Local<'s>> {
    // 1-2. Let loc be the this value; ? RequireInternalSlot(loc, [[InitializedLocale]]).
    let tag = locale::this_locale(cx, this)?;
    // 3. The preferred value of the Unicode extension, else the locale data.
    let list = match slot_text(cx, this, slot)? {
        Some(v) => vec![v],
        None => default(&tag),
    };
    // 4. Return CreateArrayFromList(list).
    string_array(cx, &list)
}

/// Intl.Locale.prototype.getCalendars ( ): CalendarsOfLocale(loc).
pub(crate) fn get_calendars<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    keyword_list(cx, this, SLOT_CALENDAR, |t| {
        calendars_of(t).iter().map(|s| (*s).to_string()).collect()
    })
}

/// Intl.Locale.prototype.getCollations ( ): CollationsOfLocale(loc).
pub(crate) fn get_collations<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    keyword_list(cx, this, SLOT_COLLATION, |t| {
        collations_of(t).iter().map(|s| (*s).to_string()).collect()
    })
}

/// Intl.Locale.prototype.getHourCycles ( ): HourCyclesOfLocale(loc).
pub(crate) fn get_hour_cycles<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    keyword_list(cx, this, SLOT_HOUR_CYCLE, |t| {
        vec![hour_cycles_of(t).to_string()]
    })
}

/// Intl.Locale.prototype.getNumberingSystems ( ): NumberingSystemsOfLocale(loc).
pub(crate) fn get_numbering_systems<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    keyword_list(cx, this, SLOT_NUMBERING_SYSTEM, |t| {
        vec![numbering_system_of(t)]
    })
}

/// Intl.Locale.prototype.getTimeZones ( ): TimeZonesOfLocale(loc): undefined if the tag has
/// no region subtag, else the primary zone ids of that region, sorted (INTL-I1-D9).
pub(crate) fn get_time_zones<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let base = locale::this_base_name(cx, this)?;
    let Some(region) = base.region else {
        return Ok(Local::undefined());
    };
    let zones = ep9_temporal::tzdb::zones_in_region(&region);
    string_array(cx, &zones)
}

/// Intl.Locale.prototype.getTextInfo ( ): { direction } (TextDirectionOfLocale).
pub(crate) fn get_text_info<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let tag = locale::this_locale(cx, this)?;
    let info = cx.new_object();
    let dir: Local<'s> = match text_direction_of(&tag) {
        Some(d) => cx.new_string(d)?.into(),
        None => Local::undefined(),
    };
    cx.create_data_property_or_throw(info, "direction", dir)?;
    Ok(info.value())
}

/// Intl.Locale.prototype.getWeekInfo ( ): { firstDay, weekend } (WeekInfoOfLocale).
pub(crate) fn get_week_info<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let tag = locale::this_locale(cx, this)?;
    let fw = slot_text(cx, this, SLOT_FIRST_DAY_OF_WEEK)?;
    let (first, weekend) = week_info_of(&tag, fw.as_deref());
    let info = cx.new_object();
    cx.create_data_property_or_throw(info, "firstDay", Local::int32(first))?;
    let days = cx.new_array();
    for (i, d) in weekend.iter().enumerate() {
        cx.create_data_property_index_or_throw(days, i as u64, Local::int32(*d))?;
    }
    cx.create_data_property_or_throw(info, "weekend", days)?;
    Ok(info.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_use_available_calendars() {
        assert!(HOUR_CYCLE_H12_REGIONS.windows(2).all(|w| w[0] < w[1]));
        for (_, cals) in CALENDAR_PREFERENCES {
            for c in *cals {
                assert!(available::CALENDARS.contains(c), "{c}");
            }
        }
    }

    /// The region priority of the test262 `getCalendars/region-priority.js` and
    /// `getWeekInfo/region-priority.js` files.
    #[test]
    fn region_preference_order() {
        assert_eq!(region_preference("fa-JP-u-rg-thzzzz-sd-inka"), "TH");
        assert_eq!(region_preference("fa-JP-u-sd-inka"), "JP");
        assert_eq!(region_preference("fa-u-sd-inka"), "IN");
        assert_eq!(region_preference("fa"), "IR");
        assert_eq!(region_preference("eo"), "001");
        assert_eq!(region_preference("en"), "US");
        assert_eq!(region_preference("en-u-rg-zzzzzz"), "US");
        assert_eq!(region_preference("en-u-sd-gbeng"), "GB");
    }

    /// node v26.13.0 results (its own "islamic" ids left out).
    #[test]
    fn lists_give_the_node_results() {
        assert_eq!(
            calendars_of("fa"),
            ["persian", "gregory", "islamic-civil", "islamic-tbla"]
        );
        assert_eq!(calendars_of("en-JP"), ["gregory", "japanese"]);
        assert_eq!(calendars_of("th"), ["buddhist", "gregory"]);
        assert_eq!(calendars_of("eo"), ["gregory"]);
        assert_eq!(collations_of("de"), ["emoji", "eor", "phonebk"]);
        assert_eq!(collations_of("de-AT"), ["emoji", "eor", "phonebk"]);
        assert_eq!(
            collations_of("zh-TW"),
            ["emoji", "eor", "pinyin", "stroke", "unihan", "zhuyin"]
        );
        assert_eq!(collations_of("und"), ["emoji", "eor"]);
        assert_eq!(collations_of("qfz"), ["emoji", "eor"]);
        assert_eq!(collations_of("abcdefgh"), ["emoji", "eor"]);
        assert_eq!(hour_cycles_of("en"), "h12");
        assert_eq!(hour_cycles_of("en-GB"), "h23");
        assert_eq!(hour_cycles_of("fr-CA"), "h23");
        assert_eq!(hour_cycles_of("en-CA"), "h12");
        assert_eq!(hour_cycles_of("en-001"), "h12");
        assert_eq!(hour_cycles_of("de-001"), "h23");
        assert_eq!(hour_cycles_of("ja"), "h23");
        assert_eq!(numbering_system_of("fa"), "arabext");
        assert_eq!(numbering_system_of("ar-EG"), "arab");
        assert_eq!(numbering_system_of("en"), "latn");
        assert_eq!(numbering_system_of("abcdefgh"), "latn");
        assert_eq!(text_direction_of("ar"), Some("rtl"));
        assert_eq!(text_direction_of("pa"), Some("ltr"));
        assert_eq!(text_direction_of("pa-PK"), Some("rtl"));
        assert_eq!(text_direction_of("und"), Some("ltr"));
        assert_eq!(text_direction_of("tlh"), None);
        assert_eq!(text_direction_of("en-Brai"), None);
        assert_eq!(text_direction_of("abcdefgh-Arab"), Some("rtl"));
        assert_eq!(week_info_of("en-US", None), (7, vec![6, 7]));
        assert_eq!(week_info_of("fa-AF", None), (6, vec![4, 5]));
        assert_eq!(week_info_of("fa-IN", None), (7, vec![7]));
        assert_eq!(week_info_of("fa", None), (6, vec![5]));
        assert_eq!(week_info_of("eo", None), (1, vec![6, 7]));
        assert_eq!(week_info_of("en-u-fw-mon", Some("mon")), (1, vec![6, 7]));
        assert_eq!(week_info_of("en", Some("lunedi")), (7, vec![6, 7]));
    }
}
