//! RX1: the Rust port of V8's irregexp (docs/plan/04-execution.md section 3, "RX1").
//!
//! ```text
//!   step 1 (M7-P4)                    step 2 (M7-P5)                    later steps
//!   pattern units + Flags             class items of the Tree           compiler (regexp-
//!     -> parser.rs (regexp-parser.cc)   -> sets.rs: ClassSet,             compiler.cc), bytecode
//!     -> Pattern { Tree (ast.rs,           StringSet (code points,         + peephole skip scans,
//!        regexp-ast.h), captures,          strings, case closure)          interpreter (regexp-
//!        names }                           charset.rs, special_case.rs,    interpreter.cc)
//!                                          unicode_sets.rs (UCD tables)
//! ```
//!
//! Nothing in the engine calls this crate yet: RX0 (`ep9_regexp`, the regress fork) is the
//! RegExp backend of every mode until the RX1 gate (M5 exit). Only `ep9_vm` may depend on it
//! later (.github/scripts/dep-guards.sh check 22, the form of check 14).
//!
//! | File | Ported from (V8 at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0) |
//! |---|---|
//! | `flags.rs` | `src/regexp/regexp-flags.h` (ep9's bit order) |
//! | `error.rs` | `src/regexp/regexp-error.h` |
//! | `ast.rs` | `src/regexp/regexp-ast.h`: the node kinds the parser makes |
//! | `parser.rs` | `src/regexp/regexp-parser.cc`, the `kBuildAST` mode |
//! | `unicode.rs`, `unicode_tables.rs` | not V8: the ICU lookups of the parser, as tables from the Unicode Character Database [`UCD_VERSION`] (`data/gen-tables.py`) |
//! | `charset.rs` | `src/regexp/regexp-ast.h` (`CharacterRange`), the `CharacterRange` functions of `regexp-compiler-tonode.cc`, `regexp-compiler.h` (the standard ranges) |
//! | `special_case.rs` | `src/regexp/special-case.h` (`CaseFolding`), `TextNode::GetCaseIndependentLetters` of `regexp-compiler.cc` |
//! | `sets.rs` | the set part of `regexp-compiler-tonode.cc` (`ClassRanges::ToNodeImpl`, `ComputeExpression`) and of `regexp-compiler.cc` (`MakeCaseIndependent`); the ICU part of `regexp-parser.cc` (`AddPropertyClassRange`) |
//! | `unicode_sets.rs` | not V8: the ICU sets and V8's generated `special-case.cc`, as tables from the same UCD (`data/gen-sets.py`, a port of `gen-regexp-special-case.cc`) |
//!
//! Known gaps of the parser port, each by name (none changes which patterns parse, except the
//! nesting cap and the last four, which follow ECMA-262 where V8 follows ICU or its own
//! parse):
//!
//! - The `kVerifySyntax` mode is not ported (one mode).
//! - V8's `--js-regexp-buffer-boundaries` escapes (`\A`, `\z`, `\Z`) and the debug-only
//!   possessive quantifier are not ported: both flags are off in V8 by default.
//! - Class escapes, `.`, and `\p{..}` stay names in the tree ([`ClassItem`]); the code point
//!   ranges and the case closure of `u`+`i` and `v`+`i` (V8 `AddClassEscape`,
//!   `AddUnicodeCaseEquivalents`, `u_foldCase` in `\q{}`) come from [`class_ranges`] and
//!   [`class_set`] (step 2), at the points where V8's parser adds them. Tree shape only.
//! - V8 checks the stack limit in `Advance`; here [`MAX_NESTING`] (256) caps the nesting of
//!   groups and `v`-mode classes ([`Error::StackOverflow`]). This changes the parse result:
//!   RX1 refuses 257 nested groups, and node v26.13.0 (V8) parses 10,000 (it refuses 50,000).
//!   No error kind is mapped to it yet. The cap stays a named gap (route RX1-DUPNAME-PERF,
//!   M7-P5): the class-set walk ([`class_set`]), the named-reference fill and the drop of
//!   the tree recurse once per level, so a higher cap needs those walks without recursion
//!   first.
//! - The empty complement of V8's `CharacterRange::Negate` at U+10FFFF (V8 drops U+10FFFF
//!   from the complement of a list that ends at U+10FFFE) is not ported: RX1 keeps it, as
//!   ECMA-262 CharacterComplement does (`charset.rs`).
//! - Property names: V8 also takes ICU's short names that ECMA-262 table 68 does not list
//!   (`\p{WSpace}`); RX1 does not.
//! - Property values with an empty set: V8 refuses them (`\p{sc=Hrkt}`, ICU); RX1 takes every
//!   value of PropertyValueAliases.txt, as ECMA-262 says.
//! - A `\u` escape of `>` in a group name: V8 ends the name there (`/(?<a\u003e>x)/`); RX1
//!   ends it only at a source `>` and refuses the escape (ECMA-262 22.2.1 and 22.2.1.1:
//!   U+003E is not an IdentifierPartChar).
//! - `{n,m}` with bounds above 2^31 - 1: V8 caps both before it compares them, so it takes
//!   `/a{2147483649,2147483648}/`; RX1 compares the digits (22.2.1.1: the MV of n must not
//!   be larger than the MV of m), as oxc does.
//!
//! The parse-equality test (tests/parse_equality.rs) parses every RegExp source of test262
//! (built-ins/RegExp, language/literals/regexp) with RX1 and with regress (RX0's engine) and
//! checks the differences against a named list. The set-equality test (tests/set_equality.rs)
//! compares the code point sets of step 2 with what regress matches, on the same sources.
#![forbid(unsafe_code)]

