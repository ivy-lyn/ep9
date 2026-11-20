//! Each drop reason, and bad input that must give a drop, never a panic (H37 Validation: "each
//! drop reason"; S3, S5, S6).

mod common;

use common::*;
use ep9_hints::{
    CLASS_ENTRY_SIZE, DropCounts, DropReason, Expect, FIELD_SIZE, FILE_ENTRY_SIZE, HEADER_SIZE,
    RECORD_SIZE, TsFlags, offsets as o, read,
};

fn drop_of(bytes: &[u8]) -> DropReason {
    read(bytes, &expect()).expect_err("the input must be dropped")
}

/// The sample with one change.
fn patched(f: impl FnOnce(&mut Vec<u8>)) -> DropReason {
    let mut b = sample_bytes();
    f(&mut b);
    drop_of(&b)
}

fn rec(i: usize, field: usize) -> usize {
    RECORDS_AT + i * RECORD_SIZE + field
}

#[test]
fn truncated_at_every_length() {
    let bytes = sample_bytes();
    for len in 0..bytes.len() {
        assert_eq!(
            drop_of(&bytes[..len]),
            DropReason::Truncated,
            "length {len}"
        );
    }
    assert!(read(&bytes, &expect()).is_ok());
}

#[test]
fn huge_counts_are_truncated_without_allocation() {
    for at in [
        o::FILE_COUNT,
        o::CLASS_COUNT,
        o::FIELD_COUNT,
        o::RECORD_COUNT,
        o::STRING_BYTES,
    ] {
        assert_eq!(patched(|b| put_u32(b, at, u32::MAX)), DropReason::Truncated);
    }
    let mut head = sample_bytes()[..HEADER_SIZE].to_vec();
    for at in [
        o::FILE_COUNT,
        o::CLASS_COUNT,
        o::FIELD_COUNT,
        o::RECORD_COUNT,
        o::STRING_BYTES,
    ] {
        put_u32(&mut head, at, u32::MAX);
    }
    assert_eq!(drop_of(&head), DropReason::Truncated);
}

#[test]
fn trailing_bytes() {
    assert_eq!(patched(|b| b.push(0)), DropReason::TrailingBytes);
}

#[test]
fn bad_magic() {
    for i in 0..8 {
        assert_eq!(patched(|b| b[i] ^= 0x20), DropReason::BadMagic);
    }
}

#[test]
fn schema_version_mismatch_ignores_the_file() {
    for v in [0, 2, u32::MAX] {
        assert_eq!(
            patched(|b| put_u32(b, o::SCHEMA_VERSION, v)),
            DropReason::SchemaVersion
        );
    }
    // A newer schema may have another layout: the version check comes before the size check.
    let mut b = sample_bytes();
    put_u32(&mut b, o::SCHEMA_VERSION, 2);
    b.truncate(HEADER_SIZE + 1);
    assert_eq!(drop_of(&b), DropReason::SchemaVersion);
}

#[test]
fn fact_schema_version_mismatch_ignores_the_file() {
    let bytes = sample_bytes();
    let e = Expect {
        fact_schema_version: FACT + 1,
        ts_flags: flags(),
    };
    assert_eq!(read(&bytes, &e), Err(DropReason::FactSchemaVersion));
    assert_eq!(
        patched(|b| put_u32(b, o::FACT_SCHEMA_VERSION, 1)),
        DropReason::FactSchemaVersion
    );
}

