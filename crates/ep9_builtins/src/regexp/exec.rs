//! RegExpBuiltinExec (§22.2.7.2), RegExpExec (§22.2.7.1), AdvanceStringIndex (§22.2.7.3) and
//! the match result.
//!
//! ```text
//! regexp_exec(R, S)  exec = Get(R, "exec")
//!   exec callable, the builtin exec, R a RegExp ──> Exec::Record (builtin_match: no array)
//!   exec callable, other ──> Call(exec, R, «S»): null ──> Exec::Null, Object ──> Exec::Object
//!   exec not callable ──> RequireInternalSlot(R, [[RegExpMatcher]]), then builtin_match
//! builtin_match(R, S)  lastIndex = ToLength(Get(R, "lastIndex")); Cx::regexp_exec from it
//!                      (sticky: only there); the lastIndex Set of a global or sticky R
//! match_array(S, m)    the result array of steps 20-33 (index, input, groups, indices)
//! ```
//!
//! The callers read an [`Exec`] in the spec's order: for an `Exec::Object` with the spec's Gets
//! ([`Exec::matched`], and the @@replace and @@split steps), for an `Exec::Record` from the
//! record: the result array of RegExpBuiltinExec is a new array whose properties are all own
//! data properties, so reading them runs no JS and the record gives the same values.

use ep9_vm::{Cx, JsArray, JsObject, JsResult, JsString, Local, MatchRecord, ObjectClass};

use super::def;

/// The internal slots of a RegExp object that this module reads (`ep9_vm` abi/slots.rs,
/// `ObjectClass::RegExp`).
pub(crate) mod slot {
    /// `[[OriginalSource]]`: the pattern text.
    pub const SOURCE: usize = 0;
    /// `[[OriginalFlags]]`: the flags text, in the order `dgimsuvy`.
    pub const FLAGS: usize = 1;
    /// `[[RegExpRecord]]`: the flag bits (int32, [`super::flag`]).
    pub const RECORD: usize = 2;
}

/// The flag bits of `[[RegExpRecord]]` (the `ep9_regexp::Flags` order: d = bit 0 ... y =
/// bit 7).
pub(crate) mod flag {
    pub const HAS_INDICES: u8 = 1 << 0;
    pub const GLOBAL: u8 = 1 << 1;
    pub const IGNORE_CASE: u8 = 1 << 2;
    pub const MULTILINE: u8 = 1 << 3;
    pub const DOT_ALL: u8 = 1 << 4;
    pub const UNICODE: u8 = 1 << 5;
    pub const UNICODE_SETS: u8 = 1 << 6;
    pub const STICKY: u8 = 1 << 7;
}

/// Whether `v` has a `[[RegExpMatcher]]` internal slot (a RegExp object).
pub(crate) fn is_regexp_object<'s>(cx: &Cx<'s>, v: impl Into<Local<'s>>) -> bool {
    cx.object_class(v) == Some(ObjectClass::RegExp)
}

/// The flag bits of the RegExp object `r` (its `[[OriginalFlags]]` as bits).
pub(crate) fn flag_bits<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>) -> JsResult<u8> {
    let bits = cx.slot(r, ObjectClass::RegExp, slot::RECORD)?;
    Ok(bits.as_int32().expect("[[RegExpRecord]] is an int32") as u8)
}

/// The code units `from..to` of `s` as a new string.
pub(crate) fn substring<'s>(
    cx: &mut Cx<'s>,
    s: Local<'s, JsString>,
    from: usize,
    to: usize,
) -> JsResult<Local<'s, JsString>> {
    let flat = cx.flatten(s)?;
    let units: Vec<u16> = (from..to)
        .map(|i| {
            cx.code_unit_at(flat, i)
                .expect("a match span is inside the string")
        })
        .collect();
    cx.new_string_utf16(&units)
}

