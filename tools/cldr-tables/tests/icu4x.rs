//! The tables against the ICU4X compiled data of the same CLDR release (INTL-V2 W2-0 TABLES),
//! and the generated files against the generator. Needs the checkout of sync-cldr-json.sh.

use std::process::Command;

use cldr_tables::{
    NO_TEXT, PLURAL_FORMS, SANCTIONED_UNITS, Tables, UNIT_WIDTHS, default_paths, read, read_pin,
    render, stale,
};
use icu_locale_core::LanguageIdentifier;
use icu_provider::prelude::*;

fn tables() -> Tables {
    let (cldr, pin, _) = default_paths();
    let pin = read_pin(&pin).expect("pin");
    read(&cldr, &pin).expect("run tools/cldr-tables/sync-cldr-json.sh first")
}

/// The CLDR version of an ICU4X data crate: the README line "generated with CLDR version X,"
/// of the package that `cargo metadata` resolves (the exact pin of Cargo.toml).
fn icu4x_data_cldr_version(package: &str) -> String {
    let out = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()
        .expect("cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let manifests: Vec<&str> = meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter(|p| p["name"] == package)
        .filter_map(|p| p["manifest_path"].as_str())
        .collect();
    assert_eq!(manifests.len(), 1, "{package}: one version in the lock");
    let readme = std::path::Path::new(manifests[0]).with_file_name("README.md");
    let text = std::fs::read_to_string(&readme).expect("README.md");
    let at = text
        .find("generated with CLDR version ")
        .expect("CLDR version line");
    let rest = &text[at + "generated with CLDR version ".len()..];
    rest[..rest.find(',').expect(",")].to_owned()
}

#[test]
fn cldr_version_equals_icu4x_data() {
    let t = tables();
    assert_eq!(t.cldr_version, "49.0.0");
    for data in ["icu_decimal_data", "icu_experimental_data"] {
        assert_eq!(icu4x_data_cldr_version(data), t.cldr_version, "{data}");
    }
}

/// Digit table: every numbering system that the ICU4X data has (`DecimalDigitsV1`, keyed by
/// the numbering system) has the same 10 digits.
#[test]
fn digits_equal_icu4x() {
    let t = tables();
    assert_eq!(t.numbering_systems.len(), 78);
    let mut found = 0;
    for (nu, digits) in &t.numbering_systems {
        let attrs = DataMarkerAttributes::try_from_str(nu).expect("attribute");
        let req = DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes(attrs),
            ..Default::default()
        };
        let res: Result<DataResponse<icu_decimal::provider::DecimalDigitsV1>, _> =
            icu_decimal::provider::Baked.load(req);
        if let Ok(res) = res {
            assert_eq!(res.payload.get(), digits, "{nu}");
            found += 1;
        }
    }
    // The ICU4X 2.4 data has the 23 numbering systems that CLDR locales use.
    assert_eq!(found, 23);
    // The value of the ep9_intl unit test.
    assert_eq!(
        t.numbering_systems
            .iter()
            .find(|(n, _)| n == "arab")
            .map(|x| x.1[0]),
        Some('٠')
    );
}

