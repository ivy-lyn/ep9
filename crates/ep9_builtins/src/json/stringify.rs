//! JSON.stringify ( value [ , replacer [ , space ] ] ) (§25.5.2) with SerializeJSONProperty
//! (§25.5.2.2), QuoteJSONString (§25.5.2.3), SerializeJSONObject (§25.5.2.5) and
//! SerializeJSONArray (§25.5.2.6).
//!
//! ```text
//! stringify   replacer: a function, or an array -> PropertyList (a ScratchList of keys)
//!             space -> gap (at most 10 code units)
//!             SerializeJSONProperty(wrapper, "") ─┐
//!   property    Get, toJSON, the replacer, unwrap ├─ object -> serialize_object ─┐
//!               primitives -> the text            └─ array  -> serialize_array  ─┴─ property
//! state       out: the text so far (Vec<u16>, no Values), indent depth, the cycle stack
//!             (a ScratchList: the objects being serialized, innermost last)
//! ```
//!
//! The text is written into one buffer as it is made, not as the spec's list of partial
//! strings: a member's `"key":` is written after its value is resolved (Get, toJSON, the
//! replacer) and only when the value has a text. A text longer than a string can be is not
//! an error at once: the buffer is dropped, the walk goes on (user code still runs, and its
//! throws and the cycle TypeError come first), and the RangeError is thrown at the end, as
//! V8 does. Each object or array level checks the native stack (a RangeError at the limit,
//! as in V8) and every loop over properties polls.

use ep9_vm::{Args, Cx, JsKey, JsObject, JsResult, Local, ScratchList};

use crate::array::{index_key, length_of_array_like};

use super::{MAX_DEPTH, enumerable_own_keys, primitive_data, too_deep};

/// The longest string (2^29 - 24 code units, the VM's limit): a longer text is a RangeError.
/// The unit tests use 64, so that they reach the limit fast.
const MAX_STRING_LENGTH: usize = if cfg!(test) { 64 } else { (1 << 29) - 24 };

/// The JSON Serialization Record (§25.5.2.1) and the text being made.
struct State<'s, 'l> {
    /// `[[ReplacerFunction]]`: a callable or undefined.
    replacer_function: Local<'s>,
    /// `[[PropertyList]]`: the keys of an array replacer (entries `0..n` of the list), or
    /// `None`. (`ScratchList::len` of an outer list also counts the entries of the lists
    /// made after it, so the count is kept here.)
    property_list: Option<(&'l ScratchList<'s>, usize)>,
    /// `[[Stack]]`: the objects being serialized (entries `0..depth`; the entries from
    /// `depth` to `stack_len` are stale).
    stack: &'l ScratchList<'s>,
    /// The entries `stack` holds.
    stack_len: usize,
    /// The number of objects on `stack`; `[[Indent]]` is `gap` repeated `depth` times.
    depth: usize,
    /// `[[Gap]]`.
    gap: Vec<u16>,
    /// The key `toJSON`.
    to_json: Local<'s, JsKey>,
    /// The text.
    out: Vec<u16>,
    /// The text grew longer than MAX_STRING_LENGTH (`out` is then dropped). It stays set;
    /// `stringify` throws the RangeError at the end.
    overflow: bool,
}