#[test]
fn ts_flags_mismatch_ignores_the_file() {
    let bytes = sample_bytes();
    for bits in 0..8 {
        let f = TsFlags::from_bits(bits).unwrap();
        let e = Expect {
            fact_schema_version: FACT,
            ts_flags: f,
        };
        let got = read(&bytes, &e).map(|_| ());
        if f == flags() {
            assert_eq!(got, Ok(()));
        } else {
            assert_eq!(got, Err(DropReason::TsFlags), "expect bits {bits}");
        }
    }
    for bits in [1 << 3, 1 << 31, 0b1001] {
        assert_eq!(
            patched(|b| put_u32(b, o::TS_FLAGS, bits)),
            DropReason::TsFlags
        );
    }
    // Each reserved bit alone, on top of the flags that match.
    for bit in 3..=31 {
        let mut b = sample_bytes();
        put_u32(&mut b, o::TS_FLAGS, flags().to_bits() | 1 << bit);
        assert_eq!(
            read(&b, &expect()).map(|_| ()),
            Err(DropReason::TsFlags),
            "bit {bit}"
        );
    }
}

#[test]
fn bad_header_reserved() {
    assert_eq!(
        patched(|b| put_u32(b, o::HEADER_RESERVED, 1)),
        DropReason::BadHeader
    );
}

#[test]
fn bad_file_table() {
    let f1 = FILES_AT + FILE_ENTRY_SIZE;
    // Path offset or record range that does not follow the previous entry.
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_PATH_OFFSET, 3)),
        DropReason::BadFileTable
    );
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_FIRST_RECORD, 2)),
        DropReason::BadFileTable
    );
    // A record range past the records, or one that leaves records without a file.
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_RECORD_COUNT, 3)),
        DropReason::BadFileTable
    );
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_RECORD_COUNT, 1)),
        DropReason::BadFileTable
    );
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_RECORD_COUNT, u32::MAX)),
        DropReason::BadFileTable
    );
    // A path past the strings part.
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_PATH_LEN, 9)),
        DropReason::BadFileTable
    );
    assert_eq!(
        patched(|b| put_u32(b, f1 + o::FILE_PATH_LEN, u32::MAX)),
        DropReason::BadFileTable
    );
    // has_type_errors other than 0 or 1; a reserved byte.
    assert_eq!(
        patched(|b| b[FILES_AT + o::FILE_HAS_TYPE_ERRORS] = 2),
        DropReason::BadFileTable
    );
    for i in 0..3 {
        assert_eq!(
            patched(|b| b[FILES_AT + o::FILE_RESERVED + i] = 1),
            DropReason::BadFileTable
        );
    }
}

#[test]
fn strings_part_holds_paths_only() {
    // S6: string bytes that no path covers (a type string, for example) drop the input.
    assert_eq!(
        patched(|b| {
            b.extend_from_slice(b"number");
            put_u32(b, o::STRING_BYTES, 12 + 6);
        }),
        DropReason::BadFileTable
    );
    // Strings with no file.
    let mut h = ep9_hints::Hints::default();
    h.header.fact_schema_version = FACT;
    h.header.ts_flags = flags();
    let mut b = ep9_hints::write(&h).unwrap();
    b.extend_from_slice(b"string");
    put_u32(&mut b, o::STRING_BYTES, 6);
    assert_eq!(drop_of(&b), DropReason::BadFileTable);
}

#[test]
fn bad_path() {
    assert_eq!(patched(|b| b[STRINGS_AT] = 0xff), DropReason::BadPath);
    assert_eq!(patched(|b| b[STRINGS_AT + 1] = 0), DropReason::BadPath);
    // An empty first path: the second path then starts at 0.
    assert_eq!(
        patched(|b| {
            put_u32(b, FILES_AT + o::FILE_PATH_LEN, 0);
            put_u32(b, FILES_AT + FILE_ENTRY_SIZE + o::FILE_PATH_OFFSET, 0);
            put_u32(b, FILES_AT + FILE_ENTRY_SIZE + o::FILE_PATH_LEN, 12);
        }),
        DropReason::BadPath
    );
}

