//! Object builtins (§20.1, Annex B §B.2.2): module `object`, block 1. Targets `%Object%` and
//! `%Object.prototype%` (`IntrinsicId::Object`, `IntrinsicId::ObjectPrototype`).
//!
//! The `Object` constructor and `Object.prototype.toString`/`valueOf` are VM builtins (block 0).
//! `Object.fromEntries` and `Object.groupBy` are definitions of the `array` module (block 3),
//! which targets `%Object%` too.
//!
//! Week 4 (P3-BLTA): `Object.prototype.hasOwnProperty`, `propertyIsEnumerable`,
//! `Object.defineProperty`, `getOwnPropertyDescriptor`, `getOwnPropertyNames`, `keys`.
//! M0 month 2 (P4-B-OBJECT): the rest of the `Object` statics but `fromEntries` and `groupBy`,
//! `Object.prototype.isPrototypeOf` and `toLocaleString`, and the Annex B `__proto__`,
//! `__defineGetter__`, `__defineSetter__`, `__lookupGetter__` and `__lookupSetter__`.
//!
//! | Builtin | Template | Pattern |
//! |---|---|---|
//! | `hasOwnProperty` | 2 | ToPropertyKey before ToObject(this), then `[[GetOwnProperty]]` |
//! | `defineProperty` | 3 | ToPropertyDescriptor runs user getters, so every step is a GC point; DefinePropertyOrThrow |
//! | `getOwnPropertyDescriptor` | 4 | FromPropertyDescriptor: a new object, filled |
//! | `getOwnPropertyNames` | 5 | a list of values in a `ScratchList`, returned with CreateArrayFromList |
//! | `propertyIsEnumerable`, `keys` | companions | as `hasOwnProperty`; as `getOwnPropertyNames` |
//! | `defineProperties`, `create` | ObjectDefineProperties | every descriptor is read before the first define; the list of descriptors is a `ScratchList` of 5 values per entry (key, value, get, set, presence bits), never a `Vec` |
//! | `freeze`, `seal`, `isFrozen`, `isSealed` | SetIntegrityLevel, TestIntegrityLevel | over `[[OwnPropertyKeys]]`, which never lists a private name (§6.2.12), so private elements stay as they are |
//! | `__proto__` | accessor | `PropDef::accessor` on `%Object.prototype%`, {e: false, c: true} |
//!
//! A loop over the own keys of an object runs no JS for an ordinary object, so it has no back
//! edge to poll: each pass takes one interrupt budget unit (`Cx::poll_interrupt`, M2-P2-POLL).
//! v0 limits come from the VM's `[[DefineOwnProperty]]`: freezing an array throws the
//! not-implemented error (non-writable `length` and elements need dictionary elements, H18).
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-object.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsArray, JsKey, JsObject,
    JsResult, Local, ObjectProps, PropDef, PropKey, PropertyDescriptor, ScratchList,
};

