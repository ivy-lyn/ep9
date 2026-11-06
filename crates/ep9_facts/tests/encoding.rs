//! The binary form: golden bytes, round trips, one encoding per value, and the decoder's
//! errors.
//!
//! `EP9_BLESS=1 cargo test -p ep9_facts --test encoding` rewrites `facts-v0.hex` and
//! `facts-v0.txt`. A change to either also changes the schema fingerprint, so it needs a
//! FACT_SCHEMA_VERSION bump (`tests/schema.rs`).

mod common;

use common::*;
use ep9_bytecode::{FeedbackSlotKind, UnitString};
use ep9_facts::*;

#[test]
fn sample_bytes_match_the_golden_file() {
    check_golden("facts-v0.hex", &hexdump(&sample().encode()));
}

#[test]
fn sample_dump_matches_the_golden_file() {
    check_golden("facts-v0.txt", &sample().dump());
}

#[test]
fn sample_covers_every_kind_tag_and_enum_value() {
    let seen = coverage(&sample());
    let missing: Vec<_> = every_variant().difference(&seen).cloned().collect();
    assert!(missing.is_empty(), "the sample does not use {missing:?}");
}

#[test]
fn sample_round_trips() {
    let s = sample();
    let bytes = s.encode();
    let back = FactSnapshot::decode(&bytes).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.encode(), bytes);
    assert_eq!(back.dump(), s.dump());
    assert_eq!(&bytes[..8], &MAGIC);
    assert_eq!(
        u32::from_le_bytes(bytes[offsets::SCHEMA_VERSION..][..4].try_into().unwrap()),
        FACT_SCHEMA_VERSION
    );
    for source in FactSource::ALL {
        let mut h = header();
        h.source = *source;
        let mut s = FactSnapshot::new(h);
        s.insert(0, 0, SlotFacts::empty(FeedbackSlotKind::JumpLoop))
            .unwrap();
        let back = FactSnapshot::decode(&s.encode()).unwrap();
        assert_eq!(back.header().source, *source);
    }
}

#[test]
fn every_kind_has_its_own_empty_payload() {
    for &k in FeedbackSlotKind::ALL {
        let f = SlotFacts::empty(k);
        assert_eq!(f.kind(), k);
        let mut s = FactSnapshot::new(header());
        s.insert(1, k as u32, f.clone()).unwrap();
        let back = FactSnapshot::decode(&s.encode()).unwrap();
        assert_eq!(back.get(1, k as u32), Some(&f));
    }
}

#[test]
fn random_snapshots_round_trip() {
    let mut rng = Rng::new(1);
    for round in 0..3000 {
        let source = *rng.pick(FactSource::ALL);
        let records = random_records(&mut rng);
        let s = snapshot_of(&records, source);
        let bytes = s.encode();
        let back = FactSnapshot::decode(&bytes).unwrap_or_else(|e| panic!("round {round}: {e}"));
        assert_eq!(back, s, "round {round}");
        assert_eq!(back.encode(), bytes, "round {round}");
    }
}

/// The same facts, inserted in another order and with every list shuffled, give the same
/// bytes: the encoding depends on the facts only, never on how they were collected.
#[test]
fn two_build_orders_give_the_same_bytes() {
    let mut rng = Rng::new(2);
    let check = |records: Vec<(u32, u32, SlotFacts)>, rng: &mut Rng| {
        let a = snapshot_of(&records, FactSource::Runtime);
        let mut shuffled = records.clone();
        rng.shuffle(&mut shuffled);
        for (_, _, f) in &mut shuffled {
            match f {
                SlotFacts::Call(c) => rng.shuffle(&mut c.callees.targets),
                SlotFacts::InstanceOf(t) => rng.shuffle(&mut t.targets),
                SlotFacts::CloneObject(c) => rng.shuffle(&mut c.cases),
                SlotFacts::LoadNamed(p)
                | SlotFacts::StoreNamed(p)
                | SlotFacts::DefineNamedOwn(p) => rng.shuffle(&mut p.cases),
                SlotFacts::LoadKeyed(k)
                | SlotFacts::StoreKeyed(k)
                | SlotFacts::DefineKeyedOwn(k)
                | SlotFacts::HasKeyed(k)
                | SlotFacts::StoreInArrayLiteral(k) => rng.shuffle(&mut k.property.cases),
                _ => {}
            }
        }
        let b = snapshot_of(&shuffled, FactSource::Runtime);
        assert_eq!(a.encode(), b.encode());
        assert_eq!(a, b);
    };
    check(sample_records(), &mut rng);
    // The sample, reversed: the shape table is built from the facts, not from record order.
    let mut reversed = sample_records();
    reversed.reverse();
    let (a, b) = (
        snapshot_of(&sample_records(), FactSource::Profiled),
        snapshot_of(&reversed, FactSource::Profiled),
    );
    assert_eq!(a.encode(), b.encode());
    for _ in 0..500 {
        let records = random_records(&mut rng);
        check(records, &mut rng);
    }
}

#[test]
fn corrupt_input_never_panics_and_accepted_input_is_canonical() {
    let mut rng = Rng::new(3);
    let mut inputs = vec![sample().encode()];
    for _ in 0..40 {
        inputs.push(snapshot_of(&random_records(&mut rng), FactSource::Hinted).encode());
    }
    let (mut accepted, mut folded) = (0, 0);
    let mut accept = |b: &[u8]| {
        if let Ok(s) = FactSnapshot::decode(b) {
            let again = s.encode();
            accepted += 1;
            if again != b {
                // Only an open value this schema does not name gives another encoding
                // (FACT_SCHEMA 2, MC-w23-8): that encoding is canonical and means the same.
                folded += 1;
                let back = FactSnapshot::decode(&again).expect("a re-encoding decodes");
                assert_eq!(back, s, "the re-encoding keeps the facts");
                assert_eq!(back.encode(), again, "the re-encoding is canonical");
            }
        }
    };
    for bytes in &inputs {
        for len in 0..bytes.len() {
            accept(&bytes[..len]);
        }
        for _ in 0..400 {
            let mut b = bytes.clone();
            let i = rng.below(b.len() as u64) as usize;
            b[i] ^= 1 << rng.below(8);
            accept(&b);
        }
        let mut longer = bytes.clone();
        longer.push(0);
        accept(&longer);
    }
    // Some flips land in values that stay valid (counts, bits), and those must re-encode.
    assert!(accepted > folded, "{accepted} accepted, {folded} folded");
}