#[test]
fn unsorted_files() {
    // "a.ts" -> "m.ts": now after "lib/b.ts".
    assert_eq!(patched(|b| b[STRINGS_AT] = b'm'), DropReason::UnsortedFiles);
    // The same path twice: "a.tsa.ts" as two files of 4 bytes, the second also "a.ts".
    assert_eq!(
        patched(|b| {
            b[STRINGS_AT + 4..STRINGS_AT + 8].copy_from_slice(b"a.ts");
            b.truncate(STRINGS_AT + 8);
            put_u32(b, o::STRING_BYTES, 8);
            put_u32(b, FILES_AT + FILE_ENTRY_SIZE + o::FILE_PATH_LEN, 4);
        }),
        DropReason::UnsortedFiles
    );
}

#[test]
fn bad_class_table() {
    let c0 = CLASSES_AT;
    let c1 = CLASSES_AT + CLASS_ENTRY_SIZE;
    assert_eq!(
        patched(|b| put_u32(b, c1 + o::CLASS_DECL_FILE, 2)),
        DropReason::BadClassTable
    );
    // A field that names a file that is not in the table.
    assert_eq!(
        patched(|b| put_u32(b, FIELDS_AT + FIELD_SIZE + o::FIELD_FILE, 2)),
        DropReason::BadClassTable
    );
    assert_eq!(
        patched(|b| put_u32(b, FIELDS_AT + o::FIELD_FILE, u32::MAX)),
        DropReason::BadClassTable
    );
    assert_eq!(
        patched(|b| put_u32(b, c1 + o::CLASS_FIRST_FIELD, 0)),
        DropReason::BadClassTable
    );
    // More fields than slots.
    assert_eq!(
        patched(|b| put_u32(b, c0 + o::CLASS_SLOT_COUNT, 0)),
        DropReason::BadClassTable
    );
    // A field range past the fields, or one that leaves a field without a class.
    assert_eq!(
        patched(|b| put_u32(b, c1 + o::CLASS_FIELD_COUNT, 3)),
        DropReason::BadClassTable
    );
    assert_eq!(
        patched(|b| put_u32(b, c1 + o::CLASS_FIELD_COUNT, 1)),
        DropReason::BadClassTable
    );
    assert_eq!(
        patched(|b| {
            put_u32(b, c1 + o::CLASS_SLOT_COUNT, u32::MAX);
            put_u32(b, c1 + o::CLASS_FIELD_COUNT, u32::MAX);
        }),
        DropReason::BadClassTable
    );
}

#[test]
fn bad_record() {
    type Patch = Box<dyn Fn(&mut Vec<u8>)>;
    let cases: Vec<(&str, Patch)> = vec![
        ("kind 0", Box::new(|b| put_u16(b, rec(0, o::REC_KIND), 0))),
        (
            "kind unknown",
            Box::new(|b| put_u16(b, rec(0, o::REC_KIND), 31)),
        ),
        ("repr unknown", Box::new(|b| b[rec(0, o::REC_REPR)] = 12)),
        (
            "element unknown",
            Box::new(|b| b[rec(1, o::REC_ELEMENT)] = 0xff),
        ),
        ("flag bit 3", Box::new(|b| b[rec(0, o::REC_FLAGS)] |= 8)),
        (
            "start after end",
            Box::new(|b| put_u32(b, rec(2, o::REC_START), 31)),
        ),
        (
            "class out of range",
            Box::new(|b| put_u32(b, rec(4, o::REC_CLASS), 2)),
        ),
        (
            "slot without class",
            Box::new(|b| put_u16(b, rec(1, o::REC_SLOT), 0)),
        ),
        (
            "slot at slot count",
            Box::new(|b| {
                put_u32(b, rec(1, o::REC_CLASS), 1);
                put_u16(b, rec(1, o::REC_SLOT), 3);
            }),
        ),
        (
            "slot on a kind that is not a member access",
            Box::new(|b| put_u16(b, rec(0, o::REC_SLOT), 0)),
        ),
        (
            "target file out of range",
            Box::new(|b| put_u32(b, rec(2, o::REC_TARGET_FILE), 2)),
        ),
        (
            "target offset with no file",
            Box::new(|b| put_u32(b, rec(0, o::REC_TARGET_OFFSET), 1)),
        ),
        (
            "enum value with flag clear",
            Box::new(|b| put_u32(b, rec(0, o::REC_ENUM_CONST), 1)),
        ),
    ];
    for (name, f) in cases {
        assert_eq!(patched(|b| f(b)), DropReason::BadRecord, "{name}");
    }
}

