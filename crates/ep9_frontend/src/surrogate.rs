//! The lone-surrogate shim (docs/plan/07-runtime.md §7; ADR H05 A4 and H05-Q3): eval and
//! `Function()` source text that holds a lone surrogate.
//!
//! A JS string can hold a lone surrogate. oxc and the compiler hooks take `&str`, which cannot.
//! So the VM gives the hooks `compile_eval` and `compile_function` the **shim text** of the
//! string, and [`compile_eval`](crate::compile_eval) decodes it here:
//!
//! ```text
//! JS string (WTF-16) --VM source_text, = eval_source_text--> shim text (&str)
//!   a lone surrogate u            -> ESC + 4 lower-case hex digits of u ("\u{FDD0}d800")
//!   U+FDD0 (ESC) itself           -> ESC ESC
//!   any other code point          -> itself (most text: no change, no copy)
//! shim text --decode--> parse text: each lone surrogate is U+FFFD (3 UTF-8 bytes, 1 UTF-16
//!   unit, so every UTF-16 column stays), plus the list of (byte offset, code unit)
//! parse text --oxc_parser--> AST; U+FFFD outside comments and literals is a SyntaxError,
//!   as the lone surrogate is (07 §7 steps 1-3)
//!   regex literals:  the regex check again on the pattern with each lone surrogate written
//!                    as an escape; its result replaces the U+FFFD result (step 5)
//!   string literals, template cooked values: cooked again from the text with each lone
//!                    surrogate written as \uXXXX (step 4)
//!   template raw values, regex patterns (a `Str`, UTF-8 only): a placeholder string; after
//!                    BytecodeGen the unit string of each placeholder becomes the real code
//!                    units, and the unit is verified again
//! ```
//!
//! # Decisions
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | S1 | The shim text form is in-band (ESC = U+FDD0, a noncharacter; ESC ESC for a real U+FDD0), so the hook type stays `fn(&str, ..)` | The `CompilerHooks` line of the H20 core golden is Frozen; an in-band form needs no new hook and no `ep9_vm` -> `ep9_frontend` edge (C2) |
//! | S2 | Only the eval entry decodes ([`compile_eval`](crate::compile_eval); `compile_function_ctor` reaches it for the joined text). Scripts and modules from a host never hold a lone surrogate, so `compile`, `compile_script` and `compile_module` do not decode | A host text with U+FDD0 stays as it is |
//! | S3 | The decoder is total: an ESC that does not start one of the 2 forms is a real U+FDD0. Two escaped surrogates that make a pair are that pair | Fuzz input and an old VM can give any text; the result is still deterministic |
//! | S4 | The unit keeps the parse text as its source (U+FFFD for each lone surrogate). `Function.prototype.toString` of a function in such eval code gives U+FFFD there | `CompiledUnit::source` is a `String` (H05 A2); the VM side of 07 §7 ("toString reads the original string") is not built |
//! | S5 | The placeholder is ESC2 = U+FDD1 repeated (m + 1) times, then the decimal index, where m is the longest run of U+FDD1 in any string value, template value or regex pattern of the AST | No string of the unit can be equal to a placeholder, so the swap after BytecodeGen changes only the placeholder strings |

use std::borrow::Cow;
use std::fmt::Write as _;

use ep9_bytecode::{UnitString, VerifiedUnit};
use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::{
    Expression, Program, RegExpLiteral, Str, StringLiteral, TaggedTemplateExpression,
    TemplateElement,
};
use oxc_ast_visit::{Visit, VisitMut};
use oxc_diagnostics::{Diagnostics, OxcDiagnostic};
use oxc_parser::{Kind, Parser, Token};
use oxc_regular_expression::{LiteralParser, Options};
use oxc_span::{SourceType, Span};

use crate::bytecodegen::CodegenError;

/// The escape of the shim text (a noncharacter: Unicode keeps it for internal use).
pub(crate) const ESC: char = '\u{FDD0}';

/// The placeholder character (S5).
const MARK: u16 = 0xFDD1;

