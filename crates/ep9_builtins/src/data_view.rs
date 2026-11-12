//! DataView (§25.3): module `data_view`, block 39. Owner: TypedArray TA-2 TA-BUFFERS (design
//! `docs/plan/notes/typedarray-design.md` §8.2). Makes `%DataView%` and `%DataView.prototype%`
//! (`IntrinsicId` 125-126). The global key `DataView` is in the lazy group of `array_buffer`
//! (TA-D14).
//!
//! ```text
//! this file: NewTarget, the buffer brand test, ToIndex, ToNumber / ToBigInt, ToBoolean and the
//!   bound checks of the constructor, in spec order (a valueOf or a `prototype` getter can
//!   detach or resize the buffer between them)
//! ep9_vm abi/typed.rs (brand-checked; no JS): new_data_view, is_data_view, data_view_buffer,
//!   data_view_byte_offset, data_view_byte_length (IsViewOutOfBounds, GetViewByteLength),
//!   data_view_get_value / data_view_set_value (GetViewValue, SetViewValue after their
//!   conversions: the out-of-bounds TypeError and the range RangeError are checked there,
//!   against the buffer's byte length at the access)
//! ```
//!
//! **Element types.** The `element: u8` of the ABI is the row of spec Table 71: Int8 0, Uint8 1,
//! Int16 3, Uint16 4, Int32 5, Uint32 6, BigInt64 7, BigUint64 8, Float16 9, Float32 10,
//! Float64 11 (a DataView has no Uint8Clamped, row 2). Float16 rounds through `ep9_math` (one
//! rounding, ties to even; TA-D12).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-data_view.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

use crate::array_buffer::{incompatible, requires_new, to_index};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 39;

mod def {
    pub const DATA_VIEW: u16 = 0;
    pub const BUFFER: u16 = 1;
    pub const BYTE_LENGTH: u16 = 2;
    pub const BYTE_OFFSET: u16 = 3;
    /// `get<Type>` is `ACCESS + 2 * i`, `set<Type>` is `ACCESS + 2 * i + 1`, for row `i` of
    /// [`super::TYPES`].
    pub const ACCESS: u16 = 4;
}

/// The element types of the get and set methods, in the order of node v26's own keys of
/// `%DataView.prototype%`: the method name suffix and the Table 71 row.
const TYPES: [(&str, u8); 11] = [
    ("Int8", 0),
    ("Uint8", 1),
    ("Int16", 3),
    ("Uint16", 4),
    ("Int32", 5),
    ("Uint32", 6),
    ("Float16", 9),
    ("Float32", 10),
    ("Float64", 11),
    ("BigInt64", 7),
    ("BigUint64", 8),
];

/// The `get<Type>` and `set<Type>` host functions of one element type.
macro_rules! accessors {
    ($get:ident, $set:ident, $row:expr) => {
        fn $get<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            get_view_value(cx, this, args, $row)
        }

        fn $set<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            set_view_value(cx, this, args, $row)
        }
    };
}

