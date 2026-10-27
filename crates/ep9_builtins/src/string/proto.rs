//! `%String.prototype%` methods without a pattern or a case mapping (§22.1.3): at, charAt,
//! codePointAt, concat, endsWith, includes, indexOf, lastIndexOf, isWellFormed, toWellFormed,
//! localeCompare, padStart, padEnd, repeat, slice, startsWith, substring, trim, trimStart,
//! trimEnd.

use core::cmp::Ordering;

use ep9_vm::{Args, Cx, JsResult, Local};

use super::{
    MAX_LENGTH, SearchWork, clamp, matches_at, new_str, pattern::is_reg_exp, push_units,
    string_index_of, this_string, to_units, units, units_range,
};

/// String.prototype.at ( index ) (§22.1.3.1).
pub(super) fn at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "at")?;
    // 3. Let len be the length of S.
    let len = cx.string_len(s) as f64;
    // 4. Let relativeIndex be ? ToIntegerOrInfinity(index).
    let relative_index = cx.to_integer_or_infinity(args.get(0))?;
    // 5. If relativeIndex ≥ 0, then
    //    a. Let k be relativeIndex.
    // 6. Else,
    //    a. Let k be len + relativeIndex.
    let k = if relative_index >= 0.0 {
        relative_index
    } else {
        len + relative_index
    };
    // 7. If k < 0 or k ≥ len, return undefined.
    if k < 0.0 || k >= len {
        return Ok(Local::undefined());
    }
    // 8. Return the substring of S from k to k + 1.
    let flat = cx.flatten(s)?;
    let unit = cx.code_unit_at(flat, k as usize).expect("k < len");
    new_str(cx, &[unit])
}

/// String.prototype.charAt ( pos ) (§22.1.3.2).
pub(super) fn char_at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "charAt")?;
    // 3. Let position be ? ToIntegerOrInfinity(pos).
    let position = cx.to_integer_or_infinity(args.get(0))?;
    // 4. Let size be the length of S.
    let size = cx.string_len(s);
    // 5. If position < 0 or position ≥ size, return the empty String.
    if position < 0.0 || position >= size as f64 {
        return Ok(cx.key("").value());
    }
    // 6. Return the substring of S from position to position + 1.
    let flat = cx.flatten(s)?;
    let unit = cx
        .code_unit_at(flat, position as usize)
        .expect("position < size");
    new_str(cx, &[unit])
}

/// String.prototype.codePointAt ( pos ) (§22.1.3.4).
pub(super) fn code_point_at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "codePointAt")?;
    // 3. Let position be ? ToIntegerOrInfinity(pos).
    let position = cx.to_integer_or_infinity(args.get(0))?;
    // 4. Let size be the length of S.
    let size = cx.string_len(s);
    // 5. If position < 0 or position ≥ size, return undefined.
    if position < 0.0 || position >= size as f64 {
        return Ok(Local::undefined());
    }
    // 6. Let cp be CodePointAt(S, position).
    // 7. Return 𝔽(cp.[[CodePoint]]).
    let flat = cx.flatten(s)?;
    let i = position as usize;
    let first = cx.code_unit_at(flat, i).expect("position < size");
    let cp = match (first, cx.code_unit_at(flat, i + 1)) {
        (0xD800..=0xDBFF, Some(second @ 0xDC00..=0xDFFF)) => {
            0x10000 + ((i32::from(first) - 0xD800) << 10) + (i32::from(second) - 0xDC00)
        }
        _ => i32::from(first),
    };
    Ok(Local::int32(cp))
}

/// String.prototype.concat ( ...args ) (§22.1.3.5).
pub(super) fn concat<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "concat")?;
    // 3. Let R be S.
    let mut r = units(cx, s)?;
    // 4. For each element next of args, do
    for next in args.iter() {
        //    a. Let nextString be ? ToString(next).
        let next_string = to_units(cx, next)?;
        //    b. Set R to the string-concatenation of R and nextString.
        push_units(cx, &mut r, &next_string)?;
    }
    // 5. Return R.
    new_str(cx, &r)
}

/// Steps 3-4 of endsWith, includes and startsWith: ? IsRegExp(searchString), a TypeError if
/// it is a RegExp, then ? ToString(searchString).
fn search_string<'s>(cx: &mut Cx<'s>, search: Local<'s>, name: &str) -> JsResult<Vec<u16>> {
    // 3. Let isRegExp be ? IsRegExp(searchString).
    // 4. If isRegExp is true, throw a TypeError exception.
    if is_reg_exp(cx, search)? {
        let msg =
            format!("First argument to String.prototype.{name} must not be a regular expression");
        return Err(cx.throw_type_error(&msg));
    }
    // 5. Let searchStr be ? ToString(searchString).
    to_units(cx, search)
}

