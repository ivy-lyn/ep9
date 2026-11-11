// Copyright 2020 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-error.h at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The parse errors (V8 `regexp-error.h`) and the message text.
//!
//! The list keeps V8's order and text. Not ported: `None` (no error is `Ok`), `NotLinear` (the
//! `l` flag) and `UnsupportedBytecode` (no tier yet). `AnalysisStackOverflow` and `TooLarge`
//! come from the compiler (RX1-1); the parser reports `StackOverflow` at the nesting cap
//! ([`crate::MAX_NESTING`]).

macro_rules! errors {
    ($($name:ident => $text:literal,)*) => {
        /// A pattern error, as V8 names it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Error {
            $(
                #[doc = $text]
                $name,
            )*
        }

        impl Error {
            /// Every error, in V8's order.
            pub const ALL: &'static [Error] = &[$(Error::$name,)*];

            /// V8's message text.
            pub const fn text(self) -> &'static str {
                match self {
                    $(Error::$name => $text,)*
                }
            }
        }
    };
}

errors! {
    StackOverflow => "Maximum call stack size exceeded",
    AnalysisStackOverflow => "Stack overflow",
    TooLarge => "Regular expression too large",
    UnterminatedGroup => "Unterminated group",
    UnmatchedParen => "Unmatched ')'",
    EscapeAtEndOfPattern => "\\ at end of pattern",
    InvalidPropertyName => "Invalid property name",
    InvalidEscape => "Invalid escape",
    InvalidDecimalEscape => "Invalid decimal escape",
    InvalidUnicodeEscape => "Invalid Unicode escape",
    NothingToRepeat => "Nothing to repeat",
    LoneQuantifierBrackets => "Lone quantifier brackets",
    RangeOutOfOrder => "numbers out of order in {} quantifier",
    IncompleteQuantifier => "Incomplete quantifier",
    InvalidQuantifier => "Invalid quantifier",
    InvalidGroup => "Invalid group",
    MultipleFlagDashes => "Multiple dashes in flag group",
    RepeatedFlag => "Repeated flag in flag group",
    InvalidFlagGroup => "Invalid flag group",
    TooManyCaptures => "Too many captures",
    InvalidCaptureGroupName => "Invalid capture group name",
    DuplicateCaptureGroupName => "Duplicate capture group name",
    InvalidNamedReference => "Invalid named reference",
    InvalidNamedCaptureReference => "Invalid named capture referenced",
    InvalidClassPropertyName => "Invalid property name in character class",
    InvalidCharacterClass => "Invalid character class",
    UnterminatedCharacterClass => "Unterminated character class",
    OutOfOrderCharacterClass => "Range out of order in character class",
    InvalidClassSetOperation => "Invalid set operation in character class",
    InvalidCharacterInClass => "Invalid character in character class",
    NegatedCharacterClassWithStrings => "Negated character class may contain strings",
}

impl Error {
    /// True for the errors that V8 throws as a RangeError (stack overflow, too large; V8
    /// `ErrorIsStackOverflow`), not a SyntaxError.
    pub const fn is_stack_overflow(self) -> bool {
        matches!(
            self,
            Error::StackOverflow | Error::AnalysisStackOverflow | Error::TooLarge
        )
    }
}

/// A pattern that does not parse: the error and the code unit index where the parser found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// What is wrong.
    pub error: Error,
    /// The code unit index (V8 `error_pos`).
    pub pos: usize,
}

impl ParseError {
    /// V8's message form: `Invalid regular expression: /<source>/<flags>: <text>`.
    pub fn message(&self, pattern: &[u16], flags: crate::Flags) -> String {
        format!(
            "Invalid regular expression: /{}/{}: {}",
            String::from_utf16_lossy(pattern),
            flags.to_letters(),
            self.error.text()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_has_v8s_31_parser_errors_with_unique_text() {
        // A value added to the macro list without a test change fails here; a value removed
        // from the list fails to build wherever it is named.
        assert_eq!(Error::ALL.len(), 31);
        let mut texts: Vec<&str> = Error::ALL.iter().map(|e| e.text()).collect();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), Error::ALL.len());
        assert_eq!(Error::NothingToRepeat.text(), "Nothing to repeat");
    }

    #[test]
    fn message_has_v8s_form() {
        let p: Vec<u16> = "a**".encode_utf16().collect();
        let e = ParseError {
            error: Error::NothingToRepeat,
            pos: 2,
        };
        assert_eq!(
            e.message(&p, crate::Flags::UNICODE),
            "Invalid regular expression: /a**/u: Nothing to repeat"
        );
        assert!(Error::StackOverflow.is_stack_overflow());
        assert!(!Error::InvalidGroup.is_stack_overflow());
    }
}