use crate::array::{index_key, length_of_array_like};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 1;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const HAS_OWN_PROPERTY: u16 = 0;
    pub const DEFINE_PROPERTY: u16 = 1;
    pub const GET_OWN_PROPERTY_DESCRIPTOR: u16 = 2;
    pub const GET_OWN_PROPERTY_NAMES: u16 = 3;
    pub const PROPERTY_IS_ENUMERABLE: u16 = 4;
    pub const KEYS: u16 = 5;
    pub const CREATE: u16 = 6;
    pub const DEFINE_PROPERTIES: u16 = 7;
    pub const GET_PROTOTYPE_OF: u16 = 8;
    pub const SET_PROTOTYPE_OF: u16 = 9;
    pub const FREEZE: u16 = 10;
    pub const IS_FROZEN: u16 = 11;
    pub const SEAL: u16 = 12;
    pub const IS_SEALED: u16 = 13;
    pub const PREVENT_EXTENSIONS: u16 = 14;
    pub const IS_EXTENSIBLE: u16 = 15;
    pub const ENTRIES: u16 = 16;
    pub const VALUES: u16 = 17;
    pub const ASSIGN: u16 = 18;
    pub const IS: u16 = 19;
    pub const HAS_OWN: u16 = 20;
    pub const GET_OWN_PROPERTY_SYMBOLS: u16 = 21;
    pub const GET_OWN_PROPERTY_DESCRIPTORS: u16 = 22;
    pub const IS_PROTOTYPE_OF: u16 = 23;
    pub const GET_PROTO: u16 = 24;
    pub const SET_PROTO: u16 = 25;
    pub const DEFINE_GETTER: u16 = 26;
    pub const DEFINE_SETTER: u16 = 27;
    pub const LOOKUP_GETTER: u16 = 28;
    pub const LOOKUP_SETTER: u16 = 29;
    pub const TO_LOCALE_STRING: u16 = 30;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("hasOwnProperty", 1, has_own_property), // 1.0
    BuiltinDef::method("defineProperty", 3, define_property),  // 1.1
    BuiltinDef::method("getOwnPropertyDescriptor", 2, get_own_property_descriptor), // 1.2
    BuiltinDef::method("getOwnPropertyNames", 1, get_own_property_names), // 1.3
    BuiltinDef::method("propertyIsEnumerable", 1, property_is_enumerable), // 1.4
    BuiltinDef::method("keys", 1, keys),                       // 1.5
    BuiltinDef::method("create", 2, create),                   // 1.6
    BuiltinDef::method("defineProperties", 2, define_properties), // 1.7
    BuiltinDef::method("getPrototypeOf", 1, get_prototype_of), // 1.8
    BuiltinDef::method("setPrototypeOf", 2, set_prototype_of), // 1.9
    BuiltinDef::method("freeze", 1, freeze),                   // 1.10
    BuiltinDef::method("isFrozen", 1, is_frozen),              // 1.11
    BuiltinDef::method("seal", 1, seal),                       // 1.12
    BuiltinDef::method("isSealed", 1, is_sealed),              // 1.13
    BuiltinDef::method("preventExtensions", 1, prevent_extensions), // 1.14
    BuiltinDef::method("isExtensible", 1, is_extensible),      // 1.15
    BuiltinDef::method("entries", 1, entries),                 // 1.16
    BuiltinDef::method("values", 1, values),                   // 1.17
    BuiltinDef::method("assign", 2, assign),                   // 1.18
    BuiltinDef::method("is", 2, is),                           // 1.19
    BuiltinDef::method("hasOwn", 2, has_own),                  // 1.20
    BuiltinDef::method("getOwnPropertySymbols", 1, get_own_property_symbols), // 1.21
    BuiltinDef::method("getOwnPropertyDescriptors", 1, get_own_property_descriptors), // 1.22
    BuiltinDef::method("isPrototypeOf", 1, is_prototype_of),   // 1.23
    BuiltinDef::getter("__proto__", get_proto),                // 1.24
    BuiltinDef::setter("__proto__", set_proto),                // 1.25
    BuiltinDef::method("__defineGetter__", 2, define_getter),  // 1.26
    BuiltinDef::method("__defineSetter__", 2, define_setter),  // 1.27
    BuiltinDef::method("__lookupGetter__", 1, lookup_getter),  // 1.28
    BuiltinDef::method("__lookupSetter__", 1, lookup_setter),  // 1.29
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 1.30
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Object,
        props: &[
            PropDef::method(
                PropKey::Str("getOwnPropertyDescriptor"),
                def::GET_OWN_PROPERTY_DESCRIPTOR,
            ),
            PropDef::method(
                PropKey::Str("getOwnPropertyNames"),
                def::GET_OWN_PROPERTY_NAMES,
            ),
            PropDef::method(PropKey::Str("defineProperty"), def::DEFINE_PROPERTY),
            PropDef::method(PropKey::Str("keys"), def::KEYS),
            PropDef::method(PropKey::Str("assign"), def::ASSIGN),
            PropDef::method(PropKey::Str("create"), def::CREATE),
            PropDef::method(PropKey::Str("defineProperties"), def::DEFINE_PROPERTIES),
            PropDef::method(PropKey::Str("entries"), def::ENTRIES),
            PropDef::method(PropKey::Str("freeze"), def::FREEZE),
            PropDef::method(
                PropKey::Str("getOwnPropertyDescriptors"),
                def::GET_OWN_PROPERTY_DESCRIPTORS,
            ),
            PropDef::method(
                PropKey::Str("getOwnPropertySymbols"),
                def::GET_OWN_PROPERTY_SYMBOLS,
            ),
            PropDef::method(PropKey::Str("getPrototypeOf"), def::GET_PROTOTYPE_OF),
            PropDef::method(PropKey::Str("hasOwn"), def::HAS_OWN),
            PropDef::method(PropKey::Str("is"), def::IS),
            PropDef::method(PropKey::Str("isExtensible"), def::IS_EXTENSIBLE),
            PropDef::method(PropKey::Str("isFrozen"), def::IS_FROZEN),
            PropDef::method(PropKey::Str("isSealed"), def::IS_SEALED),
            PropDef::method(PropKey::Str("preventExtensions"), def::PREVENT_EXTENSIONS),
            PropDef::method(PropKey::Str("seal"), def::SEAL),
            PropDef::method(PropKey::Str("setPrototypeOf"), def::SET_PROTOTYPE_OF),
            PropDef::method(PropKey::Str("values"), def::VALUES),
        ],
    },
    ObjectProps {
        target: IntrinsicId::ObjectPrototype,
        props: &[
            PropDef::method(PropKey::Str("hasOwnProperty"), def::HAS_OWN_PROPERTY),
            PropDef::method(
                PropKey::Str("propertyIsEnumerable"),
                def::PROPERTY_IS_ENUMERABLE,
            ),
            PropDef::method(PropKey::Str("isPrototypeOf"), def::IS_PROTOTYPE_OF),
            PropDef::method(PropKey::Str("toLocaleString"), def::TO_LOCALE_STRING),
            PropDef::method(PropKey::Str("__defineGetter__"), def::DEFINE_GETTER),
            PropDef::method(PropKey::Str("__defineSetter__"), def::DEFINE_SETTER),
            PropDef::method(PropKey::Str("__lookupGetter__"), def::LOOKUP_GETTER),
            PropDef::method(PropKey::Str("__lookupSetter__"), def::LOOKUP_SETTER),
            PropDef::accessor(
                PropKey::Str("__proto__"),
                Some(def::GET_PROTO),
                Some(def::SET_PROTO),
            ),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "object",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- Object.prototype --------------------------------------------------------------------------

/// Template 2. Object.prototype.hasOwnProperty ( V ) (§20.1.3.2).
fn has_own_property<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let P be ? ToPropertyKey(V). (Before ToObject: a throwing `toString` of V wins over
    //    a TypeError for an undefined `this`.)
    let p = cx.to_property_key(args.get(0))?;
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Return ? HasOwnProperty(O, P).
    Ok(Local::boolean(cx.has_own_property(o, p)?))
}

/// Object.prototype.propertyIsEnumerable ( V ) (§20.1.3.4).
fn property_is_enumerable<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let P be ? ToPropertyKey(V).
    let p = cx.to_property_key(args.get(0))?;
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Let desc be ? O.[[GetOwnProperty]](P).
    let desc = cx.get_own_property(o, p)?;
    // 4. If desc is undefined, return false.
    // 5. Return desc.[[Enumerable]].
    Ok(Local::boolean(
        desc.is_some_and(|d| d.enumerable == Some(true)),
    ))
}

/// Object.prototype.isPrototypeOf ( V ) (§20.1.3.3).
fn is_prototype_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If V is not an Object, return false. (Before ToObject: `isPrototypeOf.call(undefined,
    //    1)` is false, not a TypeError.)
    let Some(mut v) = args.get(0).as_object() else {
        return Ok(Local::boolean(false));
    };
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Repeat, (a proxy may make the chain a cycle: the step polls and is bounded, as V8's)
    let mut hops = 0;
    loop {
        // a. Set V to ? V.[[GetPrototypeOf]]().
        // b. If V is null, return false.
        let Some(p) = cx.prototype_walk_step(v, &mut hops)?.as_object() else {
            return Ok(Local::boolean(false));
        };
        // c. If SameValue(O, V) is true, return true.
        if cx.same_value(o, p) {
            return Ok(Local::boolean(true));
        }
        v = p;
    }
}

/// Object.prototype.toLocaleString ( \[ reserved1 \[ , reserved2 \] \] ) (§20.1.3.5).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. Return ? Invoke(O, "toString"). (Invoke: GetV, then Call with O as this; a primitive
    //    O stays a primitive. Call throws the TypeError for a value that is not callable.)
    let to_string = cx.get_v(this, "toString")?;
    cx.call(to_string, this, &[])
}