/// JSON.stringify ( value [ , replacer [ , space ] ] ) (§25.5.2).
pub(super) fn stringify<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (value, replacer) = (args.get(0), args.get(1));
    let mut space = args.get(2);
    // 1. Let stack be a new empty List. 2. Let indent be the empty String.
    // 3. Let PropertyList be undefined. 4. Let ReplacerFunction be undefined.
    let mut replacer_function = Local::undefined();
    let property_list = cx.scratch_list();
    let mut property_count = 0;
    let mut has_property_list = false;
    // 5. If replacer is an Object, then
    if let Some(r) = replacer.as_object() {
        if r.is_callable() {
            // a. If IsCallable(replacer) is true, set ReplacerFunction to replacer.
            replacer_function = r.value();
        } else if cx.is_array(r) {
            // b. Else, let isArray be ? IsArray(replacer); if isArray is true, then
            //    i. Set PropertyList to a new empty List.
            has_property_list = true;
            //    ii. Let len be ? LengthOfArrayLike(replacer).
            let len = length_of_array_like(cx, r)?;
            //    iii. Let k be 0. iv. Repeat, while k < len,
            for k in 0..len {
                cx.poll_interrupt()?;
                // 1. Let prop be ! ToString(𝔽(k)). 2. Let v be ? Get(replacer, prop).
                let v = cx.get_index(r, k)?;
                // 3. Let item be undefined.
                let item = if v.is_string() {
                    // 4. If v is a String, set item to v.
                    Some(cx.to_property_key(v)?)
                } else if v.is_number() {
                    // 5. Else if v is a Number, set item to ! ToString(v).
                    Some(cx.to_property_key(v)?)
                } else if let Some(o) = v.as_object() {
                    // 6. Else if v is an Object, then: if v has a [[StringData]] or
                    //    [[NumberData]] internal slot, set item to ? ToString(v).
                    match primitive_data(cx, o)? {
                        Some(p) if p.is_string() || p.is_number() => {
                            let s = cx.to_string(o)?;
                            Some(cx.intern(s))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                // 7. If item is not undefined and PropertyList does not contain item, then
                //    append item to PropertyList.
                if let Some(item) = item {
                    let present =
                        (0..property_count).any(|i| property_list.get(i).is_same_bits(&item));
                    if !present {
                        property_list.push(item);
                        property_count += 1;
                    }
                }
            }
        }
    }
    // 6. If space is an Object, then
    if let Some(o) = space.as_object() {
        match primitive_data(cx, o)? {
            // a. If space has a [[NumberData]] internal slot, set space to ? ToNumber(space).
            Some(p) if p.is_number() => space = Local::number(cx.to_number(o)?),
            // b. Else if space has a [[StringData]] internal slot, set space to
            //    ? ToString(space).
            Some(p) if p.is_string() => space = cx.to_string(o)?.value(),
            _ => {}
        }
    }
    // 7. If space is a Number, then
    let gap: Vec<u16> = if space.is_number() {
        // a. Let spaceMV be ! ToIntegerOrInfinity(space). b. Set spaceMV to min(10, spaceMV).
        let space_mv = cx.to_integer_or_infinity(space)?.min(10.0);
        // c. If spaceMV < 1, let gap be the empty String; else let gap be the String of
        //    spaceMV code units 0x0020 (SPACE).
        if space_mv < 1.0 {
            Vec::new()
        } else {
            vec![0x20; space_mv as usize]
        }
    } else if let Some(s) = space.as_string() {
        // 8. Else if space is a String, then: if the length of space ≤ 10, let gap be space;
        //    else let gap be the substring of space from 0 to 10.
        let len = cx.string_len(s).min(10);
        let flat = cx.flatten(s)?;
        (0..len)
            .map(|i| cx.code_unit_at(flat, i).expect("i < length"))
            .collect()
    } else {
        // 9. Else, let gap be the empty String.
        Vec::new()
    };
    // 10. Let wrapper be OrdinaryObjectCreate(%Object.prototype%).
    let wrapper = cx.new_object();
    // 11. Perform ! CreateDataPropertyOrThrow(wrapper, the empty String, value).
    let empty = cx.key("");
    cx.create_data_property_or_throw(wrapper, empty, value)?;
    // 12. Let state be the JSON Serialization Record { [[ReplacerFunction]]: ReplacerFunction,
    //     [[Stack]]: stack, [[Indent]]: indent, [[Gap]]: gap, [[PropertyList]]: PropertyList }.
    // The stack list comes after the property list (both LIFO), and only the stack gets
    // pushes from here on.
    let stack = cx.scratch_list();
    let to_json = cx.key("toJSON");
    let mut state = State {
        replacer_function,
        property_list: has_property_list.then_some((&property_list, property_count)),
        stack: &stack,
        stack_len: 0,
        depth: 0,
        gap,
        to_json,
        out: Vec::new(),
        overflow: false,
    };
    // 13. Return ? SerializeJSONProperty(state, the empty String, wrapper).
    if !serialize_property(cx, &mut state, Name::Key(empty), wrapper)? {
        return Ok(Local::undefined());
    }
    if state.overflow {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    Ok(cx.new_string_utf16(&state.out)?.value())
}

/// The name of a property: a key, or an array index (made into a key only when JS sees it).
#[derive(Clone, Copy)]
enum Name<'s> {
    Key(Local<'s, JsKey>),
    Index(u64),
}

impl<'s> Name<'s> {
    fn key(self, cx: &mut Cx<'s>) -> Local<'s, JsKey> {
        match self {
            Name::Key(k) => k,
            Name::Index(i) => index_key(cx, i),
        }
    }
}

/// SerializeJSONProperty ( state, key, holder ) (§25.5.2.2): appends the text of the value
/// and gives true, or appends nothing and gives false (the spec's undefined).
fn serialize_property<'s>(
    cx: &mut Cx<'s>,
    state: &mut State<'s, '_>,
    key: Name<'s>,
    holder: Local<'s, JsObject>,
) -> JsResult<bool> {
    let value = property_value(cx, state, key, holder)?;
    write_value(cx, state, value)
}

/// SerializeJSONProperty steps 1-4: the value after Get, toJSON, the replacer and the
/// unwrap of a Number, String or Boolean object.
fn property_value<'s>(
    cx: &mut Cx<'s>,
    state: &mut State<'s, '_>,
    key: Name<'s>,
    holder: Local<'s, JsObject>,
) -> JsResult<Local<'s>> {
    // 1. Let value be ? Get(holder, key).
    let mut value = match key {
        Name::Key(k) => cx.get(holder, k)?,
        Name::Index(i) => cx.get_index(holder, i)?,
    };
    // 2. If value is an Object or value is a BigInt, then (v0: no BigInt)
    if value.is_object() {
        // a. Let toJSON be ? GetV(value, "toJSON").
        let to_json = cx.get_v(value, state.to_json)?;
        // b. If IsCallable(toJSON) is true, then set value to ? Call(toJSON, value, « key »).
        if to_json.is_callable() {
            let k = key.key(cx);
            value = cx.call(to_json, value, &[k.into()])?;
        }
    }
    // 3. If state.[[ReplacerFunction]] is not undefined, then set value to
    //    ? Call(state.[[ReplacerFunction]], holder, « key, value »).
    if !state.replacer_function.is_undefined() {
        let k = key.key(cx);
        value = cx.call(state.replacer_function, holder, &[k.into(), value])?;
    }
    // 4. If value is an Object, then
    if let Some(o) = value.as_object()
        && let Some(p) = primitive_data(cx, o)?
    {
        if p.is_number() {
            // a. If value has a [[NumberData]] internal slot, set value to ? ToNumber(value).
            value = Local::number(cx.to_number(o)?);
        } else if p.is_string() {
            // b. Else if value has a [[StringData]] internal slot, set value to
            //    ? ToString(value).
            value = cx.to_string(o)?.value();
        } else {
            // c. Else if value has a [[BooleanData]] internal slot, set value to
            //    value.[[BooleanData]].
            // d. (BigInt: not in v0.)
            value = p;
        }
    }
    Ok(value)
}

/// True if SerializeJSONProperty steps 5-12 give a text for `value` (from
/// `property_value`), not undefined: what `write_value` gives true for.
fn has_text(value: Local<'_>) -> bool {
    value.is_null()
        || value.as_bool().is_some()
        || value.is_string()
        || value.is_number()
        || value.as_object().is_some_and(|o| !o.is_callable())
}

/// SerializeJSONProperty steps 5-12: appends the text of `value` (from `property_value`)
/// and gives true, or appends nothing and gives false (the spec's undefined).
fn write_value<'s>(cx: &mut Cx<'s>, state: &mut State<'s, '_>, value: Local<'s>) -> JsResult<bool> {
    // 5. If value is null, return "null".
    if value.is_null() {
        push_ascii(cx, state, "null")?;
        return Ok(true);
    }
    // 6. If value is true, return "true". 7. If value is false, return "false".
    if let Some(b) = value.as_bool() {
        push_ascii(cx, state, if b { "true" } else { "false" })?;
        return Ok(true);
    }
    // 8. If value is a String, return QuoteJSONString(value).
    if let Some(s) = value.as_string() {
        quote(cx, state, s)?;
        return Ok(true);
    }
    // 9. If value is a Number, then
    if let Some(n) = value.as_number() {
        // a. If value is finite, return ! ToString(value). b. Return "null".
        if n.is_finite() {
            let s = cx.to_string(value)?;
            let len = cx.string_len(s);
            let flat = cx.flatten(s)?;
            for i in 0..len {
                let u = cx.code_unit_at(flat, i).expect("i < length");
                state.out.push(u);
            }
        } else {
            push_ascii(cx, state, "null")?;
        }
        return Ok(true);
    }
    // 10. (BigInt: a TypeError; not in v0.)
    // 11. If value is an Object and IsCallable(value) is false, then
    if let Some(o) = value.as_object()
        && !o.is_callable()
    {
        // a. Let isArray be ? IsArray(value).
        // b. If isArray is true, return ? SerializeJSONArray(state, value).
        // c. Return ? SerializeJSONObject(state, value).
        if cx.is_array(o) {
            serialize_array(cx, state, o)?;
        } else {
            serialize_object(cx, state, o)?;
        }
        return Ok(true);
    }
    // 12. Return undefined.
    Ok(false)
}