/// `n` as a Number value (an int32 when it fits).
pub(crate) fn number<'s>(n: u64) -> Local<'s> {
    match i32::try_from(n) {
        Ok(i) => Local::int32(i),
        Err(_) => Local::number(n as f64),
    }
}

/// Set(R, "lastIndex", 𝔽(n), true).
pub(crate) fn set_last_index<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, n: u64) -> JsResult<()> {
    cx.set(r, "lastIndex", number(n), true)?;
    Ok(())
}

/// AdvanceStringIndex ( S, index, unicode ) (§22.2.7.3).
pub(crate) fn advance_string_index<'s>(
    cx: &mut Cx<'s>,
    s: Local<'s, JsString>,
    index: u64,
    unicode: bool,
) -> JsResult<u64> {
    // 1. Assert: index ≤ 2^53 - 1.
    // 2. If unicode is false, return index + 1.
    if !unicode {
        return Ok(index + 1);
    }
    // 3. Let length be the length of S.
    let length = cx.string_len(s) as u64;
    // 4. If index + 1 ≥ length, return index + 1.
    if index + 1 >= length {
        return Ok(index + 1);
    }
    // 5. Let cp be CodePointAt(S, index).
    // 6. Return index + cp.[[CodeUnitCount]].
    let flat = cx.flatten(s)?;
    let first = cx
        .code_unit_at(flat, index as usize)
        .expect("index < length");
    let second = cx.code_unit_at(flat, index as usize + 1);
    let pair =
        (0xD800..0xDC00).contains(&first) && second.is_some_and(|u| (0xDC00..0xE000).contains(&u));
    Ok(index + if pair { 2 } else { 1 })
}

/// RegExpBuiltinExec ( R, S ) (§22.2.7.2) up to the match: the `lastIndex` steps and the
/// matcher (`Cx::regexp_exec`), without the result array. `r` is a RegExp object.
pub(crate) fn builtin_match<'s>(
    cx: &mut Cx<'s>,
    r: Local<'s, JsObject>,
    s: Local<'s, JsString>,
) -> JsResult<Option<MatchRecord>> {
    // 1. Let length be the length of S.
    let length = cx.string_len(s) as u64;
    // 2. Let lastIndex be ℝ(? ToLength(! Get(R, "lastIndex"))).
    let li = cx.get(r, "lastIndex")?;
    let mut last_index = cx.to_length(li)?;
    // 3. Let flags be R.[[OriginalFlags]].
    let bits = flag_bits(cx, r)?;
    // 4. If flags contains "g", let global be true; else let global be false.
    // 5. If flags contains "y", let sticky be true; else let sticky be false.
    // 6. If flags contains "d", let hasIndices be true; else let hasIndices be false.
    let global = bits & flag::GLOBAL != 0;
    let sticky = bits & flag::STICKY != 0;
    // 7. If global is false and sticky is false, set lastIndex to 0.
    if !global && !sticky {
        last_index = 0;
    }
    // 8-12. The matcher, fullUnicode and the input are the program's (Cx::regexp_exec).
    // 13. Repeat, while matchSucceeded is false,
    //     a. If lastIndex > length, then
    //        i. If global is true or sticky is true, then
    //           1. Perform ? Set(R, "lastIndex", +0𝔽, true).
    //        ii. Return null.
    //     b-c. Let r be matcher(input, inputIndex).
    //     d. If r is failure, then
    //        i. If sticky is true, then
    //           1. Perform ? Set(R, "lastIndex", +0𝔽, true).
    //           2. Return null.
    //        ii. Set lastIndex to AdvanceStringIndex(S, lastIndex, fullUnicode).
    //     e. Else,
    //        i. Assert: r is a MatchState.
    //        ii. Set matchSucceeded to true.
    // (The backend runs the loop: the first match at or after lastIndex, or none; for a
    // sticky R only at lastIndex.)
    let found = if last_index > length {
        None
    } else {
        cx.regexp_exec(r, s, last_index as usize)?
    };
    let Some(m) = found else {
        if global || sticky {
            set_last_index(cx, r, 0)?;
        }
        return Ok(None);
    };
    // 14. Let e be r.[[EndIndex]].
    // 15. If fullUnicode is true, set e to GetStringIndex(S, e). (The record is in code
    //     units.)
    // 16. If global is true or sticky is true, then
    //     a. Perform ? Set(R, "lastIndex", 𝔽(e), true).
    if global || sticky {
        set_last_index(cx, r, m.end() as u64)?;
    }
    Ok(Some(m))
}

