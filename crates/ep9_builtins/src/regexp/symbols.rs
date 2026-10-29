//! `RegExp.prototype[@@match]`, `[@@replace]`, `[@@search]` and `[@@split]` (§22.2.6.8,
//! §22.2.6.11, §22.2.6.12, §22.2.6.14).
//!
//! ```text
//! [@@match](S)       flags = ToString(Get(rx, "flags")); no "g": RegExpExec once;
//!                    "g": lastIndex = 0, RegExpExec until null, each "0" into an array
//! [@@replace](S, r)  the results of RegExpExec (one, or all with "g"), then for each: the
//!                    captures, "index", "groups", then r(...) or GetSubstitution
//! [@@search](S)      lastIndex = 0 around one RegExpExec, restored; the "index"
//! [@@split](S, lim)  C = SpeciesConstructor(rx, %RegExp%); splitter = new C(rx, flags + "y");
//!                    a sticky RegExpExec at each position (fast path: one non-sticky search
//!                    per piece while C is %RegExp% and exec is the builtin, module doc)
//! ```
//!
//! Each loop that runs JS or allocates once per pass polls the interrupt word
//! (`Cx::poll_interrupt`). A list of results lives in a `ScratchList` (an object result) or
//! as a `MatchRecord` (a record: Rust memory), never as a `Local` in a Rust `Vec` (H20 R2).

use ep9_vm::{
    Args, Cx, IntrinsicId, JsObject, JsResult, JsString, Local, MatchRecord, ObjectClass,
    ScratchList,
};

use super::exec::{
    self, Exec, advance_string_index, is_builtin_exec, is_regexp_object, number, set_last_index,
    slot, substring,
};
use crate::array::{index_key, length_of_array_like};
use crate::string::pattern::get_substitution;
use crate::string::{push_units, units};

/// The this value as an Object, else a TypeError naming the method.
fn this_object<'s>(cx: &mut Cx<'s>, this: Local<'s>, name: &str) -> JsResult<Local<'s, JsObject>> {
    match this.as_object() {
        Some(o) => Ok(o),
        None => {
            let shown = cx.display(this);
            let msg = format!("RegExp.prototype[Symbol.{name}] called on non-object {shown}");
            Err(cx.throw_type_error(&msg))
        }
    }
}

/// ? ToString(? Get(rx, "flags")) as code units.
pub(super) fn flags_of<'s>(cx: &mut Cx<'s>, rx: Local<'s, JsObject>) -> JsResult<Vec<u16>> {
    let flags = cx.get(rx, "flags")?;
    let flags = cx.to_string(flags)?;
    units(cx, flags)
}

/// Whether the flags text contains the letter `c`.
pub(super) fn has(flags: &[u16], c: u8) -> bool {
    flags.contains(&u16::from(c))
}

/// ? ToLength(? Get(rx, "lastIndex")).
pub(super) fn last_index<'s>(cx: &mut Cx<'s>, rx: Local<'s, JsObject>) -> JsResult<u64> {
    let v = cx.get(rx, "lastIndex")?;
    cx.to_length(v)
}

/// The step after an empty match of a global loop (@@match 6.e.iii, @@replace 12.c.iii):
/// lastIndex = AdvanceStringIndex(S, ToLength(Get(rx, "lastIndex")), fullUnicode).
fn step_past_empty<'s>(
    cx: &mut Cx<'s>,
    rx: Local<'s, JsObject>,
    s: Local<'s, JsString>,
    full_unicode: bool,
) -> JsResult<()> {
    let this_index = last_index(cx, rx)?;
    let next_index = advance_string_index(cx, s, this_index, full_unicode)?;
    set_last_index(cx, rx, next_index)
}