/// The split kind list and the other open lists (FACT_SCHEMA 2; MC-w23-8, M6-PLAN-D4): a
/// value this schema does not name never refuses the snapshot. An elements kind reads as none
/// for that field only, a representation as any, a bit of an open set is dropped; the other
/// fields decode as written.
#[test]
fn an_unknown_open_value_drops_only_its_field() {
    // One record per open field, each as its own snapshot: a Literal whose shape has every
    // open field, and a LoadKeyed with a case of an element kind and a kinds set.
    let shape = common::shape(
        ProtoRef::Null,
        ObjectKind::Array,
        Some(ElementsKind::HoleyDouble),
        ShapeFlags::EXTENSIBLE,
        3,
        vec![field("a", FieldRep::Int32, true), key("b")],
    );
    let mut lit = FactSnapshot::new(header());
    lit.insert(
        0,
        0,
        SlotFacts::Literal(LiteralFacts {
            count: 4,
            shape: Some(shape.clone().into()),
            elements: Some(ElementsKind::PackedElements),
        }),
    )
    .unwrap();
    let good = lit.encode();
    // The shape is the first table entry: proto null (1), kind (1), elements (1), flags (1),
    // capacity (2), count (4), key "a": tag (1), len (4), "a", attrs (1); key "b" alike.
    let s0 = HEADER_SIZE;
    let (elements, flags, attrs_a) = (s0 + 2, s0 + 3, s0 + 10 + 6);
    assert_eq!(good[elements], ElementsKind::HoleyDouble as u8 + 1);
    assert_eq!(good[attrs_a], 0x07 | (FieldRep::Int32 as u8) << 4 | 0x80);
    let last = literal_elements_at(&lit); // the Literal's own elements kind
    assert_eq!(good[last], ElementsKind::PackedElements as u8 + 1);
    let with = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        FactSnapshot::decode(&b).expect("an open value never refuses the snapshot")
    };
    let lit_of = |s: &FactSnapshot| match s.get(0, 0) {
        Some(SlotFacts::Literal(l)) => l.clone(),
        other => panic!("{other:?}"),
    };
    // Every value past the named ones, up to the top of the byte.
    for v in [
        ElementsKind::ALL.len() as u8 + 1,
        ElementsKind::VALUES,
        0xff,
    ] {
        let l = lit_of(&with(elements, v));
        let want = ShapeDesc {
            elements: None,
            ..shape.clone()
        };
        assert_eq!(*l.shape.unwrap(), want, "shape elements byte {v}");
        assert_eq!(l.elements, Some(ElementsKind::PackedElements));
        let l = lit_of(&with(last, v));
        assert_eq!(
            (l.count, l.elements),
            (4, None),
            "literal elements byte {v}"
        );
        assert_eq!(*l.shape.unwrap(), shape);
    }
    // A shape flag bit past the named ones is dropped.
    let l = lit_of(&with(flags, ShapeFlags::EXTENSIBLE.bits() | 0x80));
    assert_eq!(*l.shape.unwrap(), shape);
    // A representation past the named ones (5-7 at FACT_SCHEMA 2) reads as any; the const
    // bit and the attributes stay.
    for rep in FieldRep::ALL.len() as u8..8 {
        let l = lit_of(&with(attrs_a, 0x07 | rep << 4 | 0x80));
        let sh = l.shape.unwrap();
        assert_eq!(
            sh.fields[0],
            FieldHint {
                rep: FieldRep::Any,
                constant: true
            }
        );
        assert_eq!(sh.keys[0].1, key("a").1);
    }

    // A keyed record: the element case's kind and the kinds set.
    let mut keyed = FactSnapshot::new(header());
    keyed
        .insert(
            0,
            0,
            SlotFacts::LoadKeyed(KeyedFacts {
                property: PropertyFacts {
                    state: IcState::Monomorphic,
                    cases: vec![PropertyCase {
                        receiver: shape.clone().into(),
                        access: Access::Element {
                            kind: Some(ElementsKind::HoleyDouble),
                        },
                    }],
                    values: TypeSet::EMPTY,
                },
                keys: KeySet::INDEX,
                hint: KeyedHint::ARRAY | KeyedHint::HOLE,
                kinds: ElementsKindSet::of(ElementsKind::HoleyDouble),
            }),
        )
        .unwrap();
    let good = keyed.encode();
    // The case's elements kind byte and the kinds set (u16), found by the bytes they change.
    let (kind_at, kinds_at) = keyed_offsets(&keyed);
    assert_eq!(good[kind_at], ElementsKind::HoleyDouble as u8 + 1);
    let decode = |at: usize, bytes: &[u8]| {
        let mut b = good.clone();
        b[at..at + bytes.len()].copy_from_slice(bytes);
        match FactSnapshot::decode(&b).unwrap().get(0, 0) {
            Some(SlotFacts::LoadKeyed(k)) => k.clone(),
            other => panic!("{other:?}"),
        }
    };
    let k = decode(kind_at, &[0xff]);
    assert_eq!(k.property.cases[0].access, Access::Element { kind: None });
    assert_eq!(
        (k.keys, k.hint),
        (KeySet::INDEX, KeyedHint::ARRAY | KeyedHint::HOLE)
    );
    // Bit 15 (the top value, never the next free one) is dropped; the named bit stays.
    let raw = ElementsKindSet::of(ElementsKind::HoleyDouble).bits() | 1 << 15;
    let k = decode(kinds_at, &raw.to_le_bytes());
    assert_eq!(k.kinds, ElementsKindSet::of(ElementsKind::HoleyDouble));
    assert_eq!(k.property.cases.len(), 1);
}

/// Two table shapes that differ only in an open value this schema does not name fold into one
/// value. The decoder then merges the cases as the exporter does: equal cases become one, and
/// a receiver with two different accesses is left out. The result is valid and canonical.
#[test]
fn shapes_that_fold_into_one_merge_their_cases() {
    let shape = |k: ElementsKind| ShapeDesc {
        elements: Some(k),
        ..bare()
    };
    // Kinds 5 and 6 sort in that order; the decoder below is shown the top two bytes
    // (UNNAMED: values no schema names) in their place.
    let (a, b) = (
        shape(ElementsKind::HoleyElements),
        shape(ElementsKind::Dictionary),
    );
    let poly = |acc_b: Access| {
        let mut s = FactSnapshot::new(header());
        s.insert(
            0,
            0,
            SlotFacts::LoadNamed(PropertyFacts {
                state: IcState::Polymorphic,
                cases: vec![
                    PropertyCase {
                        receiver: a.clone().into(),
                        access: Access::Generic,
                    },
                    PropertyCase {
                        receiver: b.clone().into(),
                        access: acc_b,
                    },
                ],
                values: TypeSet::EMPTY,
            }),
        )
        .unwrap();
        let mut bytes = s.encode();
        // Each table entry is as long as bare()'s; its elements byte is at +2.
        bytes[HEADER_SIZE + 2] = UNNAMED[0];
        bytes[HEADER_SIZE + entry_size(&a) + 2] = UNNAMED[1];
        let back = FactSnapshot::decode(&bytes).expect("a fold never refuses the snapshot");
        assert_eq!(FactSnapshot::decode(&back.encode()).unwrap(), back);
        match back.get(0, 0) {
            Some(SlotFacts::LoadNamed(p)) => p.clone(),
            other => panic!("{other:?}"),
        }
    };
    let folded = ShapeDesc {
        elements: None,
        ..bare()
    };
    // The same access: one case.
    let p = poly(Access::Generic);
    assert_eq!(p.state, IcState::Polymorphic);
    assert_eq!(
        p.cases,
        [PropertyCase {
            receiver: folded.into(),
            access: Access::Generic
        }]
    );
    // Different accesses: the receiver is left out.
    let p = poly(Access::Missing { depth: 0 });
    assert!(p.cases.is_empty(), "{p:?}");
}