/// The shim text of the WTF-16 string `units`: the text that the VM gives the compiler hooks
/// `compile_eval` and `compile_function` (H05-Q3). A lone surrogate becomes U+FDD0 and its
/// 4 lower-case hex digits, U+FDD0 becomes U+FDD0 U+FDD0, and every other code point stays.
/// [`compile_eval`](crate::compile_eval) reads it back. A string with no lone surrogate and no
/// U+FDD0 gives its plain text.
#[must_use]
pub fn eval_source_text(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len());
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok(ESC) => {
                out.push(ESC);
                out.push(ESC);
            }
            Ok(c) => out.push(c),
            Err(lone) => {
                out.push(ESC);
                // Writing to a String cannot fail.
                let _ = write!(out, "{:04x}", lone.unpaired_surrogate());
            }
        }
    }
    out
}

/// One lone surrogate of the decoded text: the byte offset of its U+FFFD in the parse text,
/// and its code unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lone {
    pub(crate) at: u32,
    pub(crate) unit: u16,
}

/// The shim state of one eval compile: the lone surrogates of the parse text, then the
/// placeholder swaps that [`Shim::patch`] makes and [`Shim::finish`] applies.
#[derive(Debug, Default)]
pub(crate) struct Shim {
    /// Sorted by `at`.
    lone: Vec<Lone>,
    /// (placeholder text, the real code units).
    swaps: Vec<(String, Vec<u16>)>,
}

/// Decode the shim text `source` (S3). The text is borrowed when it holds no ESC.
pub(crate) fn decode(source: &str) -> (Cow<'_, str>, Shim) {
    if !source.contains(ESC) {
        return (Cow::Borrowed(source), Shim::default());
    }
    // Shim text -> WTF-16 code units.
    let mut units: Vec<u16> = Vec::with_capacity(source.len());
    let mut rest = source;
    while let Some(i) = rest.find(ESC) {
        units.extend(rest[..i].encode_utf16());
        let after = &rest[i + ESC.len_utf8()..];
        if let Some(tail) = after.strip_prefix(ESC) {
            units.push(ESC as u16);
            rest = tail;
        } else if let Some(u) = escaped_surrogate(after) {
            units.push(u);
            rest = &after[4..];
        } else {
            units.push(ESC as u16);
            rest = after;
        }
    }
    units.extend(rest.encode_utf16());
    // WTF-16 -> parse text with U+FFFD for each lone surrogate.
    let mut text = String::with_capacity(source.len());
    let mut lone = Vec::new();
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok(c) => text.push(c),
            Err(e) => {
                let at = u32::try_from(text.len()).unwrap_or(u32::MAX);
                lone.push(Lone {
                    at,
                    unit: e.unpaired_surrogate(),
                });
                text.push(char::REPLACEMENT_CHARACTER);
            }
        }
    }
    (
        Cow::Owned(text),
        Shim {
            lone,
            swaps: Vec::new(),
        },
    )
}

/// The surrogate code unit of 4 lower-case hex digits at the start of `s`.
fn escaped_surrogate(s: &str) -> Option<u16> {
    let digits = s.get(..4)?;
    if !digits
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let u = u16::from_str_radix(digits, 16).ok()?;
    (0xD800..=0xDFFF).contains(&u).then_some(u)
}

