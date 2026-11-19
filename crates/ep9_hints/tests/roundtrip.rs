//! Round trips: the sample, each header field, the code lists and the layout (H37 Validation:
//! "round-trip tests for each header field").

mod common;

use common::*;
use ep9_hints::{
    DeclRef, DropReason, Expect, HINT_SCHEMA_VERSION, Hints, MAGIC, NodeKind, RECORD_SIZE, Repr,
    TsFlags, WriteError, commit_hex, offsets as o, parse_commit_hex, read, write,
};

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn sample_round_trips_and_is_canonical() {
    let bytes = sample_bytes();
    let back = read(&bytes, &expect()).unwrap();
    assert_eq!(back, sample());
    assert_eq!(write(&back).unwrap(), bytes);
    assert_eq!(bytes.len(), STRINGS_AT + "a.ts".len() + "lib/b.ts".len());
}

#[test]
fn empty_hints_round_trip() {
    let mut h = Hints::default();
    h.header.fact_schema_version = FACT;
    h.header.ts_flags = flags();
    let bytes = write(&h).unwrap();
    assert_eq!(bytes.len(), ep9_hints::HEADER_SIZE);
    assert_eq!(read(&bytes, &expect()).unwrap(), h);
}

#[test]
fn header_magic_and_schema_version() {
    let bytes = sample_bytes();
    assert_eq!(bytes[o::MAGIC..o::MAGIC + 8], MAGIC);
    assert_eq!(u32_at(&bytes, o::SCHEMA_VERSION), HINT_SCHEMA_VERSION);
    assert_eq!(HINT_SCHEMA_VERSION, 1);
}

#[test]
fn header_tsrs_commit_round_trips() {
    for commit in [[0u8; 20], [0xff; 20], std::array::from_fn(|i| i as u8)] {
        let mut h = sample();
        h.header.tsrs_commit = commit;
        let bytes = write(&h).unwrap();
        assert_eq!(bytes[o::TSRS_COMMIT..o::TSRS_COMMIT + 20], commit);
        assert_eq!(read(&bytes, &expect()).unwrap().header.tsrs_commit, commit);
    }
}

#[test]
fn header_fact_schema_version_round_trips() {
    for v in [0, 2, 3, u32::MAX] {
        let mut h = sample();
        h.header.fact_schema_version = v;
        let bytes = write(&h).unwrap();
        assert_eq!(u32_at(&bytes, o::FACT_SCHEMA_VERSION), v);
        let e = Expect {
            fact_schema_version: v,
            ts_flags: flags(),
        };
        assert_eq!(read(&bytes, &e).unwrap().header.fact_schema_version, v);
    }
}

#[test]
fn header_ts_flags_round_trip_each_bit() {
    for bits in 0..8u32 {
        let f = TsFlags::from_bits(bits).unwrap();
        assert_eq!(f.to_bits(), bits);
        let mut h = sample();
        h.header.ts_flags = f;
        let bytes = write(&h).unwrap();
        assert_eq!(u32_at(&bytes, o::TS_FLAGS), bits);
        let e = Expect {
            fact_schema_version: FACT,
            ts_flags: f,
        };
        assert_eq!(read(&bytes, &e).unwrap().header.ts_flags, f);
    }
    assert_eq!(
        TsFlags::from_bits(1).unwrap(),
        TsFlags {
            use_define_for_class_fields: true,
            ..TsFlags::default()
        }
    );
    assert!(TsFlags::from_bits(2).unwrap().experimental_decorators);
    assert!(TsFlags::from_bits(4).unwrap().emit_decorator_metadata);
    assert_eq!(TsFlags::from_bits(8), None);
}

#[test]
fn header_counts_name_each_part() {
    let bytes = sample_bytes();
    assert_eq!(u32_at(&bytes, o::FILE_COUNT), 2);
    assert_eq!(u32_at(&bytes, o::CLASS_COUNT), 2);
    assert_eq!(u32_at(&bytes, o::FIELD_COUNT), 3);
    assert_eq!(u32_at(&bytes, o::RECORD_COUNT), 5);
    assert_eq!(u32_at(&bytes, o::STRING_BYTES), 12);
    assert_eq!(u32_at(&bytes, o::HEADER_RESERVED), 0);
}