/// The merge after a fold compares shape ranks (`codec.rs`), never shapes. Two `AddField`
/// cases whose targets fold into one shape are the same case; with targets that stay apart
/// they differ, and the receiver is left out. A fold that moves a clone source before
/// another one gives the cases in the new shape order, and the result re-encodes.
#[test]
fn a_merge_after_a_fold_follows_the_ranks_of_the_folded_shapes() {
    let with = |elements: Option<ElementsKind>, capacity: u16| ShapeDesc {
        elements,
        capacity,
        ..bare()
    };
    let (s5, s6) = (
        with(Some(ElementsKind::HoleyElements), 4),
        with(Some(ElementsKind::Dictionary), 4),
    );
    let folded = with(None, 4);
    let at = |i: usize| HEADER_SIZE + entry_size(&folded) * i;
    let decode = |facts: SlotFacts, fold: &[(usize, u8)]| {
        let mut s = FactSnapshot::new(header());
        s.insert(0, 0, facts).unwrap();
        let mut bytes = s.encode();
        for &(i, v) in fold {
            bytes[at(i) + 2] = v;
        }
        let back = FactSnapshot::decode(&bytes).expect("a fold never refuses the snapshot");
        assert_eq!(FactSnapshot::decode(&back.encode()).unwrap(), back);
        back.get(0, 0).unwrap().clone()
    };
    let store = |b_to: &ShapeDesc| {
        SlotFacts::StoreNamed(PropertyFacts {
            state: IcState::Polymorphic,
            cases: [(&s5, &s5), (&s6, b_to)]
                .into_iter()
                .map(|(r, to)| PropertyCase {
                    receiver: r.clone().into(),
                    access: Access::AddField {
                        to: to.clone().into(),
                        field: 0,
                    },
                })
                .collect(),
            values: TypeSet::EMPTY,
        })
    };
    // Table: s5, s6. Both fold into `folded`, and so do both targets: one case.
    let SlotFacts::StoreNamed(p) = decode(store(&s6), &[(0, UNNAMED[0]), (1, UNNAMED[1])]) else {
        panic!()
    };
    assert_eq!(
        p.cases,
        [PropertyCase {
            receiver: folded.clone().into(),
            access: Access::AddField {
                to: folded.clone().into(),
                field: 0
            }
        }]
    );
    // Table: other, s5, s6. The targets `folded` and `other` stay apart: no case.
    let other = with(None, 9);
    let SlotFacts::StoreNamed(p) = decode(store(&other), &[(1, UNNAMED[0]), (2, UNNAMED[1])])
    else {
        panic!()
    };
    assert!(p.cases.is_empty(), "{p:?}");

    // Table: v (none, 5), r (none, 9), u (holey, 4). u folds to (none, 4), before v.
    let (v, r, u) = (with(None, 5), with(None, 9), s5.clone());
    let clone = SlotFacts::CloneObject(CloneFacts {
        state: IcState::Polymorphic,
        cases: [&v, &u]
            .into_iter()
            .map(|src| CloneCase {
                source: src.clone().into(),
                result: r.clone().into(),
            })
            .collect(),
    });
    let SlotFacts::CloneObject(c) = decode(clone, &[(2, UNNAMED[0])]) else {
        panic!()
    };
    let sources: Vec<_> = c.cases.iter().map(|c| (*c.source).clone()).collect();
    assert_eq!(sources, [folded, v]);
}

/// A folded shape does not hide the order of the shapes around it: the shapes that hold only
/// values this schema names stay strictly ascending among themselves.
#[test]
fn a_folded_shape_does_not_hide_the_order_of_its_neighbours() {
    // Three shapes that differ only in capacity (1, 2, 3), in one Polymorphic LoadNamed
    // record.
    let cap = |c: u16| ShapeDesc {
        capacity: c,
        ..bare()
    };
    let mut s = FactSnapshot::new(header());
    s.insert(
        0,
        0,
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Polymorphic,
            cases: [1, 2, 3]
                .map(|c| PropertyCase {
                    receiver: cap(c).into(),
                    access: Access::Generic,
                })
                .to_vec(),
            values: TypeSet::EMPTY,
        }),
    )
    .unwrap();
    let b = s.encode();
    // Each entry is as long as bare()'s; the flags byte is at +3 and the capacity (u16) at +4.
    let at = |i: usize| HEADER_SIZE + entry_size(&cap(1)) * i;
    assert_eq!([b[at(0) + 4], b[at(1) + 4], b[at(2) + 4]], [1, 2, 3]);
    let set_cap = |b: &mut [u8], i: usize, c: u8| b[at(i) + 4] = c;
    // 0x80 is a shape flag bit this schema does not name: shape 1 folds.
    let fold_1 = |b: &mut [u8]| b[at(1) + 3] |= 0x80;
    let not_sorted = Err(FactError::NonCanonical("shapes not strictly ascending"));

    // A fold between shapes in order is accepted.
    let mut ok = b.clone();
    fold_1(&mut ok);
    assert!(FactSnapshot::decode(&ok).is_ok());
    // Shapes 0 and 2 swapped (3, 2, 1), with and without the fold in shape 1.
    let mut swapped = b.clone();
    set_cap(&mut swapped, 0, 3);
    set_cap(&mut swapped, 2, 1);
    assert_eq!(FactSnapshot::decode(&swapped).map(|_| ()), not_sorted);
    fold_1(&mut swapped);
    assert_eq!(FactSnapshot::decode(&swapped).map(|_| ()), not_sorted);
    // The same shape on both sides of a fold (1, folded, 1).
    let mut twice = b.clone();
    set_cap(&mut twice, 2, 1);
    fold_1(&mut twice);
    assert_eq!(FactSnapshot::decode(&twice).map(|_| ()), not_sorted);
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn header_errors() {
    let good = sample().encode();
    let with = |at: usize, bytes: &[u8]| {
        let mut b = good.clone();
        b[at..at + bytes.len()].copy_from_slice(bytes);
        FactSnapshot::decode(&b)
    };
    assert_eq!(FactSnapshot::decode(&[]), Err(FactError::Truncated));
    assert_eq!(FactSnapshot::decode(&good[..11]), Err(FactError::Truncated));
    assert_eq!(FactSnapshot::decode(&good[..63]), Err(FactError::Truncated));
    assert_eq!(with(0, &[0x88]), Err(FactError::Magic));
    assert_eq!(with(7, b"\r"), Err(FactError::Magic));
    // The schema version is read before the rest: another version ends here even when the
    // rest of the input is short or garbage. The probes are the top of the range and the
    // version before this one, never the next value (an append rule, as plan v11 V11-D10).
    for other in [u32::MAX, FACT_SCHEMA_VERSION - 1] {
        assert_eq!(
            with(offsets::SCHEMA_VERSION, &other.to_le_bytes()),
            Err(FactError::SchemaVersion(other))
        );
        let mut short = good[..12].to_vec();
        put_u32(&mut short, offsets::SCHEMA_VERSION, other);
        assert_eq!(
            FactSnapshot::decode(&short),
            Err(FactError::SchemaVersion(other))
        );
    }
    assert_eq!(
        with(offsets::SOURCE, &[3]),
        Err(FactError::BadValue("fact source"))
    );
    for i in 0..3 {
        assert_eq!(with(offsets::RESERVED + i, &[1]), Err(FactError::Reserved));
    }
    assert_eq!(
        with(offsets::TOTAL_SIZE, &(good.len() as u32 + 1).to_le_bytes()),
        Err(FactError::TotalSize)
    );
    let mut longer = good.clone();
    longer.push(0);
    assert_eq!(FactSnapshot::decode(&longer), Err(FactError::TotalSize));
    put_u32(&mut longer, offsets::TOTAL_SIZE, good.len() as u32 + 1);
    assert_eq!(
        FactSnapshot::decode(&longer),
        Err(FactError::NonCanonical("bytes after the last record"))
    );
    // A huge count fails on the input size, before any allocation.
    assert_eq!(
        with(offsets::SHAPE_COUNT, &u32::MAX.to_le_bytes()),
        Err(FactError::Truncated)
    );
    assert_eq!(
        with(offsets::RECORD_COUNT, &u32::MAX.to_le_bytes()),
        Err(FactError::Truncated)
    );
}