/// get Object.prototype.__proto__ (§B.2.2.1.1).
fn get_proto<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Return ? O.[[GetPrototypeOf]]().
    cx.get_prototype_of(o)
}

/// set Object.prototype.__proto__ (§B.2.2.1.2).
fn set_proto<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let proto = args.get(0);
    // 1. Let O be ? RequireObjectCoercible(this value).
    if this.is_undefined_or_null() {
        return Err(cx.throw_type_error("Object.prototype.__proto__ called on null or undefined"));
    }
    // 2. If proto is not an Object and proto is not null, return undefined.
    if !proto.is_object() && !proto.is_null() {
        return Ok(Local::undefined());
    }
    // 3. If O is not an Object, return undefined.
    let Some(o) = this.as_object() else {
        return Ok(Local::undefined());
    };
    // 4. Let status be ? O.[[SetPrototypeOf]](proto).
    // 5. If status is false, throw a TypeError exception.
    if !cx.set_prototype_of(o, proto)? {
        return Err(cx.throw_type_error("Cyclic __proto__ value or a non-extensible object"));
    }
    // 6. Return undefined.
    Ok(Local::undefined())
}

/// Object.prototype.__defineGetter__ ( P, getter ) (§B.2.2.2).
fn define_getter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    define_accessor_half(cx, this, args, true)
}

/// Object.prototype.__defineSetter__ ( P, setter ) (§B.2.2.3).
fn define_setter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    define_accessor_half(cx, this, args, false)
}

/// The steps of `__defineGetter__` (`getter` true) and `__defineSetter__`.
fn define_accessor_half<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    getter: bool,
) -> JsResult<Local<'s>> {
    let f = args.get(1);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. If IsCallable(getter) is false, throw a TypeError exception.
    if !cx.is_callable(f) {
        let what = if getter { "Getter" } else { "Setter" };
        return Err(cx.throw_type_error(&format!("{what} must be a function")));
    }
    // 3. Let desc be PropertyDescriptor { [[Get]]: getter, [[Enumerable]]: true,
    //    [[Configurable]]: true } (or [[Set]]: setter).
    let desc = PropertyDescriptor {
        get: getter.then_some(f),
        set: (!getter).then_some(f),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    };
    // 4. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(0))?;
    // 5. Perform ? DefinePropertyOrThrow(O, key, desc).
    cx.define_property_or_throw(o, key, desc)?;
    // 6. Return undefined.
    Ok(Local::undefined())
}