/// T1: the approximately sign of each locale (default numbering system) equals the
/// `approximately_sign` of the ICU4X `DecimalPercentV1` data, for every locale that the data
/// has without fallback; the default numbering system equals `DecimalSymbolsV1` `numsys`.
/// ICU4X has no data field for NaN, infinity, the exponent symbol or the range pattern.
#[test]
fn t1_equals_icu4x() {
    let t = tables();
    let mut percent_hits = 0;
    let mut numsys_hits = 0;
    for (locale, nu) in &t.default_numbering_system {
        let langid: LanguageIdentifier = locale.parse().expect("locale id");
        let dl = DataLocale::from(&langid);
        let sym = t
            .symbols(locale, nu)
            .expect("a row for the default numbering system");

        let req = DataRequest {
            id: DataIdentifierBorrowed::for_locale(&dl),
            ..Default::default()
        };
        let res: DataResponse<icu_experimental::dimension::provider::percent::DecimalPercentV1> =
            icu_experimental::provider::Baked
                .load(req)
                .expect("percent data");
        if res.metadata.locale.is_none() {
            assert_eq!(
                res.payload.get().approximately_sign,
                sym.approximately_sign.as_str(),
                "{locale}"
            );
            percent_hits += 1;
        }

        let res: DataResponse<icu_decimal::provider::DecimalSymbolsV1> =
            icu_decimal::provider::Baked
                .load(req)
                .expect("symbols data");
        if res.metadata.locale.is_none() {
            assert_eq!(res.payload.get().numsys(), nu.as_str(), "{locale}");
            numsys_hits += 1;
        }
    }
    eprintln!(
        "exact ICU4X entries: DecimalPercentV1 {percent_hits}, DecimalSymbolsV1 {numsys_hits}"
    );
    assert!(percent_hits >= 20, "{percent_hits}");
    assert!(numsys_hits >= 20, "{numsys_hits}");
    // The value of the ep9_intl unit test.
    let en = t.symbols("en", "latn").expect("en");
    assert_eq!(
        (en.nan.as_str(), en.approximately_sign.as_str()),
        ("NaN", "~")
    );
}

/// The generator is deterministic, and the committed files are its output (`--check`).
#[test]
fn generated_files_are_current_and_deterministic() {
    let a = render(&tables());
    let b = render(&tables());
    assert_eq!(a, b);
    let (_, _, out) = default_paths();
    assert_eq!(
        stale(&out, &a),
        Vec::<String>::new(),
        "run the generator (README.md)"
    );
    let bytes: usize = ["digits.rs", "t1.rs"].iter().map(|f| a[f].len()).sum();
    // INTL-V2 plan 2.6: T1 + digits < 0.06 MB.
    assert!(bytes < 60_000, "{bytes}");
}

/// The tables that replace the W1 hand tables of `ep9_intl` (INTL W2-0 fix r1): one value of
/// each at CLDR 49.0.0, and the CLDR 48 values that CLDR 49 changed.
#[test]
fn locales_hour_cycles_and_aliases() {
    let t = tables();
    // availableLocales "full": 657 ids at CLDR 49.0.0 (766 at 48.2.1).
    assert_eq!(t.locales.len(), 657);
    assert!(t.locales.windows(2).all(|w| w[0] < w[1]));
    for l in ["en", "zh-Hant-HK", "sr-Latn", "ady", "kbd-TR"] {
        assert!(t.locales.binary_search(&l.to_owned()).is_ok(), "{l}");
    }
    for l in ["la", "co", "zh-Latn", "en-Dsrt"] {
        assert!(t.locales.binary_search(&l.to_owned()).is_err(), "{l}");
    }
    assert_eq!(
        t.locales,
        t.default_numbering_system
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    );

    // timeData: AR, CL, PY, UY, ZM prefer "H" at CLDR 49 ("h" at 48).
    let h12 = |r: &str| {
        t.hour_cycle_h12_regions
            .binary_search(&r.to_owned())
            .is_ok()
    };
    assert!(h12("US") && h12("419") && h12("KR"));
    for r in ["AR", "CL", "PY", "UY", "ZM", "GB", "001"] {
        assert!(!h12(r), "{r}");
    }
    let rows: Vec<(&str, &str, &str)> = t
        .hour_cycle_language_regions
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("ar", "001", "h12"),
            ("en", "001", "h12"),
            ("fr", "CA", "h23"),
            ("ku", "SY", "h23"),
        ]
    );

    // bcp47 value aliases.
    let has = |list: &[(String, String, String)], k: &str, a: &str, v: &str| {
        list.iter()
            .any(|(x, y, z)| (x.as_str(), y.as_str(), z.as_str()) == (k, a, v))
    };
    for (k, a, v) in [
        ("ca", "ethiopic-amete-alem", "ethioaa"),
        ("ca", "islamicc", "islamic-civil"),
        ("kn", "yes", "true"),
        ("ks", "primary", "level1"),
        ("ms", "imperial", "uksystem"),
        ("tz", "cnckg", "cnsha"),
        ("tz", "est", "papty"),
        ("tz", "eire", "iedub"),
    ] {
        assert!(has(&t.u_value_aliases, k, a, v), "{k} {a}");
    }
    // A deprecated type's own alias attribute is not a row ("islamicc" has alias
    // "islamic-civil"), and CLDR 49 dropped the CLDR 48 "est5edt" -> "usnyc" row.
    assert!(!has(&t.u_value_aliases, "ca", "islamic-civil", "islamicc"));
    assert!(!t.u_value_aliases.iter().any(|(_, a, _)| a == "est5edt"));
    assert!(has(&t.t_value_aliases, "m0", "names", "prprname"));
}

