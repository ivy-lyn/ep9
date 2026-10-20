//! Pre-scan unit tests. Each expected value comes from the rules in the module doc:
//! estimate = sum over open frames of (1 + pending chain ops); the root frame counts 1.

use super::*;

fn est(src: &str) -> u32 {
    estimate(src.as_bytes(), false).depth
}

fn est_module(src: &str) -> u32 {
    estimate(src.as_bytes(), true).depth
}

fn regexes(src: &str) -> usize {
    literal_tokens(src.as_bytes(), false)
        .iter()
        .filter(|(k, _)| *k == LitKind::Regex)
        .count()
}

#[test]
fn empty_and_flat() {
    assert_eq!(est(""), 1);
    assert_eq!(est("a; b; c;"), 1);
}

#[test]
fn every_bracket_kind_is_a_frame() {
    assert_eq!(est("[[[0]]]"), 4);
    assert_eq!(est("((0))"), 3);
    assert_eq!(est("{ { } }"), 3);
    assert_eq!(est("f(g(h()))"), 4);
    assert_eq!(est("`${`${`${0}`}`}`"), 4);
    // Closed frames do not add up.
    assert_eq!(est("[0]; [0]; [0];"), 2);
}

#[test]
fn chain_ops_count_until_their_expression_ends() {
    assert_eq!(est("!!!x"), 4);
    assert_eq!(est("a = b = c"), 3);
    assert_eq!(est("typeof void delete x"), 4);
    assert_eq!(est("a ** b ** c"), 3);
    assert_eq!(est("x => y => z"), 3);
    // `,` and `;` end every assignment expression of the frame.
    assert_eq!(est("!a, !b, !c"), 2);
    assert_eq!(est("!a; !b; !c;"), 2);
    // Inside a frame the ops of the outer frame stay pending.
    assert_eq!(est("!(!(!x))"), 6);
}

#[test]
fn asi_resets_like_a_semicolon() {
    // A newline before a token that cannot continue the expression: ASI.
    assert_eq!(est("!a\n!b\n!c"), 2);
    // U+2028 is a line terminator too.
    assert_eq!(est("!a\u{2028}!b"), 2);
    // A binary operator continues the expression: no ASI, the op stays pending.
    assert_eq!(est("x = !a\n+ !b"), 4);
}

#[test]
fn statement_ops() {
    // At the third `(`: 3 pending `if` + the paren frame + the root.
    assert_eq!(est("if (a) if (b) if (c) x;"), 5);
    assert_eq!(est("a: b: c: x;"), 4);
    assert_eq!(est("while (a) while (b) x;"), 4);
    // A do-while tail is not a new loop: at `(a)` 2 pending `do` + the paren frame.
    assert_eq!(est("do do x; while (a); while (b);"), 4);
    assert_eq!(est("do do do x; while (a); while (b); while (c);"), 5);
    // `;` keeps an `if` pending when `else` follows: at `(b)` 2 `if` + the paren frame.
    assert_eq!(est("if (a) x; else if (b) y; else z;"), 4);
}

#[test]
fn a_statement_level_close_brace_ends_the_statement() {
    // Each `if` ends with its block, so the estimate does not grow.
    assert_eq!(est("if (a) {} if (b) {} if (c) {}"), 3);
    assert_eq!(est("function f() {} function g() {} function h() {}"), 2);
    assert_eq!(est("class A {} class B {} class C {}"), 2);
}

#[test]
fn regex_or_division_from_the_previous_token() {
    assert_eq!(regexes("a / b / c"), 0);
    assert_eq!(regexes("x = /a/g"), 1);
    assert_eq!(regexes("(x) / 2"), 0);
    assert_eq!(regexes("if (x) /a/.test(y)"), 1);
    assert_eq!(regexes("return /a/"), 1);
    assert_eq!(regexes("x = {} / 2"), 0);
    assert_eq!(regexes("{} /a/"), 1);
    assert_eq!(regexes("a++ / 2"), 0);
    assert_eq!(regexes("x = y\n/a/g.exec(z)"), 0);
    // Not a generator: `yield` is an identifier, so `/` divides.
    assert_eq!(regexes("function f() { yield / 1 / 2 }"), 0);
    assert_eq!(regexes("function* g() { yield /1/ }"), 1);
}