/// String.prototype.endsWith ( searchString [ , endPosition ] ) (§22.1.3.7).
pub(super) fn ends_with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "endsWith")?;
    // 3-5. (search_string)
    let search_str = search_string(cx, args.get(0), "endsWith")?;
    // 6. Let len be the length of S.
    let len = cx.string_len(s);
    // 7. If endPosition is undefined, let pos be len; else let pos be
    //    ? ToIntegerOrInfinity(endPosition).
    let end_position = args.get(1);
    let pos = if end_position.is_undefined() {
        len as f64
    } else {
        cx.to_integer_or_infinity(end_position)?
    };
    // 8. Let end be the result of clamping pos between 0 and len.
    let end = clamp(pos, len);
    // 9. Let searchLength be the length of searchStr.
    let search_length = search_str.len();
    // 10. If searchLength = 0, return true.
    if search_length == 0 {
        return Ok(Local::boolean(true));
    }
    // 11. Let start be end - searchLength.
    // 12. If start < 0, return false.
    let Some(start) = end.checked_sub(search_length) else {
        return Ok(Local::boolean(false));
    };
    // 13. Let substring be the substring of S from start to end.
    // 14. If substring is searchStr, return true.
    // 15. Return false.
    Ok(Local::boolean(
        units_range(cx, s, start, end)? == search_str,
    ))
}

/// String.prototype.includes ( searchString [ , position ] ) (§22.1.3.8).
pub(super) fn includes<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "includes")?;
    // 3-5. (search_string)
    let search_str = search_string(cx, args.get(0), "includes")?;
    // 6. Let pos be ? ToIntegerOrInfinity(position).
    // 7. Assert: If position is undefined, then pos is 0.
    let pos = cx.to_integer_or_infinity(args.get(1))?;
    // 8. Let len be the length of S.
    // 9. Let start be the result of clamping pos between 0 and len.
    let s = units(cx, s)?;
    let start = clamp(pos, s.len());
    // 10. Let index be StringIndexOf(S, searchStr, start).
    // 11. If index is not-found, return false.
    // 12. Return true.
    Ok(Local::boolean(
        string_index_of(cx, &s, &search_str, start)?.is_some(),
    ))
}

/// String.prototype.indexOf ( searchString [ , position ] ) (§22.1.3.9).
pub(super) fn index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "indexOf")?;
    // 3. Let searchStr be ? ToString(searchString).
    let search_str = to_units(cx, args.get(0))?;
    // 4. Let pos be ? ToIntegerOrInfinity(position).
    // 5. Assert: If position is undefined, then pos is 0.
    let pos = cx.to_integer_or_infinity(args.get(1))?;
    // 6. Let len be the length of S.
    // 7. Let start be the result of clamping pos between 0 and len.
    let s = units(cx, s)?;
    let start = clamp(pos, s.len());
    // 8. Let result be StringIndexOf(S, searchStr, start).
    // 9. If result is not-found, return -1𝔽.
    // 10. Return 𝔽(result).
    Ok(index_value(string_index_of(cx, &s, &search_str, start)?))
}

/// An index result: `𝔽(i)`, or -1 for not-found (string indices fit an int32).
fn index_value<'s>(i: Option<usize>) -> Local<'s> {
    Local::int32(i.map_or(-1, |i| i as i32))
}

/// String.prototype.isWellFormed ( ) (§22.1.3.10).
pub(super) fn is_well_formed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "isWellFormed")?;
    // 3. Return IsStringWellFormedUnicode(S).
    let s = units(cx, s)?;
    Ok(Local::boolean(
        char::decode_utf16(s.iter().copied()).all(|c| c.is_ok()),
    ))
}

