//! TypedArray objects (§23.2, `sec-typedarray-objects`): module `typed_array`, block 35. Owner
//! of this file: TypedArray TA-3 TA-CTORS R1 (design `docs/plan/notes/typedarray-design.md`
//! §8.2). Makes `%TypedArray%`, `%TypedArray.prototype%` and the 12 TypedArray constructors
//! with their prototypes (`IntrinsicId` 115-140), in the order of spec Table 71. The 12 global
//! keys are in the lazy group of `array_buffer` (TA-D14; `ep9_vm` builtins.rs `LAZY_GROUPS`).
//! The `[[Prototype]]` of each constructor is `%TypedArray%` (§23.2.6): the VM's link list
//! sets it when the group installs (`ep9_vm` builtins.rs `PROTO_LINKS`), as no table entry
//! can give a function object another prototype.
//!
//! ```text
//! mod.rs      (R1) the ids, the intrinsics, the properties and the row table VIEWS
//! ctor.rs     (R2) %TypedArray%, the 12 constructors, get @@species, the getters buffer,
//!                  byteLength, byteOffset and length (decision O3), the abstract operations
//! from_of.rs  (R3) %TypedArray%.from and %TypedArray%.of
//! proto.rs    (TA-4 TA-PROTO, its own block) the other %TypedArray.prototype% methods,
//!                  @@toStringTag and @@iterator
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-typed_array.txt`. A missing definition is appended at the end (35.20 and
//! on), never inserted.

use ep9_vm::{
    Atom, BuiltinDef, BuiltinModule, ConstValue, IntrinsicDef, IntrinsicId, IntrinsicValue,
    ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

mod ctor;
mod from_of;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 35;

mod def {
    pub const TYPED_ARRAY: u16 = 0;
    pub const FROM: u16 = 1;
    pub const OF: u16 = 2;
    pub const SPECIES: u16 = 3;
    pub const BUFFER: u16 = 4;
    pub const BYTE_LENGTH: u16 = 5;
    pub const BYTE_OFFSET: u16 = 6;
    pub const LENGTH: u16 = 7;
    /// The constructor of row `r` of [`super::VIEWS`] is `VIEW + r`.
    pub const VIEW: u16 = 8;
}

/// The 12 TypedArray constructors, one row per line of spec Table 71 (the row is the `element`
/// of the `ep9_vm` typed ABI, `abi/typed.rs`): the constructor, its prototype, its name and
/// its element size (`BYTES_PER_ELEMENT`).
pub(crate) const VIEWS: [(IntrinsicId, IntrinsicId, &str, u8); 12] = [
    (
        IntrinsicId::Int8Array,
        IntrinsicId::Int8ArrayPrototype,
        "Int8Array",
        1,
    ),
    (
        IntrinsicId::Uint8Array,
        IntrinsicId::Uint8ArrayPrototype,
        "Uint8Array",
        1,
    ),
    (
        IntrinsicId::Uint8ClampedArray,
        IntrinsicId::Uint8ClampedArrayPrototype,
        "Uint8ClampedArray",
        1,
    ),
    (
        IntrinsicId::Int16Array,
        IntrinsicId::Int16ArrayPrototype,
        "Int16Array",
        2,
    ),
    (
        IntrinsicId::Uint16Array,
        IntrinsicId::Uint16ArrayPrototype,
        "Uint16Array",
        2,
    ),
    (
        IntrinsicId::Int32Array,
        IntrinsicId::Int32ArrayPrototype,
        "Int32Array",
        4,
    ),
    (
        IntrinsicId::Uint32Array,
        IntrinsicId::Uint32ArrayPrototype,
        "Uint32Array",
        4,
    ),
    (
        IntrinsicId::BigInt64Array,
        IntrinsicId::BigInt64ArrayPrototype,
        "BigInt64Array",
        8,
    ),
    (
        IntrinsicId::BigUint64Array,
        IntrinsicId::BigUint64ArrayPrototype,
        "BigUint64Array",
        8,
    ),
    (
        IntrinsicId::Float16Array,
        IntrinsicId::Float16ArrayPrototype,
        "Float16Array",
        2,
    ),
    (
        IntrinsicId::Float32Array,
        IntrinsicId::Float32ArrayPrototype,
        "Float32Array",
        4,
    ),
    (
        IntrinsicId::Float64Array,
        IntrinsicId::Float64ArrayPrototype,
        "Float64Array",
        8,
    ),
];

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("TypedArray", 0, ctor::typed_array), // 0
    BuiltinDef::method("from", 1, from_of::from),                // 1
    BuiltinDef::method("of", 0, from_of::of),                    // 2
    BuiltinDef::getter("[Symbol.species]", ctor::species),       // 3
    BuiltinDef::getter("buffer", ctor::buffer),                  // 4
    BuiltinDef::getter("byteLength", ctor::byte_length),         // 5
    BuiltinDef::getter("byteOffset", ctor::byte_offset),         // 6
    BuiltinDef::getter("length", ctor::length),                  // 7
    BuiltinDef::constructor("Int8Array", 3, ctor::int8_array),   // 8
    BuiltinDef::constructor("Uint8Array", 3, ctor::uint8_array), // 9
    BuiltinDef::constructor("Uint8ClampedArray", 3, ctor::uint8_clamped_array), // 10
    BuiltinDef::constructor("Int16Array", 3, ctor::int16_array), // 11
    BuiltinDef::constructor("Uint16Array", 3, ctor::uint16_array), // 12
    BuiltinDef::constructor("Int32Array", 3, ctor::int32_array), // 13
    BuiltinDef::constructor("Uint32Array", 3, ctor::uint32_array), // 14
    BuiltinDef::constructor("BigInt64Array", 3, ctor::big_int64_array), // 15
    BuiltinDef::constructor("BigUint64Array", 3, ctor::big_uint64_array), // 16
    BuiltinDef::constructor("Float16Array", 3, ctor::float16_array), // 17
    BuiltinDef::constructor("Float32Array", 3, ctor::float32_array), // 18
    BuiltinDef::constructor("Float64Array", 3, ctor::float64_array), // 19
];

const fn builtin(id: IntrinsicId, d: u16) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Builtin(d),
    }
}

const fn object(id: IntrinsicId, proto: IntrinsicId) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Object { proto: Some(proto) },
    }
}

/// The constructor and the prototype of row `r` of [`VIEWS`] (a prototype after
/// `%TypedArray.prototype%`, which the table check wants made before it).
macro_rules! view_intrinsics {
    ($($r:literal),*) => {
        [$(
            builtin(VIEWS[$r].0, def::VIEW + $r),
            object(VIEWS[$r].1, IntrinsicId::TypedArrayPrototype),
        )*]
    };
}

/// The intrinsics this module makes. The group's installed mark is `%ArrayBuffer%` (the first
/// intrinsic of the group's first key module, `array_buffer`), not one of these.
pub const INTRINSICS: &[IntrinsicDef] = &{
    let views = view_intrinsics!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11);
    let mut all = [builtin(IntrinsicId::TypedArray, def::TYPED_ARRAY); 26];
    all[1] = object(
        IntrinsicId::TypedArrayPrototype,
        IntrinsicId::ObjectPrototype,
    );
    let mut i = 0;
    while i < views.len() {
        all[2 + i] = views[i];
        i += 1;
    }
    all
};

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

/// `BYTES_PER_ELEMENT` = the element size of row `r` of [`VIEWS`] ({}; §23.2.6.1, §23.2.7.1).
const fn bytes_per_element(r: usize) -> PropDef {
    PropDef::constant(
        PropKey::Str("BYTES_PER_ELEMENT"),
        ConstValue::Number(VIEWS[r].3 as f64),
        PropAttrs::NONE,
    )
}

/// The global key `name` = the constructor `id` ({w, c}; §19.3): a lazy key.
const fn global(name: &'static str, id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Str(name), id, PropAttrs::METHOD)
}

/// The properties of the constructor and of the prototype of row `r` of [`VIEWS`] (the order
/// of node v26's own keys: `prototype`, `BYTES_PER_ELEMENT`; `constructor`,
/// `BYTES_PER_ELEMENT`).
macro_rules! view_props {
    ($($r:literal),*) => {
        [$(
            ObjectProps {
                target: VIEWS[$r].0,
                props: &[prototype(VIEWS[$r].1), bytes_per_element($r)],
            },
            ObjectProps {
                target: VIEWS[$r].1,
                props: &[constructor(VIEWS[$r].0), bytes_per_element($r)],
            },
        )*]
    };
}

/// The 12 global keys, in the row order of [`VIEWS`] (`tests/typed_array.rs` checks it).
const GLOBALS: &[PropDef] = &[
    global("Int8Array", IntrinsicId::Int8Array),
    global("Uint8Array", IntrinsicId::Uint8Array),
    global("Uint8ClampedArray", IntrinsicId::Uint8ClampedArray),
    global("Int16Array", IntrinsicId::Int16Array),
    global("Uint16Array", IntrinsicId::Uint16Array),
    global("Int32Array", IntrinsicId::Int32Array),
    global("Uint32Array", IntrinsicId::Uint32Array),
    global("BigInt64Array", IntrinsicId::BigInt64Array),
    global("BigUint64Array", IntrinsicId::BigUint64Array),
    global("Float16Array", IntrinsicId::Float16Array),
    global("Float32Array", IntrinsicId::Float32Array),
    global("Float64Array", IntrinsicId::Float64Array),
];

/// The properties this module adds, per object (the order of node v26's own keys): first
/// `%TypedArray%` and `%TypedArray.prototype%`, then each constructor and its prototype, then
/// the 12 global keys.
pub const PROPS: &[ObjectProps] = &{
    let views = view_props!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11);
    let mut all = [ObjectProps {
        target: IntrinsicId::TypedArray,
        props: &[
            prototype(IntrinsicId::TypedArrayPrototype),
            method("of", def::OF),
            method("from", def::FROM),
            PropDef::accessor(
                PropKey::Symbol(WellKnownSymbol::SPECIES),
                Some(def::SPECIES),
                None,
            ),
        ],
    }; 27];
    all[1] = ObjectProps {
        target: IntrinsicId::TypedArrayPrototype,
        // The 4 getters of TA-3 (decision O3); TA-4 TA-PROTO appends the rest in its block.
        props: &[
            constructor(IntrinsicId::TypedArray),
            getter("buffer", def::BUFFER),
            getter("byteLength", def::BYTE_LENGTH),
            getter("byteOffset", def::BYTE_OFFSET),
            getter("length", def::LENGTH),
        ],
    };
    let mut i = 0;
    while i < views.len() {
        all[2 + i] = views[i];
        i += 1;
    }
    all[26] = ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: GLOBALS,
    };
    all
};

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "typed_array",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