/// Object.prototype.__lookupGetter__ ( P ) (§B.2.2.4).
fn lookup_getter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    lookup_accessor_half(cx, this, args, true)
}

/// Object.prototype.__lookupSetter__ ( P ) (§B.2.2.5).
fn lookup_setter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    lookup_accessor_half(cx, this, args, false)
}

/// The steps of `__lookupGetter__` (`getter` true) and `__lookupSetter__`.
fn lookup_accessor_half<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    getter: bool,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let mut o = cx.to_object(this)?;
    // 2. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(0))?;
    // 3. Repeat, (a proxy may make the chain a cycle: the step polls and is bounded, as V8's)
    let mut hops = 0;
    loop {
        // a. Let desc be ? O.[[GetOwnProperty]](key).
        // b. If desc is not undefined, then
        if let Some(desc) = cx.get_own_property(o, key)? {
            // i. If IsAccessorDescriptor(desc) is true, return desc.[[Get]] (or [[Set]]).
            // ii. Return undefined.
            let half = if getter { desc.get } else { desc.set };
            if desc.is_accessor() {
                return Ok(half.unwrap_or(Local::undefined()));
            }
            return Ok(Local::undefined());
        }
        // c. Set O to ? O.[[GetPrototypeOf]]().
        // d. If O is null, return undefined.
        let Some(p) = cx.prototype_walk_step(o, &mut hops)?.as_object() else {
            return Ok(Local::undefined());
        };
        o = p;
    }
}

// ---- Object statics ----------------------------------------------------------------------------

/// Template 3. Object.defineProperty ( O, P, Attributes ) (§20.1.2.4).
fn define_property<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, throw a TypeError exception.
    let Some(o) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("Object.defineProperty called on non-object"));
    };
    // 2. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Let desc be ? ToPropertyDescriptor(Attributes). It reads the fields with Get, so
    //    user getters run here: `o` and `key` stay live in Rust locals (R1).
    let desc = cx.to_property_descriptor(args.get(2))?;
    // 4. Perform ? DefinePropertyOrThrow(O, key, desc).
    cx.define_property_or_throw(o, key, desc)?;
    // 5. Return O.
    Ok(o.value())
}

/// Object.defineProperties ( O, Properties ) (§20.1.2.3).
fn define_properties<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, throw a TypeError exception.
    let Some(o) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("Object.defineProperties called on non-object"));
    };
    // 2. Return ? ObjectDefineProperties(O, Properties).
    object_define_properties(cx, o, args.get(1)).map(|o| o.value())
}

/// Object.create ( O, Properties ) (§20.1.2.2).
fn create<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let proto = args.get(0);
    // 1. If O is not an Object and O is not null, throw a TypeError exception.
    if !proto.is_object() && !proto.is_null() {
        return Err(cx.throw_type_error("Object prototype may only be an Object or null"));
    }
    // 2. Let obj be OrdinaryObjectCreate(O).
    let obj = cx.new_object_with_proto(proto);
    // 3. If Properties is not undefined, then
    let properties = args.get(1);
    if !properties.is_undefined() {
        // a. Return ? ObjectDefineProperties(obj, Properties).
        return object_define_properties(cx, obj, properties).map(|o| o.value());
    }
    // 4. Return obj.
    Ok(obj.value())
}

/// The presence and value bits of a descriptor kept in a `ScratchList` entry.
mod desc_bits {
    pub const VALUE: i32 = 1 << 0;
    pub const GET: i32 = 1 << 1;
    pub const SET: i32 = 1 << 2;
    pub const HAS_WRITABLE: i32 = 1 << 3;
    pub const WRITABLE: i32 = 1 << 4;
    pub const HAS_ENUMERABLE: i32 = 1 << 5;
    pub const ENUMERABLE: i32 = 1 << 6;
    pub const HAS_CONFIGURABLE: i32 = 1 << 7;
    pub const CONFIGURABLE: i32 = 1 << 8;
}

/// The number of `ScratchList` values per descriptor: key, value, get, set, bits.
const DESC_ENTRY: usize = 5;

/// Appends (`key`, `desc`) to `list` as [`DESC_ENTRY`] values.
fn push_descriptor<'s>(
    list: &ScratchList<'s>,
    key: Local<'s, JsKey>,
    desc: &PropertyDescriptor<'s>,
) {
    use desc_bits::*;
    let flag = |o: Option<bool>, has: i32, on: i32| match o {
        Some(true) => has | on,
        Some(false) => has,
        None => 0,
    };
    let field = |present: bool, bit: i32| if present { bit } else { 0 };
    let bits = field(desc.value.is_some(), VALUE)
        | field(desc.get.is_some(), GET)
        | field(desc.set.is_some(), SET)
        | flag(desc.writable, HAS_WRITABLE, WRITABLE)
        | flag(desc.enumerable, HAS_ENUMERABLE, ENUMERABLE)
        | flag(desc.configurable, HAS_CONFIGURABLE, CONFIGURABLE);
    list.push(key);
    list.push(desc.value.unwrap_or(Local::undefined()));
    list.push(desc.get.unwrap_or(Local::undefined()));
    list.push(desc.set.unwrap_or(Local::undefined()));
    list.push(Local::int32(bits));
}

