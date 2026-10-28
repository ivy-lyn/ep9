//! The pattern methods (§22.1.3.13, §22.1.3.19-20, §22.1.3.22-23): match, replace,
//! replaceAll, search and split, plus GetSubstitution (§22.1.3.19.1). IsRegExp (§7.2.8) is
//! `crate::regexp::is_reg_exp`.
//!
//! ```text
//! s.split(sep, lim)        sep an Object with a @@split method ──> Call(it, sep, «O, lim»)
//! s.replace(pat, r)        pat an Object with a @@replace method ──> Call(it, pat, «O, r»)
//! s.replaceAll(pat, r)     the same, after the IsRegExp "g" flag check
//! otherwise                ToString(pat): a code-unit search (StringIndexOf); r a function
//!                          ──> Call(r, undefined, «match, position, string»), else
//!                          GetSubstitution with no captures ($$, $&, $`, $' only)
//! s.match(re), s.search(re)  re an Object with a @@match / @@search method ──> Call it;
//!                          otherwise RegExpCreate(re), then Invoke(rx, @@match / @@search)
//!                          (M4-P3 REGEXP1; their ids are in the regexp block)
//! ```
//!
//! A RegExp object's `[Symbol.replace]` and `[Symbol.split]` (the regexp module) take the
//! RegExp arms; `[Symbol.replace]` shares [`get_substitution`] with captures and named
//! captures.
//!
//! The symbols come from `%Symbol%` ([`super::well_known`]). The spec looks the method up only
//! when the pattern is an Object (ecma262 PR 3009, test262 `cstm-*-on-*-primitive.js`); V8 in
//! node 24 still reads it from a primitive's prototype. ep9 follows the spec.

use ep9_vm::{Args, Cx, JsArray, JsObject, JsResult, Local};

use super::{new_str, push_units, string_index_of, this_string, to_units, units, well_known};
use crate::array::index_key;
pub(crate) use crate::regexp::is_reg_exp;

/// GetMethod ( V, P ) (§7.3.11) for an object `V` and the well-known symbol `Symbol.<name>`:
/// `None` for undefined or null.
fn get_method<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s, JsObject>,
    name: &str,
) -> JsResult<Option<Local<'s>>> {
    let key = well_known(cx, name)?;
    // 1. Let func be ? GetV(V, P).
    let func = cx.get(v, key)?;
    // 2. If func is either undefined or null, return undefined.
    if func.is_undefined_or_null() {
        return Ok(None);
    }
    // 3. If IsCallable(func) is false, throw a TypeError exception.
    if !cx.is_callable(func) {
        let msg = format!("Symbol.{name} is not a function");
        return Err(cx.throw_type_error(&msg));
    }
    // 4. Return func.
    Ok(Some(func))
}

/// RequireObjectCoercible(this value): step 1 of replace, replaceAll and split, which look at
/// the pattern before ToString(O).
fn require_object_coercible<'s>(cx: &mut Cx<'s>, this: Local<'s>, name: &str) -> JsResult<()> {
    if this.is_undefined_or_null() {
        let msg = format!("String.prototype.{name} called on null or undefined");
        return Err(cx.throw_type_error(&msg));
    }
    Ok(())
}