/// RegExp.prototype [ %Symbol.match% ] ( string ) (§22.2.6.8).
pub(super) fn match_<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let rx be the this value.
    // 2. If rx is not an Object, throw a TypeError exception.
    let rx = this_object(cx, this, "match")?;
    // 3. Let S be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    // 4. Let flags be ? ToString(? Get(rx, "flags")).
    let flags = flags_of(cx, rx)?;
    // 5. If flags does not contain "g", then
    if !has(&flags, b'g') {
        //    a. Return ? RegExpExec(rx, S).
        let r = exec::regexp_exec(cx, rx, s)?;
        return r.to_value(cx, rx, s);
    }
    // 6. Else,
    //    a. If flags contains "u" or flags contains "v", let fullUnicode be true. Otherwise,
    //       let fullUnicode be false.
    let full_unicode = has(&flags, b'u') || has(&flags, b'v');
    //    b. Perform ? Set(rx, "lastIndex", +0𝔽, true).
    set_last_index(cx, rx, 0)?;
    //    c. Let A be ! ArrayCreate(0).
    let a = cx.new_array();
    //    d. Let n be 0.
    let mut n = 0u64;
    //    e. Repeat,
    loop {
        // (one JS call or one string per pass, and no back edge)
        cx.poll_interrupt()?;
        //   i. Let result be ? RegExpExec(rx, S).
        let result = exec::regexp_exec(cx, rx, s)?;
        //   ii. If result is null, then
        //       1. If n = 0, return null.
        //       2. Return A.
        if matches!(result, Exec::Null) {
            return Ok(if n == 0 { Local::null() } else { a.value() });
        }
        //   iii. Else,
        //       1. Let matchStr be ? ToString(? Get(result, "0")).
        let match_str = result.matched(cx, s)?;
        //       2. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(n)), matchStr).
        cx.create_data_property_index_or_throw(a, n, match_str)?;
        //       3. If matchStr is the empty String, then
        //          a. Let thisIndex be ℝ(? ToLength(? Get(rx, "lastIndex"))).
        //          b. Let nextIndex be AdvanceStringIndex(S, thisIndex, fullUnicode).
        //          c. Perform ? Set(rx, "lastIndex", 𝔽(nextIndex), true).
        if cx.string_len(match_str) == 0 {
            step_past_empty(cx, rx, s, full_unicode)?;
        }
        //       4. Set n to n + 1.
        n += 1;
    }
}

/// One result of the @@replace loop kept for step 15: the values of a match of the builtin
/// exec, made at once as JS values in the scratch list (so they live in the heap and count
/// against its budget, F5), or an object result of another `exec`.
enum Found {
    /// A record: its position and match length; at `at` in the scratch list the matched
    /// string, then its `n` captures (strings or undefined), then the groups object
    /// (undefined if the pattern names no group).
    Record {
        position: usize,
        match_length: usize,
        at: usize,
        n: usize,
    },
    /// The index of the result object in the scratch list.
    Object(usize),
}

/// The longest string (code units): `crate::string`'s `MAX_LENGTH` (2^29 - 24).
const MAX_STRING_UNITS: usize = (1 << 29) - 24;

/// An upper bound of the length of the GetSubstitution text of `template` in a subject of
/// `length` code units: each template code unit adds at most one subject (a `$` reference
/// is 2 or more units and adds at most the subject).
fn substitution_bound(template: &[u16], length: usize) -> usize {
    if template.contains(&u16::from(b'$')) {
        template.len().saturating_mul(length.max(1))
    } else {
        template.len()
    }
}

