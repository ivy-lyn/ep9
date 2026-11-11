//! The time-zone provider (design SPEC-T-D13, D14): `ZoneInfo64TzdbProvider` over the ICU
//! `zoneinfo64.res` in `crates/ep9_temporal/data` (see its README).
//!
//! ```text
//! include_bytes!(data/zoneinfo64.res)   149,696 bytes, little-endian
//!   -> u32::from_le_bytes per 4 bytes    WORDS: OnceLock<Box<[u32]>>   (no `unsafe`: forbid)
//!   -> ZoneInfo64::try_from_u32s         PROVIDER: OnceLock<provider>  (once per process)
//! ```
//!
//! This module imports nothing from ep9_vm, so it can move to a crate below ep9_vm when Date
//! takes the same provider (design SPEC-T-D14; H20 clock row).

use std::sync::OnceLock;

use timezone_provider::zoneinfo64::ZoneInfo64TzdbProvider;
use zoneinfo64::ZoneInfo64;

/// The ICU `zoneinfo64.res` (tzdb 2026f), the `le` build: all ep9 targets are little-endian.
static RES: &[u8] = include_bytes!("../data/zoneinfo64.res");

/// The tzdb version of `RES` (`data/README.md`).
pub const TZDB_VERSION: &str = "2026f";

static WORDS: OnceLock<Box<[u32]>> = OnceLock::new();
static PROVIDER: OnceLock<ZoneInfo64TzdbProvider<'static>> = OnceLock::new();

/// The resource bundle as 32-bit words (a safe copy; `resb::include_bytes_as_u32!` expands
/// `unsafe`, which this crate forbids).
fn words() -> &'static [u32] {
    WORDS.get_or_init(|| {
        let (words, rest) = RES.as_chunks::<4>();
        assert!(rest.is_empty(), "zoneinfo64.res is a whole number of words");
        words.iter().map(|w| u32::from_le_bytes(*w)).collect()
    })
}

/// The process-wide time-zone provider. Panics if the data file does not parse: the file is
/// part of the crate, and `tests/tzdb.rs` loads it.
pub fn provider() -> &'static ZoneInfo64TzdbProvider<'static> {
    PROVIDER.get_or_init(|| {
        let zi = ZoneInfo64::try_from_u32s(words()).expect("data/zoneinfo64.res parses");
        ZoneInfo64TzdbProvider::new(zi)
    })
}

// ---- std-typed wrappers for Intl (INTL-I1-D9; INTL W1 CORE) ---------------------------------
//
// ep9_intl reads the tz data only through these functions (std types; no timezone_provider or
// zoneinfo64 type crosses the crate edge, so ep9_intl needs no edge to either crate, check 19).

use std::borrow::Cow;

use timezone_provider::CompiledNormalizer;
use timezone_provider::provider::{NormalizedId, TimeZoneNormalizer, TimeZoneProvider};

static NORMALIZER: CompiledNormalizer = CompiledNormalizer;
static ZONE_IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
static ZONEINFO: OnceLock<ZoneInfo64<'static>> = OnceLock::new();

/// The zoneinfo64 data for the offset and region reads (a second parse of the same words;
/// the provider keeps its copy private).
fn zoneinfo() -> &'static ZoneInfo64<'static> {
    ZONEINFO.get_or_init(|| ZoneInfo64::try_from_u32s(words()).expect("data/zoneinfo64.res parses"))
}

/// The primary identifier of the normalized id `id`: the canonical IANA name, with "UTC" for
/// the UTC links ("Etc/UTC", "Etc/GMT", ...), as ECMA-262 GetAvailableNamedTimeZoneIdentifier
/// gives it.
fn primary_of(id: NormalizedId) -> Option<&'static str> {
    let c = NORMALIZER.canonicalized(id).ok()?;
    let name = NORMALIZER.identifier(c).ok()?;
    Some(if matches!(name, "Etc/UTC" | "Etc/GMT") {
        "UTC"
    } else {
        name
    })
}

/// The primary IANA time zone identifiers of the tz data, sorted (Intl.supportedValuesOf
/// "timeZone": AvailablePrimaryTimeZoneIdentifiers). Made once per process.
///
/// Only the ids that [`provider`] resolves: the id list of the normalizer can be newer than
/// `zoneinfo64.res` (the normalizer has "Asia/Hanoi"; the ICU 2026f file does not), and
/// every listed id must make a Temporal time zone (ECMA-402 AvailableNamedTimeZoneIdentifiers).
pub fn zone_ids() -> &'static [&'static str] {
    ZONE_IDS.get_or_init(|| {
        let mut out: Vec<&'static str> = Vec::new();
        let mut i = 0;
        while let Ok(name) = NORMALIZER.identifier(NormalizedId(i)) {
            if primary_of(NormalizedId(i)) == Some(name) && provider().get(name.as_bytes()).is_ok()
            {
                out.push(name);
            }
            i += 1;
        }
        out.sort_unstable();
        out.dedup();
        out
    })
}

/// The primary identifier of the time zone `id` (any case; a link gives its target:
/// "asia/calcutta" -> "Asia/Kolkata", "Etc/GMT" -> "UTC"), or `None` for an unknown name.
pub fn canonical_zone(id: &str) -> Option<&'static str> {
    let tz = provider().get(id.as_bytes()).ok()?;
    let c = provider().canonicalized(tz).ok()?;
    let name = match provider().identifier(c).ok()? {
        Cow::Borrowed(s) => s,
        Cow::Owned(s) => zone_ids().iter().copied().find(|z| *z == s)?,
    };
    Some(if matches!(name, "Etc/UTC" | "Etc/GMT") {
        "UTC"
    } else {
        name
    })
}

/// The primary identifiers of the zones of the region `region` (an ISO 3166 code, any case:
/// "US"), sorted (Intl.Locale `getTimeZones`; the zoneinfo64 region of each zone).
pub fn zones_in_region(region: &str) -> Vec<&'static str> {
    let region = region.to_ascii_uppercase();
    zone_ids()
        .iter()
        .copied()
        .filter(|id| {
            zoneinfo()
                .get(id)
                .is_some_and(|z| z.region().as_str() == region)
        })
        .collect()
}

/// The UTC offset in seconds and the DST flag ("the rule applies": not standard time) of the
/// time zone `zone` (any name that [`canonical_zone`] knows) at the instant `epoch_ns`
/// (nanoseconds since the epoch), or `None` for an unknown name.
pub fn offset_at(zone: &str, epoch_ns: i128) -> Option<(i32, bool)> {
    let name = canonical_zone(zone)?;
    let name = if name == "UTC" { "Etc/UTC" } else { name };
    let z = zoneinfo().get(name)?;
    let seconds = epoch_ns.div_euclid(1_000_000_000);
    let seconds = i64::try_from(seconds).ok()?;
    let o = z.for_timestamp(seconds);
    Some((o.offset.seconds(), o.rule_applies))
}
