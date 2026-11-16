//! ArrayBuffer and SharedArrayBuffer (§25.1, §25.2): module `array_buffer`, block 39. Owner:
//! TypedArray TA-2 TA-BUFFERS (design `docs/plan/notes/typedarray-design.md` §8.2). Makes
//! `%ArrayBuffer%`, `%ArrayBuffer.prototype%`, `%SharedArrayBuffer%` and
//! `%SharedArrayBuffer.prototype%` (`IntrinsicId` 123-126). With `data_view` (block 40) they
//! are one lazy group of the global keys `ArrayBuffer`, `SharedArrayBuffer` and `DataView`
//! (TA-D14; `ep9_vm` builtins.rs `LAZY_GROUPS`): realm init gives the keys their places, and
//! the first use installs the group.
//!
//! ```text
//! this file: the spec steps that run JS or throw first (NewTarget, ToIndex, the options bag,
//!   SpeciesConstructor and the checks of the species result, the brand and IsShared tests)
//! ep9_vm abi/typed.rs (brand-checked; no JS): allocate_array_buffer,
//!   allocate_shared_array_buffer, is_array_buffer, is_shared_array_buffer,
//!   array_buffer_byte_length, array_buffer_max_byte_length, is_detached_buffer,
//!   array_buffer_copy_and_detach, array_buffer_resize, shared_array_buffer_grow,
//!   copy_buffer_bytes
//! ```
//!
//! Every user call (a valueOf of an argument, a `prototype` getter, a species constructor) can
//! detach or resize a buffer, so each step reads the buffer state again after it, in the
//! spec's order; the VM methods check their byte ranges against the current length too.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-array_buffer.txt`.

use ep9_vm::abi::JsObject;
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 39;

mod def {
    pub const ARRAY_BUFFER: u16 = 0;
    pub const IS_VIEW: u16 = 1;
    pub const AB_SPECIES: u16 = 2;
    pub const AB_BYTE_LENGTH: u16 = 3;
    pub const AB_SLICE: u16 = 4;
    pub const AB_MAX_BYTE_LENGTH: u16 = 5;
    pub const AB_RESIZABLE: u16 = 6;
    pub const AB_RESIZE: u16 = 7;
    pub const AB_TRANSFER: u16 = 8;
    pub const AB_TRANSFER_TO_FIXED_LENGTH: u16 = 9;
    pub const AB_DETACHED: u16 = 10;
    pub const SHARED_ARRAY_BUFFER: u16 = 11;
    pub const SAB_SPECIES: u16 = 12;
    pub const SAB_BYTE_LENGTH: u16 = 13;
    pub const SAB_SLICE: u16 = 14;
    pub const SAB_MAX_BYTE_LENGTH: u16 = 15;
    pub const SAB_GROWABLE: u16 = 16;
    pub const SAB_GROW: u16 = 17;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("ArrayBuffer", 1, array_buffer_ctor), // 0
    BuiltinDef::method("isView", 1, is_view),                     // 1
    BuiltinDef::getter("[Symbol.species]", species),              // 2
    BuiltinDef::getter("byteLength", ab_byte_length),             // 3
    BuiltinDef::method("slice", 2, ab_slice),                     // 4
    BuiltinDef::getter("maxByteLength", ab_max_byte_length),      // 5
    BuiltinDef::getter("resizable", ab_resizable),                // 6
    BuiltinDef::method("resize", 1, ab_resize),                   // 7
    BuiltinDef::method("transfer", 0, ab_transfer),               // 8
    BuiltinDef::method("transferToFixedLength", 0, ab_transfer_to_fixed_length), // 9
    BuiltinDef::getter("detached", ab_detached),                  // 10
    BuiltinDef::constructor("SharedArrayBuffer", 1, shared_array_buffer_ctor), // 11
    BuiltinDef::getter("[Symbol.species]", species),              // 12
    BuiltinDef::getter("byteLength", sab_byte_length),            // 13
    BuiltinDef::method("slice", 2, sab_slice),                    // 14
    BuiltinDef::getter("maxByteLength", sab_max_byte_length),     // 15
    BuiltinDef::getter("growable", sab_growable),                 // 16
    BuiltinDef::method("grow", 1, sab_grow),                      // 17
];

const fn object(id: IntrinsicId, proto: IntrinsicId) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Object { proto: Some(proto) },
    }
}

