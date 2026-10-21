//! The validator: a snapshot is checked against the unit it claims to describe. A record
//! whose kind is not the unit's `feedback_slots[i]` is dropped, never trusted.

mod common;

use common::{OTHER_UNIT, point};
use ep9_bytecode::*;
use ep9_facts::*;

const SOURCE: &str = "function f(o) { return o.x + 1; }\nf({x: 1});\n";

/// Function 0 (the script): slot 0 CreateClosure, slot 1 Call.
/// Function 1 (`f`): slot 0 LoadNamed, slot 1 BinaryOp.
fn unit() -> VerifiedUnit {
    let opts = UnitOptions {
        code_kind: CodeKind::Script,
        strict: false,
        typescript: None,
        engine_build_id: [0; 16],
        embed_source: true,
    };
    let mut u = UnitBuilder::new(SOURCE, opts);
    let scope = u.add_scope(
        ScopeKind::Script,
        ScopeFlags::default(),
        None,
        vec![(UnitString::new("f"), VariableMode::Function)],
    );
    let mut f = FunctionBuilder::new(FunctionMeta {
        kind: FunctionKind::Normal,
        flags: FunctionFlags::SIMPLE_PARAMETERS,
        name: Some(UnitString::new("f")),
        parent: Some(FunctionId(0)),
        outer_scope: Some(scope),
        source_start: 0,
        source_end: SOURCE.find('\n').unwrap() as u32,
        parameter_count: 1,
        length: 1,
    });
    let s_load = f.feedback_slot(FeedbackSlotKind::LoadNamed);
    let s_add = f.feedback_slot(FeedbackSlotKind::BinaryOp);
    let k_x = f.string_constant("x");
    f.get_named_property(f.parameter(0), k_x, s_load);
    f.add_smi(1, s_add);
    f.return_();
    let f_code = f.finish().expect("f builds");

    let mut b = FunctionBuilder::new(FunctionMeta::entry(
        CodeKind::Script,
        false,
        SOURCE.len() as u32,
    ));
    let r = b.new_register();
    let k_f = b.function_constant(FunctionId(1));
    let s_closure = b.feedback_slot(FeedbackSlotKind::CreateClosure);
    let s_call = b.feedback_slot(FeedbackSlotKind::Call);
    b.create_closure(k_f, s_closure, 0u8).star(r);
    b.call_undefined_receiver0(r, s_call);
    b.return_();
    u.set_function(FunctionId(0), b.finish().expect("script builds"));
    u.set_function(FunctionId(1), f_code);
    u.finish().expect("unit verifies")
}

fn header_for(unit: &CompiledUnit) -> FactHeader {
    FactHeader {
        bytecode_abi: ABI_VERSION,
        unit_hash: unit.content_hash(),
        source: FactSource::Runtime,
    }
}

fn this(unit: &CompiledUnit, function: u32) -> FunctionRef {
    FunctionRef::Js {
        unit: unit.content_hash(),
        function,
    }
}

fn call(targets: Vec<FunctionRef>) -> SlotFacts {
    SlotFacts::Call(CallFacts {
        count: 1,
        callees: TargetFacts {
            state: if targets.len() > 1 {
                IcState::Polymorphic
            } else {
                IcState::Monomorphic
            },
            targets,
        },
        result: TypeSet::INT32,
    })
}

fn load_x(unit: &CompiledUnit) -> SlotFacts {
    let mut receiver = point();
    receiver.proto = ProtoRef::FunctionPrototype(this(unit, 1));
    SlotFacts::LoadNamed(PropertyFacts {
        state: IcState::Monomorphic,
        cases: vec![PropertyCase {
            receiver: receiver.into(),
            access: Access::OwnField { field: 0 },
        }],
        values: TypeSet::INT32,
    })
}

#[test]
fn matching_records_are_kept() {
    let u = unit();
    assert_eq!(
        u.functions[0].feedback_slots,
        [FeedbackSlotKind::CreateClosure, FeedbackSlotKind::Call]
    );
    assert_eq!(
        u.functions[1].feedback_slots,
        [FeedbackSlotKind::LoadNamed, FeedbackSlotKind::BinaryOp]
    );
    let mut s = FactSnapshot::new(header_for(&u));
    s.insert(
        0,
        0,
        SlotFacts::CreateClosure(ClosureFacts {
            closures: ClosureCount::One,
        }),
    )
    .unwrap();
    // A target in another unit cannot be checked here, so it stays.
    s.insert(
        0,
        1,
        call(vec![
            this(&u, 1),
            FunctionRef::Js {
                unit: OTHER_UNIT,
                function: 99,
            },
        ]),
    )
    .unwrap();
    s.insert(1, 0, load_x(&u)).unwrap();
    s.insert(1, 1, SlotFacts::BinaryOp(BinaryHint::INT32))
        .unwrap();
    let before = s.clone();
    let (kept, dropped) = s.validate(&u).unwrap();
    assert_eq!(dropped, []);
    assert_eq!(kept, before);
}