/// One record: function 0, slot 0, a JumpLoop with `n` back edges.
fn one_loop(n: u32) -> FactSnapshot {
    let mut s = FactSnapshot::new(header());
    s.insert(0, 0, SlotFacts::JumpLoop(LoopFacts { back_edges: n }))
        .unwrap();
    s
}

#[test]
fn record_errors() {
    let b = one_loop(5).encode();
    let rec = HEADER_SIZE; // no shapes
    let mut bad_kind = b.clone();
    bad_kind[rec + 8] = FeedbackSlotKind::ALL.len() as u8;
    assert_eq!(
        FactSnapshot::decode(&bad_kind),
        Err(FactError::BadValue("slot kind"))
    );

    // Two records out of order (or equal keys) are a second encoding. A record of the sample
    // header (PROFILED) is function, slot, kind, flags, confidence (11 bytes), then the
    // payload (4).
    let mut two = one_loop(5);
    two.insert(0, 1, SlotFacts::JumpLoop(LoopFacts { back_edges: 6 }))
        .unwrap();
    let mut swapped = two.encode();
    let r2 = rec + (two.encode().len() - b.len()); // the record size, from the bytes
    put_u32(&mut swapped, rec + 4, 1);
    put_u32(&mut swapped, r2 + 4, 0);
    assert_eq!(
        FactSnapshot::decode(&swapped),
        Err(FactError::NonCanonical("records not strictly ascending"))
    );
    let mut same = two.encode();
    put_u32(&mut same, r2 + 4, 0);
    assert_eq!(
        FactSnapshot::decode(&same),
        Err(FactError::NonCanonical("records not strictly ascending"))
    );

    // Reserved bits of a bitset.
    let mut s = FactSnapshot::new(header());
    s.insert(0, 0, SlotFacts::TypeOf(TypeSet::ALL)).unwrap();
    let mut b = s.encode();
    b[rec + 11 + 1] = 0x80; // TypeSet bit 15, the top bit (never the next free one)
    assert_eq!(FactSnapshot::decode(&b), Err(FactError::Reserved));

    // A reserved record flag, and the record flags' rules in decode.
    let b = one_loop(5).encode();
    let mut flag = b.clone();
    flag[rec + 9] = 1 << 7; // the top bit (never the next free one)
    assert_eq!(FactSnapshot::decode(&flag), Err(FactError::Reserved));
    let mut ran = b.clone();
    ran[rec + 9] = RecordFlags::RAN.bits(); // JumpLoop has no "ran" bit
    assert_eq!(
        FactSnapshot::decode(&ran),
        Err(FactError::Invalid(
            "ran is only at BinaryOp, CompareOp and LoadKeyed sites"
        ))
    );
    let mut exited = b.clone();
    exited[rec + 9] = RecordFlags::EXITED.bits();
    exited[rec + 10] = 7;
    let back = FactSnapshot::decode(&exited).unwrap();
    assert_eq!(
        back.meta(0, 0),
        Some(RecordMeta {
            flags: RecordFlags::EXITED,
            confidence: 7
        })
    );
}

/// FACT_SCHEMA 2: the record flags and the confidence byte (H29-Q5, MODES-D15, H30-Q1).
/// A RUNTIME record has no confidence byte; "ran" is only at the MODES-D17 kinds; both round
/// trip and print in the dump.
#[test]
fn record_flags_and_confidence() {
    use FeedbackSlotKind as K;
    let meta = |flags, confidence| RecordMeta { flags, confidence };
    for source in FactSource::ALL {
        let mut s = FactSnapshot::new(FactHeader {
            source: *source,
            ..header()
        });
        let runtime = *source == FactSource::Runtime;
        let c = if runtime { 0 } else { 200 };
        for (i, &k) in FeedbackSlotKind::ALL.iter().enumerate() {
            let flags = if RAN_KINDS.contains(&k) {
                RecordFlags::RAN | RecordFlags::EXITED
            } else {
                RecordFlags::EXITED
            };
            s.insert_with(1, i as u32, SlotFacts::empty(k), meta(flags, c))
                .unwrap();
            if !RAN_KINDS.contains(&k) {
                assert_eq!(
                    s.insert_with(2, 0, SlotFacts::empty(k), meta(RecordFlags::RAN, c)),
                    Err(FactError::Invalid(
                        "ran is only at BinaryOp, CompareOp and LoadKeyed sites"
                    ))
                );
            }
        }
        assert_eq!(
            RAN_KINDS,
            [K::BinaryOp, K::CompareOp, K::LoadKeyed],
            "MODES-D17"
        );
        let r = s.insert_with(
            3,
            0,
            SlotFacts::empty(K::JumpLoop),
            meta(RecordFlags::EMPTY, 1),
        );
        assert_eq!(
            r.is_err(),
            runtime,
            "a confidence is refused only in a runtime snapshot"
        );
        let bytes = s.encode();
        let back = FactSnapshot::decode(&bytes).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.encode(), bytes);
        let k = FeedbackSlotKind::BinaryOp as u32;
        assert_eq!(
            back.meta(1, k),
            Some(meta(RecordFlags::RAN | RecordFlags::EXITED, c))
        );
        let line = back
            .dump()
            .lines()
            .find(|l| l.starts_with(&format!("  f1 slot {k} ")))
            .unwrap()
            .to_owned();
        let want = if runtime {
            "BinaryOp flags={exited,ran}: ".to_owned()
        } else {
            "BinaryOp flags={exited,ran} confidence=200: ".to_owned()
        };
        assert!(line.contains(&want), "{line}");
    }
    // A runtime snapshot's records are 1 byte shorter: no confidence byte.
    let one = |source| {
        let mut s = FactSnapshot::new(FactHeader { source, ..header() });
        s.insert(0, 0, SlotFacts::JumpLoop(LoopFacts { back_edges: 1 }))
            .unwrap();
        s.encode().len()
    };
    assert_eq!(one(FactSource::Runtime) + 1, one(FactSource::Hinted));
    assert_eq!(
        loop_payload_at(FactSource::Runtime),
        HEADER_SIZE + 4 + 4 + 1 + 1
    );
}