impl Shim {
    /// No lone surrogate: every step below does nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.lone.is_empty()
    }

    /// The lone surrogates with `start <= at < end`.
    fn within(&self, start: u32, end: u32) -> &[Lone] {
        let a = self.lone.partition_point(|l| l.at < start);
        let b = self.lone.partition_point(|l| l.at < end);
        &self.lone[a..b]
    }

    /// The code units of `text[start..end]` (byte offsets) with the real lone surrogates.
    fn units(&self, text: &str, start: u32, end: u32) -> Vec<u16> {
        let lone = self.within(start, end);
        let slice = &text[start as usize..end as usize];
        let mut out = Vec::with_capacity(slice.len());
        let mut next = lone.iter().peekable();
        for (i, c) in slice.char_indices() {
            let at = start + i as u32;
            if let Some(l) = next.next_if(|l| l.at == at) {
                out.push(l.unit);
            } else {
                let mut buf = [0u16; 2];
                out.extend_from_slice(c.encode_utf16(&mut buf));
            }
        }
        out
    }

    /// Step 5 of 07 §7 on the oxc result: for each regex literal that holds a lone surrogate,
    /// the regex check error of the U+FFFD text (the one that `regex_check::check` pushed, if
    /// any) goes, and the error of the pattern with each lone surrogate written as an escape
    /// (if there is one) comes instead. The parser's own errors stay. `tokens` is the final
    /// token list of the parse of `text` (as `regex_check::check`).
    pub(crate) fn check_regex_literals(
        &self,
        text: &str,
        tokens: &[Token],
        diagnostics: &mut Diagnostics,
    ) {
        if self.is_empty() {
            return;
        }
        let mut scratch = Allocator::default();
        for token in tokens.iter().filter(|t| t.kind() == Kind::RegExp) {
            let (start, end) = (token.start(), token.end());
            if self.within(start, end).is_empty() {
                continue;
            }
            let literal = &text[start as usize..end as usize];
            let Some(close) = literal.rfind('/').filter(|&i| i > 0) else {
                continue;
            };
            let (pattern_end, flags) = (start + close as u32, &literal[close + 1..]);
            // The error that `regex_check::check_literal` gave for the U+FFFD text: same
            // pattern, flags and span offsets.
            let options = Options {
                pattern_span_offset: start + 1,
                flags_span_offset: pattern_end + 1,
            };
            let fffd = &text[start as usize + 1..pattern_end as usize];
            if let Err(e) = LiteralParser::new(&scratch, fffd, Some(flags), options).parse()
                && let Some(i) = diagnostics.iter().position(|d| *d == e)
            {
                let _dropped = diagnostics.remove(i);
            }
            let unicode = flags.contains('u') || flags.contains('v');
            let escaped = self.escaped_pattern(text, start + 1, pattern_end, unicode);
            if let Err(e) =
                LiteralParser::new(&scratch, &escaped, Some(flags), Options::default()).parse()
            {
                let message = e.message.to_string();
                diagnostics.push(OxcDiagnostic::error(message).with_label(Span::new(start, end)));
            }
            scratch.reset();
        }
    }

    /// The pattern `text[start..end]` with each lone surrogate written as an escape:
    /// `\u{XXXX}` in Unicode mode (a braced escape never pairs with the next one) and in a
    /// group name (its escapes are always Unicode mode); `\uXXXX` else. After a `\` the lone
    /// surrogate is an identity escape: outside Unicode mode the `\` goes and the escape is
    /// written as above; in Unicode mode the U+FFFD stays, so the escape stays an error.
    fn escaped_pattern(&self, text: &str, start: u32, end: u32, unicode: bool) -> String {
        let slice = &text[start as usize..end as usize];
        let mut out = String::with_capacity(slice.len() + 16);
        let (mut in_class, mut in_name, mut escaped) = (false, false, false);
        let mut chars = slice.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            let at = start + i as u32;
            let lone = self.within(at, at + 1).first().map(|l| l.unit);
            if escaped {
                escaped = false;
                if let Some(u) = lone {
                    if unicode {
                        out.push('\\');
                        out.push(c);
                    } else if in_name {
                        let _ = write!(out, "\\u{{{u:04X}}}");
                    } else {
                        let _ = write!(out, "\\u{u:04X}");
                    }
                    continue;
                }
                out.push('\\');
                out.push(c);
                if c == 'k' && !in_class && chars.peek().map(|&(_, n)| n) == Some('<') {
                    in_name = true;
                }
                continue;
            }
            if let Some(u) = lone {
                if unicode || in_name {
                    let _ = write!(out, "\\u{{{u:04X}}}");
                } else {
                    let _ = write!(out, "\\u{u:04X}");
                }
                continue;
            }
            match c {
                '\\' => {
                    escaped = true;
                    continue;
                }
                ']' if in_class => in_class = false,
                '[' if !in_name => in_class = true,
                '>' if in_name => in_name = false,
                '(' if !in_class && !in_name => {
                    let rest = &slice[i + 1..];
                    if rest.starts_with("?<")
                        && !rest.starts_with("?<=")
                        && !rest.starts_with("?<!")
                    {
                        in_name = true;
                    }
                }
                _ => {}
            }
            out.push(c);
        }
        if escaped {
            out.push('\\');
        }
        out
    }

    /// Steps 4 and 5 of 07 §7 on the AST (after the parse checks, before the walks of
    /// oxc_semantic and the scope pass): string literal values and template cooked values get
    /// the real code units; template raw values and regex patterns get a placeholder (S5).
    ///
    /// # Errors
    ///
    /// [`CodegenError::Internal`] if a literal does not parse again (a shim bug).
    pub(crate) fn patch<'a>(
        &mut self,
        allocator: &'a Allocator,
        text: &str,
        program: &mut Program<'a>,
    ) -> Result<(), CodegenError> {
        if self.is_empty() {
            return Ok(());
        }
        let mut longest = Longest::default();
        longest.visit_program(program);
        let mut patcher = Patcher {
            shim: self,
            allocator,
            text,
            mark: String::from_utf16_lossy(&vec![MARK; longest.run + 1]),
            error: None,
        };
        patcher.visit_program(program);
        match patcher.error {
            Some(e) => Err(CodegenError::Internal(e)),
            None => Ok(()),
        }
    }

    /// After BytecodeGen: each placeholder string of the unit becomes its real code units, and
    /// the unit is verified again. A unit with no placeholder is returned as it is.
    ///
    /// # Errors
    ///
    /// [`CodegenError::Internal`] if the changed unit does not verify (a shim bug).
    pub(crate) fn finish(&self, unit: VerifiedUnit) -> Result<VerifiedUnit, CodegenError> {
        if self.swaps.is_empty() {
            return Ok(unit);
        }
        let mut unit = unit.into_inner();
        for s in &mut unit.strings {
            if let Some((_, real)) = self.swaps.iter().find(|(p, _)| *s == UnitString::new(p)) {
                *s = UnitString::from_utf16(real);
            }
        }
        ep9_bytecode::verify(unit)
            .map_err(|e| CodegenError::Internal(format!("lone-surrogate shim: {e:?}")))
    }

    /// The literal `text[start..end]` (a string literal with its quotes, or the raw text of a
    /// template element) with each lone surrogate written as `\uXXXX`; a `\` right before a
    /// lone surrogate goes (`\` + a lone surrogate is that code unit, NonEscapeCharacter).
    fn escaped_literal(&self, text: &str, start: u32, end: u32) -> String {
        let slice = &text[start as usize..end as usize];
        let mut out = String::with_capacity(slice.len() + 16);
        let mut backslashes = 0usize;
        for (i, c) in slice.char_indices() {
            let at = start + i as u32;
            if let Some(l) = self.within(at, at + 1).first() {
                if backslashes % 2 == 1 {
                    out.pop();
                }
                let _ = write!(out, "\\u{:04X}", l.unit);
                backslashes = 0;
                continue;
            }
            backslashes = if c == '\\' { backslashes + 1 } else { 0 };
            out.push(c);
        }
        out
    }
}

