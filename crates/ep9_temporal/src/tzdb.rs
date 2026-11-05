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

/// The tzdb version of [`RES`] (`data/README.md`).
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