/// A snapshot with one LoadNamed record whose cases are `cases`, built past `insert` (which
/// would refuse or sort them): the bytes the decoder must refuse.
fn raw_load_named(state: IcState, cases: Vec<PropertyCase>) -> Vec<u8> {
    // Encode a valid snapshot with the same shapes, then patch the case list.
    let mut s = FactSnapshot::new(header());
    let mut sorted = cases.clone();
    sorted.sort();
    sorted.dedup_by(|a, b| a.receiver == b.receiver);
    let receivers: Vec<PropertyCase> = sorted
        .iter()
        .map(|c| PropertyCase {
            receiver: c.receiver.clone(),
            access: Access::Generic,
        })
        .collect();
    s.insert(
        0,
        0,
        SlotFacts::LoadNamed(PropertyFacts {
            state: if receivers.len() > 1 {
                IcState::Polymorphic
            } else {
                IcState::Monomorphic
            },
            cases: receivers,
            values: TypeSet::EMPTY,
        }),
    )
    .unwrap();
    let valid = s.encode();
    let table = s.records().next().unwrap().1;
    let mut shapes = Vec::new();
    table.for_each_shape(&mut |sh| shapes.push(sh.clone()));
    shapes.sort();
    shapes.dedup();
    // Rebuild the record body by hand: function, slot, kind, flags, confidence (11 bytes).
    let shape_end = record_start(&s);
    let mut out = valid[..shape_end].to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(FeedbackSlotKind::LoadNamed as u8);
    out.extend_from_slice(&[0, 0]);
    out.push(state as u8);
    out.extend_from_slice(&(cases.len() as u32).to_le_bytes());
    for c in &cases {
        let i = shapes.binary_search(&c.receiver).unwrap() as u32;
        out.extend_from_slice(&i.to_le_bytes());
        match c.access {
            Access::OwnField { field } => {
                out.push(0);
                out.extend_from_slice(&field.to_le_bytes());
            }
            Access::Generic => out.push(9),
            _ => unimplemented!(),
        }
    }
    // The rest of the record as encode() wrote it (the values, and any field after them).
    out.extend_from_slice(&valid[shape_end + 11 + 1 + 4 + sorted.len() * 5..]);
    let total = out.len() as u32;
    put_u32(&mut out, offsets::TOTAL_SIZE, total);
    out
}

#[test]
fn case_rules_hold_in_insert_and_in_decode() {
    let generic = |s: ShapeDesc| PropertyCase {
        receiver: s.into(),
        access: Access::Generic,
    };
    // Control: the hand encoder agrees with encode() on a valid record.
    let ok = raw_load_named(
        IcState::Polymorphic,
        vec![generic(bare()), generic(point())],
    );
    let decoded = FactSnapshot::decode(&ok).unwrap();
    assert_eq!(decoded.encode(), ok);

    let bad: Vec<(IcState, Vec<PropertyCase>, FactError)> = vec![
        (
            IcState::Monomorphic,
            vec![generic(bare()), generic(point())],
            FactError::Invalid("more cases than the IC state allows"),
        ),
        (
            IcState::Megamorphic,
            vec![generic(bare())],
            FactError::Invalid("more cases than the IC state allows"),
        ),
        (
            IcState::Polymorphic,
            vec![generic(point()), generic(bare())],
            FactError::NonCanonical("list not strictly ascending"),
        ),
        (
            IcState::Polymorphic,
            vec![
                generic(bare()),
                PropertyCase {
                    receiver: bare().into(),
                    access: Access::OwnField { field: 0 },
                },
            ],
            FactError::NonCanonical("list not strictly ascending"),
        ),
        (
            IcState::Monomorphic,
            vec![PropertyCase {
                receiver: point().into(),
                access: Access::OwnField { field: 2 },
            }],
            FactError::Invalid("field index outside the shape's keys"),
        ),
    ];
    for (state, cases, err) in bad {
        assert_eq!(
            FactSnapshot::decode(&raw_load_named(state, cases.clone())),
            Err(err),
            "{cases:?}"
        );
        // insert sorts, so only the order error goes away there.
        let mut s = FactSnapshot::new(header());
        let r = s.insert(
            0,
            0,
            SlotFacts::LoadNamed(PropertyFacts {
                state,
                cases,
                values: TypeSet::EMPTY,
            }),
        );
        assert!(r.is_err() || err == FactError::NonCanonical("list not strictly ascending"));
        if r.is_err() {
            assert!(
                s.is_empty(),
                "a refused record leaves the snapshot unchanged"
            );
        }
    }
    // A count of 5 is refused before any case is read. The count follows function, slot,
    // kind, flags, confidence (11) and the state (1).
    let mut five = ok.clone();
    let at = record_start(&decoded) + 12;
    put_u32(&mut five, at, 5);
    assert_eq!(
        FactSnapshot::decode(&five),
        Err(FactError::Invalid("more than 4 cases"))
    );
}

/// The decoder checks the order of clone sources on their table indices.
#[test]
fn clone_sources_out_of_order_are_refused_by_decode() {
    let mut s = FactSnapshot::new(header());
    s.insert(
        0,
        0,
        SlotFacts::CloneObject(CloneFacts {
            state: IcState::Polymorphic,
            cases: [(bare(), point()), (point(), bare())]
                .into_iter()
                .map(|(source, result)| CloneCase {
                    source: source.into(),
                    result: result.into(),
                })
                .collect(),
        }),
    )
    .unwrap();
    let ok = s.encode();
    // The 2 cases (source u32, result u32 each) follow the record header (11), the state (1)
    // and the count (4).
    let cases = record_start(&s) + 11 + 1 + 4;
    let mut swapped = ok.clone();
    swapped[cases..cases + 16]
        .copy_from_slice(&[&ok[cases + 8..cases + 16], &ok[cases..cases + 8]].concat());
    let mut same = ok.clone();
    same.copy_within(cases..cases + 4, cases + 8);
    for bad in [swapped, same] {
        assert_eq!(
            FactSnapshot::decode(&bad),
            Err(FactError::NonCanonical("list not strictly ascending"))
        );
    }
}

#[test]
fn shape_and_reference_rules() {
    let mono = |receiver: ShapeDesc| {
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Monomorphic,
            cases: vec![PropertyCase {
                receiver: receiver.into(),
                access: Access::Generic,
            }],
            values: TypeSet::EMPTY,
        })
    };
    let insert = |f: SlotFacts| FactSnapshot::new(header()).insert(0, 0, f);
    let mut dict_with_keys = dictionary();
    push_key(&mut dict_with_keys, key("a"));
    assert_eq!(
        insert(mono(dict_with_keys)),
        Err(FactError::Invalid("a dictionary shape lists keys"))
    );
    let mut twice = point();
    push_key(&mut twice, key("x"));
    assert_eq!(
        insert(mono(twice)),
        Err(FactError::Invalid("a key appears twice in a shape"))
    );
    // Two nameless symbols may be different symbols.
    let mut syms = point();
    push_key(
        &mut syms,
        (KeyRef::OtherSymbol, PropAttrs::EMPTY, FieldHint::default()),
    );
    push_key(
        &mut syms,
        (KeyRef::OtherSymbol, PropAttrs::EMPTY, FieldHint::default()),
    );
    assert_eq!(insert(mono(syms)), Ok(()));
    let mut wide = point();
    push_key(
        &mut wide,
        (
            KeyRef::String(UnitString::Utf16(vec![0x61].into())),
            PropAttrs::EMPTY,
            FieldHint::default(),
        ),
    );
    assert_eq!(
        insert(mono(wide)),
        Err(FactError::NonCanonical("UTF-16 key that fits Latin-1"))
    );
    let mut entry_proto = point();
    entry_proto.proto = ProtoRef::FunctionPrototype(js(0));
    assert_eq!(
        insert(mono(entry_proto)),
        Err(FactError::Invalid(
            "function 0 (the entry function) has no closure"
        ))
    );
    let call = SlotFacts::Call(CallFacts {
        count: 1,
        callees: TargetFacts {
            state: IcState::Monomorphic,
            targets: vec![js(0)],
        },
        result: TypeSet::EMPTY,
    });
    assert_eq!(
        insert(call),
        Err(FactError::Invalid(
            "function 0 (the entry function) has no closure"
        ))
    );
    let dup = SlotFacts::InstanceOf(TargetFacts {
        state: IcState::Polymorphic,
        targets: vec![js(1), js(1)],
    });
    assert_eq!(
        insert(dup),
        Err(FactError::NonCanonical("list not strictly ascending"))
    );
}

