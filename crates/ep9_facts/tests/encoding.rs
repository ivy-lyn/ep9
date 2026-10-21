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
    let mut accepted = 0;
    let mut accept = |b: &[u8]| {
        if let Ok(s) = FactSnapshot::decode(b) {
            assert_eq!(s.encode(), b, "an accepted input re-encodes to itself");
            accepted += 1;
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
    assert!(accepted > 0);
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
    // rest of the input is short or garbage.
    assert_eq!(
        with(offsets::SCHEMA_VERSION, &2u32.to_le_bytes()),
        Err(FactError::SchemaVersion(2))
    );
    let mut v2 = good[..12].to_vec();
    put_u32(&mut v2, offsets::SCHEMA_VERSION, 2);
    assert_eq!(FactSnapshot::decode(&v2), Err(FactError::SchemaVersion(2)));
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

    // Two records out of order (or equal keys) are a second encoding.
    let mut two = one_loop(5);
    two.insert(0, 1, SlotFacts::JumpLoop(LoopFacts { back_edges: 6 }))
        .unwrap();
    let mut swapped = two.encode();
    let r2 = rec + 13;
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
    b[rec + 9 + 1] = 0x10; // TypeSet bit 12
    assert_eq!(FactSnapshot::decode(&b), Err(FactError::Reserved));
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
    // Rebuild the record body by hand.
    let shape_end = valid.len() - (9 + 1 + 4 + sorted.len() * 5 + 2);
    let mut out = valid[..shape_end].to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(FeedbackSlotKind::LoadNamed as u8);
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
    out.extend_from_slice(&0u16.to_le_bytes());
    let total = out.len() as u32;
    put_u32(&mut out, offsets::TOTAL_SIZE, total);
    out
}

#[test]
fn case_rules_hold_in_insert_and_in_decode() {
    let generic = |s: ShapeDesc| PropertyCase {
        receiver: s,
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
                    receiver: bare(),
                    access: Access::OwnField { field: 0 },
                },
            ],
            FactError::NonCanonical("list not strictly ascending"),
        ),
        (
            IcState::Monomorphic,
            vec![PropertyCase {
                receiver: point(),
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
    // A count of 5 is refused before any case is read. In `ok` the record is 26 bytes:
    // function, slot, kind (9), state (1), count (4), 2 cases of 5, values (2).
    let mut five = ok.clone();
    let at = ok.len() - 26 + 10;
    put_u32(&mut five, at, 5);
    assert_eq!(
        FactSnapshot::decode(&five),
        Err(FactError::Invalid("more than 4 cases"))
    );
}

#[test]
fn shape_and_reference_rules() {
    let mono = |receiver: ShapeDesc| {
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Monomorphic,
            cases: vec![PropertyCase {
                receiver,
                access: Access::Generic,
            }],
            values: TypeSet::EMPTY,
        })
    };
    let insert = |f: SlotFacts| FactSnapshot::new(header()).insert(0, 0, f);
    let mut dict_with_keys = dictionary();
    dict_with_keys.keys.push(key("a"));
    assert_eq!(
        insert(mono(dict_with_keys)),
        Err(FactError::Invalid("a dictionary shape lists keys"))
    );
    let mut twice = point();
    twice.keys.push(key("x"));
    assert_eq!(
        insert(mono(twice)),
        Err(FactError::Invalid("a key appears twice in a shape"))
    );
    // Two nameless symbols may be different symbols.
    let mut syms = point();
    syms.keys.push((KeyRef::OtherSymbol, PropAttrs::EMPTY));
    syms.keys.push((KeyRef::OtherSymbol, PropAttrs::EMPTY));
    assert_eq!(insert(mono(syms)), Ok(()));
    let mut wide = point();
    wide.keys.push((
        KeyRef::String(UnitString::Utf16(vec![0x61].into())),
        PropAttrs::EMPTY,
    ));
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
                receiver: bare(),
                access: Access::Generic,
            }],
            values: TypeSet::EMPTY,
        }),
    )
    .unwrap();
    let b = one.encode();
    // bare(): proto null (1) + kind (1) + flags (1) + capacity (2) + count (4) + key "k":
    // tag (1) + len (4) + 1 byte + attrs (1) = 16 bytes.
    let shape = &b[HEADER_SIZE..HEADER_SIZE + 16];
    let mut bigger = shape.to_vec();
    bigger[5] = 2; // two keys
    bigger.extend_from_slice(&[0, 1, 0, 0, 0, b'l', 7]);
    let mut extra = b[..HEADER_SIZE + 16].to_vec();
    extra.extend_from_slice(&bigger);
    extra.extend_from_slice(&b[HEADER_SIZE + 16..]);
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
    swapped.extend_from_slice(&b[HEADER_SIZE + 16..]);
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
    assert_eq!(TypeSet::from_bits(1 << 12), None);
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
    big.keys = (0..200_000u32).map(|i| key(&format!("k{i}"))).collect();
    let facts = SlotFacts::Literal(LiteralFacts {
        count: 1,
        shape: Some(big.clone()),
        elements: None,
    });
    let mut s = FactSnapshot::new(header());
    let t = std::time::Instant::now();
    s.insert(0, 0, facts).unwrap();
    let back = FactSnapshot::decode(&s.encode()).unwrap();
    assert_eq!(back, s);
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
    big.keys.push(key("k7"));
    let dup = SlotFacts::Literal(LiteralFacts {
        count: 1,
        shape: Some(big),
        elements: None,
    });
    assert_eq!(
        FactSnapshot::new(header()).insert(0, 0, dup),
        Err(FactError::Invalid("a key appears twice in a shape"))
    );
}