/// RegExp.prototype [ %Symbol.replace% ] ( string, replaceValue ) (§22.2.6.11).
///
/// Step 15 for a template (no replace function) and a match of the builtin exec runs no JS
/// and reads nothing that JS can change (GetSubstitution reads the new groups object and
/// strings), so such a result is replaced at once, in the step-12 loop, instead of being
/// kept in `results` (M5-P2, F5: no per-match memory outside the heap). Each one goes into
/// the result text in the order of the list, as step 15 would do it; the first result that
/// is not such a record (an object of another `exec`), and every result after it, takes the
/// spec's list. A replacement that could pass the string length limit also goes to the list,
/// so its RangeError comes at its step-15 time, after every exec.
pub(super) fn replace<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut replace_value = args.get(1);
    // 1. Let rx be the this value.
    // 2. If rx is not an Object, throw a TypeError exception.
    let rx = this_object(cx, this, "replace")?;
    // 3. Let S be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    // 4. Let lengthS be the length of S.
    let length_s = cx.string_len(s);
    // 5. Let functionalReplace be IsCallable(replaceValue).
    let functional_replace = cx.is_callable(replace_value);
    // 6. If functionalReplace is false, then
    //    a. Set replaceValue to ? ToString(replaceValue).
    let mut template = Vec::new();
    if !functional_replace {
        let t = cx.to_string(replace_value)?;
        template = units(cx, t)?;
        replace_value = t.value();
    }
    // 7. Let flags be ? ToString(? Get(rx, "flags")).
    let flags = flags_of(cx, rx)?;
    // 8. If flags contains "g", let global be true. Otherwise, let global be false.
    let global = has(&flags, b'g');
    // 9. If global is true, then
    //    a. If flags contains "u" or flags contains "v", let fullUnicode be true. Otherwise,
    //       let fullUnicode be false.
    //    b. Perform ? Set(rx, "lastIndex", +0𝔽, true).
    let full_unicode = has(&flags, b'u') || has(&flags, b'v');
    if global {
        set_last_index(cx, rx, 0)?;
    }
    // (The code units of S, for the replaced-at-once results; step 13 in the spec.)
    let string = units(cx, s)?;
    let bound = substitution_bound(&template, length_s);
    // 13. Let accumulatedResult be the empty String.
    let mut accumulated = Vec::new();
    // 14. Let nextSourcePosition be 0.
    let mut next_source_position = 0;
    // 10. Let results be a new empty List.
    let objects = cx.scratch_list();
    let mut results: Vec<Found> = Vec::new();
    // 11. Let done be false.
    // 12. Repeat, while done is false,
    loop {
        // (one JS call or one match per pass, and no back edge)
        cx.poll_interrupt()?;
        //  a. Let result be ? RegExpExec(rx, S).
        //  b. If result is null, set done to true.
        //  c. Else,
        //     i. Append result to results.
        let result = match exec::regexp_exec(cx, rx, s)? {
            Exec::Null => break,
            r => r,
        };
        //     ii. If global is false, set done to true.
        //     iii. Else,
        //         1. Let matchStr be ? ToString(? Get(result, "0")).
        //         2. If matchStr is the empty String, then
        //            a. Let thisIndex be ℝ(? ToLength(? Get(rx, "lastIndex"))).
        //            b. Let nextIndex be AdvanceStringIndex(S, thisIndex, fullUnicode).
        //            c. Perform ? Set(rx, "lastIndex", 𝔽(nextIndex), true).
        let empty = if global {
            match &result {
                Exec::Record(m) => Some(m.start() == m.end()),
                _ => {
                    let match_str = result.matched(cx, s)?;
                    Some(cx.string_len(match_str) == 0)
                }
            }
        } else {
            None
        };
        match result {
            Exec::Record(m)
                if !functional_replace
                    && results.is_empty()
                    && accumulated.len()
                        + (m.start().saturating_sub(next_source_position))
                        + bound
                        <= MAX_STRING_UNITS =>
            {
                // Step 15 of this result now (the doc above).
                let r = replacement_of_record(cx, &m, s, &string, &template)?;
                accumulate(cx, &mut accumulated, &mut next_source_position, &string, r)?;
            }
            Exec::Record(m) => {
                let at = objects.len();
                let (position, end) = (m.start(), m.end());
                objects.push(cx.new_string_utf16(&string[position..end])?);
                for i in 1..=m.group_count() {
                    let cap = match m.get(i) {
                        Some((from, to)) => cx.new_string_utf16(&string[from..to])?.value(),
                        None => Local::undefined(),
                    };
                    objects.push(cap);
                }
                let groups = exec::groups_object(cx, s, &m)?;
                objects.push(groups);
                results.push(Found::Record {
                    position,
                    match_length: end - position,
                    at,
                    n: m.group_count(),
                });
            }
            Exec::Object(o) => results.push(Found::Object(objects.push(o))),
            Exec::Null => unreachable!(),
        }
        match empty {
            None => break,
            Some(true) => step_past_empty(cx, rx, s, full_unicode)?,
            Some(false) => {}
        }
    }
    // 15. For each element result of results, do
    for found in &results {
        // (one JS call or one string per pass)
        cx.poll_interrupt()?;
        let replaced = match *found {
            Found::Record {
                position,
                match_length,
                at,
                n,
            } => {
                //  a-j. The values of a builtin exec's result array (its own data
                //       properties: reading them runs no JS).
                let matched = objects.get(at).as_string().expect("the matched substring");
                let named_captures = objects.get(at + 1 + n);
                let replacement = replacement_text(
                    cx,
                    matched,
                    &string,
                    s,
                    position,
                    (&objects, at + 1, n),
                    named_captures,
                    functional_replace,
                    replace_value,
                    &template,
                )?;
                Replaced {
                    position,
                    match_length,
                    replacement,
                }
            }
            Found::Object(i) => replacement_of_object(
                cx,
                objects.get(i),
                s,
                &string,
                functional_replace,
                replace_value,
                &template,
            )?,
        };
        accumulate(
            cx,
            &mut accumulated,
            &mut next_source_position,
            &string,
            replaced,
        )?;
    }
    // 16. If nextSourcePosition ≥ lengthS, return accumulatedResult.
    // 17. Return the string-concatenation of accumulatedResult and the substring of S from
    //     nextSourcePosition.
    if next_source_position < length_s {
        push_units(cx, &mut accumulated, &string[next_source_position..])?;
    }
    drop(objects);
    Ok(cx.new_string_utf16(&accumulated)?.value())
}