/// String.prototype.lastIndexOf ( searchString [ , position ] ) (§22.1.3.11).
pub(super) fn last_index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "lastIndexOf")?;
    // 3. Let searchStr be ? ToString(searchString).
    let search_str = to_units(cx, args.get(0))?;
    // 4. Let numPos be ? ToNumber(position).
    // 5. Assert: If position is undefined, then numPos is NaN.
    let num_pos = cx.to_number(args.get(1))?;
    // 6. If numPos is NaN, let pos be +∞; otherwise, let pos be ! ToIntegerOrInfinity(numPos).
    let pos = if num_pos.is_nan() {
        f64::INFINITY
    } else {
        cx.to_integer_or_infinity(Local::number(num_pos))?
    };
    // 7. Let len be the length of S.
    let s = units(cx, s)?;
    let len = s.len();
    // 8. Let searchLen be the length of searchStr.
    let search_len = search_str.len();
    // (a search string longer than S matches nowhere: the clamp range below is empty)
    let Some(last) = len.checked_sub(search_len) else {
        return Ok(Local::int32(-1));
    };
    // 9. Let start be the result of clamping pos between 0 and len - searchLen.
    let start = clamp(pos, last);
    // 10. If searchStr is the empty String, return 𝔽(start).
    // 11. For each integer i such that 0 ≤ i ≤ start, in descending order, do
    //     a. Let candidate be the substring of S from i to i + searchLen.
    //     b. If candidate is searchStr, return 𝔽(i).
    // 12. Return -1𝔽.
    //     (polled per 256 code units compared, as StringIndexOf is: `SearchWork`)
    let mut work = SearchWork::default();
    for i in (0..=start).rev() {
        if matches_at(cx, &s, &search_str, i, &mut work)? {
            return Ok(index_value(Some(i)));
        }
    }
    Ok(index_value(None))
}

/// String.prototype.localeCompare ( that [ , reserved1 [ , reserved2 ] ] ) (§22.1.3.12).
/// v0 (no ECMA-402): the order of the code units, so canonically equivalent strings in
/// different normal forms do not compare as 0 (normalization is M1).
pub(super) fn locale_compare<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "localeCompare")?;
    // 3. Let thatValue be ? ToString(that).
    let that_value = to_units(cx, args.get(0))?;
    // 4. ... an implementation-defined comparison.
    let s = units(cx, s)?;
    Ok(Local::int32(match s.cmp(&that_value) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }))
}

/// Where StringPad puts the fill.
#[derive(Clone, Copy, PartialEq)]
enum Placement {
    Start,
    End,
}

/// StringPaddingBuiltinsImpl ( O, maxLength, fillString, placement ) (§22.1.3.17.1), with
/// StringPad (§22.1.3.17.2). Steps 1-2 of padStart / padEnd (RequireObjectCoercible) run in
/// [`this_string`].
fn string_padding<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    placement: Placement,
) -> JsResult<Local<'s>> {
    let name = match placement {
        Placement::Start => "padStart",
        Placement::End => "padEnd",
    };
    // 1. Let S be ? ToString(O).
    let s = this_string(cx, this, name)?;
    // 2. Let intMaxLength be ℝ(? ToLength(maxLength)).
    let int_max_length = cx.to_length(args.get(0))?;
    // 3. Let stringLength be the length of S.
    let string_length = cx.string_len(s);
    // 4. If intMaxLength ≤ stringLength, return S.
    if int_max_length <= string_length as u64 {
        return Ok(s.value());
    }
    // 5. If fillString is undefined, set fillString to the String value consisting solely of
    //    the code unit 0x0020 (SPACE).
    // 6. Else, set fillString to ? ToString(fillString).
    let fill_string = args.get(1);
    let fill_string = if fill_string.is_undefined() {
        vec![0x20]
    } else {
        to_units(cx, fill_string)?
    };
    // 7. Return StringPad(S, intMaxLength, fillString, placement).
    // StringPad 1-2. Let stringLength be the length of S. If maxLength ≤ stringLength, return S.
    // StringPad 3. If fillString is the empty String, return S.
    if fill_string.is_empty() {
        return Ok(s.value());
    }
    if int_max_length > MAX_LENGTH as u64 {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    // StringPad 4. Let fillLen be maxLength - stringLength.
    let fill_len = int_max_length as usize - string_length;
    // StringPad 5. Let truncatedStringFiller be the String value consisting of repeated
    //    concatenations of fillString truncated to length fillLen.
    let filler = fill_string.iter().copied().cycle().take(fill_len);
    let s = units(cx, s)?;
    let mut out = Vec::with_capacity(int_max_length as usize);
    // StringPad 6. If placement is start, return the string-concatenation of
    //    truncatedStringFiller and S.
    // StringPad 7. Else, return the string-concatenation of S and truncatedStringFiller.
    if placement == Placement::Start {
        out.extend(filler);
        out.extend_from_slice(&s);
    } else {
        out.extend_from_slice(&s);
        out.extend(filler);
    }
    new_str(cx, &out)
}

/// String.prototype.padEnd ( maxLength [ , fillString ] ) (§22.1.3.16).
pub(super) fn pad_end<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Return ? StringPaddingBuiltinsImpl(O, maxLength, fillString, end).
    string_padding(cx, this, args, Placement::End)
}

