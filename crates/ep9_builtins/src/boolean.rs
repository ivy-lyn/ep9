//! Boolean builtins (§20.3): module `boolean`, block 5. Targets `%Boolean%` (made here) and
//! `%Boolean.prototype%` (`IntrinsicId::Boolean`, `IntrinsicId::BooleanPrototype`).
//!
//! ```text
//! Boolean(v)              ToBoolean(v); with new: a Boolean wrapper whose prototype comes
//!                         from new.target (GetPrototypeFromConstructor, Cx::new_wrapper)
//! %Boolean.prototype%     toString ("true"/"false"), valueOf: thisBooleanValue
//!                         (Cx::this_primitive_value)
//! global                  Boolean {w, c}
//! ```
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-boolean.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 5;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Boolean", 1, boolean), // 0
    BuiltinDef::method("toString", 0, to_string),   // 1
    BuiltinDef::method("valueOf", 0, value_of),     // 2
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Boolean,
    value: IntrinsicValue::Builtin(0),
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Boolean,
        // §20.3.2.1 Boolean.prototype ({}).
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::BooleanPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::BooleanPrototype,
        props: &[
            // §20.3.3.1 Boolean.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::Boolean,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 1),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3.8 Boolean ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("Boolean"),
            IntrinsicId::Boolean,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "boolean",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Boolean ( value ) (§20.3.1.1).
fn boolean<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let b be ToBoolean(value).
    let b = Local::boolean(cx.to_boolean(args.get(0)));
    // 2. If NewTarget is undefined, return b.
    let Some(nt) = new_target.as_object() else {
        return Ok(b);
    };
    // 3. Let O be ? OrdinaryCreateFromConstructor(NewTarget, "%Boolean.prototype%",
    //    « [[BooleanData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::BooleanPrototype)?;
    // 4. Set O.[[BooleanData]] to b.
    let o = cx.new_wrapper(proto, b)?;
    // 5. Return O.
    Ok(o.value())
}

/// Boolean.prototype.toString ( ) (§20.3.3.2).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let b be ? thisBooleanValue(this value).
    let b = cx.this_primitive_value(this, PrimitiveType::Boolean)?;
    // 2. If b is true, return "true"; else return "false".
    // (Both are static atoms: the lookup allocates nothing.)
    let text = if b.as_bool() == Some(true) {
        "true"
    } else {
        "false"
    };
    Ok(cx.key(text).value())
}

/// Boolean.prototype.valueOf ( ) (§20.3.3.3).
fn value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisBooleanValue(this value).
    cx.this_primitive_value(this, PrimitiveType::Boolean)
}