/// GetSubstitution ( matched, str, position, captures, namedCaptures, replacementTemplate )
/// (§22.1.3.19.1), appended to `result`. `matched` was found at `position` in `string`;
/// `captures[i]` is capture `i + 1` (`None` = undefined; string patterns have none);
/// `named_captures` is undefined (`None`) or an Object.
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_substitution<'s>(
    cx: &mut Cx<'s>,
    matched: &[u16],
    string: &[u16],
    position: usize,
    captures: &[Option<Vec<u16>>],
    named_captures: Option<Local<'s, JsObject>>,
    template: &[u16],
    result: &mut Vec<u16>,
) -> JsResult<()> {
    const DOLLAR: u16 = b'$' as u16;
    let is_digit = |u: &u16| (u16::from(b'0')..=u16::from(b'9')).contains(u);
    // 1. Let stringLength be the length of str.
    let string_length = string.len();
    // 2. Assert: position ≤ stringLength.
    debug_assert!(position <= string_length);
    // 3. Let result be the empty String. (`result` already holds the text before the match.)
    // 4. Let templateRemainder be replacementTemplate.
    let mut rest = template;
    // 5. Repeat, while templateRemainder is not the empty String,
    while !rest.is_empty() {
        //  a. NOTE: The following steps isolate ref (a prefix of templateRemainder), determine
        //     refReplacement (its replacement), and then append that replacement to result.
        let named: Vec<u16>;
        let (ref_len, ref_replacement): (usize, &[u16]) = match rest {
            //  b. If templateRemainder starts with "$$", then
            //     i. Let ref be "$$". ii. Let refReplacement be "$".
            [DOLLAR, DOLLAR, ..] => (2, &rest[..1]),
            //  c. Else if templateRemainder starts with "$`", then
            //     i. Let ref be "$`". ii. Let refReplacement be the substring of str from 0 to
            //        position.
            [DOLLAR, 0x60, ..] => (2, &string[..position]),
            //  d. Else if templateRemainder starts with "$&", then
            //     i. Let ref be "$&". ii. Let refReplacement be matched.
            [DOLLAR, 0x26, ..] => (2, matched),
            //  e. Else if templateRemainder starts with "$'" (0x0024 (DOLLAR SIGN) followed by
            //     0x0027 (APOSTROPHE)), then
            //     i. Let ref be "$'". ii. Let matchLength be the length of matched.
            //     iii. Let tailPos be position + matchLength.
            //     iv. Let refReplacement be the substring of str from min(tailPos,
            //         stringLength).
            [DOLLAR, 0x27, ..] => {
                let tail_pos = (position + matched.len()).min(string_length);
                (2, &string[tail_pos..])
            }
            //  f. Else if templateRemainder starts with "$" followed by 1 or more decimal
            //     digits, then
            [DOLLAR, d, ..] if is_digit(d) => {
                //  i. If templateRemainder starts with "$" followed by 2 or more decimal
                //     digits, let digitCount be 2. Otherwise, let digitCount be 1.
                let two = rest.get(2).is_some_and(is_digit);
                //  ii. Let digits be the substring of templateRemainder from 1 to 1 +
                //      digitCount.
                //  iii. Let index be ℝ(StringToNumber(digits)).
                let digit = |u: u16| usize::from(u - u16::from(b'0'));
                let mut digit_count = if two { 2 } else { 1 };
                let mut index = if two {
                    digit(*d) * 10 + digit(rest[2])
                } else {
                    digit(*d)
                };
                //  iv. Assert: 0 ≤ index ≤ 99.
                //  v. Let captureLen be the number of elements in captures.
                let capture_len = captures.len();
                //  vi. If index > captureLen and digitCount = 2, then
                //      1. NOTE: When a two-digit replacement pattern specifies an index
                //         exceeding the count of capturing groups, it is reinterpreted as a
                //         one-digit replacement pattern followed by a literal digit.
                //      2. Set digitCount to 1.
                //      3. Set digits to the substring of digits from 0 to 1.
                //      4. Set index to ℝ(StringToNumber(digits)).
                if index > capture_len && digit_count == 2 {
                    digit_count = 1;
                    index = digit(*d);
                }
                //  vii. Let ref be the substring of templateRemainder from 0 to 1 +
                //       digitCount.
                let ref_len = 1 + digit_count;
                //  viii. If 1 ≤ index ≤ captureLen, then
                //        1. Let capture be captures[index - 1].
                //        2. If capture is undefined, then
                //           a. Let refReplacement be the empty String.
                //        3. Else,
                //           a. Let refReplacement be capture.
                //  ix. Else,
                //      1. Let refReplacement be ref.
                if (1..=capture_len).contains(&index) {
                    (ref_len, captures[index - 1].as_deref().unwrap_or(&[]))
                } else {
                    (ref_len, &rest[..ref_len])
                }
            }
            //  g. Else if templateRemainder starts with "$<", then
            [DOLLAR, 0x3C, ..] => {
                //  i. Let gtPos be StringIndexOf(templateRemainder, ">", 0).
                let gt_pos = rest.iter().position(|&u| u == u16::from(b'>'));
                match (gt_pos, named_captures) {
                    //  ii. If gtPos = not-found or namedCaptures is undefined, then
                    //      1. Let ref be "$<".
                    //      2. Let refReplacement be ref.
                    (None, _) | (_, None) => (2, &rest[..2]),
                    //  iii. Else,
                    (Some(gt_pos), Some(groups)) => {
                        //   1. Let ref be the substring of templateRemainder from 0 to
                        //      gtPos + 1.
                        //   2. Let groupName be the substring of templateRemainder from 2 to
                        //      gtPos.
                        //   3. Assert: namedCaptures is an Object.
                        //   4. Let capture be ? Get(namedCaptures, groupName).
                        let group_name = cx.new_string_utf16(&rest[2..gt_pos])?;
                        let capture = cx.get(groups, group_name)?;
                        //   5. If capture is undefined, then
                        //      a. Let refReplacement be the empty String.
                        //   6. Else,
                        //      a. Let refReplacement be ? ToString(capture).
                        named = if capture.is_undefined() {
                            Vec::new()
                        } else {
                            to_units(cx, capture)?
                        };
                        (gt_pos + 1, &named[..])
                    }
                }
            }
            //  h. Else,
            //     i. Let ref be the substring of templateRemainder from 0 to 1.
            //     ii. Let refReplacement be ref.
            _ => (1, &rest[..1]),
        };
        //  i. Let refLength be the length of ref.
        //  j. Set templateRemainder to the substring of templateRemainder from refLength.
        //  k. Set result to the string-concatenation of result and refReplacement.
        push_units(cx, result, ref_replacement)?;
        rest = &rest[ref_len..];
    }
    // 6. Return result.
    Ok(())
}