const fn builtin(id: IntrinsicId, d: u16) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Builtin(d),
    }
}

/// The intrinsics this module makes. `%ArrayBuffer%` comes first: it is the installed mark of
/// the lazy group (the value of its first key).
pub const INTRINSICS: &[IntrinsicDef] = &[
    builtin(IntrinsicId::ArrayBuffer, def::ARRAY_BUFFER),
    object(
        IntrinsicId::ArrayBufferPrototype,
        IntrinsicId::ObjectPrototype,
    ),
    builtin(IntrinsicId::SharedArrayBuffer, def::SHARED_ARRAY_BUFFER),
    object(
        IntrinsicId::SharedArrayBufferPrototype,
        IntrinsicId::ObjectPrototype,
    ),
];

const fn method(name: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), d)
}

/// A getter `name` ({c}).
const fn getter(name: &'static str, d: u16) -> PropDef {
    PropDef::accessor(PropKey::Str(name), Some(d), None)
}

/// `constructor` = `id` ({w, c}).
const fn constructor(id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Atom(Atom::Constructor), id, PropAttrs::METHOD)
}

/// `prototype` = `id` ({}).
const fn prototype(id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Atom(Atom::Prototype), id, PropAttrs::NONE)
}

/// `get [ @@species ]` ({c}).
const fn species_accessor(d: u16) -> PropDef {
    PropDef::accessor(PropKey::Symbol(WellKnownSymbol::SPECIES), Some(d), None)
}

/// `@@toStringTag` = `tag` ({c}).
const fn to_string_tag(tag: &'static str) -> PropDef {
    PropDef::constant(
        PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
        ConstValue::Str(tag),
        PropAttrs::CONFIGURABLE,
    )
}