/// String.prototype.padStart ( maxLength [ , fillString ] ) (§22.1.3.17).
pub(super) fn pad_start<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Return ? StringPaddingBuiltinsImpl(O, maxLength, fillString, start).
    string_padding(cx, this, args, Placement::Start)
}

/// String.prototype.repeat ( count ) (§22.1.3.18).
pub(super) fn repeat<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "repeat")?;
    // 3. Let n be ? ToIntegerOrInfinity(count).
    let count = args.get(0);
    let n = cx.to_integer_or_infinity(count)?;
    // 4. If n < 0 or n = +∞, throw a RangeError exception.
    if n < 0.0 || n == f64::INFINITY {
        let text = cx.to_rust_string(Local::number(n))?;
        return Err(cx.throw_range_error(&format!("Invalid count value: {text}")));
    }
    // 5. If n = 0, return the empty String.
    let len = cx.string_len(s);
    if n == 0.0 || len == 0 {
        return Ok(cx.key("").value());
    }
    // 6. Return the String value that is made from n copies of S appended together.
    if n > (MAX_LENGTH / len) as f64 {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    let s = units(cx, s)?;
    new_str(cx, &s.repeat(n as usize))
}

/// The start and end of a relative range as slice and substr read it: ±∞ and negative
/// values count from `len` (steps 4-5 of slice).
pub(super) fn relative_start(n: f64, len: usize) -> usize {
    if n == f64::NEG_INFINITY {
        0
    } else if n < 0.0 {
        clamp(len as f64 + n, len)
    } else {
        clamp(n, len)
    }
}

/// String.prototype.slice ( start, end ) (§22.1.3.22).
pub(super) fn slice<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "slice")?;
    // 3. Let len be the length of S.
    let len = cx.string_len(s);
    // 4. Let intStart be ? ToIntegerOrInfinity(start).
    // 5. If intStart = -∞, let from be 0.
    // 6. Else if intStart < 0, let from be max(len + intStart, 0).
    // 7. Else, let from be min(intStart, len).
    let from = relative_start(cx.to_integer_or_infinity(args.get(0))?, len);
    // 8. If end is undefined, let intEnd be len; else let intEnd be ? ToIntegerOrInfinity(end).
    // 9. If intEnd = -∞, let to be 0.
    // 10. Else if intEnd < 0, let to be max(len + intEnd, 0).
    // 11. Else, let to be min(intEnd, len).
    let end = args.get(1);
    let to = if end.is_undefined() {
        len
    } else {
        relative_start(cx.to_integer_or_infinity(end)?, len)
    };
    // 12. If from ≥ to, return the empty String.
    if from >= to {
        return Ok(cx.key("").value());
    }
    // 13. Return the substring of S from from to to.
    let substring = units_range(cx, s, from, to)?;
    new_str(cx, &substring)
}

/// String.prototype.startsWith ( searchString [ , position ] ) (§22.1.3.24).
pub(super) fn starts_with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "startsWith")?;
    // 3-5. (search_string)
    let search_str = search_string(cx, args.get(0), "startsWith")?;
    // 6. Let len be the length of S.
    let len = cx.string_len(s);
    // 7. If position is undefined, let pos be 0; else let pos be ? ToIntegerOrInfinity(position).
    let position = args.get(1);
    let pos = if position.is_undefined() {
        0.0
    } else {
        cx.to_integer_or_infinity(position)?
    };
    // 8. Let start be the result of clamping pos between 0 and len.
    let start = clamp(pos, len);
    // 9. Let searchLength be the length of searchStr.
    let search_length = search_str.len();
    // 10. If searchLength = 0, return true.
    if search_length == 0 {
        return Ok(Local::boolean(true));
    }
    // 11. Let end be start + searchLength.
    let end = start + search_length;
    // 12. If end > len, return false.
    if end > len {
        return Ok(Local::boolean(false));
    }
    // 13. Let substring be the substring of S from start to end.
    // 14. If substring is searchStr, return true.
    // 15. Return false.
    Ok(Local::boolean(
        units_range(cx, s, start, end)? == search_str,
    ))
}