/// The cycle check and push of SerializeJSONObject and SerializeJSONArray steps 1-2, after
/// the native stack check: a TypeError if `value` is already on the stack.
fn enter<'s>(
    cx: &mut Cx<'s>,
    state: &mut State<'s, '_>,
    value: Local<'s, JsObject>,
) -> JsResult<()> {
    cx.check_native_stack()?;
    if state.depth >= MAX_DEPTH {
        return Err(too_deep(cx));
    }
    // 1. If state.[[Stack]] contains value, throw a TypeError exception because the
    //    structure is cyclical.
    if (0..state.depth).any(|i| state.stack.get(i).is_same_bits(&value)) {
        return Err(cx.throw_type_error("Converting circular structure to JSON"));
    }
    // 2. Append value to state.[[Stack]].
    if state.depth < state.stack_len {
        state.stack.set(state.depth, value);
    } else {
        state.stack.push(value);
        state.stack_len += 1;
    }
    state.depth += 1;
    Ok(())
}

/// A newline and `[[Indent]]` (the gap `depth` times), for a non-empty gap.
fn newline_indent<'s>(cx: &mut Cx<'s>, state: &mut State<'s, '_>, depth: usize) -> JsResult<()> {
    state.out.push(0x0A);
    for _ in 0..depth {
        let gap = core::mem::take(&mut state.gap);
        let r = push_units(cx, state, &gap);
        state.gap = gap;
        r?;
    }
    Ok(())
}