/// The properties this module adds, per object (the order of node v26's own keys).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::ArrayBuffer,
        props: &[
            prototype(IntrinsicId::ArrayBufferPrototype),
            method("isView", def::IS_VIEW),
            species_accessor(def::AB_SPECIES),
        ],
    },
    ObjectProps {
        target: IntrinsicId::ArrayBufferPrototype,
        props: &[
            constructor(IntrinsicId::ArrayBuffer),
            getter("byteLength", def::AB_BYTE_LENGTH),
            method("slice", def::AB_SLICE),
            getter("maxByteLength", def::AB_MAX_BYTE_LENGTH),
            getter("resizable", def::AB_RESIZABLE),
            method("resize", def::AB_RESIZE),
            method("transfer", def::AB_TRANSFER),
            method("transferToFixedLength", def::AB_TRANSFER_TO_FIXED_LENGTH),
            getter("detached", def::AB_DETACHED),
            to_string_tag("ArrayBuffer"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::SharedArrayBuffer,
        props: &[
            prototype(IntrinsicId::SharedArrayBufferPrototype),
            species_accessor(def::SAB_SPECIES),
        ],
    },
    ObjectProps {
        target: IntrinsicId::SharedArrayBufferPrototype,
        props: &[
            constructor(IntrinsicId::SharedArrayBuffer),
            getter("byteLength", def::SAB_BYTE_LENGTH),
            method("slice", def::SAB_SLICE),
            getter("maxByteLength", def::SAB_MAX_BYTE_LENGTH),
            getter("growable", def::SAB_GROWABLE),
            method("grow", def::SAB_GROW),
            to_string_tag("SharedArrayBuffer"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 ArrayBuffer ( . . . ), SharedArrayBuffer ( . . . ) ({w, c}): the lazy keys.
        props: &[
            PropDef::intrinsic(
                PropKey::Str("ArrayBuffer"),
                IntrinsicId::ArrayBuffer,
                PropAttrs::METHOD,
            ),
            PropDef::intrinsic(
                PropKey::Str("SharedArrayBuffer"),
                IntrinsicId::SharedArrayBuffer,
                PropAttrs::METHOD,
            ),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "array_buffer",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- shared steps --------------------------------------------------------------------------

/// ToIndex (§7.1.22): an integer in [0, 2^53 - 1], else a RangeError with the text `what`.
pub(crate) fn to_index<'s>(cx: &mut Cx<'s>, v: Local<'s>, what: &str) -> JsResult<u64> {
    // 1. Let integer be ? ToIntegerOrInfinity(value).
    let integer = cx.to_integer_or_infinity(v)?;
    // 2. If integer is not in the inclusive interval from 0 to 2^53 - 1, throw a RangeError.
    if !(0.0..=9_007_199_254_740_991.0).contains(&integer) {
        return Err(cx.throw_range_error(what));
    }
    // 3. Return integer.
    Ok(integer as u64)
}

/// The RangeError text of a byte length that is no index.
const LENGTH: &str = "Invalid array buffer length";

/// The RangeError text of a maximum byte length that is no index or is below the length.
const MAX_LENGTH: &str = "Invalid array buffer max length";

/// The TypeError of a constructor called without `new` (step 1).
pub(crate) fn requires_new(cx: &mut Cx<'_>, name: &str) -> ep9_vm::Throw {
    let msg = format!("Constructor {name} requires 'new'");
    cx.throw_type_error(&msg)
}

/// The TypeError of RequireInternalSlot (§10.1.15) or of an IsSharedArrayBuffer test.
pub(crate) fn incompatible(cx: &mut Cx<'_>, what: &str) -> ep9_vm::Throw {
    let msg = format!("Method {what} called on incompatible receiver");
    cx.throw_type_error(&msg)
}

/// `this` if it is an ArrayBuffer (`RequireInternalSlot(O, [[ArrayBufferData]])` and
/// IsSharedArrayBuffer(O) false), else a TypeError naming `what`.
fn this_array_buffer<'s>(cx: &mut Cx<'s>, this: Local<'s>, what: &str) -> JsResult<Local<'s>> {
    if cx.is_array_buffer(this) {
        Ok(this)
    } else {
        Err(incompatible(cx, what))
    }
}

/// `this` if it is a SharedArrayBuffer (`RequireInternalSlot(O, [[ArrayBufferData]])` and
/// IsSharedArrayBuffer(O) true), else a TypeError naming `what`.
fn this_shared<'s>(cx: &mut Cx<'s>, this: Local<'s>, what: &str) -> JsResult<Local<'s>> {
    if cx.is_shared_array_buffer(this) {
        Ok(this)
    } else {
        Err(incompatible(cx, what))
    }
}

/// GetArrayBufferMaxByteLengthOption ( options ) (§25.1.3.7).
fn max_byte_length_option<'s>(cx: &mut Cx<'s>, options: Local<'s>) -> JsResult<Option<u64>> {
    // 1. If options is not an Object, return empty.
    let Some(options) = options.as_object() else {
        return Ok(None);
    };
    // 2. Let maxByteLength be ? Get(options, "maxByteLength").
    let max = cx.get(options, "maxByteLength")?;
    // 3. If maxByteLength is undefined, return empty.
    if max.is_undefined() {
        return Ok(None);
    }
    // 4. Return ? ToIndex(maxByteLength).
    to_index(cx, max, MAX_LENGTH).map(Some)
}

/// The `first` and `final` of a slice of `len` bytes (§25.1.6.7 steps 6-13, §25.2.5.6 steps
/// 5-12): the relative indices clamped to [0, len].
fn slice_range<'s>(
    cx: &mut Cx<'s>,
    len: u64,
    start: Local<'s>,
    end: Local<'s>,
) -> JsResult<(u64, u64)> {
    let len_f = len as f64;
    let clamp = |rel: f64| {
        if rel < 0.0 {
            (len_f + rel).max(0.0)
        } else {
            rel.min(len_f)
        }
    };
    // Let relativeStart be ? ToIntegerOrInfinity(start); -∞ gives 0, a negative value
    // max(len + relativeStart, 0), else min(relativeStart, len).
    let first = clamp(cx.to_integer_or_infinity(start)?);
    // If end is undefined, let relativeEnd be len; else ? ToIntegerOrInfinity(end).
    let rel_end = if end.is_undefined() {
        len_f
    } else {
        cx.to_integer_or_infinity(end)?
    };
    let fin = clamp(rel_end);
    Ok((first as u64, fin as u64))
}

/// SpeciesConstructor ( O, defaultConstructor ) (§7.3.22).
fn species_constructor<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
    default: IntrinsicId,
) -> JsResult<Local<'s>> {
    let o = o.as_object().expect("a buffer is an object");
    // 1. Let C be ? Get(O, "constructor").
    let c = cx.get(o, "constructor")?;
    // 2. If C is undefined, return defaultConstructor.
    if c.is_undefined() {
        return Ok(cx.intrinsic(default).value());
    }
    // 3. If C is not an Object, throw a TypeError exception.
    let Some(c) = c.as_object() else {
        return Err(cx.throw_type_error("The .constructor property is not an object"));
    };
    // 4. Let S be ? Get(C, %Symbol.species%).
    let species = crate::string::well_known(cx, "species")?;
    let s = cx.get(c, species)?;
    // 5. If S is either undefined or null, return defaultConstructor.
    if s.is_undefined_or_null() {
        return Ok(cx.intrinsic(default).value());
    }
    // 6. If IsConstructor(S) is true, return S.
    if cx.is_constructor(s) {
        return Ok(s);
    }
    // 7. Throw a TypeError exception.
    Err(cx.throw_type_error("object.constructor[Symbol.species] is not a constructor"))
}

/// `get ArrayBuffer [ @@species ]` (§25.1.5.3) and `get SharedArrayBuffer [ @@species ]`
/// (§25.2.4.2): the this value.
fn species<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return the this value.
    Ok(this)
}