#[test]
fn strings_hold_the_paths_only() {
    let bytes = sample_bytes();
    assert_eq!(&bytes[STRINGS_AT..], b"a.tslib/b.ts");
}

#[test]
fn field_entry_layout() {
    assert_eq!(ep9_hints::FIELD_SIZE, 8);
    let bytes = sample_bytes();
    // Field 1 of the sample: the inherited slot 0 field of class 1, in "lib/b.ts".
    let at = FIELDS_AT + ep9_hints::FIELD_SIZE;
    assert_eq!(u32_at(&bytes, at + o::FIELD_FILE), 1);
    assert_eq!(u32_at(&bytes, at + o::FIELD_OFFSET), 2);
}

#[test]
fn record_layout_and_null_values() {
    assert_eq!(RECORD_SIZE, 32);
    let bytes = sample_bytes();
    // Record 3 of the sample: an enum const read, every id null.
    let r = &bytes[RECORDS_AT + 3 * RECORD_SIZE..RECORDS_AT + 4 * RECORD_SIZE];
    let expected: [u8; RECORD_SIZE] = [
        3, 0, 0, 0, // start
        3, 0, 0, 0, // end
        1, 0, // kind IdentifierReference
        1, // repr Number
        0, // element None
        4, // flags: enum const
        0, // confidence none
        0xff, 0xff, // slot null
        0xff, 0xff, 0xff, 0xff, // class null
        0xff, 0xff, 0xff, 0xff, // call-target file null
        0, 0, 0, 0, // call-target offset 0
        0xf9, 0xff, 0xff, 0xff, // enum const -7
    ];
    assert_eq!(r, expected);
}

