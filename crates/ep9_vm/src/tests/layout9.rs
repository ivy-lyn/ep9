//! OBJECT_LAYOUT 9 (TypedArray TA-0 TA-DECL; design `docs/plan/notes/typedarray-design.md` §2,
//! §3.2; TA-D13): the version pins, the 12 view kind ids and BYTES with their range tests, the
//! `ep9_vm::layout` exports of the view and BYTES cells, the elements kind values 7
//! TYPED_FIXED and 8 TYPED_VARIABLE with their storage form and kinds-set bits, and the
//! ObjectClass appends SharedArrayBuffer and DataView. Only facts that stay true after TA-1
//! builds the cells are pinned here (the stub bodies are tested in `abi/typed.rs`).

use std::hint::black_box;

use super::{config, isolate};
use crate::abi::ObjectClass;
use crate::elements::{self, Storage, kind as ek};
use crate::frame::{UNDEFINED, int32_bits};
use crate::{Dispatch, kinds, layout, object, shape, versions};

/// The one bump of the TypedArray workflow (TA-D13): OBJECT_LAYOUT 8 -> 9; IC_LAYOUT 4 (the
/// kinds-set bits 17 and 18 of values 7 and 8 are in it already), VM_LAYOUT 1, FRAME 2,
/// DEOPT_META 1, the builtin ABI 1, the bytecode ABI 6, Value encoding 1 and FACT_SCHEMA 2 stay.
#[test]
fn layout_9_versions() {
    assert_eq!(black_box(versions::OBJECT_LAYOUT_VERSION), 9);
    assert_eq!(
        (
            versions::IC_LAYOUT_VERSION,
            versions::VM_LAYOUT_VERSION,
            versions::FRAME_VERSION,
            versions::DEOPT_META_VERSION,
            versions::BUILTIN_ABI_VERSION
        ),
        (4, 1, 2, 1, 1)
    );
    assert_eq!(ep9_bytecode::ABI_VERSION, 6);
    assert_eq!(ep9_value::encoding::VERSION, 1);
    assert_eq!(ep9_facts::FACT_SCHEMA_VERSION, 2);
}

/// The view kinds 29-40 in spec Table 71 order (TA-D1) with their element sizes, and BYTES 85
/// (TA-D4). A view is a JS object (16-63), not a routed kind (the range 25..=28 stays closed,
/// TA-D6), and not a kind that keeps index keys at +16 (`elements::plain_elements_kind`).
#[test]
fn the_view_kinds_and_bytes() {
    let views = [
        (kinds::INT8_VIEW, 1),
        (kinds::UINT8_VIEW, 1),
        (kinds::UINT8_CLAMPED_VIEW, 1),
        (kinds::INT16_VIEW, 2),
        (kinds::UINT16_VIEW, 2),
        (kinds::INT32_VIEW, 4),
        (kinds::UINT32_VIEW, 4),
        (kinds::BIGINT64_VIEW, 8),
        (kinds::BIGUINT64_VIEW, 8),
        (kinds::FLOAT16_VIEW, 2),
        (kinds::FLOAT32_VIEW, 4),
        (kinds::FLOAT64_VIEW, 8),
    ];
    assert_eq!(usize::from(kinds::VIEW_KINDS), views.len());
    assert_eq!(layout::VIEW_ELEMENT_SIZES.len(), views.len());
    for (row, (k, size)) in views.iter().enumerate() {
        let id = k.get();
        assert_eq!(id, layout::KIND_FIRST_VIEW + row as u8, "row {row}");
        assert_eq!(layout::VIEW_ELEMENT_SIZES[row], *size, "row {row}");
        assert!(kinds::is_view_kind(id));
        assert_eq!(
            kinds::is_bigint_view_kind(id),
            row == 7 || row == 8,
            "row {row}"
        );
        assert_eq!(
            kinds::is_float_view_kind(id),
            (9..=11).contains(&row),
            "row {row}"
        );
        assert!((16..=63).contains(&id), "a view is a JS object");
        assert!(!crate::exotic::is_routed(id));
        assert!(!elements::plain_elements_kind(id));
    }
    assert_eq!((layout::KIND_FIRST_VIEW, layout::KIND_LAST_VIEW), (29, 40));
    assert_eq!(layout::KIND_BYTES, 85);
    assert_eq!(layout::KIND_BYTES, kinds::BYTES.get());
    assert!(
        (64..=127).contains(&layout::KIND_BYTES),
        "BYTES is an internal cell"
    );
    assert!(!kinds::is_view_kind(layout::KIND_BYTES));
    // The neighbours of the range are no views.
    for k in [0, 28, 41, 63, 64, 85, 128, 255] {
        assert!(!kinds::is_view_kind(k), "{k}");
    }
    // A value that is no cell of a view kind is no view (an Array and a non-cell).
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let a = object::new_array(&iso.rt, 0);
    assert!(!kinds::is_view(a));
    assert!(!kinds::is_view(int32_bits(29)));
    assert!(!kinds::is_view(UNDEFINED));
}