#[test]
fn shape_table_errors() {
    // A shape no record uses: one LoadNamed record, plus an extra copy of its table entry
    // with one more key, placed after it.
    let mut one = FactSnapshot::new(header());
    one.insert(
        0,
        0,
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Monomorphic,
            cases: vec![PropertyCase {
                receiver: bare().into(),
                access: Access::Generic,
            }],
            values: TypeSet::EMPTY,
        }),
    )
    .unwrap();
    let b = one.encode();
    // The table entry of bare() (17 bytes at FACT_SCHEMA 2) and the entry of bare() with one
    // more key, both as encode() writes them.
    let n = entry_size(&bare());
    let shape = &b[HEADER_SIZE..HEADER_SIZE + n];
    let mut with_l = bare();
    push_key(&mut with_l, key("l"));
    let bigger = table_entry(&with_l);
    let mut extra = b[..HEADER_SIZE + n].to_vec();
    extra.extend_from_slice(&bigger);
    extra.extend_from_slice(&b[HEADER_SIZE + n..]);
    put_u32(&mut extra, offsets::SHAPE_COUNT, 2);
    let total = extra.len() as u32;
    put_u32(&mut extra, offsets::TOTAL_SIZE, total);
    assert_eq!(
        FactSnapshot::decode(&extra),
        Err(FactError::NonCanonical("a shape no record uses"))
    );
    // The same two shapes in the other order.
    let mut swapped = b[..HEADER_SIZE].to_vec();
    swapped.extend_from_slice(&bigger);
    swapped.extend_from_slice(shape);
    swapped.extend_from_slice(&b[HEADER_SIZE + n..]);
    put_u32(&mut swapped, offsets::SHAPE_COUNT, 2);
    let total = swapped.len() as u32;
    put_u32(&mut swapped, offsets::TOTAL_SIZE, total);
    assert_eq!(
        FactSnapshot::decode(&swapped),
        Err(FactError::NonCanonical("shapes not strictly ascending"))
    );
}

#[test]
fn bitsets_merge_by_or() {
    let a = TypeSet::INT32 | TypeSet::STRING;
    let b = TypeSet::DOUBLE;
    let mut m = a;
    m |= b;
    assert_eq!(m, a.union(b));
    assert!(m.contains(a) && m.contains(b));
    assert_eq!(m | m, m);
    assert_eq!(TypeSet::from_bits(1 << 15), None); // the top bit
    assert_eq!(ForInHint::EMPTY.hint(), None);
    assert_eq!(
        (ForInHint::KEYS_AND_INDICES | ForInHint::KEYS).hint(),
        Some(ForInHint::KEYS)
    );
    assert_eq!(ForInHint::ALL.hint(), Some(ForInHint::ANY));
}

/// A shape with many keys is checked in O(n log n): 200,000 distinct keys decode quickly,
/// and one repeated key at the end is found.
#[test]
fn a_shape_with_many_keys_is_checked_without_a_pairwise_scan() {
    let mut big = point();
    set_keys(&mut big, (0..200_000u32).map(|i| key(&format!("k{i}"))));
    let facts = SlotFacts::Literal(LiteralFacts {
        count: 1,
        shape: Some(big.clone().into()),
        elements: None,
    });
    let mut s = FactSnapshot::new(header());
    let t = std::time::Instant::now();
    s.insert(0, 0, facts).unwrap();
    let back = FactSnapshot::decode(&s.encode()).unwrap();
    assert_eq!(back, s);
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
    push_key(&mut big, key("k7"));
    let dup = SlotFacts::Literal(LiteralFacts {
        count: 1,
        shape: Some(big.into()),
        elements: None,
    });
    assert_eq!(
        FactSnapshot::new(header()).insert(0, 0, dup),
        Err(FactError::Invalid("a key appears twice in a shape"))
    );
}

/// `records` copies of the one record `facts`, in slots 0, 1, ...: one shape table, then the
/// record bytes copied, so no record goes through `insert`.
fn repeat_record(facts: SlotFacts, records: u32) -> Vec<u8> {
    let mut s = FactSnapshot::new(header());
    s.insert(0, 0, facts.clone()).unwrap();
    let one = s.encode();
    s.insert(0, 1, facts).unwrap();
    let size = s.encode().len() - one.len();
    let (head, record) = one.split_at(one.len() - size);
    let mut out = head.to_vec();
    for slot in 0..records {
        let at = out.len();
        out.extend_from_slice(record);
        put_u32(&mut out, at + 4, slot);
    }
    put_u32(&mut out, offsets::RECORD_COUNT, records);
    let total = out.len() as u32;
    put_u32(&mut out, offsets::TOTAL_SIZE, total);
    out
}

/// The distinct shape descriptions a snapshot holds, by address, and the references to them.
fn shapes_held(s: &FactSnapshot) -> (usize, usize) {
    let mut held = std::collections::BTreeSet::new();
    let mut refs = 0;
    for (_, f) in s.records() {
        f.for_each_shape(&mut |sh| {
            held.insert(std::ptr::from_ref(sh));
            refs += 1;
        });
    }
    (held.len(), refs)
}

/// Every reference to a table shape decodes to that one shape, not to a copy of it. With a
/// copy per reference, 13,385 bytes (one shape of 2,048 keys, 512 records that use it; 14,410
/// in FACT_SCHEMA 2) held 33.7 MB, and took as much before a corrupt last byte was refused.
#[test]
fn every_reference_to_a_table_shape_shares_one_decoded_shape() {
    let mut big = bare();
    set_keys(
        &mut big,
        vec![(KeyRef::OtherSymbol, PropAttrs::EMPTY, FieldHint::default()); 2048],
    );
    let literal = SlotFacts::Literal(LiteralFacts {
        count: 1,
        shape: Some(big.clone().into()),
        elements: None,
    });
    let bytes = repeat_record(literal.clone(), 512);
    // The header, one table entry and 512 records (14,410 bytes at FACT_SCHEMA 2), with the
    // sizes read from the bytes, so a field append keeps the result.
    let record = repeat_record(literal.clone(), 2).len() - repeat_record(literal, 1).len();
    assert_eq!(bytes.len(), HEADER_SIZE + entry_size(&big) + 512 * record);
    let s = FactSnapshot::decode(&bytes).unwrap();
    assert_eq!(shapes_held(&s), (1, 512));
    assert_eq!(s.encode(), bytes);
    // The shared shapes are `Arc`s, so a snapshot stays Send + Sync.
    fn send_sync<T: Send + Sync>(_: &T) {}
    send_sync(&s);
    // The last record's elements kind, a value no schema names: an open list, so that field
    // reads as none and the rest decodes (MC-w23-8).
    let mut open = bytes;
    *open.last_mut().unwrap() = 0xff;
    let back = FactSnapshot::decode(&open).unwrap();
    assert_eq!(shapes_held(&back), (1, 512));
    let SlotFacts::Literal(l) = back.get(0, 511).unwrap() else {
        panic!()
    };
    assert_eq!((l.count, l.elements), (1, None));
}

