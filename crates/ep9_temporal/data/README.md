# Time-zone data for ep9_temporal

| Item | Value |
|---|---|
| File | `zoneinfo64.res` (ICU4C resource bundle, little-endian) |
| Source | `unicode-org/icu-data`, path `tzdata/icunew/2026f/44/le/zoneinfo64.res` |
| tzdb version | 2026f (`ep9_temporal::tzdb::TZDB_VERSION`) |
| Size | 149,696 bytes |
| SHA-256 | `d9382726ee8a499514310e63a9e97f1dbdd7fc2db328e1a785dde05f2afbb7e6` |
| Licence | Unicode-3.0 (`THIRD-PARTY-LICENSE`, entry "ICU time-zone data") |
| Reader | `timezone_provider::zoneinfo64::ZoneInfo64TzdbProvider` (design SPEC-T-D13) |

Why this provider: at `timezone_provider` 0.2.6 the `tzif` and `experimental_tzif` providers
give wrong offsets for zones whose recent rules come from the POSIX TZ string (temporal_rs
issue 759). The zoneinfo64 provider is correct and is the one V8 uses.

## Update (one PR per tzdb release)

1. Download `tzdata/icunew/<version>/44/le/zoneinfo64.res` from `unicode-org/icu-data`.
2. Replace `zoneinfo64.res`; update the version, size and SHA-256 above and `TZDB_VERSION` in
   `src/tzdb.rs`.
3. Run `cargo test --locked -p ep9_temporal` (`tests/tzdb.rs` loads the file and checks
   known offsets).