/// Steps 17-34 of RegExpBuiltinExec (§22.2.7.2): the result array of the match `m` of a
/// RegExp with flag bits `bits` in `s`.
pub(crate) fn match_array<'s>(
    cx: &mut Cx<'s>,
    s: Local<'s, JsString>,
    m: &MatchRecord,
    bits: u8,
) -> JsResult<Local<'s, JsArray>> {
    // 17. Let n be the number of elements in r.[[Captures]].
    let n = m.group_count();
    // 18-19. (asserts)
    // 20. Let A be ! ArrayCreate(n + 1).
    // 21. Assert: The mathematical value of A's "length" property is n + 1.
    let a = cx.new_array();
    // 22. Perform ! CreateDataPropertyOrThrow(A, "index", 𝔽(lastIndex)).
    cx.create_data_property_or_throw(a, "index", number(m.start() as u64))?;
    // 23. Perform ! CreateDataPropertyOrThrow(A, "input", S).
    cx.create_data_property_or_throw(a, "input", s)?;
    // 24. Let match be the Match Record { [[StartIndex]]: lastIndex, [[EndIndex]]: e }.
    // 25-26. (indices and groupNames: the record and its names)
    // 27. Let matchedSubstr be GetMatchString(S, match).
    let matched = substring(cx, s, m.start(), m.end())?;
    // 28. Perform ! CreateDataPropertyOrThrow(A, "0", matchedSubstr).
    cx.create_data_property_index_or_throw(a, 0, matched)?;
    // 29. If R contains any GroupName, then
    //     a. Let groups be OrdinaryObjectCreate(null).
    //     b. Let hasGroups be true.
    // 30. Else,
    //     a. Let groups be undefined.
    //     b. Let hasGroups be false.
    let groups = m
        .has_group_names()
        .then(|| cx.new_object_with_proto(Local::null()));
    // 31. Perform ! CreateDataPropertyOrThrow(A, "groups", groups).
    let groups_value = groups.map_or(Local::undefined(), |g| g.value());
    cx.create_data_property_or_throw(a, "groups", groups_value)?;
    // 32. For each integer i such that 1 ≤ i ≤ n, in ascending order, do
    let mut matched_names: Vec<&[u16]> = Vec::new();
    for i in 1..=n {
        //  a-d. Let capturedValue be undefined, or GetMatchString(S, capture i).
        let captured = match m.get(i) {
            Some((from, to)) => substring(cx, s, from, to)?.value(),
            None => Local::undefined(),
        };
        //  e. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(i)), capturedValue).
        cx.create_data_property_index_or_throw(a, i as u64, captured)?;
        //  f. If the ith capture of R was defined with a GroupName, then
        if let (Some(g), Some(name)) = (groups, m.group_name(i)) {
            //   i. Let s be the CapturingGroupName of that GroupName.
            //   ii. If matchedGroupNames contains s, then
            //       1. Assert: capturedValue is undefined.
            //       2. Append undefined to groupNames.
            //   iii. Else,
            //       1. If capturedValue is not undefined, append s to matchedGroupNames.
            //       2. NOTE: If there are multiple groups named s, groups may already have
            //          an s property at this point. However, because groups is an ordinary
            //          object whose properties are all writable data properties, the call
            //          to CreateDataPropertyOrThrow is nevertheless guaranteed to succeed.
            //       3. Perform ! CreateDataPropertyOrThrow(groups, s, capturedValue).
            //       4. Append s to groupNames.
            if !matched_names.contains(&name) {
                if !captured.is_undefined() {
                    matched_names.push(name);
                }
                let key = cx.new_string_utf16(name)?;
                cx.create_data_property_or_throw(g, key, captured)?;
            }
        }
        //  g. Else, append undefined to groupNames.
    }
    // 33. If hasIndices is true, then
    //     a. Let indicesArray be MakeMatchIndicesIndexPairArray(S, indices, groupNames,
    //        hasGroups).
    //     b. Perform ! CreateDataPropertyOrThrow(A, "indices", indicesArray).
    if bits & flag::HAS_INDICES != 0 {
        let indices = indices_array(cx, m)?;
        cx.create_data_property_or_throw(a, "indices", indices)?;
    }
    // 34. Return A.
    Ok(a)
}