/// Decoding checks each shape once, in the table, and the case order on table indices, so
/// its work grows with the input, not with shape size times references. Two shapes of
/// 262,144 keys that differ in one last key and 20,000 records with both as cases (1.6 MB)
/// decode in about 0.05 s in a debug build. Comparing the two shapes per record took 36 s
/// there (9.9 s in release), and checking each record's shapes again took 56 s.
///
/// The same bound holds when a shape holds an open value this schema does not name
/// (FACT_SCHEMA 2, MC-w23-8). Then the decoder merges the cases of each record. When it
/// compared the shapes of each record for that merge, the input above with one such flag bit
/// took 5.2 s in release.
#[test]
fn decoding_work_does_not_grow_with_shape_size_times_references() {
    let mut a = bare();
    set_keys(
        &mut a,
        vec![(KeyRef::OtherSymbol, PropAttrs::EMPTY, FieldHint::default()); 262_144],
    );
    let mut b = a.clone();
    push_key(&mut b, key("z"));
    let poly = |a: &ShapeDesc, b: &ShapeDesc| {
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Polymorphic,
            cases: [a, b]
                .into_iter()
                .map(|s| PropertyCase {
                    receiver: s.clone().into(),
                    access: Access::Generic,
                })
                .collect(),
            values: TypeSet::EMPTY,
        })
    };
    // Shared shapes first: with a copy per reference the large input below needs 335 GB.
    let small = FactSnapshot::decode(&repeat_record(poly(&a, &b), 2)).unwrap();
    assert_eq!(shapes_held(&small), (2, 4));
    let bytes = repeat_record(poly(&a, &b), 20_000);
    // The header, two table entries and 20,000 records (1,608,667 bytes at FACT_SCHEMA 2).
    let record = repeat_record(poly(&a, &b), 2).len() - repeat_record(poly(&a, &b), 1).len();
    let table = entry_size(&a) + entry_size(&b);
    assert_eq!(bytes.len(), HEADER_SIZE + table + 20_000 * record);
    let timed = |bytes: &[u8]| {
        let t = std::time::Instant::now();
        let s = FactSnapshot::decode(bytes).unwrap();
        assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
        s
    };
    assert_eq!(shapes_held(&timed(&bytes)), (2, 40_000));

    // 0x80 is a shape flag bit no schema names: shape 0 folds, but stays apart from shape 1.
    // Both cases of each record stay.
    let mut flag = bytes.clone();
    assert_eq!(flag[HEADER_SIZE + 3], ShapeFlags::EXTENSIBLE.bits());
    flag[HEADER_SIZE + 3] |= 0x80;
    assert_eq!(shapes_held(&timed(&flag)), (2, 40_000));

    // Two shapes that fold into one: `a` with two elements kinds, which the decoder is shown
    // as values no schema names (UNNAMED), so both read as none. The two cases of each
    // record then have one receiver and the same access, and merge into one case.
    let with = |k: ElementsKind| ShapeDesc {
        elements: Some(k),
        ..a.clone()
    };
    let (x, y) = (
        with(ElementsKind::HoleyElements),
        with(ElementsKind::Dictionary),
    );
    // The size of one table entry: the input with one record less is one record shorter.
    let one = repeat_record(poly(&x, &y), 1);
    let entry = (2 * one.len() - repeat_record(poly(&x, &y), 2).len() - HEADER_SIZE) / 2;
    let mut same = repeat_record(poly(&x, &y), 20_000);
    // The elements byte is at +2 of each table entry (`ElementsKind as u8 + 1`).
    assert_eq!(same[HEADER_SIZE + 2], ElementsKind::HoleyElements as u8 + 1);
    assert_eq!(
        same[HEADER_SIZE + entry + 2],
        ElementsKind::Dictionary as u8 + 1
    );
    same[HEADER_SIZE + 2] = UNNAMED[0];
    same[HEADER_SIZE + entry + 2] = UNNAMED[1];
    let s = timed(&same);
    assert_eq!(shapes_held(&s), (1, 20_000));
    let Some(SlotFacts::LoadNamed(p)) = s.get(0, 19_999) else {
        panic!()
    };
    assert_eq!(
        p.cases,
        [PropertyCase {
            receiver: a.into(),
            access: Access::Generic
        }]
    );
}

// ---- MC-P5-FACTTOP: places and sizes read from the bytes, and the top values -------------------
//
// A closed-list append (H16 "IC lists (append-only)", the M6-P5 note) adds a value at the next
// free place, or a field at the end of a payload or of `ShapeDesc`. So these tests never pin a
// count, a last value or a fixed entry size, and never probe the next free value: they read
// places and sizes from the bytes that `encode` writes, and they probe the top value or bit
// (README section 3.4). Then an append keeps every result (H16:236).

/// Two elements-kind bytes of a shape entry (kind + 1) that no schema names: the top two values
/// of the byte, in order. They read as none (an open list), and no append reaches them (the
/// kinds set has 16 bits).
const UNNAMED: [u8; 2] = [0xfe, 0xff];

/// The first byte where `a` and `b` differ. They differ only in the `width` bytes from there.
fn changed_at(a: &[u8], b: &[u8], width: usize) -> usize {
    assert_eq!(a.len(), b.len());
    let first = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .expect("the bytes differ");
    let last = a.iter().zip(b).rposition(|(x, y)| x != y).unwrap();
    assert!(last < first + width, "bytes {first} to {last} differ");
    first
}

/// The start of the one record of `s`: a copy of it in the next slot makes the bytes one record
/// longer, and the shape table stays.
fn record_start(s: &FactSnapshot) -> usize {
    assert_eq!(s.len(), 1);
    let (key, facts, meta) = s.records_with_meta().next().unwrap();
    let mut two = s.clone();
    two.insert_with(key.function, key.slot + 1, facts.clone(), meta)
        .unwrap();
    let one = s.encode().len();
    one - (two.encode().len() - one)
}

/// A snapshot with one monomorphic LoadNamed record on `shape`: the only table entry.
fn one_shape(shape: &ShapeDesc) -> FactSnapshot {
    let mut s = FactSnapshot::new(header());
    s.insert(
        0,
        0,
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Monomorphic,
            cases: vec![PropertyCase {
                receiver: shape.clone().into(),
                access: Access::Generic,
            }],
            values: TypeSet::EMPTY,
        }),
    )
    .unwrap();
    s
}

/// The size of the shape-table entry of `shape`, from the bytes.
fn entry_size(shape: &ShapeDesc) -> usize {
    record_start(&one_shape(shape)) - HEADER_SIZE
}

