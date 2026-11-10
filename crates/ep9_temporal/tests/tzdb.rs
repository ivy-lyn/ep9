//! The time-zone provider (design SPEC-T-D13): the data file loads, and known offsets are
//! right, with the temporal_rs issue 759 probe (2026-03-07T12:00 America/New_York is -05:00:
//! the `tzif` providers of 0.2.6 give a time one hour off).

use temporal_rs::options::{Disambiguation, OffsetDisambiguation};
use temporal_rs::{PlainDateTime, TimeZone, ZonedDateTime};

#[test]
fn provider_loads_and_resolves_zones() {
    let p = ep9_temporal::tzdb::provider();
    for id in [
        "UTC",
        "America/New_York",
        "Europe/Paris",
        "Asia/Kolkata",
        "Australia/Lord_Howe",
    ] {
        let tz = TimeZone::try_from_identifier_str_with_provider(id, p).expect(id);
        assert_eq!(tz.identifier_with_provider(p).unwrap(), id);
    }
    assert!(TimeZone::try_from_identifier_str_with_provider("Mars/Olympus_Mons", p).is_err());
}

#[test]
fn issue_759_probe_has_the_right_offset() {
    let p = ep9_temporal::tzdb::provider();
    let tz = TimeZone::try_from_identifier_str_with_provider("America/New_York", p).unwrap();
    let dt = PlainDateTime::try_new_iso(2026, 3, 7, 12, 0, 0, 0, 0, 0).unwrap();
    let zdt = dt
        .to_zoned_date_time_with_provider(tz, Disambiguation::Compatible, p)
        .unwrap();
    assert_eq!(
        zdt.to_ixdtf_string_with_provider(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            p
        )
        .unwrap(),
        "2026-03-07T12:00:00-05:00[America/New_York]"
    );
    let z = ZonedDateTime::from_utf8_with_provider(
        b"2026-03-07T12:00:00-05:00[America/New_York]",
        Disambiguation::Compatible,
        OffsetDisambiguation::Reject,
        p,
    )
    .unwrap();
    assert_eq!(z.offset(), "-05:00");
    // Summer time after the 2026-03-08 transition.
    let z = ZonedDateTime::from_utf8_with_provider(
        b"2026-07-01T12:00:00[America/New_York]",
        Disambiguation::Compatible,
        OffsetDisambiguation::Reject,
        p,
    )
    .unwrap();
    assert_eq!(z.offset(), "-04:00");
}

/// The std-typed wrappers of INTL-I1-D9 (the Intl reads of the tz data).
#[test]
fn intl_wrappers_read_the_tz_data() {
    use ep9_temporal::tzdb::{canonical_zone, offset_at, zone_ids, zones_in_region};
    let ids = zone_ids();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "sorted, no repeats");
    for id in [
        "UTC",
        "America/New_York",
        "Asia/Kolkata",
        "Etc/GMT+1",
        "Etc/GMT-14",
    ] {
        assert!(ids.contains(&id), "{id}");
    }
    for id in ["Etc/UTC", "Etc/GMT", "Asia/Calcutta", "US/Pacific"] {
        assert!(!ids.contains(&id), "{id} is a link");
    }
    assert_eq!(canonical_zone("asia/calcutta"), Some("Asia/Kolkata"));
    assert_eq!(canonical_zone("US/Pacific"), Some("America/Los_Angeles"));
    assert_eq!(canonical_zone("Etc/UTC"), Some("UTC"));
    assert_eq!(canonical_zone("Mars/Olympus_Mons"), None);
    let us = zones_in_region("us");
    assert!(us.contains(&"America/New_York") && us.contains(&"Pacific/Honolulu"));
    assert!(!us.contains(&"Europe/Paris"));
    // 2026-07-01T00:00Z: New York is on DST (-4 h); 2026-01-01T00:00Z it is not (-5 h).
    let jul = 1_782_864_000_000_000_000i128;
    let jan = 1_767_225_600_000_000_000i128;
    assert_eq!(offset_at("America/New_York", jul), Some((-4 * 3600, true)));
    assert_eq!(offset_at("America/New_York", jan), Some((-5 * 3600, false)));
    assert_eq!(offset_at("UTC", jan), Some((0, false)));
    assert_eq!(offset_at("Asia/Kolkata", jan), Some((19_800, false)));
    assert_eq!(offset_at("Nowhere/City", jan), None);
}