/// The replacement for a match at `position`: ToString(Call(replaceValue, undefined,
/// «searchString, 𝔽(position), string»)) when it is a function, else GetSubstitution.
/// Appended to `result`.
struct Replacer<'s> {
    /// The replace function, or `None` for a template.
    function: Option<Local<'s>>,
    /// The template (ToString(replaceValue)) when `function` is `None`.
    template: Vec<u16>,
    /// searchString and string as JS values, the function's arguments.
    search_value: Local<'s>,
    string_value: Local<'s>,
}

impl<'s> Replacer<'s> {
    fn append(
        &self,
        cx: &mut Cx<'s>,
        search: &[u16],
        string: &[u16],
        position: usize,
        result: &mut Vec<u16>,
    ) -> JsResult<()> {
        match self.function {
            Some(f) => {
                let args = [
                    self.search_value,
                    Local::int32(position as i32),
                    self.string_value,
                ];
                let r = cx.call(f, Local::undefined(), &args)?;
                let replacement = to_units(cx, r)?;
                push_units(cx, result, &replacement)
            }
            None => get_substitution(
                cx,
                search,
                string,
                position,
                &[],
                None,
                &self.template,
                result,
            ),
        }
    }
}

/// Steps 3-6 of replace and replaceAll: ToString(O), ToString(searchValue), then
/// replaceValue as a function or ToString(replaceValue).
fn prepare<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    search_value: Local<'s>,
    replace_value: Local<'s>,
    name: &str,
) -> JsResult<(Vec<u16>, Vec<u16>, Replacer<'s>)> {
    // 3. Let string be ? ToString(O).
    let string_value = this_string(cx, this, name)?;
    // 4. Let searchString be ? ToString(searchValue).
    let search_string = cx.to_string(search_value)?;
    // 5. Let functionalReplace be IsCallable(replaceValue).
    // 6. If functionalReplace is false, then
    //    a. Set replaceValue to ? ToString(replaceValue).
    let (function, template) = if cx.is_callable(replace_value) {
        (Some(replace_value), Vec::new())
    } else {
        (None, to_units(cx, replace_value)?)
    };
    let string = units(cx, string_value)?;
    let search = units(cx, search_string)?;
    let replacer = Replacer {
        function,
        template,
        search_value: search_string.value(),
        string_value: string_value.value(),
    };
    Ok((string, search, replacer))
}