/// MakeMatchIndicesIndexPairArray ( S, indices, groupNames, hasGroups ) (§22.2.7.8) for the
/// match `m`.
fn indices_array<'s>(cx: &mut Cx<'s>, m: &MatchRecord) -> JsResult<Local<'s, JsArray>> {
    // 1. Let n be the number of elements in indices.
    let n = m.group_count() + 1;
    // 2-3. (asserts)
    // 4. Let A be ! ArrayCreate(n).
    let a = cx.new_array();
    // 5. Assert: n < 2^32 - 1.
    // 6. If hasGroups is true, then
    //    a. Let groups be OrdinaryObjectCreate(null).
    // 7. Else,
    //    a. Let groups be undefined.
    let groups = m
        .has_group_names()
        .then(|| cx.new_object_with_proto(Local::null()));
    // 8. Perform ! CreateDataPropertyOrThrow(A, "groups", groups).
    let groups_value = groups.map_or(Local::undefined(), |g| g.value());
    cx.create_data_property_or_throw(a, "groups", groups_value)?;
    // 9. For each integer i such that 0 ≤ i < n, in ascending order, do
    let mut matched_names: Vec<&[u16]> = Vec::new();
    for i in 0..n {
        //  a. Let matchIndices be indices[i].
        //  b. If matchIndices is not undefined, then
        //     i. Let matchIndexPair be GetMatchIndexPair(S, matchIndices).
        //  c. Else,
        //     i. Let matchIndexPair be undefined.
        let pair = match m.get(i) {
            Some((from, to)) => {
                let list = [number(from as u64), number(to as u64)];
                cx.create_array_from_list(&list)?.value()
            }
            None => Local::undefined(),
        };
        //  d. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(i)), matchIndexPair).
        cx.create_data_property_index_or_throw(a, i as u64, pair)?;
        //  e. If i > 0 and groupNames[i - 1] is not undefined, then
        //     i. Assert: groups is not undefined.
        //     ii. Perform ! CreateDataPropertyOrThrow(groups, groupNames[i - 1],
        //         matchIndexPair).
        // (groupNames[i - 1] is undefined for a name an earlier group already matched)
        if let (Some(g), Some(name)) = (groups, m.group_name(i))
            && !matched_names.contains(&name)
        {
            if !pair.is_undefined() {
                matched_names.push(name);
            }
            let key = cx.new_string_utf16(name)?;
            cx.create_data_property_or_throw(g, key, pair)?;
        }
    }
    // 10. Return A.
    Ok(a)
}

/// The groups object of the match `m` (as [`match_array`] makes it), or undefined if the
/// pattern names no group.
pub(crate) fn groups_object<'s>(
    cx: &mut Cx<'s>,
    s: Local<'s, JsString>,
    m: &MatchRecord,
) -> JsResult<Local<'s>> {
    if !m.has_group_names() {
        return Ok(Local::undefined());
    }
    let g = cx.new_object_with_proto(Local::null());
    let mut matched_names: Vec<&[u16]> = Vec::new();
    for i in 1..=m.group_count() {
        let Some(name) = m.group_name(i) else {
            continue;
        };
        if matched_names.contains(&name) {
            continue;
        }
        let captured = match m.get(i) {
            Some((from, to)) => {
                matched_names.push(name);
                substring(cx, s, from, to)?.value()
            }
            None => Local::undefined(),
        };
        let key = cx.new_string_utf16(name)?;
        cx.create_data_property_or_throw(g, key, captured)?;
    }
    Ok(g.value())
}