accessors!(get_int8, set_int8, 0);
accessors!(get_uint8, set_uint8, 1);
accessors!(get_int16, set_int16, 3);
accessors!(get_uint16, set_uint16, 4);
accessors!(get_int32, set_int32, 5);
accessors!(get_uint32, set_uint32, 6);
accessors!(get_float16, set_float16, 9);
accessors!(get_float32, set_float32, 10);
accessors!(get_float64, set_float64, 11);
accessors!(get_big_int64, set_big_int64, 7);
accessors!(get_big_uint64, set_big_uint64, 8);

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DataView", 1, data_view_ctor), // 0
    BuiltinDef::getter("buffer", buffer),                   // 1
    BuiltinDef::getter("byteLength", byte_length),          // 2
    BuiltinDef::getter("byteOffset", byte_offset),          // 3
    BuiltinDef::method("getInt8", 1, get_int8),             // 4
    BuiltinDef::method("setInt8", 2, set_int8),             // 5
    BuiltinDef::method("getUint8", 1, get_uint8),           // 6
    BuiltinDef::method("setUint8", 2, set_uint8),           // 7
    BuiltinDef::method("getInt16", 1, get_int16),           // 8
    BuiltinDef::method("setInt16", 2, set_int16),           // 9
    BuiltinDef::method("getUint16", 1, get_uint16),         // 10
    BuiltinDef::method("setUint16", 2, set_uint16),         // 11
    BuiltinDef::method("getInt32", 1, get_int32),           // 12
    BuiltinDef::method("setInt32", 2, set_int32),           // 13
    BuiltinDef::method("getUint32", 1, get_uint32),         // 14
    BuiltinDef::method("setUint32", 2, set_uint32),         // 15
    BuiltinDef::method("getFloat16", 1, get_float16),       // 16
    BuiltinDef::method("setFloat16", 2, set_float16),       // 17
    BuiltinDef::method("getFloat32", 1, get_float32),       // 18
    BuiltinDef::method("setFloat32", 2, set_float32),       // 19
    BuiltinDef::method("getFloat64", 1, get_float64),       // 20
    BuiltinDef::method("setFloat64", 2, set_float64),       // 21
    BuiltinDef::method("getBigInt64", 1, get_big_int64),    // 22
    BuiltinDef::method("setBigInt64", 2, set_big_int64),    // 23
    BuiltinDef::method("getBigUint64", 1, get_big_uint64),  // 24
    BuiltinDef::method("setBigUint64", 2, set_big_uint64),  // 25
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::DataView,
        value: IntrinsicValue::Builtin(def::DATA_VIEW),
    },
    IntrinsicDef {
        id: IntrinsicId::DataViewPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The method `get<Type>` or `set<Type>` of row `i` of [`TYPES`] (`set`: the setter).
const fn access(name: &'static str, i: u16, set: bool) -> PropDef {
    let d = def::ACCESS + 2 * i + if set { 1 } else { 0 };
    PropDef::method(PropKey::Str(name), d)
}

/// A getter `name` ({c}).
const fn getter(name: &'static str, d: u16) -> PropDef {
    PropDef::accessor(PropKey::Str(name), Some(d), None)
}

/// The properties this module adds, per object (the order of node v26's own keys).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::DataView,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::DataViewPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::DataViewPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::DataView,
                PropAttrs::METHOD,
            ),
            getter("buffer", def::BUFFER),
            getter("byteLength", def::BYTE_LENGTH),
            getter("byteOffset", def::BYTE_OFFSET),
            access("getInt8", 0, false),
            access("setInt8", 0, true),
            access("getUint8", 1, false),
            access("setUint8", 1, true),
            access("getInt16", 2, false),
            access("setInt16", 2, true),
            access("getUint16", 3, false),
            access("setUint16", 3, true),
            access("getInt32", 4, false),
            access("setInt32", 4, true),
            access("getUint32", 5, false),
            access("setUint32", 5, true),
            access("getFloat16", 6, false),
            access("setFloat16", 6, true),
            access("getFloat32", 7, false),
            access("setFloat32", 7, true),
            access("getFloat64", 8, false),
            access("setFloat64", 8, true),
            access("getBigInt64", 9, false),
            access("setBigInt64", 9, true),
            access("getBigUint64", 10, false),
            access("setBigUint64", 10, true),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("DataView"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 DataView ( . . . ) ({w, c}): a lazy key.
        props: &[PropDef::intrinsic(
            PropKey::Str("DataView"),
            IntrinsicId::DataView,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "data_view",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The RangeError text of a byte offset or a byte index that is no index.
const OFFSET: &str = "Offset is outside the bounds of the DataView";

/// `this` if it is a DataView (`RequireInternalSlot(O, [[DataView]])`), else a TypeError naming
/// `what`.
fn this_data_view<'s>(cx: &mut Cx<'s>, this: Local<'s>, what: &str) -> JsResult<Local<'s>> {
    if cx.is_data_view(this) {
        Ok(this)
    } else {
        Err(incompatible(cx, what))
    }
}

/// The TypeError of a DataView that is out of bounds (IsViewOutOfBounds; a detached buffer
/// included).
fn out_of_bounds(cx: &mut Cx<'_>, what: &str) -> ep9_vm::Throw {
    let msg = format!("Cannot perform {what} on a detached or out-of-bounds ArrayBuffer");
    cx.throw_type_error(&msg)
}

/// DataView ( buffer [ , byteOffset [ , byteLength ] ] ) (§25.3.2.1).
fn data_view_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (buffer, byte_offset, byte_length) = (args.get(0), args.get(1), args.get(2));
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "DataView"));
    };
    // 2. Perform ? RequireInternalSlot(buffer, [[ArrayBufferData]]).
    if !cx.is_array_buffer(buffer) && !cx.is_shared_array_buffer(buffer) {
        return Err(
            cx.throw_type_error("First argument to DataView constructor must be an ArrayBuffer")
        );
    }
    // 3. Let offset be ? ToIndex(byteOffset).
    let offset = to_index(
        cx,
        byte_offset,
        "Start offset is outside the bounds of the buffer",
    )?;
    // 4. If IsDetachedBuffer(buffer) is true, throw a TypeError exception.
    if cx.is_detached_buffer(buffer) {
        return Err(
            cx.throw_type_error("Cannot perform DataView constructor on a detached ArrayBuffer")
        );
    }
    // 5. Let bufferByteLength be ArrayBufferByteLength(buffer, seq-cst).
    let buffer_len = cx.array_buffer_byte_length(buffer)?;
    // 6. If offset > bufferByteLength, throw a RangeError exception.
    if offset > buffer_len {
        return Err(cx.throw_range_error("Start offset is outside the bounds of the buffer"));
    }
    // 7-8. If byteLength is undefined: a fixed-length buffer gives bufferByteLength - offset,
    //      a resizable or growable one an auto view (`None`: `new_data_view` makes either).
    // 9. Else let viewByteLength be ? ToIndex(byteLength); if offset + viewByteLength >
    //    bufferByteLength, throw a RangeError exception.
    let view_len = if byte_length.is_undefined() {
        None
    } else {
        let n = to_index(cx, byte_length, "Invalid DataView length")?;
        if offset.checked_add(n).is_none_or(|end| end > buffer_len) {
            return Err(cx.throw_range_error("Invalid DataView length"));
        }
        Some(n)
    };
    // 10. Let O be ? OrdinaryCreateFromConstructor(NewTarget, "%DataView.prototype%", ...)
    //     (a `prototype` getter can run here).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::DataViewPrototype)?;
    // 11. If IsDetachedBuffer(buffer) is true, throw a TypeError exception.
    if cx.is_detached_buffer(buffer) {
        return Err(
            cx.throw_type_error("Cannot perform DataView constructor on a detached ArrayBuffer")
        );
    }
    // 12. Set bufferByteLength to ArrayBufferByteLength(buffer, seq-cst).
    let buffer_len = cx.array_buffer_byte_length(buffer)?;
    // 13. If offset > bufferByteLength, throw a RangeError exception.
    if offset > buffer_len {
        return Err(cx.throw_range_error("Start offset is outside the bounds of the buffer"));
    }
    // 14. If byteLength is not undefined, then: if offset + viewByteLength > bufferByteLength,
    //     throw a RangeError exception.
    if view_len.is_some_and(|n| offset + n > buffer_len) {
        return Err(cx.throw_range_error("Invalid DataView length"));
    }
    // 15-19. The slots: [[ViewedArrayBuffer]], [[ByteLength]] (for byteLength undefined on a
    //        fixed-length buffer: bufferByteLength - offset), [[ByteOffset]]. Return O.
    Ok(cx.new_data_view(proto, buffer, offset, view_len)?.value())
}