/// SerializeJSONObject ( state, value ) (§25.5.2.5).
fn serialize_object<'s>(
    cx: &mut Cx<'s>,
    state: &mut State<'s, '_>,
    value: Local<'s, JsObject>,
) -> JsResult<()> {
    // 1-2. The cycle check; append value to state.[[Stack]].
    enter(cx, state, value)?;
    // 3. Let stepback be state.[[Indent]].
    // 4. Set state.[[Indent]] to the string-concatenation of state.[[Indent]] and
    //    state.[[Gap]]. (The depth is the indent.)
    let indent = state.depth;
    // 5. If state.[[PropertyList]] is not undefined, let K be state.[[PropertyList]].
    // 6. Else, let K be ? EnumerableOwnProperties(value, key).
    let own = match state.property_list {
        Some(_) => None,
        None => Some(enumerable_own_keys(cx, value)?),
    };
    let count = match (own, state.property_list) {
        (Some((_, n)), _) => n,
        (None, Some((_, n))) => n as u64,
        (None, None) => unreachable!("K is one of the two"),
    };
    // 7. Let partial be a new empty List.
    state.out.push(0x7B);
    let mut any = false;
    // 8. For each element P of K, do
    for i in 0..count {
        cx.poll_interrupt()?;
        let p = match (own, state.property_list) {
            (Some((keys, _)), _) => {
                let p = cx.get_index(keys, i)?;
                cx.to_property_key(p)?
            }
            (None, Some((list, _))) => {
                let p = list.get(i as usize);
                cx.to_property_key(p)?
            }
            (None, None) => unreachable!("K is one of the two"),
        };
        // a. Let strP be ? SerializeJSONProperty(state, P, value).
        // b. If strP is not undefined, then
        //    i. Let member be QuoteJSONString(P).
        //    ii. Set member to the string-concatenation of member and ":".
        //    iii. If state.[[Gap]] is not the empty String, set member to the
        //         string-concatenation of member and the code unit 0x0020 (SPACE).
        //    iv. Set member to the string-concatenation of member and strP.
        //    v. Append member to partial.
        // The value is resolved first (steps 1-4 of strP: the user code); then, if strP is
        // not undefined, the separator of steps 10-11, the member's key and strP's text go
        // into the buffer.
        let v = property_value(cx, state, Name::Key(p), value)?;
        if !has_text(v) {
            continue;
        }
        if any {
            state.out.push(0x2C);
        }
        if !state.gap.is_empty() {
            newline_indent(cx, state, indent)?;
        }
        let s = p.value().as_string().expect("a key of K is a string");
        quote(cx, state, s)?;
        state.out.push(0x3A);
        if !state.gap.is_empty() {
            state.out.push(0x20);
        }
        let wrote = write_value(cx, state, v)?;
        debug_assert!(wrote, "has_text and write_value agree");
        any = true;
    }
    // 9. If partial is empty, let final be "{}".
    // 10. Else if state.[[Gap]] is the empty String: "{" + the members joined by "," + "}".
    // 11. Else: "{" + LF + indent + the members joined by "," + LF + indent + LF + stepback
    //     + "}".
    if any && !state.gap.is_empty() {
        newline_indent(cx, state, indent - 1)?;
    }
    state.out.push(0x7D);
    // 12. Remove the last element of state.[[Stack]]. 13. Set state.[[Indent]] to stepback.
    state.depth -= 1;
    check_length(cx, state)
}