#[test]
fn unsorted_records() {
    // Swap records 0 and 1 of "a.ts": same span, kinds out of order.
    assert_eq!(
        patched(|b| {
            let (a, c) = (rec(0, 0), rec(1, 0));
            let first: Vec<u8> = b[a..a + RECORD_SIZE].to_vec();
            b.copy_within(c..c + RECORD_SIZE, a);
            b[c..c + RECORD_SIZE].copy_from_slice(&first);
        }),
        DropReason::UnsortedRecords
    );
    // The same key twice.
    assert_eq!(
        patched(|b| put_u16(b, rec(1, o::REC_KIND), 2)),
        DropReason::UnsortedRecords
    );
    // A start below the previous start.
    assert_eq!(
        patched(|b| {
            put_u32(b, rec(4, o::REC_START), 1);
            put_u32(b, rec(4, o::REC_END), 1);
        }),
        DropReason::UnsortedRecords
    );
    // Same start and kind, two ends (`a.b` then `a.b.c`): ascending ends read, reversed ends drop.
    let mut b = sample_bytes();
    put_u16(&mut b, rec(0, o::REC_KIND), 3);
    put_u32(&mut b, rec(0, o::REC_END), 3);
    assert!(read(&b, &expect()).is_ok());
    put_u32(&mut b, rec(0, o::REC_END), 9);
    assert_eq!(drop_of(&b), DropReason::UnsortedRecords);
    // The end sorts before the kind: (0, 3, kind 3) then (0, 5, kind 2) reads.
    put_u32(&mut b, rec(0, o::REC_END), 3);
    put_u16(&mut b, rec(1, o::REC_KIND), 2);
    assert!(read(&b, &expect()).is_ok());
    // The start sorts before the end: (0, 7, kind 8) then (1, 6, kind 19), as in `(x + y)`,
    // reads; the two records swapped drop.
    let mut b = sample_bytes();
    for (i, start, end, kind) in [(3, 0, 7, 8), (4, 1, 6, 19)] {
        put_u32(&mut b, rec(i, o::REC_START), start);
        put_u32(&mut b, rec(i, o::REC_END), end);
        put_u16(&mut b, rec(i, o::REC_KIND), kind);
    }
    assert!(read(&b, &expect()).is_ok());
    let (a, c) = (rec(3, 0), rec(4, 0));
    let first: Vec<u8> = b[a..a + RECORD_SIZE].to_vec();
    b.copy_within(c..c + RECORD_SIZE, a);
    b[c..c + RECORD_SIZE].copy_from_slice(&first);
    assert_eq!(drop_of(&b), DropReason::UnsortedRecords);
    // The order restarts at each file: record 3 (file 1) starts below record 2 (file 0).
    assert!(read(&sample_bytes(), &expect()).is_ok());
}

#[test]
fn file_level_drops() {
    let h = read(&sample_bytes(), &expect()).unwrap();
    assert_eq!(h.file_hints("a.ts", &hash(1)).unwrap(), &h.records[0..3]);
    assert_eq!(
        h.file_hints("a.ts", &hash(9)),
        Err(DropReason::HashMismatch)
    );
    assert_eq!(
        h.file_hints("lib/b.ts", &hash(2)),
        Err(DropReason::TypeErrors)
    );
    assert_eq!(h.file_hints("c.ts", &hash(1)), Err(DropReason::NotInTable));
}