/// String.prototype.substring ( start, end ) (§22.1.3.25).
pub(super) fn substring<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "substring")?;
    // 3. Let len be the length of S.
    let len = cx.string_len(s);
    // 4. Let intStart be ? ToIntegerOrInfinity(start).
    let int_start = cx.to_integer_or_infinity(args.get(0))?;
    // 5. If end is undefined, let intEnd be len; else let intEnd be ? ToIntegerOrInfinity(end).
    let end = args.get(1);
    let int_end = if end.is_undefined() {
        len as f64
    } else {
        cx.to_integer_or_infinity(end)?
    };
    // 6. Let finalStart be the result of clamping intStart between 0 and len.
    // 7. Let finalEnd be the result of clamping intEnd between 0 and len.
    let (final_start, final_end) = (clamp(int_start, len), clamp(int_end, len));
    // 8. Let from be min(finalStart, finalEnd).
    // 9. Let to be max(finalStart, finalEnd).
    let (from, to) = (final_start.min(final_end), final_start.max(final_end));
    // 10. Return the substring of S from from to to.
    let substring = units_range(cx, s, from, to)?;
    new_str(cx, &substring)
}

/// String.prototype.toWellFormed ( ) (§22.1.3.31).
pub(super) fn to_well_formed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "toWellFormed")?;
    // 3. Let strLen be the length of S.
    // 4. Let k be 0.
    // 5. Let result be the empty String.
    // 6. Repeat, while k < strLen,
    //    a. Let cp be CodePointAt(S, k).
    //    b. If cp.[[IsUnpairedSurrogate]] is true, then
    //       i. Set result to the string-concatenation of result and 0xFFFD (REPLACEMENT
    //          CHARACTER).
    //    c. Else,
    //       i. Set result to the string-concatenation of result and
    //          UTF16EncodeCodePoint(cp.[[CodePoint]]).
    //    d. Set k to k + cp.[[CodeUnitCount]].
    // 7. Return result.
    let units = units(cx, s)?;
    let mut out = Vec::with_capacity(units.len());
    let mut buf = [0u16; 2];
    for c in char::decode_utf16(units.iter().copied()) {
        out.extend_from_slice(c.unwrap_or('\u{FFFD}').encode_utf16(&mut buf));
    }
    new_str(cx, &out)
}

/// The WhiteSpace and LineTerminator code points (§12.2, §12.3): TAB, VT, FF, ZWNBSP, the
/// Unicode "Space_Separator" (Zs) code points of Unicode 17 (all in the BMP), LF, CR, LS and
/// PS. Rust's `char::is_whitespace` differs: it has U+0085 and lacks U+FEFF.
pub(super) fn is_white_space_or_line_terminator(u: u16) -> bool {
    matches!(
        u,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

/// Which ends TrimString trims.
#[derive(Clone, Copy)]
enum Where {
    Start,
    End,
    StartAndEnd,
}

/// TrimString ( string, where ) (§22.1.3.32.1).
fn trim_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    r#where: Where,
    name: &str,
) -> JsResult<Local<'s>> {
    // 1. Let str be ? RequireObjectCoercible(string).
    // 2. Let S be ? ToString(str).
    let s = this_string(cx, this, name)?;
    let s = units(cx, s)?;
    let ws = |u: &u16| is_white_space_or_line_terminator(*u);
    // 3. If where is start, then
    //    a. Let T be the String value that is a copy of S with leading white space removed.
    // 4. Else if where is end, then
    //    a. Let T be the String value that is a copy of S with trailing white space removed.
    // 5. Else,
    //    a. Assert: where is start+end.
    //    b. Let T be the String value that is a copy of S with both leading and trailing
    //       white space removed.
    let start = match r#where {
        Where::Start | Where::StartAndEnd => s.iter().position(|u| !ws(u)).unwrap_or(s.len()),
        Where::End => 0,
    };
    let end = match r#where {
        Where::End | Where::StartAndEnd => s.iter().rposition(|u| !ws(u)).map_or(start, |i| i + 1),
        Where::Start => s.len(),
    };
    // 6. Return T.
    new_str(cx, &s[start..end.max(start)])
}

/// String.prototype.trim ( ) (§22.1.3.32).
pub(super) fn trim<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let S be the this value.
    // 2. Return ? TrimString(S, start+end).
    trim_string(cx, this, Where::StartAndEnd, "trim")
}

/// String.prototype.trimEnd ( ) (§22.1.3.33).
pub(super) fn trim_end<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let S be the this value.
    // 2. Return ? TrimString(S, end).
    trim_string(cx, this, Where::End, "trimEnd")
}

/// String.prototype.trimStart ( ) (§22.1.3.34).
pub(super) fn trim_start<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let S be the this value.
    // 2. Return ? TrimString(S, start).
    trim_string(cx, this, Where::Start, "trimStart")
}
