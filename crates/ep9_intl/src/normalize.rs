//! Unicode normalization for `String.prototype.normalize` (ECMA-262 §22.1.3.15; block 6 member
//! 6.48 in `ep9_builtins` string.rs). The ICU4X call lives here, not in `ep9_builtins`: only
//! `ep9_intl` depends on `icu_normalizer` (dep-guards checks 24 and 25; INTL-W4-NORMALIZE).
//!
//! Stub (INTL W4 DTF-OPT skeleton): empty. INTL W4 NORMALIZE writes the body (NFC, NFD, NFKC
//! and NFKD over UTF-16 code units; lone surrogates pass through).