/// The descriptor of entry `i` of a list that [`push_descriptor`] filled.
fn descriptor_at<'s>(list: &ScratchList<'s>, i: usize) -> PropertyDescriptor<'s> {
    use desc_bits::*;
    let at = i * DESC_ENTRY;
    let bits = list.get(at + 4).as_int32().expect("descriptor bits");
    let field = |bit: i32, j: usize| (bits & bit != 0).then(|| list.get(at + j));
    let flag = |has: i32, on: i32| (bits & has != 0).then_some(bits & on != 0);
    PropertyDescriptor {
        value: field(VALUE, 1),
        get: field(GET, 2),
        set: field(SET, 3),
        writable: flag(HAS_WRITABLE, WRITABLE),
        enumerable: flag(HAS_ENUMERABLE, ENUMERABLE),
        configurable: flag(HAS_CONFIGURABLE, CONFIGURABLE),
    }
}

/// ObjectDefineProperties ( O, Properties ) (§20.1.2.3.1).
fn object_define_properties<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    properties: Local<'s>,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Let props be ? ToObject(Properties).
    let props = cx.to_object(properties)?;
    // 2. Let keys be ? props.[[OwnPropertyKeys]]().
    let (keys, len) = own_keys(cx, props)?;
    // 3. Let descriptors be a new empty List.
    let descriptors = cx.scratch_list();
    // 4. For each element nextKey of keys, do
    for i in 0..len {
        cx.poll_interrupt()?;
        let next_key = key_at(cx, keys, i)?;
        // a. Let propDesc be ? props.[[GetOwnProperty]](nextKey).
        let prop_desc = cx.get_own_property(props, next_key)?;
        // b. If propDesc is not undefined and propDesc.[[Enumerable]] is true, then
        if prop_desc.is_some_and(|d| d.enumerable == Some(true)) {
            // i. Let descObj be ? Get(props, nextKey).
            let desc_obj = cx.get(props, next_key)?;
            // ii. Let desc be ? ToPropertyDescriptor(descObj).
            let desc = cx.to_property_descriptor(desc_obj)?;
            // iii. Append the Record { [[Key]]: nextKey, [[Descriptor]]: desc } to
            //      descriptors.
            push_descriptor(&descriptors, next_key, &desc);
        }
    }
    // 5. For each element property of descriptors, do
    for i in 0..descriptors.len() / DESC_ENTRY {
        cx.poll_interrupt()?;
        // a. Perform ? DefinePropertyOrThrow(O, property.[[Key]], property.[[Descriptor]]).
        let key = cx.to_property_key(descriptors.get(i * DESC_ENTRY))?;
        let desc = descriptor_at(&descriptors, i);
        cx.define_property_or_throw(o, key, desc)?;
    }
    // 6. Return O.
    Ok(o)
}

/// Template 4. Object.getOwnPropertyDescriptor ( O, P ) (§20.1.2.8).
fn get_own_property_descriptor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(args.get(0))?;
    // 2. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Let desc be ? obj.[[GetOwnProperty]](key).
    let desc = cx.get_own_property(obj, key)?;
    // 4. Return FromPropertyDescriptor(desc).
    cx.from_property_descriptor(desc)
}

/// Object.getOwnPropertyDescriptors ( O ) (§20.1.2.9).
fn get_own_property_descriptors<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(args.get(0))?;
    // 2. Let ownKeys be ? obj.[[OwnPropertyKeys]]().
    let (own, len) = own_keys(cx, obj)?;
    // 3. Let descriptors be OrdinaryObjectCreate(%Object.prototype%).
    let descriptors = cx.new_object();
    // 4. For each element key of ownKeys, do
    for i in 0..len {
        cx.poll_interrupt()?;
        let key = key_at(cx, own, i)?;
        // a. Let desc be ? obj.[[GetOwnProperty]](key).
        let desc = cx.get_own_property(obj, key)?;
        // b. Let descriptor be FromPropertyDescriptor(desc).
        let descriptor = cx.from_property_descriptor(desc)?;
        // c. If descriptor is not undefined, perform ! CreateDataPropertyOrThrow(descriptors,
        //    key, descriptor).
        if !descriptor.is_undefined() {
            cx.create_data_property_or_throw(descriptors, key, descriptor)?;
        }
    }
    // 5. Return descriptors.
    Ok(descriptors.value())
}

