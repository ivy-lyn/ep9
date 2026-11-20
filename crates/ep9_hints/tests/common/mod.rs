//! A small sample `.ep9h` that uses every field except the slot, with null and set values.
#![allow(dead_code)]

use ep9_hints::{
    ClassEntry, DeclRef, Expect, FileEntry, Header, Hints, NodeKind, Record, Repr, TsFlags,
};

pub const FACT: u32 = 2;

pub fn flags() -> TsFlags {
    TsFlags {
        use_define_for_class_fields: true,
        experimental_decorators: false,
        emit_decorator_metadata: false,
    }
}

pub fn expect() -> Expect {
    Expect {
        fact_schema_version: FACT,
        ts_flags: flags(),
    }
}

pub fn hash(seed: u8) -> [u8; 32] {
    std::array::from_fn(|i| seed.wrapping_add(i as u8))
}

pub fn record(start: u32, end: u32, kind: NodeKind) -> Record {
    Record {
        start,
        end,
        kind,
        repr: Repr::None,
        element: Repr::None,
        non_null: false,
        dts_lib: false,
        enum_const: None,
        confidence: 0,
        class: None,
        slot: None,
        call_target: None,
    }
}

/// Files "a.ts" (3 records, has_type_errors 0) and "lib/b.ts" (2 records, has_type_errors 1);
/// 2 classes with 3 fields. Class 1 is a subclass in "a.ts": its slot 0 field is inherited
/// from a base class in "lib/b.ts", its slot 1 field is in "a.ts".
pub fn sample() -> Hints {
    let mut r0 = record(0, 5, NodeKind::BindingIdentifier);
    r0.repr = Repr::ClassInstance;
    r0.class = Some(1);
    r0.non_null = true;
    r0.confidence = 200;
    let mut r1 = record(0, 5, NodeKind::StaticMemberExpression);
    r1.repr = Repr::Array;
    r1.element = Repr::Number;
    r1.dts_lib = true;
    let mut r2 = record(7, 30, NodeKind::CallExpression);
    r2.call_target = Some(DeclRef {
        file: 1,
        offset: 12,
    });
    r2.repr = Repr::String;
    r2.confidence = 1;
    let mut r3 = record(3, 3, NodeKind::IdentifierReference);
    r3.enum_const = Some(-7);
    r3.repr = Repr::Number;
    let mut r4 = record(4, 9, NodeKind::ParenthesizedExpression);
    r4.class = Some(0);
    r4.repr = Repr::ClassInstance;
    r4.enum_const = Some(0);
    Hints {
        header: Header {
            tsrs_commit: std::array::from_fn(|i| 0xa0 + i as u8),
            fact_schema_version: FACT,
            ts_flags: flags(),
        },
        files: vec![
            FileEntry {
                path: "a.ts".into(),
                text_hash: hash(1),
                has_type_errors: false,
                first_record: 0,
                record_count: 3,
            },
            FileEntry {
                path: "lib/b.ts".into(),
                text_hash: hash(2),
                has_type_errors: true,
                first_record: 3,
                record_count: 2,
            },
        ],
        classes: vec![
            ClassEntry {
                decl: DeclRef {
                    file: 0,
                    offset: 40,
                },
                slot_count: 1,
                first_field: 0,
                field_count: 1,
            },
            ClassEntry {
                decl: DeclRef {
                    file: 0,
                    offset: 50,
                },
                slot_count: 3,
                first_field: 1,
                field_count: 2,
            },
        ],
        fields: vec![
            DeclRef {
                file: 0,
                offset: 44,
            },
            DeclRef { file: 1, offset: 2 },
            DeclRef {
                file: 0,
                offset: 52,
            },
        ],
        records: vec![r0, r1, r2, r3, r4],
    }
}

pub fn sample_bytes() -> Vec<u8> {
    ep9_hints::write(&sample()).expect("the sample writes")
}

/// Byte offsets of the parts of the sample bytes.
pub const FILES_AT: usize = ep9_hints::HEADER_SIZE;
pub const CLASSES_AT: usize = FILES_AT + 2 * ep9_hints::FILE_ENTRY_SIZE;
pub const FIELDS_AT: usize = CLASSES_AT + 2 * ep9_hints::CLASS_ENTRY_SIZE;
pub const RECORDS_AT: usize = FIELDS_AT + 3 * ep9_hints::FIELD_SIZE;
pub const STRINGS_AT: usize = RECORDS_AT + 5 * ep9_hints::RECORD_SIZE;

pub fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