/// The ICU4X `UnitsNames*V1` markers of the 5 unit categories (area, duration, length, mass,
/// volume): the "other" pattern of `attributes` ("<width>-<unit>") for `locale`, if the data
/// has it without fallback.
fn icu4x_other<M>(attributes: &str, locale: &DataLocale) -> Option<String>
where
    M: DataMarker<
        DataStruct = icu_experimental::dimension::provider::units::display_names::UnitsDisplayNames<
            'static,
        >,
    >,
    icu_experimental::provider::Baked: DataProvider<M>,
{
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(
            DataMarkerAttributes::try_from_str(attributes).ok()?,
            locale,
        ),
        metadata,
    };
    let res = icu_experimental::provider::Baked.load(req).ok()?;
    if res.metadata.locale.is_some() {
        return None;
    }
    // INTL W3 correction 4 (b3): only the default ("other") pattern here; the check per plural
    // category is an ep9_intl test (`cldr::tests::t2_equals_icu4x_in_every_plural_category`),
    // because a read per category names icu_plurals, which this crate does not list.
    let p: &DataPayload<M> = &res.payload;
    Some(
        p.get()
            .elements
            .get_default()
            .1
            .interpolate_to_string(["{0}"]),
    )
}

/// T2: the "other" unit pattern of each unit, width and locale that the ICU4X units data has
/// without fallback equals the T2 pattern (both CLDR 49.0.0).
#[test]
fn t2_equals_icu4x() {
    use icu_experimental::dimension::provider::units::categorized_display_names::*;
    type Load = fn(&str, &DataLocale) -> Option<String>;
    let loaders: [Load; 15] = [
        icu4x_other::<UnitsNamesAreaCoreV1>,
        icu4x_other::<UnitsNamesAreaExtendedV1>,
        icu4x_other::<UnitsNamesAreaOutlierV1>,
        icu4x_other::<UnitsNamesDurationCoreV1>,
        icu4x_other::<UnitsNamesDurationExtendedV1>,
        icu4x_other::<UnitsNamesDurationOutlierV1>,
        icu4x_other::<UnitsNamesLengthCoreV1>,
        icu4x_other::<UnitsNamesLengthExtendedV1>,
        icu4x_other::<UnitsNamesLengthOutlierV1>,
        icu4x_other::<UnitsNamesMassCoreV1>,
        icu4x_other::<UnitsNamesMassExtendedV1>,
        icu4x_other::<UnitsNamesMassOutlierV1>,
        icu4x_other::<UnitsNamesVolumeCoreV1>,
        icu4x_other::<UnitsNamesVolumeExtendedV1>,
        icu4x_other::<UnitsNamesVolumeOutlierV1>,
    ];
    let t = tables();
    let (mut compared, mut units) = (0usize, std::collections::BTreeSet::new());
    let mut wrong = Vec::new();
    for locale in &t.locales {
        let langid: LanguageIdentifier = locale.parse().expect("locale id");
        let dl = DataLocale::from(&langid);
        for (w, width) in UNIT_WIDTHS.iter().enumerate() {
            for unit in SANCTIONED_UNITS {
                let attributes = format!("{width}-{unit}");
                let Some(icu) = loaders.iter().find_map(|load| load(&attributes, &dl)) else {
                    continue;
                };
                let record = t.unit_record(locale, unit, w).expect("a T2 record");
                let ours = t.unit_pattern(record, 5).expect("an other form");
                if ours != icu {
                    wrong.push(format!("{locale} {attributes}: T2 {ours:?}, ICU4X {icu:?}"));
                }
                compared += 1;
                units.insert(unit);
            }
        }
    }
    eprintln!(
        "T2 vs ICU4X: {compared} other patterns, {} units",
        units.len()
    );
    assert!(
        wrong.is_empty(),
        "{} differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    // The 5 ICU4X categories hold 30 of the 45 sanctioned units.
    assert_eq!(units.len(), 30, "{units:?}");
    assert!(compared > 10_000, "{compared}");
}

/// T2: its shape, one value of each kind (the test262 `unit-*.js` strings at CLDR 49.0.0),
/// and its size (INTL-V2 plan 2.6: T2 0.3-0.6 MB).
#[test]
fn t2_values_and_size() {
    let t = tables();
    assert_eq!(t.unit_ids.len(), 50);
    for u in SANCTIONED_UNITS {
        assert!(t.unit_ids.binary_search(&u.to_owned()).is_ok(), "{u}");
    }
    assert_eq!(t.t2_locale_sets.len(), t.locales.len());
    assert_eq!(t.t2_texts[usize::from(NO_TEXT)], "");
    let pattern = |l: &str, u: &str, w: &str, form: &str| {
        let w = UNIT_WIDTHS.iter().position(|x| *x == w).expect("width");
        let f = PLURAL_FORMS.iter().position(|x| *x == form).expect("form");
        let r = t.unit_record(l, u, w).expect("record");
        t.unit_pattern(r, f).map(str::to_owned)
    };
    let p = |l, u, w, f| pattern(l, u, w, f).expect("pattern");
    assert_eq!(p("en", "kilometer-per-hour", "short", "other"), "{0} km/h");
    assert_eq!(p("en", "kilometer-per-hour", "narrow", "other"), "{0}km/h");
    assert_eq!(
        p("en", "kilometer-per-hour", "long", "one"),
        "{0} kilometer per hour"
    );
    assert_eq!(p("de", "kilometer-per-hour", "short", "other"), "{0} km/h");
    assert_eq!(p("ko", "kilometer-per-hour", "short", "other"), "{0}km/h");
    assert_eq!(
        p("zh-Hant", "kilometer-per-hour", "long", "other"),
        "每小時 {0} 公里"
    );
    assert_eq!(p("ar", "hour", "long", "one"), "ساعة");
    // A form with the other pattern is not stored: the other pattern serves it.
    assert_eq!(p("ja", "hour", "long", "one"), "{0} 時間");
    let en_hour = t.unit_record("en", "hour", 0).expect("en hour");
    assert_eq!(t.t2_text(en_hour[6]), Some("{0}/h"));
    assert_eq!(t.per_pattern("en", 2), Some("{0} per {1}"));
    assert_eq!(t.per_pattern("ja", 2), Some("{0}毎{1}"));
    // The data size of T2 in the binary (est.: texts, 4 B per text offset, 14 B per record,
    // 6 B per set and unit, 1 B per locale).
    let bytes = t.t2_texts.iter().map(String::len).sum::<usize>()
        + 4 * (t.t2_texts.len() + 1)
        + 14 * t.t2_records.len()
        + t.t2_sets.len() * (6 + 6 * t.unit_ids.len())
        + t.t2_locale_sets.len();
    eprintln!(
        "T2: {} texts, {} records, {} sets, {bytes} B of data",
        t.t2_texts.len(),
        t.t2_records.len(),
        t.t2_sets.len()
    );
    assert!(bytes < 600_000, "{bytes}");
}