/// Step 15.p of @@replace for one result.
fn accumulate<'s>(
    cx: &mut Cx<'s>,
    accumulated: &mut Vec<u16>,
    next_source_position: &mut usize,
    string: &[u16],
    replaced: Replaced,
) -> JsResult<()> {
    let Replaced {
        position,
        match_length,
        replacement,
    } = replaced;
    //  p. If position ≥ nextSourcePosition, then
    //     i. NOTE: position should not normally move backwards. If it does, it is an
    //        indication of an ill-behaving RegExp subclass or use of an access triggered
    //        side-effect to change the global flag or other characteristics of rx. In
    //        such cases, the corresponding substitution is ignored.
    //     ii. Set accumulatedResult to the string-concatenation of accumulatedResult, the
    //         substring of S from nextSourcePosition to position, and replacement.
    //     iii. Set nextSourcePosition to position + matchLength.
    if position >= *next_source_position {
        push_units(cx, accumulated, &string[*next_source_position..position])?;
        push_units(cx, accumulated, &replacement)?;
        *next_source_position = position + match_length;
    }
    Ok(())
}

/// The part of step 15 of @@replace that depends on one result: its position, the length of
/// its match and its replacement text.
struct Replaced {
    position: usize,
    match_length: usize,
    replacement: Vec<u16>,
}

/// Steps 15.a-o of @@replace for a result object `result` (the spec's Gets, in order).
fn replacement_of_object<'s>(
    cx: &mut Cx<'s>,
    result: Local<'s>,
    s: Local<'s, JsString>,
    string: &[u16],
    functional_replace: bool,
    replace_value: Local<'s>,
    template: &[u16],
) -> JsResult<Replaced> {
    let result = result.as_object().expect("an object result");
    //  a. Let resultLength be ? LengthOfArrayLike(result).
    let result_length = length_of_array_like(cx, result)?;
    //  b. Let nCaptures be max(resultLength - 1, 0).
    let n_captures = result_length.saturating_sub(1);
    //  c. Let matched be ? ToString(? Get(result, "0")).
    let matched = cx.get_index(result, 0)?;
    let matched = cx.to_string(matched)?;
    //  d. Let matchLength be the length of matched.
    let match_length = cx.string_len(matched);
    //  e. Let position be ? ToIntegerOrInfinity(? Get(result, "index")).
    let position = cx.get(result, "index")?;
    let position = cx.to_integer_or_infinity(position)?;
    //  f. Set position to the result of clamping position between 0 and lengthS.
    let position = position.clamp(0.0, string.len() as f64) as usize;
    //  g. Let captures be a new empty List.
    let captures = cx.scratch_list();
    //  h. Let n be 1.
    //  i. Repeat, while n ≤ nCaptures,
    let mut n = 1;
    while n <= n_captures {
        // (one Get per pass, and no back edge)
        cx.poll_interrupt()?;
        //   i. Let capN be ? Get(result, ! ToString(𝔽(n))).
        let mut cap_n = cx.get_index(result, n)?;
        //   ii. If capN is not undefined, then
        //       1. Set capN to ? ToString(capN).
        if !cap_n.is_undefined() {
            cap_n = cx.to_string(cap_n)?.value();
        }
        //   iii. Append capN to captures.
        captures.push(cap_n);
        //   iv. NOTE: When n = 1, the preceding step puts the first element into captures (at
        //       index 0). More generally, the nth capture (the first capture having index 1)
        //       is put at index n - 1.
        //   v. Set n to n + 1.
        n += 1;
    }
    //  j. Let namedCaptures be ? Get(result, "groups").
    let named_captures = cx.get(result, "groups")?;
    let count = captures.len();
    let replacement = replacement_text(
        cx,
        matched,
        string,
        s,
        position,
        (&captures, 0, count),
        named_captures,
        functional_replace,
        replace_value,
        template,
    )?;
    drop(captures);
    Ok(Replaced {
        position,
        match_length,
        replacement,
    })
}

