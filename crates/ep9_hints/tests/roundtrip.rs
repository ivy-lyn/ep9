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
    // Each u32 of the header has its own value (schema version 1, fact-schema version 9, TS
    // flags 7, counts 2, 3, 4 and 5, string bytes 12, reserved 0). In each other part, one
    // entry (file 1, class 2, every field entry, record 1) gives each field a value that no
    // other field of that entry has. So a swap of any two offsets of one part in the shared
    // offsets module changes the bytes or the decoded hints and fails here (checked for every
    // pair in a scratch copy).
    let mut h = Hints::default();
    h.header.fact_schema_version = 9;
    h.header.ts_flags = TsFlags::from_bits(7).unwrap();
    h.header.tsrs_commit = parse_commit_hex("745eb10a650b65ea689155e36ba848f9bfba790d").unwrap();
    h.files.push(ep9_hints::FileEntry {
        path: "lib/n.ts".into(),
        text_hash: std::array::from_fn(|i| i as u8),
        has_type_errors: false,
        first_record: 0,
        record_count: 2,
    });
    h.files.push(ep9_hints::FileEntry {
        path: "m.ts".into(),
        text_hash: std::array::from_fn(|i| 0x20 + i as u8),
        has_type_errors: true,
        first_record: 2,
        record_count: 3,
    });
    h.classes.push(ep9_hints::ClassEntry {
        decl: DeclRef { file: 1, offset: 3 },
        slot_count: 2,
        first_field: 0,
        field_count: 1,
    });
    h.classes.push(ep9_hints::ClassEntry {
        decl: DeclRef {
            file: 0,
            offset: 0x40,
        },
        slot_count: 4,
        first_field: 1,
        field_count: 0,
    });
    h.classes.push(ep9_hints::ClassEntry {
        decl: DeclRef {
            file: 0,
            offset: 0x50,
        },
        slot_count: 5,
        first_field: 1,
        field_count: 3,
    });
    h.fields.push(DeclRef {
        file: 0,
        offset: 0x21,
    });
    h.fields.push(DeclRef {
        file: 1,
        offset: 0x0a,
    });
    h.fields.push(DeclRef {
        file: 0,
        offset: 0x51,
    });
    h.fields.push(DeclRef {
        file: 0,
        offset: 0x52,
    });
    // "lib/n.ts": records 0 and 1.
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
    // "m.ts": records 2, 3 and 4.
    let mut r = record(0, 4, NodeKind::IdentifierReference);
    r.repr = Repr::ClassInstance;
    r.non_null = true;
    r.confidence = 1;
    r.class = Some(2);
    h.records.push(r);
    let mut r = record(0, 4, NodeKind::CallExpression);
    r.repr = Repr::String;
    r.call_target = Some(DeclRef { file: 1, offset: 7 });
    h.records.push(r);
    let mut r = record(6, 6, NodeKind::NewExpression);
    r.repr = Repr::Number;
    r.enum_const = Some(3);
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
        "09000000",                                 // fact-schema version 9
        "07000000",                                 // TS flags 0b111
        "745eb10a650b65ea689155e36ba848f9bfba790d", // tsrs commit
        "02000000",                                 // file count 2
        "03000000",                                 // class count 3
        "04000000",                                 // field count 4
        "05000000",                                 // record count 5
        "0c000000",                                 // string bytes 12
        "00000000",                                 // reserved
        // file entry 0
        "00000000", // path offset 0
        "08000000", // path length 8
        "00000000", // first record 0
        "02000000", // record count 2
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", // text hash
        "00",       // has_type_errors 0
        "000000",   // reserved
        // file entry 1
        "08000000", // path offset 8
        "04000000", // path length 4
        "02000000", // first record 2
        "03000000", // record count 3
        "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f", // text hash
        "01",       // has_type_errors 1
        "000000",   // reserved
        // class entry 0
        "01000000", // decl file 1
        "03000000", // decl offset 3
        "02000000", // slot count 2
        "00000000", // first field 0
        "01000000", // field count 1
        // class entry 1
        "00000000", // decl file 0
        "40000000", // decl offset 0x40
        "04000000", // slot count 4
        "01000000", // first field 1
        "00000000", // field count 0
        // class entry 2
        "00000000", // decl file 0
        "50000000", // decl offset 0x50
        "05000000", // slot count 5
        "01000000", // first field 1
        "03000000", // field count 3
        // field entry 0
        "00000000", // decl file 0
        "21000000", // decl offset 0x21
        // field entry 1
        "01000000", // decl file 1
        "0a000000", // decl offset 0x0a
        // field entry 2
        "00000000", // decl file 0
        "51000000", // decl offset 0x51
        // field entry 3
        "00000000", // decl file 0
        "52000000", // decl offset 0x52
        // record 0 ("lib/n.ts"): every id null
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
        // record 1 ("lib/n.ts"): every field set
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
        // record 2 ("m.ts")
        "00000000", // start 0
        "04000000", // end 4
        "0100",     // kind 1 IdentifierReference
        "0b",       // repr 11 ClassInstance
        "00",       // element 0 None
        "01",       // flags: bit 0 non-null
        "01",       // confidence 1
        "ffff",     // slot null
        "02000000", // class 2
        "ffffffff", // call-target file null
        "00000000", // call-target offset 0
        "00000000", // enum const 0 (flag bit 2 clear)
        // record 3 ("m.ts")
        "00000000", // start 0
        "04000000", // end 4
        "0600",     // kind 6 CallExpression
        "02",       // repr 2 String
        "00",       // element 0 None
        "00",       // flags: none
        "00",       // confidence 0 (none given)
        "ffff",     // slot null
        "ffffffff", // class null
        "01000000", // call-target file 1
        "07000000", // call-target offset 7
        "00000000", // enum const 0 (flag bit 2 clear)
        // record 4 ("m.ts")
        "06000000", // start 6
        "06000000", // end 6
        "0700",     // kind 7 NewExpression
        "01",       // repr 1 Number
        "00",       // element 0 None
        "04",       // flags: bit 2 enum const
        "00",       // confidence 0 (none given)
        "ffff",     // slot null
        "ffffffff", // class null
        "ffffffff", // call-target file null
        "00000000", // call-target offset 0
        "03000000", // enum const 3
        // strings
        "6c69622f6e2e7473", // "lib/n.ts"
        "6d2e7473",         // "m.ts"
    );
    assert_eq!(hex, golden);
    // The reader decodes the golden bytes apart from the writer.
    let e = Expect {
        fact_schema_version: 9,
        ts_flags: TsFlags::from_bits(7).unwrap(),
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

#[test]
fn member_access_class_is_the_receiver() {
    // `a.b` with `a: A` and `b: B`: the record of `a` names A as its value class, and the
    // record of `a.b` names A, the receiver class that owns the slot, not B.
    let class = |slot_count| ep9_hints::ClassEntry {
        decl: DeclRef { file: 0, offset: 0 },
        slot_count,
        first_field: 0,
        field_count: 0,
    };
    let mut h = Hints::default();
    h.files.push(ep9_hints::FileEntry {
        path: "m.ts".into(),
        text_hash: hash(3),
        has_type_errors: false,
        first_record: 0,
        record_count: 2,
    });
    h.classes.push(class(3)); // A
    h.classes.push(class(1)); // B
    let mut a = record(0, 1, NodeKind::IdentifierReference);
    a.repr = Repr::ClassInstance;
    a.class = Some(0);
    h.records.push(a);
    let mut ab = record(0, 3, NodeKind::StaticMemberExpression);
    ab.repr = Repr::ClassInstance;
    ab.class = Some(0);
    ab.slot = Some(2);
    h.records.push(ab);
    let bytes = write(&h).unwrap();
    assert_eq!(read(&bytes, &Expect::default()), Ok(h.clone()));
    // The same in-range slot on each kind: it round-trips on the three member-access kinds,
    // and the writer and the reader refuse it on every other kind.
    let kind_at = bytes.len() - "m.ts".len() - RECORD_SIZE + o::REC_KIND;
    let member = [
        NodeKind::StaticMemberExpression,
        NodeKind::ComputedMemberExpression,
        NodeKind::PrivateFieldExpression,
    ];
    for &kind in NodeKind::ALL {
        let mut k = h.clone();
        k.records[1].kind = kind;
        let mut raw = bytes.clone();
        put_u16(&mut raw, kind_at, kind.code());
        if member.contains(&kind) {
            assert_eq!(write(&k).as_ref(), Ok(&raw), "{}", kind.name());
            assert_eq!(read(&raw, &Expect::default()), Ok(k), "{}", kind.name());
        } else {
            assert_eq!(
                write(&k),
                Err(WriteError::Invalid(DropReason::BadRecord)),
                "{}",
                kind.name()
            );
            assert_eq!(
                read(&raw, &Expect::default()),
                Err(DropReason::BadRecord),
                "{}",
                kind.name()
            );
        }
    }
    // `a` is not a member access, so it has no slot, though slot 0 is below A's 3 slots.
    let mut bad = h.clone();
    bad.records[0].slot = Some(0);
    assert_eq!(write(&bad), Err(WriteError::Invalid(DropReason::BadRecord)));
    // Slot 2 is not a slot of B, so the record cannot name the class of the value.
    h.records[1].class = Some(1);
    assert_eq!(write(&h), Err(WriteError::Invalid(DropReason::BadRecord)));
}

#[test]
fn a1h_r4_class_id_needs_class_instance_off_member_kinds() {
    // A1H-R4 option A (DL M10-P1-CLOSE-5, call 3): on each kind that is not a member access, a
    // class id with a repr other than ClassInstance is refused by the writer and dropped by the
    // reader. A member-access kind (3-5) keeps any repr with its receiver class. With no class
    // id, every repr reads on every kind.
    let mut h = Hints::default();
    h.files.push(ep9_hints::FileEntry {
        path: "m.ts".into(),
        text_hash: hash(4),
        has_type_errors: false,
        first_record: 0,
        record_count: 1,
    });
    h.classes.push(ep9_hints::ClassEntry {
        decl: DeclRef { file: 0, offset: 0 },
        slot_count: 1,
        first_field: 0,
        field_count: 0,
    });
    let mut r = record(0, 3, NodeKind::IdentifierReference);
    r.repr = Repr::ClassInstance;
    r.class = Some(0);
    h.records.push(r);
    let bytes = write(&h).unwrap();
    assert_eq!(read(&bytes, &Expect::default()), Ok(h.clone()));
    let at = bytes.len() - "m.ts".len() - RECORD_SIZE;
    let member = [
        NodeKind::StaticMemberExpression,
        NodeKind::ComputedMemberExpression,
        NodeKind::PrivateFieldExpression,
    ];
    let mut dropped = 0;
    for &kind in NodeKind::ALL {
        for &repr in Repr::ALL {
            for class in [Some(0), None] {
                let mut k = h.clone();
                k.records[0].kind = kind;
                k.records[0].repr = repr;
                k.records[0].class = class;
                let mut raw = bytes.clone();
                put_u16(&mut raw, at + o::REC_KIND, kind.code());
                raw[at + o::REC_REPR] = repr.code();
                put_u32(&mut raw, at + o::REC_CLASS, class.unwrap_or(u32::MAX));
                let why = format!("{} {} {class:?}", kind.name(), repr.name());
                if class.is_some() && !member.contains(&kind) && repr != Repr::ClassInstance {
                    dropped += 1;
                    assert_eq!(
                        write(&k),
                        Err(WriteError::Invalid(DropReason::BadRecord)),
                        "{why}"
                    );
                    assert_eq!(
                        read(&raw, &Expect::default()),
                        Err(DropReason::BadRecord),
                        "{why}"
                    );
                } else {
                    assert_eq!(write(&k).as_ref(), Ok(&raw), "{why}");
                    assert_eq!(read(&raw, &Expect::default()), Ok(k), "{why}");
                }
            }
        }
    }
    // 27 kinds that are not a member access x 11 reprs that are not ClassInstance.
    assert_eq!(dropped, (NodeKind::ALL.len() - 3) * (Repr::ALL.len() - 1));
}