/// SerializeJSONArray ( state, value ) (§25.5.2.6).
fn serialize_array<'s>(
    cx: &mut Cx<'s>,
    state: &mut State<'s, '_>,
    value: Local<'s, JsObject>,
) -> JsResult<()> {
    // 1-2. The cycle check; append value to state.[[Stack]].
    enter(cx, state, value)?;
    // 3-4. stepback and the new indent (the depth).
    let indent = state.depth;
    // 5. Let partial be a new empty List.
    // 6. Let len be ? LengthOfArrayLike(value).
    let len = length_of_array_like(cx, value)?;
    state.out.push(0x5B);
    // 7. Let index be 0. 8. Repeat, while index < len,
    for index in 0..len {
        cx.poll_interrupt()?;
        if index > 0 {
            state.out.push(0x2C);
        }
        if !state.gap.is_empty() {
            newline_indent(cx, state, indent)?;
        }
        // a. Let strP be ? SerializeJSONProperty(state, ! ToString(𝔽(index)), value).
        // b. If strP is undefined, append "null" to partial. c. Else, append strP.
        if !serialize_property(cx, state, Name::Index(index), value)? {
            push_ascii(cx, state, "null")?;
        }
        check_length(cx, state)?;
    }
    // 9. If partial is empty, let final be "[]".
    // 10. Else if state.[[Gap]] is the empty String: "[" + the elements joined by "," + "]".
    // 11. Else: "[" + LF + indent + the elements joined by ("," + LF + indent) + LF +
    //     stepback + "]".
    if len > 0 && !state.gap.is_empty() {
        newline_indent(cx, state, indent - 1)?;
    }
    state.out.push(0x5D);
    // 12. Remove the last element of state.[[Stack]]. 13. Set state.[[Indent]] to stepback.
    state.depth -= 1;
    check_length(cx, state)
}

/// QuoteJSONString ( value ) (§25.5.2.3), appended to the text.
fn quote<'s>(
    cx: &mut Cx<'s>,
    state: &mut State<'s, '_>,
    value: Local<'s, ep9_vm::JsString>,
) -> JsResult<()> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // The text is already too long: nothing to add (and no long copy of a long string).
    if state.overflow {
        return Ok(());
    }
    let len = cx.string_len(value);
    let flat = cx.flatten(value)?;
    let out = &mut state.out;
    // 1. Let product be the String value consisting solely of the code unit 0x0022
    //    (QUOTATION MARK).
    out.push(0x22);
    // 2. For each code point C of StringToCodePoints(value), do
    let mut i = 0;
    while i < len {
        let c = cx.code_unit_at(flat, i).expect("i < length");
        i += 1;
        // a. If C is listed in the "Code Point" column of Table 77, use its escape.
        let escape = match c {
            0x08 => Some(b'b'),
            0x09 => Some(b't'),
            0x0A => Some(b'n'),
            0x0C => Some(b'f'),
            0x0D => Some(b'r'),
            0x22 => Some(b'"'),
            0x5C => Some(b'\\'),
            _ => None,
        };
        if let Some(e) = escape {
            out.extend_from_slice(&[0x5C, u16::from(e)]);
            continue;
        }
        // A surrogate pair is one code point: copied as it is.
        if (0xD800..0xDC00).contains(&c)
            && let Some(next) = cx.code_unit_at(flat, i)
            && (0xDC00..0xE000).contains(&next)
        {
            out.extend_from_slice(&[c, next]);
            i += 1;
            continue;
        }
        // b. Else if C has a numeric value less than 0x0020 (SPACE) or C has the same
        //    numeric value as a leading surrogate or trailing surrogate, then
        //    UnicodeEscape(C): "\u" and 4 lowercase hex digits.
        if c < 0x20 || (0xD800..0xE000).contains(&c) {
            out.extend_from_slice(&[0x5C, u16::from(b'u')]);
            for shift in [12, 8, 4, 0] {
                out.push(u16::from(HEX[usize::from((c >> shift) & 0xF)]));
            }
            continue;
        }
        // c. Else, append UTF16EncodeCodePoint(C).
        out.push(c);
    }
    // 3. Set product to the string-concatenation of product and the code unit 0x0022.
    out.push(0x22);
    // 4. Return product.
    check_length(cx, state)
}