/// Steps 15.a-o of @@replace for a match record of the builtin exec and a template (no
/// replace function): no JS can see the result array, so its values come from the record.
fn replacement_of_record<'s>(
    cx: &mut Cx<'s>,
    m: &MatchRecord,
    s: Local<'s, JsString>,
    string: &[u16],
    template: &[u16],
) -> JsResult<Replaced> {
    let (position, end) = (m.start(), m.end());
    // Fast case: a template without "$" needs no capture values.
    if !template.contains(&u16::from(b'$')) {
        return Ok(Replaced {
            position,
            match_length: end - position,
            replacement: template.to_vec(),
        });
    }
    let matched = cx.new_string_utf16(&string[position..end])?;
    let captures = cx.scratch_list();
    for i in 1..=m.group_count() {
        let cap = match m.get(i) {
            Some((from, to)) => cx.new_string_utf16(&string[from..to])?.value(),
            None => Local::undefined(),
        };
        captures.push(cap);
    }
    let named_captures = exec::groups_object(cx, s, m)?;
    let count = captures.len();
    let replacement = replacement_text(
        cx,
        matched,
        string,
        s,
        position,
        (&captures, 0, count),
        named_captures,
        false,
        Local::undefined(),
        template,
    )?;
    drop(captures);
    Ok(Replaced {
        position,
        match_length: end - position,
        replacement,
    })
}

/// Steps 15.k-l of @@replace: the replacement of one match; `captures` are entries of a
/// scratch list (list, first, count).
#[allow(clippy::too_many_arguments)]
fn replacement_text<'s>(
    cx: &mut Cx<'s>,
    matched: Local<'s, JsString>,
    string: &[u16],
    s: Local<'s, JsString>,
    position: usize,
    (list, first, count): (&ScratchList<'s>, usize, usize),
    named_captures: Local<'s>,
    functional_replace: bool,
    replace_value: Local<'s>,
    template: &[u16],
) -> JsResult<Vec<u16>> {
    // `captures`: the entries `first..first + count` of `list`.
    let capture = |i: usize| list.get(first + i);
    //  k. If functionalReplace is true, then
    if functional_replace {
        //   i. Let replacerArgs be the list-concatenation of « matched », captures, and
        //      « 𝔽(position), S ».
        //   ii. If namedCaptures is not undefined, then
        //       1. Append namedCaptures to replacerArgs.
        let mut replacer_args = cx.args_builder();
        replacer_args.push(cx, matched)?;
        for i in 0..count {
            replacer_args.push(cx, capture(i))?;
        }
        replacer_args.push(cx, number(position as u64))?;
        replacer_args.push(cx, s)?;
        if !named_captures.is_undefined() {
            replacer_args.push(cx, named_captures)?;
        }
        //   iii. Let replValue be ? Call(replaceValue, undefined, replacerArgs).
        let repl_value = cx.call_built(replace_value, Local::undefined(), replacer_args)?;
        //   iv. Let replacement be ? ToString(replValue).
        let replacement = cx.to_string(repl_value)?;
        return units(cx, replacement);
    }
    //  l. Else,
    //     i. If namedCaptures is not undefined, then
    //        1. Set namedCaptures to ? ToObject(namedCaptures).
    let named_captures = if named_captures.is_undefined() {
        None
    } else {
        Some(cx.to_object(named_captures)?)
    };
    //     ii. Let replacement be ? GetSubstitution(matched, S, position, captures,
    //         namedCaptures, replaceValue).
    let matched = units(cx, matched)?;
    let mut caps = Vec::with_capacity(count);
    for i in 0..count {
        let c = capture(i);
        caps.push(match c.as_string() {
            Some(c) => Some(units(cx, c)?),
            None => None,
        });
    }
    let mut replacement = Vec::new();
    get_substitution(
        cx,
        &matched,
        string,
        position,
        &caps,
        named_captures,
        template,
        &mut replacement,
    )?;
    Ok(replacement)
}

