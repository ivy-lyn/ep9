//! The tables against the ICU4X compiled data of the same CLDR release (INTL-V2 W2-0 TABLES),
//! and the generated files against the generator. Needs the checkout of sync-cldr-json.sh.

use std::process::Command;

use cldr_tables::{Tables, default_paths, read, read_pin, render, stale};
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
    let bytes: usize = a.values().map(String::len).sum();
    // INTL-V2 plan 2.6: T1 + digits < 0.06 MB.
    assert!(bytes < 60_000, "{bytes}");
}