/// The view and BYTES cell exports (design §2.2): the 4 fixed fields from `obj::FIXED` (+24
/// buffer, +32 data, +40 length, +48 byteOffset), the in-object slots from +56, the "auto"
/// length -1, and BYTES +8 length, +16 data.
#[test]
fn the_view_and_bytes_cell_exports() {
    assert_eq!(layout::OBJECT_FIELDS, 24);
    assert_eq!(
        (
            layout::VIEW_BUFFER,
            layout::VIEW_DATA,
            layout::VIEW_LENGTH,
            layout::VIEW_BYTE_OFFSET,
            layout::VIEW_FIELDS
        ),
        (24, 32, 40, 48, 56)
    );
    assert_eq!(layout::VIEW_BUFFER, layout::OBJECT_FIELDS);
    assert_eq!(layout::VIEW_LENGTH_AUTO, -1);
    assert_eq!((layout::BYTES_LENGTH, layout::BYTES_DATA), (8, 16));
    assert!(
        layout::BYTES_DATA.is_multiple_of(8),
        "the bytes are 8-aligned"
    );
    const { assert!(layout::ARRAY_ELEMENTS < layout::VIEW_BUFFER) };
}

/// H18 "elements kind values (append-only)": 7 TYPED_FIXED and 8 TYPED_VARIABLE (Proposal A,
/// design §3.2) take the next two values; 9-15 stay free. Their storage form is `Typed`, they
/// are not holey, they have no FactSnapshot name yet, and their kinds-set bits 17 and 18 are
/// inside the IC_LAYOUT 4 mask (so no IC_LAYOUT move). An Array whose shape holds 7 fails the
/// storage-form check: only a view has 7 or 8.
#[test]
fn elements_kinds_7_and_8() {
    assert_eq!((ek::TYPED_FIXED, ek::TYPED_VARIABLE), (7, 8));
    assert_eq!(ek::COUNT, 9);
    assert_eq!(
        (
            layout::ELEMENTS_KIND_TYPED_FIXED,
            layout::ELEMENTS_KIND_TYPED_VARIABLE
        ),
        (7, 8)
    );
    for k in [ek::TYPED_FIXED, ek::TYPED_VARIABLE] {
        assert_eq!(elements::storage(k), Some(Storage::Typed), "{k}");
        assert!(!elements::is_holey(k), "{k}");
        assert_eq!(elements::to_fact(k), None, "{k}");
        assert_eq!(elements::kinds_set_bit(k), 1 << (10 + u32::from(k)));
        assert_ne!(elements::kinds_set_bit(k) & elements::KINDS_SET_MASK, 0);
    }
    for k in 9..=ek::MAX {
        assert_eq!(elements::storage(k), None, "{k} stays free");
    }
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    let a = object::new_array(rt, 0);
    let s = shape::shape_of(a);
    assert!(elements::storage_form_ok(a));
    for k in [ek::TYPED_FIXED, ek::TYPED_VARIABLE] {
        let t = shape::with_elements_kind(rt, s, k).expect("a shape of kind 7 or 8");
        assert_eq!(shape::elements_kind(t), k);
        shape::install(rt, a, t);
        assert!(!elements::storage_form_ok(a), "an Array with kind {k}");
        shape::install(rt, a, s);
    }
    assert!(elements::storage_form_ok(a));
    elements::check(a).unwrap();
}

/// H20 part 2: ObjectClass 36 SharedArrayBuffer and 37 DataView (TA-D3), after the INTL W3
/// classes; ArrayBuffer keeps id 5. The build check below fails if a class is missing from
/// `ObjectClass::ALL` or out of its id order.
#[test]
fn object_classes_36_and_37() {
    const _: () = {
        let all = ObjectClass::ALL;
        assert!(all.len() == ObjectClass::DataView as usize + 1);
        let mut i = 0;
        while i < all.len() {
            assert!(all[i] as usize == i);
            i += 1;
        }
    };
    assert_eq!(ObjectClass::ArrayBuffer.id(), 5);
    assert_eq!(
        (
            ObjectClass::SharedArrayBuffer.id(),
            ObjectClass::DataView.id()
        ),
        (36, 37)
    );
    assert_eq!(ObjectClass::SharedArrayBuffer.name(), "SharedArrayBuffer");
    assert_eq!(ObjectClass::DataView.name(), "DataView");
    assert_eq!(ObjectClass::from_id(37), Some(ObjectClass::DataView));
    assert_eq!(ObjectClass::from_id(38), None);
}