/// Template 5. Object.getOwnPropertyNames ( O ) (§20.1.2.10).
fn get_own_property_names<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return CreateArrayFromList(? GetOwnPropertyKeys(O, string)).
    let names = cx.scratch_list();
    get_own_property_keys(cx, args.get(0), KeyType::String, &names)?;
    create_array_from_scratch(cx, &names).map(|a| a.value())
}

/// Object.getOwnPropertySymbols ( O ) (§20.1.2.11).
fn get_own_property_symbols<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return CreateArrayFromList(? GetOwnPropertyKeys(O, symbol)).
    let symbols = cx.scratch_list();
    get_own_property_keys(cx, args.get(0), KeyType::Symbol, &symbols)?;
    create_array_from_scratch(cx, &symbols).map(|a| a.value())
}

/// The `type` argument of GetOwnPropertyKeys.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyType {
    String,
    Symbol,
}

/// GetOwnPropertyKeys ( O, type ) (§20.1.2.11.1): the keys of `type` go to `name_list`. A
/// list of values is a `ScratchList` (precise roots), never a `Vec<Local>`. A private name is
/// never among `[[OwnPropertyKeys]]` (§6.2.12), so it is never listed.
fn get_own_property_keys<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
    ty: KeyType,
    name_list: &ScratchList<'s>,
) -> JsResult<()> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(o)?;
    // 2. Let keys be ? obj.[[OwnPropertyKeys]]() (a new array).
    let (keys, len) = own_keys(cx, obj)?;
    // 3. Let nameList be a new empty List.
    // 4. For each element nextKey of keys, do
    for i in 0..len {
        cx.poll_interrupt()?;
        let next_key = cx.get_index(keys, i)?;
        // a. If nextKey is a Symbol and type is symbol, or if nextKey is a String and type is
        //    string, then
        let wanted = match ty {
            KeyType::String => next_key.is_string(),
            KeyType::Symbol => next_key.is_symbol(),
        };
        if wanted {
            // i. Append nextKey to nameList.
            name_list.push(next_key);
        }
    }
    // 5. Return nameList.
    Ok(())
}

/// Object.getPrototypeOf ( O ) (§20.1.2.12).
fn get_prototype_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(args.get(0))?;
    // 2. Return ? obj.[[GetPrototypeOf]]().
    cx.get_prototype_of(obj)
}

/// Object.setPrototypeOf ( O, proto ) (§20.1.2.23).
fn set_prototype_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (o, proto) = (args.get(0), args.get(1));
    // 1. Set O to ? RequireObjectCoercible(O).
    if o.is_undefined_or_null() {
        return Err(cx.throw_type_error("Object.setPrototypeOf called on null or undefined"));
    }
    // 2. If proto is not an Object and proto is not null, throw a TypeError exception.
    if !proto.is_object() && !proto.is_null() {
        return Err(cx.throw_type_error("Object prototype may only be an Object or null"));
    }
    // 3. If O is not an Object, return O.
    let Some(obj) = o.as_object() else {
        return Ok(o);
    };
    // 4. Let status be ? O.[[SetPrototypeOf]](proto).
    // 5. If status is false, throw a TypeError exception.
    if !cx.set_prototype_of(obj, proto)? {
        return Err(cx.throw_type_error("Cyclic __proto__ value or a non-extensible object"));
    }
    // 6. Return O.
    Ok(o)
}

/// The level of SetIntegrityLevel and TestIntegrityLevel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Sealed,
    Frozen,
}

/// Object.freeze ( O ) (§20.1.2.6).
fn freeze<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    integrity_static(cx, args.get(0), Level::Frozen)
}

/// Object.seal ( O ) (§20.1.2.22).
fn seal<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    integrity_static(cx, args.get(0), Level::Sealed)
}

/// The steps of Object.freeze and Object.seal.
fn integrity_static<'s>(cx: &mut Cx<'s>, o: Local<'s>, level: Level) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, return O.
    let Some(obj) = o.as_object() else {
        return Ok(o);
    };
    // 2. Let status be ? SetIntegrityLevel(O, level).
    // 3. If status is false, throw a TypeError exception.
    if !set_integrity_level(cx, obj, level)? {
        return Err(cx.throw_type_error("Cannot prevent extensions of the object"));
    }
    // 4. Return O.
    Ok(o)
}