#[test]
fn wrong_records_are_dropped() {
    let u = unit();
    let mut s = FactSnapshot::new(header_for(&u));
    s.insert(
        0,
        0,
        SlotFacts::CreateClosure(ClosureFacts {
            closures: ClosureCount::Many,
        }),
    )
    .unwrap();
    s.insert(0, 1, call(vec![this(&u, 2)])).unwrap(); // the unit has functions 0 and 1
    s.insert(1, 0, load_x(&u)).unwrap();
    s.insert(1, 1, SlotFacts::TypeOf(TypeSet::STRING)).unwrap(); // the slot is BinaryOp
    s.insert(1, 2, SlotFacts::BinaryOp(BinaryHint::INT32))
        .unwrap(); // f has 2 slots
    s.insert(2, 0, SlotFacts::JumpLoop(LoopFacts { back_edges: 1 }))
        .unwrap(); // no function 2
    // A shape whose prototype names a missing function of this unit.
    let mut bad_proto = point();
    bad_proto.proto = ProtoRef::FunctionPrototype(this(&u, 7));
    let mut s2 = FactSnapshot::new(header_for(&u));
    s2.insert(
        1,
        0,
        SlotFacts::LoadNamed(PropertyFacts {
            state: IcState::Monomorphic,
            cases: vec![PropertyCase {
                receiver: bad_proto.into(),
                access: Access::Generic,
            }],
            values: TypeSet::EMPTY,
        }),
    )
    .unwrap();

    let (kept, dropped) = s.validate(&u).unwrap();
    let key = |function, slot| SlotKey { function, slot };
    assert_eq!(
        dropped,
        [
            Dropped {
                key: key(0, 1),
                reason: DropReason::NoTarget
            },
            Dropped {
                key: key(1, 1),
                reason: DropReason::KindMismatch {
                    unit: FeedbackSlotKind::BinaryOp,
                    record: FeedbackSlotKind::TypeOf,
                },
            },
            Dropped {
                key: key(1, 2),
                reason: DropReason::NoSlot
            },
            Dropped {
                key: key(2, 0),
                reason: DropReason::NoFunction
            },
        ]
    );
    let keys: Vec<SlotKey> = kept.records().map(|(k, _)| k).collect();
    assert_eq!(keys, [key(0, 0), key(1, 0)]);

    let (kept, dropped) = s2.validate(&u).unwrap();
    assert!(kept.is_empty());
    assert_eq!(
        dropped,
        [Dropped {
            key: key(1, 0),
            reason: DropReason::NoTarget
        }]
    );
}

/// The kind byte on the wire is checked against the unit too: a record that decodes fine
/// (its payload matches its own kind byte) is still dropped when the unit's slot differs.
#[test]
fn a_wrong_kind_record_from_bytes_is_dropped() {
    let u = unit();
    let mut s = FactSnapshot::new(header_for(&u));
    s.insert(1, 0, SlotFacts::LoadKeyed(KeyedFacts::default()))
        .unwrap(); // unit: LoadNamed
    s.insert(1, 1, SlotFacts::BinaryOp(BinaryHint::NUMBER))
        .unwrap();
    let bytes = s.encode();
    let decoded = FactSnapshot::decode(&bytes).unwrap();
    let (kept, dropped) = decoded.validate(&u).unwrap();
    assert_eq!(
        dropped,
        [Dropped {
            key: SlotKey {
                function: 1,
                slot: 0
            },
            reason: DropReason::KindMismatch {
                unit: FeedbackSlotKind::LoadNamed,
                record: FeedbackSlotKind::LoadKeyed,
            },
        }]
    );
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept.get(1, 1),
        Some(&SlotFacts::BinaryOp(BinaryHint::NUMBER))
    );
}

#[test]
fn a_stale_snapshot_is_refused_whole() {
    let u = unit();
    let mut h = header_for(&u);
    h.unit_hash[0] ^= 1;
    let s = FactSnapshot::new(h);
    assert_eq!(s.validate(&u).unwrap_err(), Stale::UnitHash);

    let mut h = header_for(&u);
    h.bytecode_abi = ABI_VERSION + 1;
    assert_eq!(
        FactSnapshot::new(h).validate(&u).unwrap_err(),
        Stale::BytecodeAbi {
            snapshot: ABI_VERSION + 1,
            current: ABI_VERSION
        }
    );

    // Another unit (one source byte differs) has another hash.
    let mut other = u.unit().clone();
    other.source = Some(SOURCE.replace("x: 1", "x: 2"));
    assert_ne!(other.content_hash(), u.content_hash());
    assert_eq!(
        FactSnapshot::new(header_for(&u))
            .validate(&other)
            .unwrap_err(),
        Stale::UnitHash
    );
}
