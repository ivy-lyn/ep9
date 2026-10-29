//! `RegExp.prototype[@@matchAll]` (§22.2.6.9) and `%RegExpStringIteratorPrototype%`
//! (§22.2.9, M5-P2 REGEXP2).
//!
//! ```text
//! rx[@@matchAll](S)  C = SpeciesConstructor(rx, %RegExp%); flags = ToString(Get(rx, "flags"));
//!                    matcher = Construct(C, «rx, flags»); matcher.lastIndex =
//!                    ToLength(Get(rx, "lastIndex")); CreateRegExpStringIterator(matcher, S,
//!                    "g" in flags, "u" or "v" in flags)
//! it.next()          [[Done]]: {undefined, true}; RegExpExec(matcher, S): null -> done;
//!                    not global -> the match once, then done; global -> the match, and
//!                    after an empty match lastIndex = AdvanceStringIndex(...)
//! ```
//!
//! The iterator is a slot object of `ObjectClass::RegExpStringIterator` (prototype
//! `%RegExpStringIteratorPrototype%`): slot 0 `[[IteratingRegExp]]`, slot 1
//! `[[IteratedString]]`, slot 2 the state bits ([`state`]). The spec writes the iterator as a
//! generator closure (CreateRegExpStringIterator); `next` is its GeneratorResume: a call while
//! the closure runs (from a user `exec`) is a TypeError, as GeneratorValidate makes it.

use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local, ObjectClass};

use super::exec::{self, Exec, advance_string_index, set_last_index};
use super::symbols::{flags_of, has, last_index, species_constructor};

/// The slots of a RegExp String Iterator.
mod slot {
    /// `[[IteratingRegExp]]`.
    pub const REGEXP: usize = 0;
    /// `[[IteratedString]]`.
    pub const STRING: usize = 1;
    /// The state bits ([`super::state`]), an int32.
    pub const STATE: usize = 2;
    /// The slot count.
    pub const COUNT: usize = 3;
}

/// The state bits of slot [`slot::STATE`].
mod state {
    /// `[[Global]]`.
    pub const GLOBAL: i32 = 1 << 0;
    /// `[[Unicode]]` (fullUnicode).
    pub const UNICODE: i32 = 1 << 1;
    /// `[[Done]]`: the closure returned.
    pub const DONE: i32 = 1 << 2;
    /// The closure runs (GeneratorState executing).
    pub const RUNNING: i32 = 1 << 3;
}

/// CreateIteratorResultObject ( value, done ) (§7.4.14).
fn iter_result<'s>(cx: &mut Cx<'s>, value: Local<'s>, done: bool) -> JsResult<Local<'s>> {
    let o = cx.new_object();
    cx.create_data_property_or_throw(o, "value", value)?;
    cx.create_data_property_or_throw(o, "done", Local::boolean(done))?;
    Ok(o.value())
}

/// RegExp.prototype [ %Symbol.matchAll% ] ( string ) (§22.2.6.9).
pub(super) fn match_all<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let R be the this value.
    // 2. If R is not an Object, throw a TypeError exception.
    let Some(r) = this.as_object() else {
        let shown = cx.display(this);
        let msg = format!("RegExp.prototype[Symbol.matchAll] called on non-object {shown}");
        return Err(cx.throw_type_error(&msg));
    };
    // 3. Let S be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    // 4. Let C be ? SpeciesConstructor(R, %RegExp%).
    let c = species_constructor(cx, r)?;
    // 5. Let flags be ? ToString(? Get(R, "flags")).
    let flags = flags_of(cx, r)?;
    // 6. Let matcher be ? Construct(C, « R, flags »).
    let flags_value = cx.new_string_utf16(&flags)?;
    let matcher = cx.construct(c, &[r.value(), flags_value.value()], c)?;
    // 7. Let lastIndex be ? ToLength(? Get(R, "lastIndex")).
    let li = last_index(cx, r)?;
    // 8. Perform ? Set(matcher, "lastIndex", lastIndex, true).
    set_last_index(cx, matcher, li)?;
    // 9. If flags contains "g", let global be true. 10. Else, let global be false.
    // 11. If flags contains "u" or flags contains "v", let fullUnicode be true.
    // 12. Else, let fullUnicode be false.
    let mut bits = 0;
    if has(&flags, b'g') {
        bits |= state::GLOBAL;
    }
    if has(&flags, b'u') || has(&flags, b'v') {
        bits |= state::UNICODE;
    }
    // 13. Return CreateRegExpStringIterator(matcher, S, global, fullUnicode).
    let proto = cx.intrinsic(IntrinsicId::RegExpStringIteratorPrototype);
    let it = cx.new_object_with_slots(proto, ObjectClass::RegExpStringIterator, slot::COUNT);
    cx.set_slot(it, ObjectClass::RegExpStringIterator, slot::REGEXP, matcher)?;
    cx.set_slot(it, ObjectClass::RegExpStringIterator, slot::STRING, s)?;
    cx.set_slot(
        it,
        ObjectClass::RegExpStringIterator,
        slot::STATE,
        Local::int32(bits),
    )?;
    Ok(it.value())
}