/// SetIntegrityLevel ( O, level ) (§7.3.15).
fn set_integrity_level<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    level: Level,
) -> JsResult<bool> {
    // 1. Let status be ? O.[[PreventExtensions]]().
    // 2. If status is false, return false.
    if !cx.prevent_extensions(o)? {
        return Ok(false);
    }
    // 3. Let keys be ? O.[[OwnPropertyKeys]](). (No private name: §6.2.12.)
    let (keys, len) = own_keys(cx, o)?;
    let non_configurable = PropertyDescriptor {
        configurable: Some(false),
        ..PropertyDescriptor::default()
    };
    for i in 0..len {
        cx.poll_interrupt()?;
        let k = key_at(cx, keys, i)?;
        let desc = match level {
            // 4. If level is sealed, then
            //    a. For each element k of keys, do
            //       i. Perform ? DefinePropertyOrThrow(O, k, PropertyDescriptor {
            //          [[Configurable]]: false }).
            Level::Sealed => non_configurable,
            // 5. Else (level is frozen),
            //    a. For each element k of keys, do
            //       i. Let currentDesc be ? O.[[GetOwnProperty]](k).
            //       ii. If currentDesc is not undefined, then
            Level::Frozen => match cx.get_own_property(o, k)? {
                None => continue,
                // 1. If IsAccessorDescriptor(currentDesc) is true, then
                //    a. Let desc be the PropertyDescriptor { [[Configurable]]: false }.
                Some(current) if current.is_accessor() => non_configurable,
                // 2. Else,
                //    a. Let desc be the PropertyDescriptor { [[Configurable]]: false,
                //       [[Writable]]: false }.
                Some(_) => PropertyDescriptor {
                    writable: Some(false),
                    ..non_configurable
                },
            },
        };
        //       3. Perform ? DefinePropertyOrThrow(O, k, desc).
        cx.define_property_or_throw(o, k, desc)?;
    }
    // 6. Return true.
    Ok(true)
}

/// Object.isFrozen ( O ) (§20.1.2.16).
fn is_frozen<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, return true.
    // 2. Return ? TestIntegrityLevel(O, frozen).
    let Some(o) = args.get(0).as_object() else {
        return Ok(Local::boolean(true));
    };
    test_integrity_level(cx, o, Level::Frozen).map(Local::boolean)
}

/// Object.isSealed ( O ) (§20.1.2.17).
fn is_sealed<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, return true.
    // 2. Return ? TestIntegrityLevel(O, sealed).
    let Some(o) = args.get(0).as_object() else {
        return Ok(Local::boolean(true));
    };
    test_integrity_level(cx, o, Level::Sealed).map(Local::boolean)
}

/// TestIntegrityLevel ( O, level ) (§7.3.16).
fn test_integrity_level<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    level: Level,
) -> JsResult<bool> {
    // 1. Let extensible be ? IsExtensible(O).
    // 2. If extensible is true, return false.
    if cx.is_extensible(o)? {
        return Ok(false);
    }
    // 3. NOTE: If the object is extensible, none of its properties are examined.
    // 4. Let keys be ? O.[[OwnPropertyKeys]](). (No private name: §6.2.12.)
    let (keys, len) = own_keys(cx, o)?;
    // 5. For each element k of keys, do
    for i in 0..len {
        cx.poll_interrupt()?;
        let k = key_at(cx, keys, i)?;
        // a. Let currentDesc be ? O.[[GetOwnProperty]](k).
        // b. If currentDesc is not undefined, then
        if let Some(current) = cx.get_own_property(o, k)? {
            // i. If currentDesc.[[Configurable]] is true, return false.
            if current.configurable == Some(true) {
                return Ok(false);
            }
            // ii. If level is frozen and IsDataDescriptor(currentDesc) is true, then
            //     1. If currentDesc.[[Writable]] is true, return false.
            if level == Level::Frozen && current.is_data() && current.writable == Some(true) {
                return Ok(false);
            }
        }
    }
    // 6. Return true.
    Ok(true)
}

/// Object.preventExtensions ( O ) (§20.1.2.20).
fn prevent_extensions<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let o = args.get(0);
    // 1. If O is not an Object, return O.
    let Some(obj) = o.as_object() else {
        return Ok(o);
    };
    // 2. Let status be ? O.[[PreventExtensions]]().
    // 3. If status is false, throw a TypeError exception.
    if !cx.prevent_extensions(obj)? {
        return Err(cx.throw_type_error("Cannot prevent extensions of the object"));
    }
    // 4. Return O.
    Ok(o)
}

/// Object.isExtensible ( O ) (§20.1.2.15).
fn is_extensible<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, return false.
    let Some(o) = args.get(0).as_object() else {
        return Ok(Local::boolean(false));
    };
    // 2. Return ? IsExtensible(O).
    cx.is_extensible(o).map(Local::boolean)
}

/// Object.keys ( O ) (§20.1.2.18).
fn keys<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    enumerable_own_properties_static(cx, args.get(0), Kind::Key)
}

/// Object.values ( O ) (§20.1.2.24).
fn values<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    enumerable_own_properties_static(cx, args.get(0), Kind::Value)
}

/// Object.entries ( O ) (§20.1.2.5).
fn entries<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    enumerable_own_properties_static(cx, args.get(0), Kind::KeyValue)
}

/// The steps of Object.keys, values and entries.
fn enumerable_own_properties_static<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
    kind: Kind,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(o)?;
    // 2. Let list be ? EnumerableOwnProperties(obj, kind).
    let list = cx.scratch_list();
    enumerable_own_properties(cx, obj, kind, &list)?;
    // 3. Return CreateArrayFromList(list).
    create_array_from_scratch(cx, &list).map(|a| a.value())
}

/// The `kind` argument of EnumerableOwnProperties.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Key,
    Value,
    KeyValue,
}

