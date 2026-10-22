//! JSON (§25.5): module `json`, block 14. Owner: M3-P1 B-JSON. Makes `%JSON%`
//! (`IntrinsicId::Json`), an ordinary object whose prototype is `%Object.prototype%`, with
//! `parse` and `stringify` ({w, c}) and `@@toStringTag` "JSON" ({c}), and the global binding
//! `JSON` {w, c}.
//!
//! ```text
//! JSON.parse(text, reviver)            json/parse.rs
//!   ToString(text) -> code units -> one loop over the text (no Rust recursion):
//!     the open arrays and objects and the pending keys are on a ScratchList (precise roots),
//!     so the nesting depth is bounded by the heap, not by the native stack (as in V8)
//!   reviver callable -> InternalizeJSONProperty: recursive, Cx::check_native_stack per level
//!     (and at most MAX_DEPTH levels), Cx::poll_interrupt per property
//! JSON.stringify(value, replacer, space)  json/stringify.rs
//!   SerializeJSONProperty / SerializeJSONObject / SerializeJSONArray: recursive,
//!     Cx::check_native_stack per level and at most MAX_DEPTH levels (a RangeError at the
//!     limit, as in V8)
//!   the cycle stack is a ScratchList; the property list of an array replacer is another one
//!   the text is a Rust Vec<u16> of code units (no Values), made a string once at the end
//! ```
//!
//! | Spec step | Here |
//! |---|---|
//! | `[[NumberData]]`, `[[StringData]]`, `[[BooleanData]]` tests (SerializeJSONProperty step 4, the replacer list, `space`) | `primitive_data` |
//! | EnumerableOwnProperties(O, key) (§7.3.23) | `enumerable_own_keys`: the keys array of `[[OwnPropertyKeys]]`, compacted in place |
//! | Number::toString of a finite number | `Cx::to_string` (numconv `number_to_string`, with the even-tie rule of §6.1.6.1.20 step 5) |
//!
//! Not built: `json-parse-with-source` (`JSON.rawJSON`, `JSON.isRawJSON` and the reviver
//! `context` argument). `rawJSON` objects need an `[[IsRawJSON]]` internal slot, that is a new
//! `ObjectClass`, and the class list is not this module's. v0 has no BigInt values, so the
//! BigInt steps (toJSON of a BigInt, the TypeError) cannot be reached.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-json.txt`.

mod parse;
mod stringify;

use ep9_vm::{
    BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsArray,
    JsObject, JsResult, Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// How many object and array levels `JSON.stringify` and the `JSON.parse` reviver walk
/// enter before the RangeError a native stack overflow gives (both recurse once per level;
/// node 24 throws the same RangeError between 5,000 and 10,000 levels). Each level also
/// calls `Cx::check_native_stack`, which throws first on a small native stack. The cap keeps
/// the cycle check of `stringify` (a scan of the open objects) at most 10,000 long.
/// `JSON.parse` without a reviver has no cap: it does not recurse.
const MAX_DEPTH: usize = 10_000;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 14;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const PARSE: u16 = 0;
    pub const STRINGIFY: u16 = 1;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("parse", 2, parse::parse), // 14.0
    BuiltinDef::method("stringify", 3, stringify::stringify), // 14.1
];

/// The intrinsics this module makes: `%JSON%` (§25.5: an ordinary object, not a function).
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Json,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Json,
        // §25.5.1-2 ({w, c}), then §25.5.3 JSON [ %Symbol.toStringTag% ] ({c}); V8's order
        // (node 24 `Reflect.ownKeys(JSON)` without rawJSON and isRawJSON).
        props: &[
            PropDef::method(PropKey::Str("parse"), def::PARSE),
            PropDef::method(PropKey::Str("stringify"), def::STRINGIFY),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("JSON"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.4.2 JSON ({w, c}).
        props: &[PropDef::intrinsic(
            PropKey::Str("JSON"),
            IntrinsicId::Json,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "json",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The RangeError at [`MAX_DEPTH`] levels: V8's text for a stack overflow.
pub(crate) fn too_deep(cx: &mut Cx<'_>) -> ep9_vm::Throw {
    cx.throw_range_error("Maximum call stack size exceeded")
}

/// The primitive inside `o` if `o` has a `[[NumberData]]`, `[[StringData]]` or
/// `[[BooleanData]]` internal slot (a Number, String or Boolean wrapper object), else `None`.
/// Never runs JS.
///
/// The builtin ABI has no test for a wrapper that does not throw, so this asks
/// `Cx::this_primitive_value` (thisNumberValue and the others) and catches its TypeError:
/// up to three error objects for an object that is not a wrapper. A `Cx` method that answers
/// without a throw (a non-throwing `this_primitive_value`) replaces the body; it is reported
/// to the M3-P1 integration fixer as an H20 part-2 row.
pub(crate) fn primitive_data<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
) -> JsResult<Option<Local<'s>>> {
    // Arrays, functions and objects of an internal-slot class are never wrappers.
    if cx.is_array(o) || o.is_callable() || cx.object_class(o).is_some() {
        return Ok(None);
    }
    // A String wrapper always has an own `length` (§10.4.3.5, not configurable), so an
    // object without one needs no String probe.
    let string_length = cx.get_own_property(o, "length")?.is_some();
    for ty in [
        PrimitiveType::Number,
        PrimitiveType::String,
        PrimitiveType::Boolean,
    ] {
        if ty == PrimitiveType::String && !string_length {
            continue;
        }
        match cx.this_primitive_value(o, ty) {
            Ok(v) => return Ok(Some(v)),
            // The probe's TypeError is not a JS-visible completion: drop it. A termination
            // cannot be caught and goes on (`catch_exception` gives it back).
            Err(t) => {
                cx.catch_exception(t)?;
            }
        }
    }
    Ok(None)
}

/// EnumerableOwnProperties(O, key) (§7.3.23): the own enumerable string keys of `o`, in
/// `[[OwnPropertyKeys]]` order, as the first `n` elements of a new array with a null
/// prototype (so no write to it reaches a prototype). Each key is a string (an atom). The
/// whole list is made before the caller reads any value, as the spec orders it. Polls once
/// per key.
pub(crate) fn enumerable_own_keys<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
) -> JsResult<(Local<'s, JsArray>, u64)> {
    // 1. Let ownKeys be ? O.[[OwnPropertyKeys]]().
    let keys = cx.own_property_keys(o)?;
    cx.set_prototype_of(keys, Local::null())?;
    let len = cx.get(keys, "length")?;
    let len = cx.to_length(len)?;
    // 2. Let results be a new empty List.
    let mut n = 0;
    // 3. For each element key of ownKeys, do
    for i in 0..len {
        cx.poll_interrupt()?;
        let key = cx.get_index(keys, i)?;
        // a. If key is a String, then
        let Some(s) = key.as_string() else {
            continue;
        };
        //    i. Let desc be ? O.[[GetOwnProperty]](key).
        let k = cx.intern(s);
        let desc = cx.get_own_property(o, k)?;
        //    ii. If desc is not undefined and desc.[[Enumerable]] is true, then append key.
        if desc.is_some_and(|d| d.enumerable == Some(true)) {
            cx.set_index(keys, n, key, true)?;
            n += 1;
        }
    }
    // 4. Return results.
    Ok((keys, n))
}