/// The longest run of U+FDD1 in any string value, template value or regex pattern (S5).
#[derive(Default)]
struct Longest {
    run: usize,
}

impl Longest {
    fn see(&mut self, units: impl Iterator<Item = u16>) {
        let mut run = 0;
        for u in units {
            run = if u == MARK { run + 1 } else { 0 };
            self.run = self.run.max(run);
        }
    }
}

impl<'a> Visit<'a> for Longest {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.see(it.value.encode_utf16());
    }

    fn visit_template_element(&mut self, it: &TemplateElement<'a>) {
        self.see(it.value.raw.as_str().encode_utf16());
        if let Some(c) = it.value.cooked {
            self.see(c.encode_utf16());
        }
    }

    fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'a>) {
        self.see(it.regex.pattern.text.as_str().encode_utf16());
    }
}

struct Patcher<'s, 'a, 't> {
    shim: &'s mut Shim,
    allocator: &'a Allocator,
    text: &'t str,
    mark: String,
    error: Option<String>,
}

impl<'a> Patcher<'_, 'a, '_> {
    fn has_lone(&self, span: Span) -> bool {
        !self.shim.within(span.start, span.end).is_empty()
    }

    /// A new placeholder for the real code units `real`.
    fn placeholder(&mut self, real: Vec<u16>) -> Str<'a> {
        let p = format!("{}{}", self.mark, self.shim.swaps.len());
        let s = Str::from(self.allocator.alloc_str(&p));
        self.shim.swaps.push((p, real));
        s
    }

    /// Patch a template element: its cooked value, and for a tagged template its raw value.
    fn template_element(&mut self, it: &mut TemplateElement<'a>, tagged: bool) {
        if !self.has_lone(it.span) {
            return;
        }
        if it.value.cooked.is_some() {
            let raw = self
                .shim
                .escaped_literal(self.text, it.span.start, it.span.end);
            it.value.cooked = self.reparse(&format!("`{raw}`"), |e, a| match e {
                Expression::TemplateLiteral(t) if t.quasis.len() == 1 => {
                    t.quasis[0].value.cooked.map(|c| c.clone_in(a))
                }
                _ => None,
            });
        }
        if !tagged {
            return;
        }
        // The TRV: CR LF and CR become LF (as `literal::template_raw`).
        let units = self.shim.units(self.text, it.span.start, it.span.end);
        let mut real = Vec::with_capacity(units.len());
        let mut iter = units.iter().copied().peekable();
        while let Some(u) = iter.next() {
            if u == u16::from(b'\r') {
                iter.next_if_eq(&u16::from(b'\n'));
                real.push(u16::from(b'\n'));
            } else {
                real.push(u);
            }
        }
        it.value.raw = self.placeholder(real);
    }

    /// Parse `source` again (one string literal or one template literal) and give what `take`
    /// reads from it, copied into the program's arena.
    fn reparse<R>(
        &mut self,
        source: &str,
        take: impl for<'b> FnOnce(&Expression<'b>, &'a Allocator) -> Option<R>,
    ) -> Option<R> {
        let scratch = Allocator::default();
        let parsed = Parser::new(&scratch, source, SourceType::script()).parse_expression();
        let value = parsed.ok().and_then(|e| take(&e, self.allocator));
        if value.is_none() {
            self.error
                .get_or_insert_with(|| format!("lone-surrogate shim: no value for {source:?}"));
        }
        value
    }
}

