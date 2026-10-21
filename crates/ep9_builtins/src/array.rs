//! Array builtins (§23.1): module `array`, block 3. Targets `%Array%` and `%Array.prototype%`
//! (`IntrinsicId::Array`, `IntrinsicId::ArrayPrototype`).
//!
//! The `Array` constructor and `Array.prototype.join`/`toString`/`push` are VM builtins (block 0).
//! Week 4 (P3-BLTA): `Array.isArray`, `Array.prototype.pop`, `indexOf`.
//!
//! | Builtin | Template | Pattern |
//! |---|---|---|
//! | `Array.isArray` | 1 | a pure static predicate |
//! | `pop`, `indexOf` | companions | generic over array-likes: every element access goes through the index ObjectOps (`get_index`, `has_index`, `delete_index`), which need no key for an element or a missing index |
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-array.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsKey, JsObject, JsResult,
    Local, ObjectProps, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 3;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const IS_ARRAY: u16 = 0;
    pub const POP: u16 = 1;
    pub const INDEX_OF: u16 = 2;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("isArray", 1, is_array), // 3.0
    BuiltinDef::method("pop", 0, pop),          // 3.1
    BuiltinDef::method("indexOf", 1, index_of), // 3.2
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Array,
        props: &[PropDef::method(PropKey::Str("isArray"), def::IS_ARRAY)],
    },
    ObjectProps {
        target: IntrinsicId::ArrayPrototype,
        props: &[
            PropDef::method(PropKey::Str("pop"), def::POP),
            PropDef::method(PropKey::Str("indexOf"), def::INDEX_OF),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "array",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- helpers shared with the other modules of this crate ------------------------------------

/// The property key of the integer index `i` (`! ToString(𝔽(i))`, an atom), for `i` up to
/// 2^53 - 1: decimal digits, the same text as Number::toString for these integers. Formats
/// on the native stack; interning finds an existing atom without allocating.
pub(crate) fn index_key<'s>(cx: &mut Cx<'s>, i: u64) -> Local<'s, JsKey> {
    let mut buf = [0u8; 20];
    let mut at = buf.len();
    let mut n = i;
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let text = core::str::from_utf8(&buf[at..]).expect("ASCII digits");
    cx.key(text)
}

/// LengthOfArrayLike ( obj ) (§7.3.18).
pub(crate) fn length_of_array_like<'s>(cx: &mut Cx<'s>, obj: Local<'s, JsObject>) -> JsResult<u64> {
    // 1. Return ℝ(? ToLength(? Get(obj, "length"))).
    let len = cx.get(obj, "length")?;
    cx.to_length(len)
}

// ---- builtins ----------------------------------------------------------------------------------

/// Template 1. Array.isArray ( arg ) (§23.1.2.2).
fn is_array<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? IsArray(arg). (v0: no proxies, so IsArray never throws.)
    Ok(Local::boolean(cx.is_array(args.get(0))))
}

/// Array.prototype.pop ( ) (§23.1.3.22).
fn pop<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, then
    if len == 0 {
        // a. Perform ? Set(O, "length", +0𝔽, true).
        cx.set(o, "length", Local::int32(0), true)?;
        // b. Return undefined.
        return Ok(Local::undefined());
    }
    // 4. Else,
    //    a. Assert: len > 0.
    //    b. Let newLen be 𝔽(len - 1).
    let new_len = len - 1;
    //    c. Let index be ! ToString(newLen). (`get_index` and `delete_index` take newLen.)
    //    d. Let element be ? Get(O, index).
    let element = cx.get_index(o, new_len)?;
    //    e. Perform ? DeletePropertyOrThrow(O, index).
    cx.delete_index(o, new_len, true)?;
    //    f. Perform ? Set(O, "length", newLen, true).
    cx.set(o, "length", Local::number(new_len as f64), true)?;
    //    g. Return element.
    Ok(element)
}

/// Array.prototype.indexOf ( searchElement [ , fromIndex ] ) (§23.1.3.17).
fn index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, return -1𝔽.
    if len == 0 {
        return Ok(Local::int32(-1));
    }
    // 4. Let n be ? ToIntegerOrInfinity(fromIndex).
    // 5. Assert: If fromIndex is undefined, then n is 0.
    let mut n = cx.to_integer_or_infinity(args.get(1))?;
    // 6. If n = +∞, return -1𝔽.
    if n == f64::INFINITY {
        return Ok(Local::int32(-1));
    }
    // 7. Else if n = -∞, set n to 0.
    if n == f64::NEG_INFINITY {
        n = 0.0;
    }
    // 8. If n ≥ 0, then
    let mut k = if n >= 0.0 {
        // a. Let k be n.
        n
    } else {
        // 9. Else,
        //    a. Let k be len + n.
        //    b. If k < 0, set k to 0.
        (len as f64 + n).max(0.0)
    } as u64;
    // 10. Repeat, while k < len,
    while k < len {
        // The loop has no back edge and a hole runs no JS: one budget unit per index, so
        // termination stops a walk to 2^53 - 1.
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)). (`has_index` and `get_index` take k: no key for an
        //    element or a missing index.)
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let elementK be ? Get(O, Pk).
            let element_k = cx.get_index(o, k)?;
            // ii. If IsStrictlyEqual(searchElement, elementK) is true, return 𝔽(k).
            if cx.is_strictly_equal(search_element, element_k) {
                return Ok(Local::number(k as f64));
            }
        }
        // d. Set k to k + 1.
        k += 1;
    }
    // 11. Return -1𝔽.
    Ok(Local::int32(-1))
}