/// RegExp.prototype [ %Symbol.search% ] ( string ) (§22.2.6.12).
pub(super) fn search<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let rx be the this value.
    // 2. If rx is not an Object, throw a TypeError exception.
    let rx = this_object(cx, this, "search")?;
    // 3. Let S be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    // 4. Let previousLastIndex be ? Get(rx, "lastIndex").
    let previous_last_index = cx.get(rx, "lastIndex")?;
    // 5. If SameValue(previousLastIndex, +0𝔽) is false, then
    //    a. Perform ? Set(rx, "lastIndex", +0𝔽, true).
    if !cx.same_value(previous_last_index, Local::int32(0)) {
        set_last_index(cx, rx, 0)?;
    }
    // 6. Let result be ? RegExpExec(rx, S).
    let result = exec::regexp_exec(cx, rx, s)?;
    // 7. Let currentLastIndex be ? Get(rx, "lastIndex").
    let current_last_index = cx.get(rx, "lastIndex")?;
    // 8. If SameValue(currentLastIndex, previousLastIndex) is false, then
    //    a. Perform ? Set(rx, "lastIndex", previousLastIndex, true).
    if !cx.same_value(current_last_index, previous_last_index) {
        cx.set(rx, "lastIndex", previous_last_index, true)?;
    }
    // 9. If result is null, return -1𝔽.
    // 10. Return ? Get(result, "index").
    match result {
        Exec::Null => Ok(Local::int32(-1)),
        Exec::Record(m) => Ok(number(m.start() as u64)),
        Exec::Object(o) => cx.get(o, "index"),
    }
}

/// SpeciesConstructor ( O, %RegExp% ) (§7.3.22).
pub(super) fn species_constructor<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
) -> JsResult<Local<'s>> {
    // 1. Let C be ? Get(O, "constructor").
    let c = cx.get(o, "constructor")?;
    // 2. If C is undefined, return defaultConstructor.
    if c.is_undefined() {
        return Ok(cx.intrinsic(IntrinsicId::RegExp).value());
    }
    // 3. If C is not an Object, throw a TypeError exception.
    let Some(c) = c.as_object() else {
        return Err(cx.throw_type_error("The .constructor property is not an object"));
    };
    // 4. Let S be ? Get(C, %Symbol.species%).
    let species = crate::string::well_known(cx, "species")?;
    let s = cx.get(c, species)?;
    // 5. If S is either undefined or null, return defaultConstructor.
    if s.is_undefined_or_null() {
        return Ok(cx.intrinsic(IntrinsicId::RegExp).value());
    }
    // 6. If IsConstructor(S) is true, return S.
    if cx.is_constructor(s) {
        return Ok(s);
    }
    // 7. Throw a TypeError exception.
    Err(cx.throw_type_error("object.constructor[Symbol.species] is not a constructor"))
}

/// Appends `v` at index `*length` of the array `a` (CreateDataPropertyOrThrow) and counts it;
/// true when the count reached `lim`.
fn append<'s>(
    cx: &mut Cx<'s>,
    a: Local<'s, ep9_vm::JsArray>,
    length: &mut u64,
    v: impl Into<Local<'s>>,
    lim: u64,
) -> JsResult<bool> {
    let key = index_key(cx, *length);
    cx.create_data_property_or_throw(a, key, v)?;
    *length += 1;
    Ok(*length == lim)
}