// ---- %ArrayBuffer% -------------------------------------------------------------------------

/// ArrayBuffer ( length [ , options ] ) (§25.1.4.1).
fn array_buffer_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "ArrayBuffer"));
    };
    // 2. Let byteLength be ? ToIndex(length).
    let byte_length = to_index(cx, args.get(0), LENGTH)?;
    // 3. Let requestedMaxByteLength be ? GetArrayBufferMaxByteLengthOption(options).
    let max = max_byte_length_option(cx, args.get(1))?;
    // 4. Return ? AllocateArrayBuffer(NewTarget, byteLength, requestedMaxByteLength).
    //    AllocateArrayBuffer step 3.a: byteLength > maxByteLength is a RangeError before
    //    OrdinaryCreateFromConstructor.
    if max.is_some_and(|m| byte_length > m) {
        return Err(cx.throw_range_error(MAX_LENGTH));
    }
    //    Step 4: OrdinaryCreateFromConstructor (a `prototype` getter can run here).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::ArrayBufferPrototype)?;
    //    Steps 5-9: CreateByteDataBlock (a RangeError when it cannot be made) and the slots.
    Ok(cx.allocate_array_buffer(proto, byte_length, max)?.value())
}

/// ArrayBuffer.isView ( arg ) (§25.1.5.1).
fn is_view<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let arg = args.get(0);
    // 1. If arg is not an Object, return false. 2. If arg has a [[ViewedArrayBuffer]] internal
    // slot, return true. 3. Return false.
    let view = cx.typed_array_element(arg).is_some() || cx.is_data_view(arg);
    Ok(Local::boolean(view))
}

/// get ArrayBuffer.prototype.byteLength (§25.1.6.2).
fn ab_byte_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) is a TypeError.
    let o = this_array_buffer(cx, this, "get ArrayBuffer.prototype.byteLength")?;
    // 4. If IsDetachedBuffer(O) is true, return +0𝔽. 5. Return O.[[ArrayBufferByteLength]].
    let n = cx.array_buffer_byte_length(o)?;
    Ok(Local::number(n as f64))
}

/// get ArrayBuffer.prototype.maxByteLength (§25.1.6.4).
fn ab_max_byte_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) is a TypeError.
    let o = this_array_buffer(cx, this, "get ArrayBuffer.prototype.maxByteLength")?;
    // 4. If IsDetachedBuffer(O) is true, return +0𝔽.
    if cx.is_detached_buffer(o) {
        return Ok(Local::number(0.0));
    }
    // 5-6. IsFixedLengthArrayBuffer(O): [[ArrayBufferByteLength]], else
    //      [[ArrayBufferMaxByteLength]].
    let n = match cx.array_buffer_max_byte_length(o)? {
        Some(max) => max,
        None => cx.array_buffer_byte_length(o)?,
    };
    Ok(Local::number(n as f64))
}

/// get ArrayBuffer.prototype.resizable (§25.1.6.5).
fn ab_resizable<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) is a TypeError.
    let o = this_array_buffer(cx, this, "get ArrayBuffer.prototype.resizable")?;
    // 4. If IsFixedLengthArrayBuffer(O) is false, return true; otherwise return false.
    let resizable = cx.array_buffer_max_byte_length(o)?.is_some();
    Ok(Local::boolean(resizable))
}

