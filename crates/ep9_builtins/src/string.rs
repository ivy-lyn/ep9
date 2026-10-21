//! String builtins (§22.1): module `string`, block 6. Targets `%String%` (made here) and
//! `%String.prototype%` (`IntrinsicId::String`, `IntrinsicId::StringPrototype`).
//!
//! ```text
//! String(v)              ToString(v); a symbol gives SymbolDescriptiveString
//!                        (Cx::descriptive_string) when called, a TypeError with new
//! new String(v)          StringCreate: a String wrapper (Cx::new_wrapper) whose prototype
//!                        comes from new.target; `length` and indices are virtual (R6)
//! String.fromCharCode    ToUint16 of each argument, in order
//! %String.prototype%     charCodeAt; toString, valueOf: thisStringValue
//!                        (Cx::this_primitive_value)
//! global                 String {w, c}
//! ```
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-string.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 6;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("String", 1, string), // 0
    BuiltinDef::method("fromCharCode", 1, from_char_code), // 1
    BuiltinDef::method("charCodeAt", 1, char_code_at), // 2
    BuiltinDef::method("toString", 0, to_string), // 3
    BuiltinDef::method("valueOf", 0, value_of),   // 4
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::String,
    value: IntrinsicValue::Builtin(0),
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::String,
        props: &[
            // §22.1.2.3 String.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::StringPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("fromCharCode"), 1),
        ],
    },
    ObjectProps {
        target: IntrinsicId::StringPrototype,
        props: &[
            // §22.1.3.6 String.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::String,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Str("charCodeAt"), 2),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 3),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 4),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 String ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("String"),
            IntrinsicId::String,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "string",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// String ( value ) (§22.1.1.1).
fn string<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If value is not present, then
    //    a. Let s be the empty String.
    let s = if args.is_empty() {
        cx.key("").value()
    } else {
        // 2. Else,
        let value = args.get(0);
        //    a. If NewTarget is undefined and value is a Symbol, return
        //       SymbolDescriptiveString(value).
        if new_target.is_undefined()
            && let Some(sym) = value.as_symbol()
        {
            return Ok(cx.descriptive_string(sym)?.value());
        }
        //    b. Let s be ? ToString(value).
        cx.to_string(value)?.value()
    };
    // 3. If NewTarget is undefined, return s.
    let Some(nt) = new_target.as_object() else {
        return Ok(s);
    };
    // 4. Return StringCreate(s, ? GetPrototypeFromConstructor(NewTarget,
    //    "%String.prototype%")).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::StringPrototype)?;
    Ok(cx.new_wrapper(proto, s)?.value())
}

/// String.fromCharCode ( ...codeUnits ) (§22.1.2.1).
fn from_char_code<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let result be the empty String.
    // (code units are plain u16: no list of Locals is needed)
    let mut result = Vec::with_capacity(args.len());
    // 2. For each element next of codeUnits, do
    for next in args.iter() {
        //    a. Let nextCU be the code unit whose numeric value is ℝ(? ToUint16(next)).
        //       (ToUint16 is ToUint32 modulo 2^16: 2^16 divides 2^32.)
        let next_cu = cx.to_uint32(next)? as u16;
        //    b. Set result to the string-concatenation of result and nextCU.
        result.push(next_cu);
    }
    // 3. Return result.
    Ok(cx.new_string_utf16(&result)?.value())
}

/// String.prototype.charCodeAt ( pos ) (§22.1.3.3).
fn char_code_at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    if this.is_undefined_or_null() {
        return Err(cx.throw_type_error("String.prototype.charCodeAt called on null or undefined"));
    }
    // 2. Let S be ? ToString(O).
    let s = cx.to_string(this)?;
    // 3. Let position be ? ToIntegerOrInfinity(pos).
    let position = cx.to_integer_or_infinity(args.get(0))?;
    // 4. Let size be the length of S.
    let size = cx.string_len(s);
    // 5. If position < 0 or position ≥ size, return NaN.
    if position < 0.0 || position >= size as f64 {
        return Ok(Local::number(f64::NAN));
    }
    // 6. Return the Number value for the numeric value of the code unit at index position
    //    within the String S.
    let s = cx.flatten(s)?;
    let unit = cx
        .code_unit_at(s, position as usize)
        .expect("position < size");
    Ok(Local::int32(i32::from(unit)))
}

/// String.prototype.toString ( ) (§22.1.3.29).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisStringValue(this value).
    cx.this_primitive_value(this, PrimitiveType::String)
}

/// String.prototype.valueOf ( ) (§22.1.3.35).
fn value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisStringValue(this value).
    cx.this_primitive_value(this, PrimitiveType::String)
}
