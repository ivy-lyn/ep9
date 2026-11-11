//! Case mapping (§22.1.3.26-28, §22.1.3.30): toLowerCase, toUpperCase and their toLocale
//! forms. The toLocale forms are the ECMA-402 ones (§20.1.2-3): TransformCase in
//! `ep9_intl::list_format::casemap` picks the locale; for tr, az, lt and el it maps with
//! icu_casemap, else ("und") with the Default Case Conversion below, as toLowerCase and
//! toUpperCase do.
//!
//! The mapping is the Unicode Default Case Conversion of Rust std: `str::to_lowercase` and
//! `str::to_uppercase` use the full mappings (UnicodeData plus the unconditional
//! SpecialCasing rows, so 'ß' upper-cases to "SS"), and `str::to_lowercase` applies the one
//! language-independent conditional rule, Final_Sigma (Unicode §3.13, Table 3-17), with the
//! Cased and Case_Ignorable properties. std's tables are at `char::UNICODE_VERSION`, Unicode 17
//! in rustc 1.99.1: the H26 version (docs/adr/H26-toolchain-and-unicode.md). No new
//! dependency.
//!
//! ```text
//! code units ──> runs of valid code points, split at each lone surrogate
//!   run ──> String ──> str::to_lowercase / to_uppercase ──> UTF-16
//!   lone surrogate ──> copied as it is (the spec maps code points; a lone surrogate has no
//!                      case mapping, §6.1.4)
//! ```
//!
//! Splitting at a lone surrogate keeps Final_Sigma exact: a surrogate code point is neither
//! Cased nor Case_Ignorable (General_Category Cs), so the context of a Σ never reaches past
//! one, and std sees the end of a run where the context stops. An ASCII string maps with no
//! copy through `String`.

use ep9_intl::list_format::casemap::{self, TargetCase};
use ep9_vm::{Args, Cx, JsResult, Local};

use super::{new_str, this_string, units};

/// Which way to map.
#[derive(Clone, Copy)]
enum Case {
    Lower,
    Upper,
}

/// The case mapping of the code units `s`.
fn map_case(s: &[u16], case: Case) -> Vec<u16> {
    if s.iter().all(|&u| u < 0x80) {
        return s
            .iter()
            .map(|&u| {
                let b = u as u8;
                u16::from(match case {
                    Case::Lower => b.to_ascii_lowercase(),
                    Case::Upper => b.to_ascii_uppercase(),
                })
            })
            .collect();
    }
    let mut out = Vec::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut Vec<u16>| {
        if run.is_empty() {
            return;
        }
        let mapped = match case {
            Case::Lower => run.to_lowercase(),
            Case::Upper => run.to_uppercase(),
        };
        out.extend(mapped.encode_utf16());
        run.clear();
    };
    for c in char::decode_utf16(s.iter().copied()) {
        match c {
            Ok(c) => run.push(c),
            Err(e) => {
                flush(&mut run, &mut out);
                out.push(e.unpaired_surrogate());
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Steps 1-2 and the mapping of the four methods.
fn convert<'s>(cx: &mut Cx<'s>, this: Local<'s>, case: Case, name: &str) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, name)?;
    // 3. Let sText be StringToCodePoints(S).
    // 4. Let lowerText be toLowercase(sText) (toUppercase(sText)) according to the Unicode
    //    Default Case Conversion algorithm.
    // 5. Let L be CodePointsToString(lowerText).
    // 6. Return L.
    let s = units(cx, s)?;
    new_str(cx, &map_case(&s, case))
}

/// The ECMA-402 toLocaleLowerCase and toLocaleUpperCase (§20.1.2-3).
fn convert_locale<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    locales: Local<'s>,
    case: Case,
    name: &str,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, name)?;
    let s = units(cx, s)?;
    // 3. Return ? TransformCase(S, locales, lower / upper).
    let target = match case {
        Case::Lower => TargetCase::Lower,
        Case::Upper => TargetCase::Upper,
    };
    match casemap::transform_case(cx, &s, locales, target)? {
        Some(mapped) => new_str(cx, &mapped),
        // The locale "und": the Default Case Conversion.
        None => new_str(cx, &map_case(&s, case)),
    }
}

/// String.prototype.toLocaleLowerCase ( [ locales ] ) (ECMA-402 §20.1.2).
pub(super) fn to_locale_lower_case<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    convert_locale(cx, this, args.get(0), Case::Lower, "toLocaleLowerCase")
}

/// String.prototype.toLocaleUpperCase ( [ locales ] ) (ECMA-402 §20.1.3).
pub(super) fn to_locale_upper_case<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    convert_locale(cx, this, args.get(0), Case::Upper, "toLocaleUpperCase")
}

/// String.prototype.toLowerCase ( ) (§22.1.3.28).
pub(super) fn to_lower_case<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    convert(cx, this, Case::Lower, "toLowerCase")
}

/// String.prototype.toUpperCase ( ) (§22.1.3.30). As toLowerCase, with toUppercase.
pub(super) fn to_upper_case<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    convert(cx, this, Case::Upper, "toUpperCase")
}