/// The bytes of a hex string (the golden files below); 0 dependencies.
fn hex_decode(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn golden_bytes_of_a_tiny_file() {
    // Every field of every part has a value that differs from its neighbours, so a swap of two
    // offsets, a width change or a null-value change in the reader or the writer fails here.
    let mut h = Hints::default();
    h.header.fact_schema_version = 2;
    h.header.ts_flags = TsFlags::from_bits(5).unwrap();
    h.header.tsrs_commit = parse_commit_hex("745eb10a650b65ea689155e36ba848f9bfba790d").unwrap();
    h.files.push(ep9_hints::FileEntry {
        path: "m.ts".into(),
        text_hash: [0x11; 32],
        has_type_errors: false,
        first_record: 0,
        record_count: 2,
    });
    h.classes.push(ep9_hints::ClassEntry {
        decl: DeclRef { file: 0, offset: 3 },
        slot_count: 2,
        first_field: 0,
        field_count: 1,
    });
    h.classes.push(ep9_hints::ClassEntry {
        decl: DeclRef {
            file: 0,
            offset: 0x40,
        },
        slot_count: 3,
        first_field: 1,
        field_count: 0,
    });
    h.fields.push(DeclRef {
        file: 0,
        offset: 0x21,
    });
    let mut r = record(1, 2, NodeKind::ThisExpression);
    r.repr = Repr::Object;
    r.non_null = true;
    r.confidence = 9;
    h.records.push(r);
    let mut r = record(5, 9, NodeKind::StaticMemberExpression);
    r.repr = Repr::Array;
    r.element = Repr::Number;
    r.dts_lib = true;
    r.enum_const = Some(-2);
    r.confidence = 200;
    r.class = Some(1);
    r.slot = Some(2);
    r.call_target = Some(DeclRef {
        file: 0,
        offset: 0x0105,
    });
    h.records.push(r);
    let hex: String = write(&h)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let golden = concat!(
        // header
        "894550394854",                             // magic "\x89EP9HT"
        "0d0a",                                     // magic "\r\n"
        "01000000",                                 // schema version 1
        "02000000",                                 // fact-schema version 2
        "05000000",                                 // TS flags 0b101
        "745eb10a650b65ea689155e36ba848f9bfba790d", // tsrs commit
        "01000000",                                 // file count 1
        "02000000",                                 // class count 2
        "01000000",                                 // field count 1
        "02000000",                                 // record count 2
        "04000000",                                 // string bytes 4
        "00000000",                                 // reserved
        // file entry 0
        "00000000", // path offset 0
        "04000000", // path length 4
        "00000000", // first record 0
        "02000000", // record count 2
        "1111111111111111111111111111111111111111111111111111111111111111", // text hash
        "00",       // has_type_errors 0
        "000000",   // reserved
        // class entry 0
        "00000000", // decl file 0
        "03000000", // decl offset 3
        "02000000", // slot count 2
        "00000000", // first field 0
        "01000000", // field count 1
        // class entry 1
        "00000000", // decl file 0
        "40000000", // decl offset 0x40
        "03000000", // slot count 3
        "01000000", // first field 1
        "00000000", // field count 0
        // field entry 0
        "00000000", // decl file 0
        "21000000", // decl offset 0x21
        // record 0: every id null
        "01000000", // start 1
        "02000000", // end 2
        "1100",     // kind 17 ThisExpression
        "08",       // repr 8 Object
        "00",       // element 0 None
        "01",       // flags: bit 0 non-null
        "09",       // confidence 9
        "ffff",     // slot null
        "ffffffff", // class null
        "ffffffff", // call-target file null
        "00000000", // call-target offset 0
        "00000000", // enum const 0 (flag bit 2 clear)
        // record 1: every field set
        "05000000", // start 5
        "09000000", // end 9
        "0300",     // kind 3 StaticMemberExpression
        "09",       // repr 9 Array
        "01",       // element 1 Number
        "06",       // flags: bit 1 dts/lib, bit 2 enum const
        "c8",       // confidence 200
        "0200",     // slot 2
        "01000000", // class 1
        "00000000", // call-target file 0
        "05010000", // call-target offset 0x0105
        "feffffff", // enum const -2
        // strings
        "6d2e7473", // "m.ts"
    );
    assert_eq!(hex, golden);
    // The reader decodes the golden bytes apart from the writer.
    let e = Expect {
        fact_schema_version: 2,
        ts_flags: TsFlags::from_bits(5).unwrap(),
    };
    assert_eq!(read(&hex_decode(golden), &e).unwrap(), h);
}

#[test]
fn code_lists_are_append_only() {
    // A change of an existing line is a schema change; an append adds a line here and bumps
    // HINT_SCHEMA_VERSION.
    let kinds: Vec<(u16, &str)> = NodeKind::ALL.iter().map(|k| (k.code(), k.name())).collect();
    assert_eq!(
        kinds,
        [
            (1, "IdentifierReference"),
            (2, "BindingIdentifier"),
            (3, "StaticMemberExpression"),
            (4, "ComputedMemberExpression"),
            (5, "PrivateFieldExpression"),
            (6, "CallExpression"),
            (7, "NewExpression"),
            (8, "ParenthesizedExpression"),
            (9, "Function"),
            (10, "ArrowFunctionExpression"),
            (11, "Class"),
            (12, "PropertyDefinition"),
            (13, "MethodDefinition"),
            (14, "FormalParameter"),
            (15, "ObjectExpression"),
            (16, "ArrayExpression"),
            (17, "ThisExpression"),
            (18, "VariableDeclarator"),
            (19, "BinaryExpression"),
            (20, "LogicalExpression"),
            (21, "AssignmentExpression"),
            (22, "UnaryExpression"),
            (23, "UpdateExpression"),
            (24, "ConditionalExpression"),
            (25, "NumericLiteral"),
            (26, "StringLiteral"),
            (27, "TemplateLiteral"),
            (28, "AwaitExpression"),
            (29, "TSAsExpression"),
            (30, "TSNonNullExpression"),
        ]
    );
    let reprs: Vec<(u8, &str)> = Repr::ALL.iter().map(|r| (r.code(), r.name())).collect();
    assert_eq!(
        reprs,
        [
            (0, "None"),
            (1, "Number"),
            (2, "String"),
            (3, "Boolean"),
            (4, "BigInt"),
            (5, "Symbol"),
            (6, "Undefined"),
            (7, "Null"),
            (8, "Object"),
            (9, "Array"),
            (10, "Function"),
            (11, "ClassInstance"),
        ]
    );
    for code in 0..=u16::MAX {
        assert_eq!(
            NodeKind::from_code(code).map(|k| k.code()),
            (1..=30).contains(&code).then_some(code)
        );
    }
    for code in 0..=u8::MAX {
        assert_eq!(
            Repr::from_code(code).map(|r| r.code()),
            (code <= 11).then_some(code)
        );
    }
}

#[test]
fn r13_core_kinds_have_a_code() {
    // The A1h gate counts the R13 core node kinds (H37:11; `CORE` in
    // docs/research/artifacts/tsrs-type-oracle/scripts/join.py). The key holds the kind (S1),
    // so each core kind needs a code. oxc's `FunctionExpression` is `Function` here.
    let core = [
        "IdentifierReference",
        "StaticMemberExpression",
        "ComputedMemberExpression",
        "PrivateFieldExpression",
        "CallExpression",
        "NewExpression",
        "BinaryExpression",
        "LogicalExpression",
        "AssignmentExpression",
        "UnaryExpression",
        "UpdateExpression",
        "ThisExpression",
        "ConditionalExpression",
        "ArrayExpression",
        "ObjectExpression",
        "NumericLiteral",
        "StringLiteral",
        "TemplateLiteral",
        "ArrowFunctionExpression",
        "FunctionExpression",
        "AwaitExpression",
        "TSAsExpression",
        "TSNonNullExpression",
        "ParenthesizedExpression",
    ];
    assert_eq!(core.len(), 24);
    for n in core {
        let n = if n == "FunctionExpression" {
            "Function"
        } else {
            n
        };
        assert!(
            NodeKind::ALL.iter().any(|k| k.name() == n),
            "core kind {n} has no code"
        );
    }
}

#[test]
fn commit_hex_round_trips() {
    let text = "745eb10a650b65ea689155e36ba848f9bfba790d";
    let c = parse_commit_hex(&format!("{text}\n")).unwrap();
    assert_eq!(commit_hex(&c), text);
    assert_eq!(parse_commit_hex(&text.to_uppercase()), Some(c));
    assert_eq!(parse_commit_hex(&text[..39]), None);
    assert_eq!(parse_commit_hex(&format!("{}g", &text[..39])), None);
    assert_eq!(parse_commit_hex(&format!("{}+1", &text[..38])), None);
}

#[test]
fn class_fields_and_find_file() {
    let h = sample();
    let d = |file, offset| DeclRef { file, offset };
    assert_eq!(h.class_fields(&h.classes[0]), &[d(0, 44)]);
    // An inherited field names its base class file, not the subclass file.
    assert_eq!(h.class_fields(&h.classes[1]), &[d(1, 2), d(0, 52)]);
    let back = read(&sample_bytes(), &expect()).unwrap();
    assert_eq!(back.class_fields(&back.classes[1]), &[d(1, 2), d(0, 52)]);
    assert_eq!(h.find_file("a.ts"), Some(0));
    assert_eq!(h.find_file("lib/b.ts"), Some(1));
    assert_eq!(h.find_file("b.ts"), None);
}

#[test]
fn writer_refuses_what_the_reader_would_drop_or_change() {
    let mut h = sample();
    h.records.swap(0, 1);
    assert_eq!(
        write(&h),
        Err(WriteError::Invalid(DropReason::UnsortedRecords))
    );

    let mut h = sample();
    h.files.swap(0, 1);
    assert!(matches!(write(&h), Err(WriteError::Invalid(_))));

    let mut h = sample();
    h.files[0].path.clear();
    assert_eq!(write(&h), Err(WriteError::Invalid(DropReason::BadPath)));

    let mut h = sample();
    h.files[1].record_count = 1;
    assert_eq!(
        write(&h),
        Err(WriteError::Invalid(DropReason::BadFileTable))
    );

    // A class id of u32::MAX is the null value, so it would read back as no class.
    let mut h = sample();
    h.records[4].class = Some(u32::MAX);
    assert_eq!(write(&h), Err(WriteError::Lossy));
}
