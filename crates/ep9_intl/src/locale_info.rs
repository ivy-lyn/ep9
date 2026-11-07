#![allow(dead_code)] // m7-stub: INTL W1 LOCALE fills this module.
//! Locale info (Intl Locale Info proposal, ECMA-402 §14.3 additions): the support code of the
//! Intl.Locale.prototype methods `getCalendars`, `getCollations`, `getHourCycles`,
//! `getNumberingSystems`, `getTimeZones`, `getTextInfo` and `getWeekInfo`. No builtin of its
//! own: the ids are in the locale module (block 30).
//!
//! ```text
//! getTimeZones   ep9_temporal::tzdb::zones_in_region (INTL-I1-D9; zoneinfo64 regions)
//! getWeekInfo    icu_calendar week data (the H36 via of INTL-I1-D2 covers icu_calendar)
//! getCalendars   crate::available::CALENDARS order by the locale preference
//! ```
//!
//! Stub (INTL W1 CORE): LOCALE fills it.