#[test]
fn every_reason_has_a_test_and_a_count() {
    let mut counts = DropCounts::default();
    for (i, r) in DropReason::ALL.iter().enumerate() {
        assert_eq!(r.index(), i);
        for _ in 0..=i {
            counts.add(*r);
        }
    }
    for (i, (r, n)) in counts.iter().enumerate() {
        assert_eq!(r, DropReason::ALL[i]);
        assert_eq!(n, i as u64 + 1);
        assert_eq!(counts.get(r), n);
    }
    assert_eq!(counts.total(), (1..=16).sum::<u64>());
    let mut names: Vec<&str> = DropReason::ALL.iter().map(|r| r.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), DropReason::ALL.len());
    assert_eq!(DropReason::HashMismatch.to_string(), "hash-mismatch");
}

#[test]
fn any_one_byte_change_never_panics() {
    let bytes = sample_bytes();
    for at in 0..bytes.len() {
        for v in [0x00, 0x01, 0x7f, 0x80, 0xff] {
            let mut b = bytes.clone();
            b[at] = v;
            if let Ok(h) = read(&b, &expect()) {
                // An accepted input is canonical.
                assert_eq!(ep9_hints::write(&h).unwrap(), b, "byte {at} = {v:#x}");
            }
        }
    }
}

#[test]
fn random_input_never_panics() {
    let bytes = sample_bytes();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..20_000 {
        let mut b = bytes.clone();
        // Keep the header prefix so the reader gets past the version checks.
        for _ in 0..(next() % 8 + 1) {
            let at = HEADER_SIZE.min(b.len()) + (next() as usize) % (b.len() - HEADER_SIZE);
            b[at] = next() as u8;
        }
        if next() % 4 == 0 {
            b.truncate((next() as usize) % (b.len() + 1));
        }
        if let Ok(h) = read(&b, &expect()) {
            assert_eq!(ep9_hints::write(&h).unwrap(), b);
        }
    }
}

#[test]
fn a1h_r4_value_class_with_another_repr_is_a_bad_record() {
    use ep9_hints::{NodeKind, Repr};
    // A1H-R4 option A (DL M10-P1-CLOSE-5, call 3): on a kind that is not a member access, the
    // class id is the class of the value, so the repr must be ClassInstance. Record 0 of the
    // sample is a BindingIdentifier, ClassInstance, class 1; record 1 is a StaticMemberExpression.
    let number = Repr::Number.code();
    let ident = NodeKind::IdentifierReference.code();
    // Number + class on IdentifierReference: a bad record.
    assert_eq!(
        patched(|b| {
            put_u16(b, rec(0, o::REC_KIND), ident);
            b[rec(0, o::REC_REPR)] = number;
        }),
        DropReason::BadRecord
    );
    // Each repr other than ClassInstance, with the class, on the BindingIdentifier.
    for &repr in Repr::ALL {
        let mut b = sample_bytes();
        b[rec(0, o::REC_REPR)] = repr.code();
        if repr == Repr::ClassInstance {
            assert!(read(&b, &expect()).is_ok());
        } else {
            assert_eq!(drop_of(&b), DropReason::BadRecord, "{}", repr.name());
        }
    }
    // ClassInstance + class on IdentifierReference reads.
    let mut b = sample_bytes();
    put_u16(&mut b, rec(0, o::REC_KIND), ident);
    let h = read(&b, &expect()).unwrap();
    assert_eq!(h.records[0].kind, NodeKind::IdentifierReference);
    assert_eq!(
        (h.records[0].repr, h.records[0].class),
        (Repr::ClassInstance, Some(1))
    );
    // Number + receiver class on StaticMemberExpression reads: the class is the receiver's.
    let mut b = sample_bytes();
    put_u32(&mut b, rec(1, o::REC_CLASS), 1);
    b[rec(1, o::REC_REPR)] = number;
    let h = read(&b, &expect()).unwrap();
    assert_eq!(h.records[1].kind, NodeKind::StaticMemberExpression);
    assert_eq!(
        (h.records[1].repr, h.records[1].class),
        (Repr::Number, Some(1))
    );
}