/// get ArrayBuffer.prototype.detached (§25.1.6.3).
fn ab_detached<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) is a TypeError.
    let o = this_array_buffer(cx, this, "get ArrayBuffer.prototype.detached")?;
    // 4. Return IsDetachedBuffer(O).
    Ok(Local::boolean(cx.is_detached_buffer(o)))
}

/// ArrayBuffer.prototype.resize ( newLength ) (§25.1.6.6).
fn ab_resize<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    const WHAT: &str = "ArrayBuffer.prototype.resize";
    // 1-3. RequireInternalSlot(O, [[ArrayBufferMaxByteLength]]); IsSharedArrayBuffer(O) is a
    //      TypeError.
    let o = this_array_buffer(cx, this, WHAT)?;
    if cx.array_buffer_max_byte_length(o)?.is_none() {
        return Err(incompatible(cx, WHAT));
    }
    // 4. Let newByteLength be ? ToIndex(newLength).
    let new_len = to_index(cx, args.get(0), LENGTH)?;
    // 5. If IsDetachedBuffer(O) is true, throw a TypeError exception.
    if cx.is_detached_buffer(o) {
        return Err(cx.throw_type_error(
            "Cannot perform ArrayBuffer.prototype.resize on a detached ArrayBuffer",
        ));
    }
    // 6. If newByteLength > O.[[ArrayBufferMaxByteLength]], throw a RangeError exception.
    // 7-12. The resize (new bytes are 0; the store keeps its address).
    cx.array_buffer_resize(o, new_len)?;
    // 13. Return undefined.
    Ok(Local::undefined())
}

/// ArrayBuffer.prototype.slice ( start, end ) (§25.1.6.7).
fn ab_slice<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) is a TypeError.
    let o = this_array_buffer(cx, this, "ArrayBuffer.prototype.slice")?;
    // 4. If IsDetachedBuffer(O) is true, throw a TypeError exception.
    if cx.is_detached_buffer(o) {
        return Err(cx.throw_type_error(
            "Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer",
        ));
    }
    // 5. Let len be O.[[ArrayBufferByteLength]].
    let len = cx.array_buffer_byte_length(o)?;
    // 6-13. first and final.
    let (first, fin) = slice_range(cx, len, args.get(0), args.get(1))?;
    // 14. Let newLen be max(final - first, 0).
    let new_len = fin.saturating_sub(first);
    // 15. Let ctor be ? SpeciesConstructor(O, %ArrayBuffer%).
    let ctor = species_constructor(cx, o, IntrinsicId::ArrayBuffer)?;
    // 16. Let new be ? Construct(ctor, « 𝔽(newLen) »).
    let new = cx.construct(ctor, &[Local::number(new_len as f64)], ctor)?;
    // 17-18. RequireInternalSlot(new, [[ArrayBufferData]]); IsSharedArrayBuffer(new) is a
    //        TypeError.
    if !cx.is_array_buffer(new) {
        return Err(cx.throw_type_error(
            "The species constructor of ArrayBuffer.prototype.slice made no ArrayBuffer",
        ));
    }
    // 19. If IsDetachedBuffer(new) is true, throw a TypeError exception.
    if cx.is_detached_buffer(new) {
        return Err(cx.throw_type_error(
            "The species constructor of ArrayBuffer.prototype.slice made a detached ArrayBuffer",
        ));
    }
    // 20. If SameValue(new, O) is true, throw a TypeError exception.
    if cx.same_value(new, o) {
        return Err(
            cx.throw_type_error("ArrayBuffer subclass returned this from species constructor")
        );
    }
    // 21. If new.[[ArrayBufferByteLength]] < newLen, throw a TypeError exception.
    if cx.array_buffer_byte_length(new)? < new_len {
        return Err(cx.throw_type_error(
            "Derived ArrayBuffer constructor created a buffer which was too small",
        ));
    }
    // 22. NOTE: Side-effects of the above steps may have detached or resized O.
    // 23. If IsDetachedBuffer(O) is true, throw a TypeError exception.
    if cx.is_detached_buffer(o) {
        return Err(cx.throw_type_error(
            "Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer",
        ));
    }
    // 24-26. fromBuf, toBuf; let currentLen be O.[[ArrayBufferByteLength]].
    let current_len = cx.array_buffer_byte_length(o)?;
    // 27. If first < currentLen, copy min(newLen, currentLen - first) bytes from first.
    if first < current_len {
        let count = new_len.min(current_len - first);
        cx.copy_buffer_bytes(new, 0, o, first, count)?;
    }
    // 28. Return new.
    Ok(new.value())
}