/// String.prototype.replace ( searchValue, replaceValue ) (§22.1.3.19).
pub(super) fn replace<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (search_value, replace_value) = (args.get(0), args.get(1));
    // 1. Let O be ? RequireObjectCoercible(this value).
    require_object_coercible(cx, this, "replace")?;
    // 2. If searchValue is an Object, then
    if let Some(o) = search_value.as_object() {
        //    a. Let replacer be ? GetMethod(searchValue, @@replace).
        //    b. If replacer is not undefined, then
        //       i. Return ? Call(replacer, searchValue, « O, replaceValue »).
        if let Some(replacer) = get_method(cx, o, "replace")? {
            return cx.call(replacer, search_value, &[this, replace_value]);
        }
    }
    // 3-6. (prepare)
    let (string, search, replacer) = prepare(cx, this, search_value, replace_value, "replace")?;
    // 7. Let searchLength be the length of searchString.
    let search_length = search.len();
    // 8. Let position be StringIndexOf(string, searchString, 0).
    // 9. If position is not-found, return string.
    let Some(position) = string_index_of(cx, &string, &search, 0)? else {
        return Ok(replacer.string_value);
    };
    // 10. Let preceding be the substring of string from 0 to position.
    let mut result = string[..position].to_vec();
    // 11. Let following be the substring of string from position + searchLength.
    // 12. If functionalReplace is true, then
    //     a. Let replacement be ? ToString(? Call(replaceValue, undefined, « searchString,
    //        𝔽(position), string »)).
    // 13. Else,
    //     a. Assert: replaceValue is a String.
    //     b. Let captures be a new empty List.
    //     c. Let replacement be ! GetSubstitution(searchString, string, position, captures,
    //        undefined, replaceValue).
    replacer.append(cx, &search, &string, position, &mut result)?;
    // 14. Return the string-concatenation of preceding, replacement, and following.
    push_units(cx, &mut result, &string[position + search_length..])?;
    new_str(cx, &result)
}

