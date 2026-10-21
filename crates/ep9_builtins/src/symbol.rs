//! Symbol builtins (§20.4): module `symbol`, block 7. Targets `%Symbol%` (made here) and
//! `%Symbol.prototype%` (`IntrinsicId::Symbol`, `IntrinsicId::SymbolPrototype`; realm init
//! makes the prototype, `symbol::install` in ep9_vm).
//!
//! ```text
//! Symbol(d)             a new symbol, description ToString(d) or undefined; `new Symbol`
//!                       throws TypeError (it has [[Construct]]: IsConstructor is true)
//! Symbol.for / keyFor   the registry (Cx::symbol_for, Cx::key_for)
//! Symbol.<well-known>   the 15 well-known symbols (PropValue::Symbol, {})
//! %Symbol.prototype%    toString (SymbolDescriptiveString), valueOf, the `description`
//!                       getter ({c}), [@@toPrimitive] ({c}), [@@toStringTag] = "Symbol"
//!                       ({c}): thisSymbolValue (Cx::this_primitive_value)
//! global                Symbol {w, c}
//! ```
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-symbol.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
    PropValue, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 7;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Symbol", 0, symbol), // 0
    BuiltinDef::method("for", 1, symbol_for),     // 1
    BuiltinDef::method("keyFor", 1, key_for),     // 2
    BuiltinDef::method("toString", 0, to_string), // 3
    BuiltinDef::method("valueOf", 0, value_of),   // 4
    BuiltinDef::method("[Symbol.toPrimitive]", 1, to_primitive), // 5
    BuiltinDef::getter("description", description), // 6
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Symbol,
    value: IntrinsicValue::Builtin(0),
}];

/// `Symbol.<name>` = the well-known symbol `w` ({}: §20.4.2).
const fn well_known(w: WellKnownSymbol) -> PropDef {
    PropDef {
        key: PropKey::Str(w.name()),
        value: PropValue::Symbol(w),
        attrs: PropAttrs::NONE,
    }
}

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Symbol,
        props: &[
            // §20.4.2.9 Symbol.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::SymbolPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("for"), 1),
            PropDef::method(PropKey::Str("keyFor"), 2),
            // §20.4.2.1-16, in V8's order.
            well_known(WellKnownSymbol::ASYNC_ITERATOR),
            well_known(WellKnownSymbol::HAS_INSTANCE),
            well_known(WellKnownSymbol::IS_CONCAT_SPREADABLE),
            well_known(WellKnownSymbol::ITERATOR),
            well_known(WellKnownSymbol::MATCH),
            well_known(WellKnownSymbol::MATCH_ALL),
            well_known(WellKnownSymbol::REPLACE),
            well_known(WellKnownSymbol::SEARCH),
            well_known(WellKnownSymbol::SPECIES),
            well_known(WellKnownSymbol::SPLIT),
            well_known(WellKnownSymbol::TO_PRIMITIVE),
            well_known(WellKnownSymbol::TO_STRING_TAG),
            well_known(WellKnownSymbol::UNSCOPABLES),
            well_known(WellKnownSymbol::DISPOSE),
            well_known(WellKnownSymbol::ASYNC_DISPOSE),
        ],
    },
    ObjectProps {
        target: IntrinsicId::SymbolPrototype,
        props: &[
            // §20.4.3.1 Symbol.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::Symbol,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 3),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 4),
            // §20.4.3.2 get Symbol.prototype.description ({c}).
            PropDef::accessor(PropKey::Str("description"), Some(6), None),
            // §20.4.3.6 Symbol.prototype [ %Symbol.toStringTag% ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Symbol"),
                PropAttrs::CONFIGURABLE,
            ),
            // §20.4.3.5 Symbol.prototype [ %Symbol.toPrimitive% ] ({c}).
            PropDef {
                key: PropKey::Symbol(WellKnownSymbol::TO_PRIMITIVE),
                value: PropValue::Builtin(5),
                attrs: PropAttrs::CONFIGURABLE,
            },
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Symbol ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("Symbol"),
            IntrinsicId::Symbol,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "symbol",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Symbol ( [ description ] ) (§20.4.1.1).
fn symbol<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is not undefined, throw a TypeError exception.
    if !new_target.is_undefined() {
        return Err(cx.throw_type_error("Symbol is not a constructor"));
    }
    // 2. If description is undefined, let descString be undefined.
    // 3. Else, let descString be ? ToString(description).
    let description = args.get(0);
    let desc_string = if description.is_undefined() {
        description
    } else {
        cx.to_string(description)?.value()
    };
    // 4. Return a new Symbol whose [[Description]] is descString.
    Ok(cx.new_symbol(desc_string)?.value())
}

/// Symbol.for ( key ) (§20.4.2.2).
fn symbol_for<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let stringKey be ? ToString(key).
    let string_key = cx.to_string(args.get(0))?;
    // 2-6. The registry's symbol for stringKey, or a new registered one (Cx::symbol_for).
    Ok(cx.symbol_for(string_key)?.value())
}

/// Symbol.keyFor ( sym ) (§20.4.2.6).
fn key_for<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If sym is not a Symbol, throw a TypeError exception.
    let sym = args.get(0);
    let Some(sym) = sym.as_symbol() else {
        // V8's text: "<value> is not a symbol". ToString of an object would run JS, so an
        // object is named by its type; ToString of any other non-symbol runs none.
        let text = if sym.is_object() {
            cx.type_of(sym).to_string()
        } else {
            cx.to_rust_string(sym)?
        };
        return Err(cx.throw_type_error(&format!("{text} is not a symbol")));
    };
    // 2. Return KeyForSymbol(sym).
    Ok(match cx.key_for(sym) {
        Some(key) => key.value(),
        None => Local::undefined(),
    })
}

/// thisSymbolValue ( value ) (§20.4.3.4.1).
fn this_symbol_value<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    cx.this_primitive_value(this, PrimitiveType::Symbol)
}

/// Symbol.prototype.toString ( ) (§20.4.3.3).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let sym be ? thisSymbolValue(this value).
    let sym = this_symbol_value(cx, this)?;
    let sym = sym.as_symbol().expect("thisSymbolValue gives a symbol");
    // 2. Return SymbolDescriptiveString(sym).
    Ok(cx.descriptive_string(sym)?.value())
}

/// Symbol.prototype.valueOf ( ) (§20.4.3.4).
fn value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisSymbolValue(this value).
    this_symbol_value(cx, this)
}

/// Symbol.prototype [ %Symbol.toPrimitive% ] ( hint ) (§20.4.3.5).
fn to_primitive<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisSymbolValue(this value).
    // (The hint is not read.)
    this_symbol_value(cx, this)
}

/// get Symbol.prototype.description (§20.4.3.2).
fn description<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let s be the this value.
    // 2. Let sym be ? thisSymbolValue(s).
    let sym = this_symbol_value(cx, this)?;
    let sym = sym.as_symbol().expect("thisSymbolValue gives a symbol");
    // 3. Return sym.[[Description]].
    Ok(cx.symbol_description(sym))
}