/// get DataView.prototype.buffer (§25.3.4.1).
fn buffer<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[DataView]]).
    let o = this_data_view(cx, this, "get DataView.prototype.buffer")?;
    // 3-4. Return O.[[ViewedArrayBuffer]].
    Ok(cx.data_view_buffer(o)?.value())
}

/// get DataView.prototype.byteLength (§25.3.4.2).
fn byte_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    const WHAT: &str = "get DataView.prototype.byteLength";
    // 1-2. RequireInternalSlot(O, [[DataView]]).
    let o = this_data_view(cx, this, WHAT)?;
    // 3-5. If IsViewOutOfBounds(viewRecord) is true, throw a TypeError exception.
    // 6-7. Return 𝔽(GetViewByteLength(viewRecord)).
    match cx.data_view_byte_length(o)? {
        Some(n) => Ok(Local::number(n as f64)),
        None => Err(out_of_bounds(cx, WHAT)),
    }
}

/// get DataView.prototype.byteOffset (§25.3.4.3).
fn byte_offset<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    const WHAT: &str = "get DataView.prototype.byteOffset";
    // 1-2. RequireInternalSlot(O, [[DataView]]).
    let o = this_data_view(cx, this, WHAT)?;
    // 3-5. If IsViewOutOfBounds(viewRecord) is true, throw a TypeError exception.
    // 6-7. Return 𝔽(O.[[ByteOffset]]).
    match cx.data_view_byte_offset(o)? {
        Some(n) => Ok(Local::number(n as f64)),
        None => Err(out_of_bounds(cx, WHAT)),
    }
}