/// Appends ASCII text.
fn push_ascii<'s>(cx: &mut Cx<'s>, state: &mut State<'s, '_>, text: &str) -> JsResult<()> {
    state.out.extend(text.bytes().map(u16::from));
    check_length(cx, state)
}

/// Appends code units.
fn push_units<'s>(cx: &mut Cx<'s>, state: &mut State<'s, '_>, units: &[u16]) -> JsResult<()> {
    state.out.extend_from_slice(units);
    check_length(cx, state)
}

/// Marks the overflow once the text is longer than a string can be, and drops the text;
/// `stringify` throws the RangeError after the walk (V8 also runs the rest of the user code
/// first, so its throws and the cycle TypeError win).
fn check_length<'s>(_cx: &mut Cx<'s>, state: &mut State<'s, '_>) -> JsResult<()> {
    if state.out.len() > MAX_STRING_LENGTH {
        state.overflow = true;
        state.out = Vec::new();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
    use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};

    thread_local! {
        static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn print<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        let mut parts = Vec::new();
        for v in args.iter() {
            parts.push(cx.to_rust_string(v)?);
        }
        OUT.with(|o| o.borrow_mut().push(parts.join(" ")));
        Ok(Local::undefined())
    }

    /// X-JSON2: a key that alone makes the text too long (here MAX_STRING_LENGTH is 64; the
    /// node v24.22.1 results are for a key of 2^29 - 25 units). The key is not written before
    /// its value is resolved, and the RangeError comes after the rest of the user code: an
    /// undefined or function value gives "{}", a throwing getter or replacer throws first, a
    /// later throw or a cycle wins over the overflow, and only the overflow alone is a
    /// RangeError.
    #[test]
    fn output_limit_comes_after_user_code() {
        let src = r#"
var k = 'a'.repeat(63);
function t(f) { try { return String(f()); } catch (e) { return e === 'S' ? 'sentinel' : e.name; } }
var o1 = {}; o1[k] = undefined;
var o2 = {}; Object.defineProperty(o2, k, { get: function () { throw 'S'; }, enumerable: true });
var big = {}; big[k] = 1;
var o3 = {}; o3[k] = function () {};
print(t(function () { return JSON.stringify(o1); }), t(function () { return JSON.stringify(o2); }), t(function () { return JSON.stringify(o3); }));
print(t(function () { return JSON.stringify(big, function (key, v) { return key === k ? undefined : v; }); }));
print(t(function () { return JSON.stringify(big, function (key, v) { if (key === k) throw 'S'; return v; }); }));
print(t(function () { return JSON.stringify([big, { get x() { throw 'S'; } }]); }));
var nested = {}; nested[k] = { get x() { throw 'S'; } };
print(t(function () { return JSON.stringify(nested); }));
var cyc = [big]; cyc.push(cyc);
print(t(function () { return JSON.stringify(cyc); }), t(function () { return JSON.stringify([big]); }), t(function () { return JSON.stringify(k + 'aa'); }));
print(JSON.stringify({ a: 1, b: undefined, c: 'x' }, null, 1).split('\n').join('|'));
"#;
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .expect("compiles");
        let mut iso = Isolate::new(IsolateConfig {
            builtins: Some(&crate::TABLE),
            ..IsolateConfig::default()
        })
        .expect("an isolate");
        iso.register_host_function("print", 1, print);
        OUT.with(|o| o.borrow_mut().clear());
        iso.run_script(unit, "t.js")
            .map_err(|e| e.message)
            .expect("no uncaught exception");
        let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
        assert_eq!(
            out,
            [
                "{} sentinel {}",
                "{}",
                "sentinel",
                "sentinel",
                "sentinel",
                "TypeError RangeError RangeError",
                "{| \"a\": 1,| \"c\": \"x\"|}",
            ]
        );
    }
}