#[test]
fn regex_groups_and_classes_weigh_more() {
    // Root frame + 3 groups x REGEX_WEIGHT.
    assert_eq!(est("x = /(((a)))/"), 1 + 1 + 3 * REGEX_WEIGHT);
    assert_eq!(est("x = /[a]/v"), 1 + 1 + REGEX_WEIGHT);
    // A division with parentheses is not a regex.
    assert_eq!(est("x = a / ((b))"), 1 + 1 + 2);
}

#[test]
fn v_flag_classes_stay_open_after_a_nested_class() {
    // With `v`, the outer class stays open after `[a]`: 3 levels at the innermost `[a]`.
    assert_eq!(est("x = /[[a]&&[[a]&&[a]]]/v"), 1 + 1 + 3 * REGEX_WEIGHT);
    assert_eq!(est("x = /[[a]--[[a]--[a]]]/gv"), 1 + 1 + 3 * REGEX_WEIGHT);
    assert_eq!(
        est("x = /[\\q{a|bc}[\\q{d}[e]]]/v"),
        1 + 1 + 3 * REGEX_WEIGHT
    );
    assert_eq!(est("x = /[[[[a]]]]/v"), 1 + 1 + 4 * REGEX_WEIGHT);
    // Closed classes do not add up; a group holds its classes.
    assert_eq!(est("x = /[[a]][[b]]/v"), 1 + 1 + 2 * REGEX_WEIGHT);
    assert_eq!(est("x = /([[a]])([b])/v"), 1 + 1 + 3 * REGEX_WEIGHT);
    // Escaped brackets are characters.
    assert_eq!(est("x = /[\\[\\[a\\]]/v"), 1 + 1 + REGEX_WEIGHT);
    // Without `v`, classes do not nest: `[` in a class is a character, `]` outside one too.
    assert_eq!(est("x = /[[a]&&[[a]&&[a]]]/"), 1 + 1 + REGEX_WEIGHT);
    assert_eq!(est("x = /[[[[a]/u"), 1 + 1 + REGEX_WEIGHT);
    assert_eq!(est("x = /]]]((a))/"), 1 + 1 + 2 * REGEX_WEIGHT);
}

#[test]
fn keywords_in_key_position_are_names() {
    assert_eq!(est("({ if: 1, for: 2, while: 3 })"), 3);
    assert_eq!(est("a.if.for.while"), 1);
    assert_eq!(est("class A { static if() {} }"), 3);
}

#[test]
fn strings_and_comments_hide_brackets() {
    // Binary `+` is not a chain op.
    assert_eq!(est("'(((((' + \"[[[[[\""), 1);
    assert_eq!(est("// (((((\n/* [[[[[ */ x"), 1);
    assert_eq!(est("`((((( ${0} [[[[[`"), 2);
}

#[test]
fn html_comments_only_in_scripts() {
    assert_eq!(est("x = 1 <!-- (((((("), 2);
    assert!(est_module("x = 1 <!-- ((((((") > 7);
    assert_eq!(est("--> ((((((\nx"), 1);
}

#[test]
fn await_is_an_operator_only_where_it_can_be() {
    // Script, not async: an identifier.
    assert_eq!(est("await + await + x"), 1);
    // Module top level: an operator.
    assert_eq!(est_module("await await x"), 3);
    assert_eq!(est("async function f() { await await x }"), 4);
}

#[test]
fn the_position_of_the_deepest_token() {
    let e = estimate(b"a;\n[[[0]]]", false);
    assert_eq!(e.depth, 4);
    // First reached at the third `[`.
    assert_eq!(e.at, 5);
}