/// %RegExpStringIteratorPrototype%.next ( ) (§22.2.9.2.1): GeneratorResume(this value,
/// empty, "%RegExpStringIteratorPrototype%") of the closure of CreateRegExpStringIterator
/// (§22.2.9.1).
pub(super) fn next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    const CLASS: ObjectClass = ObjectClass::RegExpStringIterator;
    // GeneratorValidate: the brand (a RegExp String Iterator), and not executing.
    let it = match this.as_object() {
        Some(o) if cx.object_class(o) == Some(CLASS) => o,
        _ => {
            let shown = cx.display(this);
            let msg = format!(
                "%RegExpStringIteratorPrototype%.next called on incompatible receiver {shown}"
            );
            return Err(cx.throw_type_error(&msg));
        }
    };
    let bits = cx
        .slot(it, CLASS, slot::STATE)?
        .as_int32()
        .expect("the state is an int32");
    if bits & state::RUNNING != 0 {
        return Err(cx.throw_type_error("Generator is already running"));
    }
    // GeneratorState completed: {undefined, true}.
    if bits & state::DONE != 0 {
        return iter_result(cx, Local::undefined(), true);
    }
    let r = cx.slot(it, CLASS, slot::REGEXP)?;
    let r = r.as_object().expect("[[IteratingRegExp]] is an object");
    let s = cx.slot(it, CLASS, slot::STRING)?;
    let s = s.as_string().expect("[[IteratedString]] is a string");
    cx.set_slot(it, CLASS, slot::STATE, Local::int32(bits | state::RUNNING))?;
    let step = resume(cx, r, s, bits);
    // The closure stops at a yield, or ends (a return, a yield followed by a return, or an
    // abrupt completion: GeneratorState becomes completed).
    let ends = !matches!(step, Ok((Some(_), false)));
    let after = if ends { bits | state::DONE } else { bits };
    cx.set_slot(it, CLASS, slot::STATE, Local::int32(after))?;
    match step? {
        (Some(m), _) => iter_result(cx, m, false),
        (None, _) => iter_result(cx, Local::undefined(), true),
    }
}

/// One step of the closure of CreateRegExpStringIterator (§22.2.9.1 step 1): the value it
/// yields (`None`: it returns undefined) and whether it returns right after that yield (a
/// non-global match).
fn resume<'s>(
    cx: &mut Cx<'s>,
    r: Local<'s, ep9_vm::JsObject>,
    s: Local<'s, ep9_vm::JsString>,
    bits: i32,
) -> JsResult<(Option<Local<'s>>, bool)> {
    // a. Repeat,
    //    i. Let match be ? RegExpExec(R, S).
    let result = exec::regexp_exec(cx, r, s)?;
    //    ii. If match is null, then
    //        1. Return NormalCompletion(undefined).
    if matches!(result, Exec::Null) {
        return Ok((None, true));
    }
    let m = result.to_value(cx, r, s)?;
    //    iii. If global is false, then
    //         1. Perform ? GeneratorYield(CreateIteratorResultObject(match, false)).
    //         2. Return NormalCompletion(undefined).
    if bits & state::GLOBAL == 0 {
        return Ok((Some(m), true));
    }
    //    iv. Let matchStr be ? ToString(? Get(match, "0")).
    //    v. If matchStr is the empty String, then
    //       1. Let thisIndex be ℝ(? ToLength(? Get(R, "lastIndex"))).
    //       2. Let nextIndex be AdvanceStringIndex(S, thisIndex, fullUnicode).
    //       3. Perform ? Set(R, "lastIndex", 𝔽(nextIndex), true).
    let empty = match &result {
        Exec::Record(rec) => rec.start() == rec.end(),
        _ => {
            let match_str = result.matched(cx, s)?;
            cx.string_len(match_str) == 0
        }
    };
    if empty {
        let this_index = last_index(cx, r)?;
        let next_index = advance_string_index(cx, s, this_index, bits & state::UNICODE != 0)?;
        set_last_index(cx, r, next_index)?;
    }
    //    vi. Perform ? GeneratorYield(CreateIteratorResultObject(match, false)).
    Ok((Some(m), false))
}