/// EnumerableOwnProperties ( O, kind ) (§7.3.23): the results go to `results`.
fn enumerable_own_properties<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    kind: Kind,
    results: &ScratchList<'s>,
) -> JsResult<()> {
    // 1. Let ownKeys be ? O.[[OwnPropertyKeys]]().
    let (own, len) = own_keys(cx, o)?;
    // 2. Let results be a new empty List.
    // 3. For each element key of ownKeys, do
    for i in 0..len {
        cx.poll_interrupt()?;
        let key = cx.get_index(own, i)?;
        // a. If key is a String, then
        if !key.is_string() {
            continue;
        }
        // i. Let desc be ? O.[[GetOwnProperty]](key).
        let k = cx.to_property_key(key)?;
        let desc = cx.get_own_property(o, k)?;
        // ii. If desc is not undefined and desc.[[Enumerable]] is true, then
        if !desc.is_some_and(|d| d.enumerable == Some(true)) {
            continue;
        }
        // 1. If kind is key, then
        if kind == Kind::Key {
            // a. Append key to results.
            results.push(key);
            continue;
        }
        // 2. Else,
        //    a. Let value be ? Get(O, key).
        let value = cx.get(o, k)?;
        //    b. If kind is value, then
        if kind == Kind::Value {
            //   i. Append value to results.
            results.push(value);
        } else {
            //   c. Else,
            //      i. Assert: kind is key+value.
            //      ii. Let entry be CreateArrayFromList(« key, value »).
            let entry = cx.create_array_from_list(&[key, value])?;
            //      iii. Append entry to results.
            results.push(entry);
        }
    }
    // 4. Return results.
    Ok(())
}

/// Object.assign ( target, ...sources ) (§20.1.2.1).
fn assign<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let to be ? ToObject(target).
    let to = cx.to_object(args.get(0))?;
    // 2. If only one argument was passed, return to.
    // 3. For each element nextSource of sources, do
    for next_source in args.rest(1).iter() {
        // a. If nextSource is neither undefined nor null, then
        if next_source.is_undefined_or_null() {
            continue;
        }
        // i. Let from be ! ToObject(nextSource).
        let from = cx.to_object(next_source)?;
        // ii. Let keys be ? from.[[OwnPropertyKeys]]().
        let (keys, len) = own_keys(cx, from)?;
        // iii. For each element nextKey of keys, do
        for i in 0..len {
            cx.poll_interrupt()?;
            let next_key = key_at(cx, keys, i)?;
            // 1. Let desc be ? from.[[GetOwnProperty]](nextKey).
            let desc = cx.get_own_property(from, next_key)?;
            // 2. If desc is not undefined and desc.[[Enumerable]] is true, then
            if desc.is_some_and(|d| d.enumerable == Some(true)) {
                // a. Let propValue be ? Get(from, nextKey).
                let prop_value = cx.get(from, next_key)?;
                // b. Perform ? Set(to, nextKey, propValue, true).
                cx.set(to, next_key, prop_value, true)?;
            }
        }
    }
    // 4. Return to.
    Ok(to.value())
}

/// Object.is ( value1, value2 ) (§20.1.2.14).
fn is<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return SameValue(value1, value2).
    Ok(Local::boolean(cx.same_value(args.get(0), args.get(1))))
}

/// Object.hasOwn ( O, P ) (§20.1.2.13).
fn has_own<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(args.get(0))?;
    // 2. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Return ? HasOwnProperty(obj, key).
    Ok(Local::boolean(cx.has_own_property(obj, key)?))
}

// ---- helpers -----------------------------------------------------------------------------------

/// O.`[[OwnPropertyKeys]]`() as an array and its length (the array is new and dense: no JS
/// can see or change it).
fn own_keys<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<(Local<'s, JsArray>, u64)> {
    let keys = cx.own_property_keys(o)?;
    let len = length_of_array_like(cx, keys.into())?;
    Ok((keys, len))
}

/// Element `i` of an [`own_keys`] array as a property key (an atom or a symbol already).
fn key_at<'s>(cx: &mut Cx<'s>, keys: Local<'s, JsArray>, i: u64) -> JsResult<Local<'s, JsKey>> {
    let k = cx.get_index(keys, i)?;
    cx.to_property_key(k)
}

/// CreateArrayFromList ( elements ) (§7.3.17) for a `ScratchList`.
fn create_array_from_scratch<'s>(
    cx: &mut Cx<'s>,
    elements: &ScratchList<'s>,
) -> JsResult<Local<'s, JsArray>> {
    // 1. Let array be ! ArrayCreate(0).
    let array = cx.new_array();
    // 2. Let n be 0.
    // 3. For each element e of elements, do
    for n in 0..elements.len() {
        // a. Perform ! CreateDataPropertyOrThrow(array, ! ToString(𝔽(n)), e).
        // b. Set n to n + 1.
        let key = index_key(cx, n as u64);
        cx.create_data_property_or_throw(array, key, elements.get(n))?;
    }
    // 4. Return array.
    Ok(array)
}