/// The shape-table entry of `shape`, as `encode` writes it.
fn table_entry(shape: &ShapeDesc) -> Vec<u8> {
    let b = one_shape(shape).encode();
    b[HEADER_SIZE..HEADER_SIZE + entry_size(shape)].to_vec()
}

/// Another named elements kind than `k`.
fn other_kind(k: ElementsKind) -> ElementsKind {
    if k == ElementsKind::PackedInt32 {
        ElementsKind::HoleyInt32
    } else {
        ElementsKind::PackedInt32
    }
}

/// The elements-kind byte of the one Literal record of `s` (the record's own kind, not the
/// shape's).
fn literal_elements_at(s: &FactSnapshot) -> usize {
    let Some(SlotFacts::Literal(l)) = s.get(0, 0) else {
        panic!("{s:?}")
    };
    let mut other = l.clone();
    other.elements = Some(other_kind(l.elements.unwrap()));
    let mut t = FactSnapshot::new(*s.header());
    t.insert(0, 0, SlotFacts::Literal(other)).unwrap();
    changed_at(&s.encode(), &t.encode(), 1)
}

/// The elements-kind byte of the one case (an Element access) and the kinds set (u16) of the
/// one LoadKeyed record of `s`.
fn keyed_offsets(s: &FactSnapshot) -> (usize, usize) {
    let Some(SlotFacts::LoadKeyed(k)) = s.get(0, 0) else {
        panic!("{s:?}")
    };
    let with = |k: KeyedFacts| {
        let mut t = FactSnapshot::new(*s.header());
        t.insert(0, 0, SlotFacts::LoadKeyed(k)).unwrap();
        t.encode()
    };
    let mut case = k.clone();
    let Access::Element { kind } = &mut case.property.cases[0].access else {
        panic!("{k:?}")
    };
    *kind = Some(other_kind(kind.unwrap()));
    let mut kinds = k.clone();
    let first = k.kinds.kinds().next().unwrap();
    kinds.kinds = ElementsKindSet::of(other_kind(first));
    let b = s.encode();
    (
        changed_at(&b, &with(case), 1),
        changed_at(&b, &with(kinds), 2),
    )
}

/// Where the payload of a one-JumpLoop snapshot of `source` starts: the first byte that
/// another back-edge count changes.
fn loop_payload_at(source: FactSource) -> usize {
    let bytes = |n: u32| {
        let mut s = FactSnapshot::new(FactHeader { source, ..header() });
        s.insert(0, 0, SlotFacts::JumpLoop(LoopFacts { back_edges: n }))
            .unwrap();
        s.encode()
    };
    changed_at(&bytes(0), &bytes(u32::MAX), 4)
}

/// MC-P5-FACTTOP: the top value of each closed enum, tag and bitset byte is refused, and so is
/// every value past the named ones (a range that moves with each append, so no result
/// changes).
#[test]
fn the_top_values_of_the_closed_lists_are_refused() {
    // FactSource: the top byte, and every byte past the named sources. (Byte 3 is also
    // `header_errors`'s probe: the H30 provenance set names three sources.)
    let good = sample().encode();
    for v in FactSource::ALL.len()..=0xff {
        let mut b = good.clone();
        b[offsets::SOURCE] = v as u8;
        assert_eq!(
            FactSnapshot::decode(&b),
            Err(FactError::BadValue("fact source")),
            "source byte {v}"
        );
    }

    // The slot-kind byte of a record (MC-P4-SLOTTOP): the top, and every byte past the kinds.
    let b = one_loop(5).encode();
    let rec = HEADER_SIZE;
    for v in FeedbackSlotKind::ALL.len()..=0xff {
        let mut m = b.clone();
        m[rec + 8] = v as u8;
        assert_eq!(
            FactSnapshot::decode(&m),
            Err(FactError::BadValue("slot kind")),
            "kind byte {v}"
        );
    }

    // RecordFlags (u8): bit 7, the top bit, and every bit no flag names.
    let unnamed = !RecordFlags::ALL.bits();
    assert_ne!(unnamed & 0x80, 0);
    for bit in (0..8).filter(|i| unnamed & 1 << i != 0) {
        let mut m = b.clone();
        m[rec + 9] = 1 << bit;
        assert_eq!(
            FactSnapshot::decode(&m),
            Err(FactError::Reserved),
            "flag bit {bit}"
        );
    }

    // TypeSet (u16): bit 15, the top bit, and every bit no type names; the same in
    // `from_bits`.
    let mut s = FactSnapshot::new(header());
    s.insert(0, 0, SlotFacts::TypeOf(TypeSet::ALL)).unwrap();
    let t = s.encode();
    let unnamed = !TypeSet::ALL.bits();
    assert_ne!(unnamed & 0x8000, 0);
    for bit in (0..16).filter(|i| unnamed & 1 << i != 0) {
        assert_eq!(TypeSet::from_bits(1 << bit), None, "type bit {bit}");
        let mut m = t.clone();
        let at = rec + 11;
        let v = u16::from_le_bytes([m[at], m[at + 1]]) | 1 << bit;
        m[at..at + 2].copy_from_slice(&v.to_le_bytes());
        assert_eq!(
            FactSnapshot::decode(&m),
            Err(FactError::Reserved),
            "type bit {bit}"
        );
    }

    // The tags: Access (one case: its tag byte is the byte that another tag changes),
    // ProtoRef and KeyRef (at +0 and +10 of the entry of bare(): proto null, kind, elements,
    // flags, capacity, count, then the tag of key "k"), FunctionRef (the byte before a
    // builtin id). Each refuses the top byte 0xff.
    let generic = one_shape(&bare());
    let mut length = FactSnapshot::new(header());
    length
        .insert(
            0,
            0,
            SlotFacts::LoadNamed(PropertyFacts {
                state: IcState::Monomorphic,
                cases: vec![PropertyCase {
                    receiver: bare().into(),
                    access: Access::ArrayLength,
                }],
                values: TypeSet::EMPTY,
            }),
        )
        .unwrap();
    let g = generic.encode();
    let access_at = changed_at(&g, &length.encode(), 1);
    assert_eq!(g[access_at], Access::Generic.tag());
    let call = |id: u16| {
        let mut s = FactSnapshot::new(header());
        s.insert(
            0,
            0,
            SlotFacts::Call(CallFacts {
                count: 1,
                callees: TargetFacts {
                    state: IcState::Monomorphic,
                    targets: vec![FunctionRef::Builtin(id)],
                },
                result: TypeSet::EMPTY,
            }),
        )
        .unwrap();
        s.encode()
    };
    let c = call(5);
    let function_at = changed_at(&c, &call(6), 1) - 1;
    assert_eq!(c[function_at], FunctionRef::Builtin(5).tag());
    assert_eq!(g[HEADER_SIZE], ProtoRef::Null.tag());
    assert_eq!(g[HEADER_SIZE + 10], key("k").0.tag());
    for (what, bytes, at) in [
        ("access tag", &g, access_at),
        ("proto ref tag", &g, HEADER_SIZE),
        ("key tag", &g, HEADER_SIZE + 10),
        ("function ref tag", &c, function_at),
    ] {
        let mut m = bytes.clone();
        m[at] = 0xff;
        assert_eq!(FactSnapshot::decode(&m), Err(FactError::BadValue(what)));
    }
}