/// ArrayBufferCopyAndDetach ( arrayBuffer, newLength, preserveResizability ) (§25.1.3.3) for
/// `transfer` (`preserve` true) and `transferToFixedLength` (false).
fn copy_and_detach<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    new_length: Local<'s>,
    preserve: bool,
    what: &str,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(arrayBuffer, [[ArrayBufferData]]); IsSharedArrayBuffer is a
    //      TypeError.
    let o = this_array_buffer(cx, this, what)?;
    // 3. If newLength is undefined, let newByteLength be arrayBuffer.[[ArrayBufferByteLength]];
    //    else let newByteLength be ? ToIndex(newLength).
    let new_len = if new_length.is_undefined() {
        cx.array_buffer_byte_length(o)?
    } else {
        to_index(cx, new_length, LENGTH)?
    };
    // 4. If IsDetachedBuffer(arrayBuffer) is true, throw a TypeError exception.
    if cx.is_detached_buffer(o) {
        let msg = format!("Cannot perform {what} on a detached ArrayBuffer");
        return Err(cx.throw_type_error(&msg));
    }
    // 5-12. The new maximum, the detach key check, AllocateArrayBuffer(%ArrayBuffer%, ...),
    //       the copy (or the move of the store) and DetachArrayBuffer(arrayBuffer).
    let proto = cx.intrinsic(IntrinsicId::ArrayBufferPrototype);
    Ok(cx
        .array_buffer_copy_and_detach(o, proto, new_len, preserve)?
        .value())
}

/// `ArrayBuffer.prototype.transfer ( [ newLength ] )` (§25.1.6.8).
fn ab_transfer<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Return ? ArrayBufferCopyAndDetach(O, newLength, preserve-resizability).
    copy_and_detach(
        cx,
        this,
        args.get(0),
        true,
        "ArrayBuffer.prototype.transfer",
    )
}

/// `ArrayBuffer.prototype.transferToFixedLength ( [ newLength ] )` (§25.1.6.9).
fn ab_transfer_to_fixed_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Return ? ArrayBufferCopyAndDetach(O, newLength, fixed-length).
    let what = "ArrayBuffer.prototype.transferToFixedLength";
    copy_and_detach(cx, this, args.get(0), false, what)
}

// ---- %SharedArrayBuffer% -------------------------------------------------------------------

/// SharedArrayBuffer ( length [ , options ] ) (§25.2.3.1).
fn shared_array_buffer_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "SharedArrayBuffer"));
    };
    // 2. Let byteLength be ? ToIndex(length).
    let byte_length = to_index(cx, args.get(0), LENGTH)?;
    // 3. Let requestedMaxByteLength be ? GetArrayBufferMaxByteLengthOption(options).
    let max = max_byte_length_option(cx, args.get(1))?;
    // 4. Return ? AllocateSharedArrayBuffer(NewTarget, byteLength, requestedMaxByteLength).
    //    AllocateSharedArrayBuffer step 3: byteLength > maxByteLength is a RangeError before
    //    OrdinaryCreateFromConstructor.
    if max.is_some_and(|m| byte_length > m) {
        return Err(cx.throw_range_error(MAX_LENGTH));
    }
    //    Step 4: OrdinaryCreateFromConstructor (a `prototype` getter can run here).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::SharedArrayBufferPrototype)?;
    //    Steps 5-11: CreateSharedByteDataBlock (a RangeError when it cannot be made) and the
    //    slots.
    Ok(cx
        .allocate_shared_array_buffer(proto, byte_length, max)?
        .value())
}

/// get SharedArrayBuffer.prototype.byteLength (§25.2.5.2).
fn sab_byte_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) false is a
    //      TypeError.
    let o = this_shared(cx, this, "get SharedArrayBuffer.prototype.byteLength")?;
    // 4. Return 𝔽(ArrayBufferByteLength(O, seq-cst)).
    let n = cx.array_buffer_byte_length(o)?;
    Ok(Local::number(n as f64))
}