/// The method name of row `t` (`get` or `set`), for an error text.
fn method_name(t: u8, set: bool) -> String {
    let name = TYPES
        .iter()
        .find(|&&(_, r)| r == t)
        .map_or("?", |&(n, _)| n);
    let verb = if set { "set" } else { "get" };
    format!("DataView.prototype.{verb}{name}")
}

/// GetViewValue ( view, requestIndex, isLittleEndian, type ) (§25.3.1.5) for `get<Type>`
/// ( byteOffset [ , littleEndian ] ) (§25.3.4.5-§25.3.4.15). Int8 and Uint8 take no
/// littleEndian argument (the spec passes true; one byte has no order).
fn get_view_value<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    t: u8,
) -> JsResult<Local<'s>> {
    // 1-2. Perform ? RequireInternalSlot(view, [[DataView]]).
    if !cx.is_data_view(this) {
        return Err(incompatible(cx, &method_name(t, false)));
    }
    // 3. Let getIndex be ? ToIndex(requestIndex).
    let index = to_index(cx, args.get(0), OFFSET)?;
    // 4. Set isLittleEndian to ToBoolean(isLittleEndian).
    let little_endian = t <= 1 || cx.to_boolean(args.get(1));
    // 5-11. The out-of-bounds TypeError, the RangeError of getIndex + elementSize >
    //       viewSize, and GetValueFromBuffer at getIndex + viewOffset (in `ep9_vm`, against
    //       the buffer's byte length now).
    cx.data_view_get_value(this, index, t, little_endian)
}

/// SetViewValue ( view, requestIndex, isLittleEndian, type, value ) (§25.3.1.6) for
/// `set<Type>` ( byteOffset, value [ , littleEndian ] ) (§25.3.4.16-§25.3.4.26).
fn set_view_value<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    t: u8,
) -> JsResult<Local<'s>> {
    // 1-2. Perform ? RequireInternalSlot(view, [[DataView]]).
    if !cx.is_data_view(this) {
        return Err(incompatible(cx, &method_name(t, true)));
    }
    // 3. Let getIndex be ? ToIndex(requestIndex).
    let index = to_index(cx, args.get(0), OFFSET)?;
    // 4-5. If IsBigIntElementType(type), let numberValue be ? ToBigInt(value); otherwise let
    //      numberValue be ? ToNumber(value).
    let value = if t == 7 || t == 8 {
        cx.to_bigint(args.get(1))?
    } else {
        let n = cx.to_number(args.get(1))?;
        Local::number(n)
    };
    // 6. Set isLittleEndian to ToBoolean(isLittleEndian).
    let little_endian = t <= 1 || cx.to_boolean(args.get(2));
    // 7-14. The out-of-bounds TypeError, the RangeError, and SetValueInBuffer (in `ep9_vm`).
    cx.data_view_set_value(this, index, t, value, little_endian)?;
    // 15. Return undefined.
    Ok(Local::undefined())
}