impl<'a> VisitMut<'a> for Patcher<'_, 'a, '_> {
    fn visit_string_literal(&mut self, it: &mut StringLiteral<'a>) {
        if !self.has_lone(it.span) {
            return;
        }
        let source = self
            .shim
            .escaped_literal(self.text, it.span.start, it.span.end);
        let value = self.reparse(&source, |e, a| match e {
            Expression::StringLiteral(s) => Some(s.value.clone_in(a)),
            _ => None,
        });
        if let Some(v) = value {
            it.value = v;
        }
    }

    /// An untagged template: only the cooked values are read (BytecodeGen `template`).
    fn visit_template_element(&mut self, it: &mut TemplateElement<'a>) {
        self.template_element(it, false);
    }

    /// A tagged template: the raw values are read too (the template object).
    fn visit_tagged_template_expression(&mut self, it: &mut TaggedTemplateExpression<'a>) {
        self.visit_expression(&mut it.tag);
        for q in &mut it.quasi.quasis {
            self.template_element(q, true);
        }
        for e in &mut it.quasi.expressions {
            self.visit_expression(e);
        }
    }

    fn visit_reg_exp_literal(&mut self, it: &mut RegExpLiteral<'a>) {
        if !self.has_lone(it.span) {
            return;
        }
        let literal = &self.text[it.span.start as usize..it.span.end as usize];
        let Some(close) = literal.rfind('/').filter(|&i| i > 0) else {
            return;
        };
        let real = self
            .shim
            .units(self.text, it.span.start + 1, it.span.start + close as u32);
        it.regex.pattern.text = self.placeholder(real);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parse text and the lone list that `units` must decode to.
    fn expected(units: &[u16]) -> (String, Vec<Lone>) {
        let mut text = String::new();
        let mut lone = Vec::new();
        for c in char::decode_utf16(units.iter().copied()) {
            match c {
                Ok(c) => text.push(c),
                Err(e) => {
                    lone.push(Lone {
                        at: text.len() as u32,
                        unit: e.unpaired_surrogate(),
                    });
                    text.push(char::REPLACEMENT_CHARACTER);
                }
            }
        }
        (text, lone)
    }

    /// decode(eval_source_text(x)) is x for WTF-16 strings made of the hard parts: lone leads
    /// and trails, pairs, U+FDD0, hex digits and ASCII (a fixed xorshift sequence).
    #[test]
    fn shim_text_round_trips() {
        const PARTS: [u16; 12] = [
            0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xD83D, 0xDE00, 0xFDD0, 0xFFFD, 0x0064, 0x0038, 0x0030,
            0x0061,
        ];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for len in 0..400usize {
            let mut units = Vec::with_capacity(len % 23);
            for _ in 0..len % 23 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                units.push(PARTS[(x % PARTS.len() as u64) as usize]);
            }
            let shim = eval_source_text(&units);
            let (text, state) = decode(&shim);
            let (want_text, want_lone) = expected(&units);
            assert_eq!(text, want_text, "{units:04x?}");
            assert_eq!(state.lone, want_lone, "{units:04x?}");
        }
    }

    /// Text with no lone surrogate and no U+FDD0 is its own shim text and is not copied.
    #[test]
    fn plain_text_is_borrowed() {
        let src = "var a = '\u{1F600}\u{FFFD}'; // é";
        let units: Vec<u16> = src.encode_utf16().collect();
        assert_eq!(eval_source_text(&units), src);
        let (text, state) = decode(src);
        assert!(matches!(text, Cow::Borrowed(_)));
        assert!(state.is_empty());
    }

    /// S3: an ESC that starts neither form is a real U+FDD0; upper-case digits and non-surrogate
    /// values are not an escaped surrogate; two escaped surrogates that pair are a pair.
    #[test]
    fn the_decoder_is_total() {
        for (src, want) in [
            ("\u{FDD0}", "\u{FDD0}"),
            ("a\u{FDD0}", "a\u{FDD0}"),
            ("\u{FDD0}D800", "\u{FDD0}D800"),
            ("\u{FDD0}0041", "\u{FDD0}0041"),
            ("\u{FDD0}d80", "\u{FDD0}d80"),
            ("\u{FDD0}\u{FDD0}d800", "\u{FDD0}d800"),
            ("\u{FDD0}d83d\u{FDD0}de00", "\u{1F600}"),
        ] {
            let (text, state) = decode(src);
            assert_eq!(text, want, "{src:?}");
            assert!(state.is_empty(), "{src:?}");
        }
        let (text, state) = decode("x\u{FDD0}dc00\u{FDD0}");
        assert_eq!(text, "x\u{FFFD}\u{FDD0}");
        assert_eq!(
            state.lone,
            [Lone {
                at: 1,
                unit: 0xDC00
            }]
        );
    }

    /// The validation text of a regex pattern (07 §7 step 5).
    #[test]
    fn escaped_patterns() {
        let pattern = |parts: &[(&str, Option<u16>)], unicode: bool| {
            let mut units = Vec::new();
            for (t, u) in parts {
                units.extend(t.encode_utf16());
                units.extend(u.iter().copied());
            }
            let shim = eval_source_text(&units);
            let (text, state) = decode(&shim);
            state.escaped_pattern(&text, 0, text.len() as u32, unicode)
        };
        assert_eq!(
            pattern(&[("[", Some(0xDBFF)), ("-a]", None)], false),
            "[\\uDBFF-a]"
        );
        assert_eq!(
            pattern(&[("[", Some(0xDBFF)), ("-a]", None)], true),
            "[\\u{DBFF}-a]"
        );
        assert_eq!(
            pattern(&[("(?<a", Some(0xD801)), (">.)", None)], false),
            "(?<a\\u{D801}>.)"
        );
        assert_eq!(
            pattern(&[("(?<=a", Some(0xD801)), (")", None)], false),
            "(?<=a\\uD801)"
        );
        assert_eq!(
            pattern(&[("\\k<a", Some(0xD801)), (">", None)], false),
            "\\k<a\\u{D801}>"
        );
        assert_eq!(
            pattern(&[("[(?<", Some(0xD801)), ("]", None)], false),
            "[(?<\\uD801]"
        );
        assert_eq!(pattern(&[("\\", Some(0xD800))], false), "\\uD800");
        assert_eq!(pattern(&[("\\", Some(0xD800))], true), "\\\u{FFFD}");
        assert_eq!(pattern(&[("\\\\", Some(0xD800))], false), "\\\\\\uD800");
    }

    /// String and template text: `\` + a lone surrogate is the code unit, `\\` + one is a
    /// backslash and the code unit.
    #[test]
    fn escaped_literals() {
        let lit = |parts: &[(&str, Option<u16>)]| {
            let mut units = Vec::new();
            for (t, u) in parts {
                units.extend(t.encode_utf16());
                units.extend(u.iter().copied());
            }
            let shim = eval_source_text(&units);
            let (text, state) = decode(&shim);
            state.escaped_literal(&text, 0, text.len() as u32)
        };
        assert_eq!(lit(&[("'a", Some(0xD800)), ("'", None)]), "'a\\uD800'");
        assert_eq!(lit(&[("'\\", Some(0xD800)), ("'", None)]), "'\\uD800'");
        assert_eq!(
            lit(&[("'\\\\", Some(0xD800)), ("'", None)]),
            "'\\\\\\uD800'"
        );
        assert_eq!(
            lit(&[("'\\\\\\", Some(0xDC00)), ("'", None)]),
            "'\\\\\\uDC00'"
        );
    }
}