/// String.prototype.replaceAll ( searchValue, replaceValue ) (§22.1.3.20).
pub(super) fn replace_all<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (search_value, replace_value) = (args.get(0), args.get(1));
    // 1. Let O be ? RequireObjectCoercible(this value).
    require_object_coercible(cx, this, "replaceAll")?;
    // 2. If searchValue is an Object, then
    if let Some(o) = search_value.as_object() {
        //    a. Let isRegExp be ? IsRegExp(searchValue).
        //    b. If isRegExp is true, then
        if is_reg_exp(cx, search_value)? {
            //   i. Let flags be ? Get(searchValue, "flags").
            let flags = cx.get(o, "flags")?;
            //   ii. Perform ? RequireObjectCoercible(flags).
            if flags.is_undefined_or_null() {
                return Err(cx.throw_type_error(
                    "String.prototype.replaceAll called with a RegExp whose flags are undefined",
                ));
            }
            //   iii. If ? ToString(flags) does not contain "g", throw a TypeError exception.
            let flags = to_units(cx, flags)?;
            if !flags.contains(&u16::from(b'g')) {
                return Err(cx.throw_type_error("replaceAll must be called with a global RegExp"));
            }
        }
        //    c. Let replacer be ? GetMethod(searchValue, @@replace).
        //    d. If replacer is not undefined, then
        //       i. Return ? Call(replacer, searchValue, « O, replaceValue »).
        if let Some(replacer) = get_method(cx, o, "replace")? {
            return cx.call(replacer, search_value, &[this, replace_value]);
        }
    }
    // 3-6. (prepare)
    let (string, search, replacer) = prepare(cx, this, search_value, replace_value, "replaceAll")?;
    // 7. Let searchLength be the length of searchString.
    let search_length = search.len();
    // 8. Let advanceBy be max(1, searchLength).
    let advance_by = search_length.max(1);
    // 9. Let matchPositions be a new empty List.
    // 10. Let position be StringIndexOf(string, searchString, 0).
    // 11. Repeat, while position is not not-found,
    //     a. Append position to matchPositions.
    //     b. Set position to StringIndexOf(string, searchString, position + advanceBy).
    // (the positions do not depend on the replacements, so each one is used as it is found)
    // 12. Let endOfLastMatch be 0.
    let mut end_of_last_match = 0;
    // 13. Let result be the empty String.
    let mut result = Vec::new();
    // 14. For each element p of matchPositions, do
    let mut next = string_index_of(cx, &string, &search, 0)?;
    while let Some(p) = next {
        // (one JS call or one string per pass, and no back edge)
        cx.poll_interrupt()?;
        //  a. Let preserved be the substring of string from endOfLastMatch to p.
        push_units(cx, &mut result, &string[end_of_last_match..p])?;
        //  b. If functionalReplace is true, then
        //     i. Let replacement be ? ToString(? Call(replaceValue, undefined, « searchString,
        //        𝔽(p), string »)).
        //  c. Else,
        //     i. Assert: replaceValue is a String.
        //     ii. Let captures be a new empty List.
        //     iii. Let replacement be ! GetSubstitution(searchString, string, p, captures,
        //          undefined, replaceValue).
        //  d. Set result to the string-concatenation of result, preserved, and replacement.
        replacer.append(cx, &search, &string, p, &mut result)?;
        //  e. Set endOfLastMatch to p + searchLength.
        end_of_last_match = p + search_length;
        next = string_index_of(cx, &string, &search, p + advance_by)?;
    }
    // 15. If endOfLastMatch < the length of string, then
    //     a. Set result to the string-concatenation of result and the substring of string from
    //        endOfLastMatch.
    if end_of_last_match < string.len() {
        push_units(cx, &mut result, &string[end_of_last_match..])?;
    }
    // 16. Return result.
    new_str(cx, &result)
}

