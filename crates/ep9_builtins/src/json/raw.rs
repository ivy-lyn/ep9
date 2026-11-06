//! JSON.rawJSON ( text ) and JSON.isRawJSON ( O ) (json-parse-with-source).
//!
//! ```text
//! rawJSON(text)   ToString -> code units -> the edge checks (empty; tab, LF, CR or space
//!                 first or last) -> the JSON-text check (the JSON.parse loop; an object or
//!                 array outside is refused) -> a null-prototype object of ObjectClass::RawJson
//!                 (0 slots: the class is [[IsRawJSON]]) with `rawJSON` = the string, frozen
//! isRawJSON(O)    the class test: no property is read, a proxy is never a raw JSON object
//! ```
//!
//! JSON.stringify gives the `rawJSON` string of such an object as it is (stringify.rs).

use ep9_vm::{Args, Cx, ErrorKind, JsResult, Local, ObjectClass, PropertyDescriptor};

use super::parse::{Parser, code_units, is_ws};

/// JSON.rawJSON ( text ).
pub(super) fn raw_json<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let jsonString be ? ToString(text).
    let json_string = cx.to_string(args.get(0))?;
    let flat = cx.flatten(json_string)?;
    let text = code_units(cx, flat);
    // 2. Throw a SyntaxError exception if jsonString is the empty String, or if either the
    //    first or last code unit of jsonString is any of 0x0009 (CHARACTER TABULATION),
    //    0x000A (LINE FEED), 0x000D (CARRIAGE RETURN), or 0x0020 (SPACE).
    match (text.first(), text.last()) {
        (Some(&first), Some(&last)) if !is_ws(first) && !is_ws(last) => {}
        _ => {
            return Err(cx.throw_error(ErrorKind::SyntaxError, "Invalid value for JSON.rawJSON"));
        }
    }
    // 3. Parse StringToCodePoints(jsonString) as a JSON text as specified in ECMA-404. Throw
    //    a SyntaxError exception if it is not a valid JSON text as defined in that
    //    specification, or if its outermost value is an object or array as defined in that
    //    specification.
    if matches!(text[0], 0x7B | 0x5B) {
        let c = char::from(text[0] as u8);
        let msg = format!("Unexpected token '{c}' in JSON at position 0");
        return Err(cx.throw_error(ErrorKind::SyntaxError, &msg));
    }
    {
        let list = cx.scratch_list();
        Parser::new(&text, false).run(cx, &list)?;
    }
    // 4. Let internalSlotsList be « [[IsRawJSON]] ».
    // 5. Let obj be OrdinaryObjectCreate(null, internalSlotsList).
    let obj = cx.new_object_with_slots(Local::null(), ObjectClass::RawJson, 0);
    // 6. Perform ! CreateDataPropertyOrThrow(obj, "rawJSON", jsonString).
    // 7. Perform ! SetIntegrityLevel(obj, frozen).
    // The property is made non-writable and non-configurable at once, then the object is
    // made non-extensible: the same object SetIntegrityLevel leaves (it has no other key).
    let key = cx.key("rawJSON");
    let desc = PropertyDescriptor::data(json_string.value(), false, true, false);
    cx.define_property_or_throw(obj, key, desc)?;
    let frozen = cx.prevent_extensions(obj)?;
    debug_assert!(frozen, "a new ordinary object can be made non-extensible");
    // 8. Return obj.
    Ok(obj.value())
}

/// JSON.isRawJSON ( O ).
pub(super) fn is_raw_json<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is an Object and O has an [[IsRawJSON]] internal slot, return true.
    // 2. Return false.
    let raw = cx.object_class(args.get(0)) == Some(ObjectClass::RawJson);
    Ok(Local::boolean(raw))
}