mod ast;
mod charset;
mod error;
mod flags;
mod parser;
mod sets;
mod special_case;
mod unicode;
#[rustfmt::skip]
mod unicode_sets;
mod unicode_tables;

pub use ast::{
    AssertionType, BackReference, Capture, ClassItem, ClassRanges, ClassSetExpression,
    ClassSetOperand, ClassSetOperation, ClassString, ClassStrings, Group, INFINITY, Lookaround,
    LookaroundType, Property, Quantifier, QuantifierType, StandardSet, Tree,
};
pub use charset::{
    CharacterRange, MAX_CODE_POINT, add_case_equivalents, add_class_escape,
    add_unicode_case_equivalents, canonicalize, clamp_to_one_byte, contains, intersect, intersects,
    is_canonical, negate, subtract,
};
pub use error::{Error, ParseError};
pub use flags::Flags;
pub use parser::{MAX_CAPTURES, MAX_NESTING, Pattern, Unit};
pub use sets::{ClassSet, StringSet, class_ranges, class_set, property_members};
pub use special_case::{
    Mode as CaseMode, case_independent_letters, equivalence_key, has_supplementary_equivalents,
    simple_fold,
};
pub use unicode_tables::UCD_VERSION;

/// Parses a pattern of UTF-16 code units (lone surrogates allowed) with `flags` (V8
/// `ParseRegExpFromHeapString`, two-byte form).
///
/// # Errors
///
/// The first error, as V8 reports it, with its code unit index.
pub fn parse(pattern: &[u16], flags: Flags) -> Result<Pattern, ParseError> {
    parser::Parser::new(pattern, flags).parse()
}

/// Parses a Latin-1 pattern (one byte per code unit; V8's one-byte form).
///
/// # Errors
///
/// As [`parse`].
pub fn parse_latin1(pattern: &[u8], flags: Flags) -> Result<Pattern, ParseError> {
    parser::Parser::new(pattern, flags).parse()
}

/// Parses a pattern given as Rust text (its UTF-16 form).
///
/// # Errors
///
/// As [`parse`].
pub fn parse_str(pattern: &str, flags: Flags) -> Result<Pattern, ParseError> {
    let units: Vec<u16> = pattern.encode_utf16().collect();
    parse(&units, flags)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_sets;