/// String.prototype.split ( separator, limit ) (§22.1.3.23).
pub(super) fn split<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (separator, limit) = (args.get(0), args.get(1));
    // 1. Let O be ? RequireObjectCoercible(this value).
    require_object_coercible(cx, this, "split")?;
    // 2. If separator is an Object, then
    if let Some(o) = separator.as_object() {
        //    a. Let splitter be ? GetMethod(separator, @@split).
        //    b. If splitter is not undefined, then
        //       i. Return ? Call(splitter, separator, « O, limit »).
        if let Some(splitter) = get_method(cx, o, "split")? {
            return cx.call(splitter, separator, &[this, limit]);
        }
    }
    // 3. Let S be ? ToString(O).
    let s = this_string(cx, this, "split")?;
    // 4. If limit is undefined, let lim be 2^32 - 1; else let lim be ℝ(? ToUint32(limit)).
    let lim = if limit.is_undefined() {
        u32::MAX
    } else {
        cx.to_uint32(limit)?
    } as usize;
    // 5. Let R be ? ToString(separator).
    let r = to_units(cx, separator)?;
    let array = cx.new_array();
    // 6. If lim = 0, then
    //    a. Return CreateArrayFromList(« »).
    if lim == 0 {
        return Ok(array.value());
    }
    let mut count = 0u64;
    // 7. If separator is undefined, then
    //    a. Return CreateArrayFromList(« S »).
    if separator.is_undefined() {
        let s = units(cx, s)?;
        push(cx, array, &mut count, &s)?;
        return Ok(array.value());
    }
    // 8. Let separatorLength be the length of R.
    let separator_length = r.len();
    let s = units(cx, s)?;
    // 9. If separatorLength = 0, then
    if separator_length == 0 {
        //    a. Let head be the substring of S from 0 to lim.
        //    b. Let codeUnits be a List consisting of the sequence of code units that are the
        //       elements of head.
        //    c. Return CreateArrayFromList(codeUnits).
        for unit in s.iter().take(lim) {
            push(cx, array, &mut count, core::slice::from_ref(unit))?;
        }
        return Ok(array.value());
    }
    // 10. If S is the empty String, return CreateArrayFromList(« S »).
    if s.is_empty() {
        push(cx, array, &mut count, &s)?;
        return Ok(array.value());
    }
    // 11. Let substrings be a new empty List.
    // 12. Let i be 0.
    let mut i = 0;
    // 13. Let j be StringIndexOf(S, R, 0).
    let mut j = string_index_of(cx, &s, &r, 0)?;
    // 14. Repeat, while j is not not-found,
    while let Some(jj) = j {
        //  a. Let T be the substring of S from i to j.
        //  b. Append T to substrings.
        push(cx, array, &mut count, &s[i..jj])?;
        //  c. If the number of elements in substrings is lim, return
        //     CreateArrayFromList(substrings).
        if count == lim as u64 {
            return Ok(array.value());
        }
        //  d. Set i to j + separatorLength.
        i = jj + separator_length;
        //  e. Set j to StringIndexOf(S, R, i).
        j = string_index_of(cx, &s, &r, i)?;
    }
    // 15. Let T be the substring of S from i.
    // 16. Append T to substrings.
    push(cx, array, &mut count, &s[i..])?;
    // 17. Return CreateArrayFromList(substrings).
    Ok(array.value())
}

/// Appends a new string of `piece` at index `count` of the new array `array`
/// (CreateDataPropertyOrThrow, as CreateArrayFromList does), and counts it.
fn push<'s>(
    cx: &mut Cx<'s>,
    array: Local<'s, JsArray>,
    count: &mut u64,
    piece: &[u16],
) -> JsResult<()> {
    // (one string per pass, and no back edge)
    cx.poll_interrupt()?;
    let v = new_str(cx, piece)?;
    let key = index_key(cx, *count);
    cx.create_data_property_or_throw(array, key, v)?;
    *count += 1;
    Ok(())
}

/// String.prototype.match ( regexp ) (§22.1.3.13).
pub(crate) fn match_<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    matcher_call(cx, this, args.get(0), "match")
}

/// String.prototype.search ( regexp ) (§22.1.3.22).
pub(crate) fn search<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    matcher_call(cx, this, args.get(0), "search")
}

/// The steps of String.prototype.match and search (§22.1.3.13, §22.1.3.22; they differ only
/// in the symbol, `Symbol.<name>`).
fn matcher_call<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    regexp: Local<'s>,
    name: &str,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    require_object_coercible(cx, this, name)?;
    // 2. If regexp is an Object, then
    if let Some(o) = regexp.as_object() {
        //    a. Let matcher (searcher) be ? GetMethod(regexp, @@match (@@search)).
        //    b. If it is not undefined, then
        //       i. Return ? Call(matcher, regexp, « O »).
        if let Some(m) = get_method(cx, o, name)? {
            return cx.call(m, regexp, &[this]);
        }
    }
    // 3. Let S be ? ToString(O).
    let s = this_string(cx, this, name)?;
    // 4. Let rx be ? RegExpCreate(regexp, undefined).
    let rx = crate::regexp::regexp_create(cx, regexp, "")?;
    // 5. Return ? Invoke(rx, @@match (@@search), « S »).
    let key = well_known(cx, name)?;
    let f = cx.get(rx, key)?;
    cx.call(f, rx, &[s.value()])
}