/// RegExp.prototype [ %Symbol.split% ] ( string, limit ) (§22.2.6.14).
pub(super) fn split<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let limit = args.get(1);
    // 1. Let rx be the this value.
    // 2. If rx is not an Object, throw a TypeError exception.
    let rx = this_object(cx, this, "split")?;
    // 3. Let S be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    // 4. Let C be ? SpeciesConstructor(rx, %RegExp%).
    let c = species_constructor(cx, rx)?;
    // 5. Let flags be ? ToString(? Get(rx, "flags")).
    let flags = flags_of(cx, rx)?;
    // 6. If flags contains "u" or flags contains "v", let unicodeMatching be true.
    // 7. Else, let unicodeMatching be false.
    let unicode_matching = has(&flags, b'u') || has(&flags, b'v');
    // 8. If flags contains "y", let newFlags be flags.
    // 9. Else, let newFlags be the string-concatenation of flags and "y".
    let mut new_flags = flags.clone();
    if !has(&flags, b'y') {
        new_flags.push(u16::from(b'y'));
    }
    // 10. Let splitter be ? Construct(C, « rx, newFlags »).
    let new_flags_value = cx.new_string_utf16(&new_flags)?;
    let splitter = cx.construct(c, &[rx.value(), new_flags_value.value()], c)?;
    // 11. Let A be ! ArrayCreate(0).
    let a = cx.new_array();
    // 12. Let lengthA be 0.
    let mut length_a = 0u64;
    // 13. If limit is undefined, let lim be 2^32 - 1; else let lim be ℝ(? ToUint32(limit)).
    let lim = if limit.is_undefined() {
        u64::from(u32::MAX)
    } else {
        u64::from(cx.to_uint32(limit)?)
    };
    // 14. If lim = 0, return A.
    if lim == 0 {
        return Ok(a.value());
    }
    // 15. If S is the empty String, then
    let size = cx.string_len(s);
    if size == 0 {
        //  a. Let z be ? RegExpExec(splitter, S).
        let z = exec::regexp_exec(cx, splitter, s)?;
        //  b. If z is not null, return A.
        if !matches!(z, Exec::Null) {
            return Ok(a.value());
        }
        //  c. Perform ! CreateDataPropertyOrThrow(A, "0", S).
        //  d. Return A.
        append(cx, a, &mut length_a, s, lim)?;
        return Ok(a.value());
    }
    // (fast path, module doc: the splitter is a %RegExp% made here, and exec is the builtin)
    if cx.same_value(c, cx.intrinsic(IntrinsicId::RegExp)) && is_regexp_object(cx, splitter) {
        let proto = cx.intrinsic(IntrinsicId::RegExpPrototype);
        let exec = cx.get_own_property(proto, "exec")?;
        if let Some(d) = exec
            && d.is_data()
            && d.value.is_some_and(|f| is_builtin_exec(cx, f))
        {
            return split_by_search(cx, splitter, s, unicode_matching, lim);
        }
    }
    // 16. Let size be the length of S.
    // 17. Let p be 0.
    let mut p = 0usize;
    // 18. Let q be p.
    let mut q = p;
    // 19. Repeat, while q < size,
    while q < size {
        // (one JS call per pass, and no back edge)
        cx.poll_interrupt()?;
        //  a. Perform ? Set(splitter, "lastIndex", 𝔽(q), true).
        set_last_index(cx, splitter, q as u64)?;
        //  b. Let z be ? RegExpExec(splitter, S).
        let z = exec::regexp_exec(cx, splitter, s)?;
        //  c. If z is null, then
        //     i. Set q to AdvanceStringIndex(S, q, unicodeMatching).
        if matches!(z, Exec::Null) {
            q = advance_string_index(cx, s, q as u64, unicode_matching)? as usize;
            continue;
        }
        //  d. Else,
        //     i. Let e be ℝ(? ToLength(? Get(splitter, "lastIndex"))).
        //     ii. Set e to min(e, size).
        let e = (last_index(cx, splitter)?).min(size as u64) as usize;
        //     iii. If e = p, then
        //          1. Set q to AdvanceStringIndex(S, q, unicodeMatching).
        if e == p {
            q = advance_string_index(cx, s, q as u64, unicode_matching)? as usize;
            continue;
        }
        //     iv. Else,
        //         1. Let T be the substring of S from p to q.
        //         2. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(lengthA)), T).
        //         3. Set lengthA to lengthA + 1.
        //         4. If lengthA = lim, return A.
        let t = substring(cx, s, p, q)?;
        if append(cx, a, &mut length_a, t, lim)? {
            return Ok(a.value());
        }
        //         5. Set p to e.
        p = e;
        //         6. Let numberOfCaptures be ? LengthOfArrayLike(z).
        //         7. Set numberOfCaptures to max(numberOfCaptures - 1, 0).
        //         8. Let i be 1.
        //         9. Repeat, while i ≤ numberOfCaptures,
        //            a. Let nextCapture be ? Get(z, ! ToString(𝔽(i))).
        //            b. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(lengthA)),
        //               nextCapture).
        //            c. Set i to i + 1.
        //            d. Set lengthA to lengthA + 1.
        //            e. If lengthA = lim, return A.
        match z {
            Exec::Record(m) => {
                for i in 1..=m.group_count() {
                    let cap = match m.get(i) {
                        Some((from, to)) => substring(cx, s, from, to)?.value(),
                        None => Local::undefined(),
                    };
                    if append(cx, a, &mut length_a, cap, lim)? {
                        return Ok(a.value());
                    }
                }
            }
            Exec::Object(z) => {
                let number_of_captures = length_of_array_like(cx, z)?.saturating_sub(1);
                let mut i = 1;
                while i <= number_of_captures {
                    cx.poll_interrupt()?;
                    let next_capture = cx.get_index(z, i)?;
                    if append(cx, a, &mut length_a, next_capture, lim)? {
                        return Ok(a.value());
                    }
                    i += 1;
                }
            }
            Exec::Null => unreachable!(),
        }
        //         10. Set q to p.
        q = p;
    }
    // 20. Let T be the substring of S from p to size.
    // 21. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(lengthA)), T).
    // 22. Return A.
    let t = substring(cx, s, p, size)?;
    append(cx, a, &mut length_a, t, lim)?;
    Ok(a.value())
}