/// get SharedArrayBuffer.prototype.growable (§25.2.5.4).
fn sab_growable<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) false is a
    //      TypeError.
    let o = this_shared(cx, this, "get SharedArrayBuffer.prototype.growable")?;
    // 4. If IsFixedLengthArrayBuffer(O) is false, return true; otherwise return false.
    let growable = cx.array_buffer_max_byte_length(o)?.is_some();
    Ok(Local::boolean(growable))
}

/// get SharedArrayBuffer.prototype.maxByteLength (§25.2.5.5).
fn sab_max_byte_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) false is a
    //      TypeError.
    let o = this_shared(cx, this, "get SharedArrayBuffer.prototype.maxByteLength")?;
    // 4-5. IsFixedLengthArrayBuffer(O): [[ArrayBufferByteLength]], else
    //      [[ArrayBufferMaxByteLength]].
    let n = match cx.array_buffer_max_byte_length(o)? {
        Some(max) => max,
        None => cx.array_buffer_byte_length(o)?,
    };
    Ok(Local::number(n as f64))
}

/// SharedArrayBuffer.prototype.grow ( newLength ) (§25.2.5.3).
fn sab_grow<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    const WHAT: &str = "SharedArrayBuffer.prototype.grow";
    // 1-3. RequireInternalSlot(O, [[ArrayBufferMaxByteLength]]); IsSharedArrayBuffer(O) false
    //      is a TypeError.
    let o = this_shared(cx, this, WHAT)?;
    if cx.array_buffer_max_byte_length(o)?.is_none() {
        return Err(incompatible(cx, WHAT));
    }
    // 4. Let newByteLength be ? ToIndex(newLength).
    let new_len = to_index(cx, args.get(0), LENGTH)?;
    // 5-12. A RangeError for a length below the byte length or above the maximum; else the
    //       grow (new bytes are 0; a seq-cst store of the length).
    cx.shared_array_buffer_grow(o, new_len)?;
    Ok(Local::undefined())
}

/// SharedArrayBuffer.prototype.slice ( start, end ) (§25.2.5.6).
fn sab_slice<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. RequireInternalSlot(O, [[ArrayBufferData]]); IsSharedArrayBuffer(O) false is a
    //      TypeError.
    let o = this_shared(cx, this, "SharedArrayBuffer.prototype.slice")?;
    // 4. Let len be ArrayBufferByteLength(O, seq-cst).
    let len = cx.array_buffer_byte_length(o)?;
    // 5-12. first and final.
    let (first, fin) = slice_range(cx, len, args.get(0), args.get(1))?;
    // 13. Let newLen be max(final - first, 0).
    let new_len = fin.saturating_sub(first);
    // 14. Let ctor be ? SpeciesConstructor(O, %SharedArrayBuffer%).
    let ctor = species_constructor(cx, o, IntrinsicId::SharedArrayBuffer)?;
    // 15. Let new be ? Construct(ctor, « 𝔽(newLen) »).
    let new: Local<'s, JsObject> = cx.construct(ctor, &[Local::number(new_len as f64)], ctor)?;
    // 16-17. RequireInternalSlot(new, [[ArrayBufferData]]); IsSharedArrayBuffer(new) false is
    //        a TypeError.
    if !cx.is_shared_array_buffer(new) {
        return Err(cx.throw_type_error(
            "The species constructor of SharedArrayBuffer.prototype.slice made no SharedArrayBuffer",
        ));
    }
    // 18. If new.[[ArrayBufferData]] is O.[[ArrayBufferData]], throw a TypeError exception.
    //     (The block test, brand-checked: two objects of one agent can share a block.)
    if cx.is_same_shared_block(new, o) {
        return Err(cx.throw_type_error(
            "SharedArrayBuffer subclass returned this from species constructor",
        ));
    }
    // 19. If ArrayBufferByteLength(new, seq-cst) < newLen, throw a TypeError exception.
    if cx.array_buffer_byte_length(new)? < new_len {
        return Err(cx.throw_type_error(
            "Derived SharedArrayBuffer constructor created a buffer which was too small",
        ));
    }
    // 20-22. CopyDataBlockBytes(toBuf, 0, fromBuf, first, newLen) (a SharedArrayBuffer never
    //        shrinks, so first + newLen is inside O).
    cx.copy_buffer_bytes(new, 0, o, first, new_len)?;
    // 23. Return new.
    Ok(new.value())
}