/// The result of RegExpExec: null, a match of the builtin exec kept as a record (no JS can
/// see its result array, so none is made), or the object another `exec` returned.
pub(crate) enum Exec<'s> {
    Null,
    Record(MatchRecord),
    Object(Local<'s, JsObject>),
}

/// Whether `f` is the builtin `RegExp.prototype.exec` of the current realm (the exec fuse,
/// MC-w14-18).
pub(crate) fn is_builtin_exec<'s>(cx: &Cx<'s>, f: Local<'s>) -> bool {
    cx.is_builtin(f, super::MODULE.id(def::EXEC))
}

/// RegExpExec ( R, S ) (§22.2.7.1).
pub(crate) fn regexp_exec<'s>(
    cx: &mut Cx<'s>,
    r: Local<'s, JsObject>,
    s: Local<'s, JsString>,
) -> JsResult<Exec<'s>> {
    // 1. Let exec be ? Get(R, "exec").
    let exec = cx.get(r, "exec")?;
    // 2. If IsCallable(exec) is true, then
    if cx.is_callable(exec) {
        // (fast path: Call(exec, R, «S») of the builtin exec on a RegExp R is
        // RegExpBuiltinExec(R, ToString(S)), and ToString(S) is S)
        if is_builtin_exec(cx, exec) && is_regexp_object(cx, r) {
            return Ok(match builtin_match(cx, r, s)? {
                Some(m) => Exec::Record(m),
                None => Exec::Null,
            });
        }
        //    a. Let result be ? Call(exec, R, « S »).
        let result = cx.call(exec, r, &[s.value()])?;
        //    b. If result is not an Object and result is not null, throw a TypeError
        //       exception.
        //    c. Return result.
        if result.is_null() {
            return Ok(Exec::Null);
        }
        return match result.as_object() {
            Some(o) => Ok(Exec::Object(o)),
            None => Err(cx.throw_type_error("The result of RegExp exec must be an object or null")),
        };
    }
    // 3. Perform ? RequireInternalSlot(R, [[RegExpMatcher]]).
    if !is_regexp_object(cx, r) {
        return Err(cx.throw_type_error("RegExp exec method called on an incompatible receiver"));
    }
    // 4. Return ? RegExpBuiltinExec(R, S).
    Ok(match builtin_match(cx, r, s)? {
        Some(m) => Exec::Record(m),
        None => Exec::Null,
    })
}

impl<'s> Exec<'s> {
    /// The match as a JS value (null, or the result array; a record makes its array now).
    pub(crate) fn to_value(
        &self,
        cx: &mut Cx<'s>,
        r: Local<'s, JsObject>,
        s: Local<'s, JsString>,
    ) -> JsResult<Local<'s>> {
        match self {
            Exec::Null => Ok(Local::null()),
            Exec::Object(o) => Ok(o.value()),
            Exec::Record(m) => {
                let bits = flag_bits(cx, r)?;
                Ok(match_array(cx, s, m, bits)?.value())
            }
        }
    }

    /// ? ToString(? Get(result, "0")).
    pub(crate) fn matched(
        &self,
        cx: &mut Cx<'s>,
        s: Local<'s, JsString>,
    ) -> JsResult<Local<'s, JsString>> {
        match self {
            Exec::Null => unreachable!("no match"),
            Exec::Record(m) => substring(cx, s, m.start(), m.end()),
            Exec::Object(o) => {
                let v = cx.get_index(*o, 0)?;
                cx.to_string(v)
            }
        }
    }
}