/// Steps 16-22 of @@split when no JS can observe them: the splitter is a RegExp that this
/// call made with %RegExp% (so it is reachable from nowhere else), and `exec` on it is the
/// builtin exec. A sticky exec at each position q from p on fails until the first position
/// where the pattern matches, and there it gives the match a search from q finds first, so
/// one non-sticky search per piece (a second program, the same pattern and flags without
/// `y`) gives the same pieces as the position loop; the `lastIndex` writes and reads go to
/// the unreachable splitter and are left out. Each match is noted for the Annex B legacy
/// static values, as each successful sticky exec would note it.
fn split_by_search<'s>(
    cx: &mut Cx<'s>,
    splitter: Local<'s, JsObject>,
    s: Local<'s, JsString>,
    unicode_matching: bool,
    lim: u64,
) -> JsResult<Local<'s>> {
    let source = cx.slot(splitter, ObjectClass::RegExp, slot::SOURCE)?;
    let source = source.as_string().expect("[[OriginalSource]] is a string");
    let flags = cx.slot(splitter, ObjectClass::RegExp, slot::FLAGS)?;
    let flags = flags.as_string().expect("[[OriginalFlags]] is a string");
    let mut flags = units(cx, flags)?;
    flags.retain(|&u| u != u16::from(b'y'));
    let flags = cx.new_string_utf16(&flags)?;
    let proto = cx.intrinsic(IntrinsicId::RegExpPrototype);
    let searcher = cx.regexp_create(proto, source, flags)?;
    let size = cx.string_len(s);
    let a = cx.new_array();
    let mut length_a = 0u64;
    let (mut p, mut q) = (0usize, 0usize);
    while q < size {
        cx.poll_interrupt()?;
        // The first position ≥ q where the sticky exec would match, and its match.
        let Some(m) = cx.regexp_exec(searcher, s, q)? else {
            break;
        };
        // (Unicode mode starts a search inside a surrogate pair at the pair, and so does the
        // sticky exec of the spec loop, whose q stays where it was: never move q back.)
        q = m.start().max(q);
        if q >= size {
            break;
        }
        // (The spec's sticky exec succeeds here: its RegExpBuiltinExec updates the Annex B
        // legacy static values. The searcher has the splitter's realm and legacy bit.)
        cx.regexp_note_match(searcher, s, &m);
        let e = m.end().min(size);
        if e == p {
            q = advance_string_index(cx, s, q as u64, unicode_matching)? as usize;
            continue;
        }
        let t = substring(cx, s, p, q)?;
        if append(cx, a, &mut length_a, t, lim)? {
            return Ok(a.value());
        }
        p = e;
        for i in 1..=m.group_count() {
            let cap = match m.get(i) {
                Some((from, to)) => substring(cx, s, from, to)?.value(),
                None => Local::undefined(),
            };
            if append(cx, a, &mut length_a, cap, lim)? {
                return Ok(a.value());
            }
        }
        q = p;
    }
    let t = substring(cx, s, p, size)?;
    append(cx, a, &mut length_a, t, lim)?;
    Ok(a.value())
}
