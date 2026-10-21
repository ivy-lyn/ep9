//! Shared fixtures: golden-file helpers, a PRNG, the sample snapshot (every variant of every
//! type) and random snapshots.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use ep9_bytecode::{FeedbackSlotKind, UnitString};
use ep9_facts::*;

// ---- golden files ----

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

pub fn blessing() -> bool {
    std::env::var_os("EP9_BLESS").is_some()
}

/// Compares `actual` with a golden file; `EP9_BLESS=1` rewrites the file.
pub fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if blessing() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", path.display()))
        .replace("\r\n", "\n");
    if expected != actual {
        let line = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(expected.lines().count().min(actual.lines().count()));
        panic!(
            "{name} differs from the golden file at line {}:\n  golden: {:?}\n  actual: {:?}\n\
             If the change is intended, bump FACT_SCHEMA_VERSION, run with EP9_BLESS=1 and \
             review the diff.",
            line + 1,
            expected.lines().nth(line),
            actual.lines().nth(line)
        );
    }
}

pub fn read_golden(name: &str) -> String {
    std::fs::read_to_string(golden_path(name))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

pub fn write_golden(name: &str, text: &str) {
    std::fs::write(golden_path(name), text).unwrap();
}

pub fn hexdump(bytes: &[u8]) -> String {
    let mut s = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&format!("{:06x}  {}\n", i * 16, hex.join(" ")));
    }
    s
}

// ---- PRNG ----

/// splitmix64: the same sequence on every target.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }
}

// ---- the sample snapshot ----

/// The unit hash of the sample snapshot.
pub const UNIT: UnitHash = [0x5a; 32];
/// Another unit, for cross-unit function references.
pub const OTHER_UNIT: UnitHash = [0xa5; 32];

/// The bytecode ABI in the sample header: a fixed number, not `ABI_VERSION`, so a bytecode
/// ABI bump that keeps the slot kinds does not change the sample (and the schema
/// fingerprint). The validator tests use the real `ABI_VERSION`.
pub const SAMPLE_ABI: u32 = 2;

pub fn header() -> FactHeader {
    FactHeader {
        bytecode_abi: SAMPLE_ABI,
        unit_hash: UNIT,
        source: FactSource::Profiled,
    }
}

pub fn js(function: u32) -> FunctionRef {
    FunctionRef::Js {
        unit: UNIT,
        function,
    }
}

const WEC: PropAttrs = PropAttrs::WRITABLE
    .union(PropAttrs::ENUMERABLE)
    .union(PropAttrs::CONFIGURABLE);

pub fn key(s: &str) -> (KeyRef, PropAttrs) {
    (KeyRef::String(UnitString::new(s)), WEC)
}

/// `{x, y}` on Object.prototype (intrinsic 1 in the sample), in-object capacity 4.
pub fn point() -> ShapeDesc {
    ShapeDesc {
        proto: ProtoRef::Intrinsic(1),
        kind: ObjectKind::Ordinary,
        flags: ShapeFlags::EXTENSIBLE,
        capacity: 4,
        keys: vec![key("x"), key("y")],
    }
}

/// `point` plus `z`.
pub fn point3() -> ShapeDesc {
    let mut s = point();
    s.keys.push(key("z"));
    s
}

/// An instance of `new F` (function 2) with a non-Latin-1 key, a well-known symbol key and
/// a symbol key without a name.
pub fn instance() -> ShapeDesc {
    ShapeDesc {
        proto: ProtoRef::FunctionPrototype(js(2)),
        kind: ObjectKind::Ordinary,
        flags: ShapeFlags::EXTENSIBLE,
        capacity: 8,
        keys: vec![
            key("caf\u{e9}"),
            (
                KeyRef::String(UnitString::new("\u{20ac}")),
                PropAttrs::WRITABLE,
            ),
            (
                KeyRef::WellKnownSymbol(WellKnownSymbol::Iterator),
                PropAttrs::CONFIGURABLE,
            ),
            (
                KeyRef::OtherSymbol,
                PropAttrs::ACCESSOR.union(PropAttrs::CONFIGURABLE),
            ),
        ],
    }
}

/// A non-extensible dictionary-mode array with no prototype it can name.
pub fn dictionary() -> ShapeDesc {
    ShapeDesc {
        proto: ProtoRef::Unknown,
        kind: ObjectKind::Array,
        flags: ShapeFlags::DICTIONARY,
        capacity: 0,
        keys: vec![],
    }
}

/// A null-prototype object.
pub fn bare() -> ShapeDesc {
    ShapeDesc {
        proto: ProtoRef::Null,
        kind: ObjectKind::Ordinary,
        flags: ShapeFlags::EXTENSIBLE,
        capacity: 4,
        keys: vec![key("k")],
    }
}

/// An object of `kind` with one key, on intrinsic `10 + kind`.
pub fn of_kind(kind: ObjectKind) -> ShapeDesc {
    ShapeDesc {
        proto: ProtoRef::Intrinsic(10 + kind as u32),
        kind,
        flags: ShapeFlags::EXTENSIBLE,
        capacity: 2,
        keys: vec![key("length")],
    }
}

fn case(receiver: ShapeDesc, access: Access) -> PropertyCase {
    PropertyCase { receiver, access }
}

/// One record per slot kind, in kind order: (function, slot, facts). Together they use every
/// variant of every enum and every tag of every reference type (`tests/encoding.rs` checks
/// it).
pub fn sample_records() -> Vec<(u32, u32, SlotFacts)> {
    use SlotFacts as F;
    let other = FunctionRef::Js {
        unit: OTHER_UNIT,
        function: 7,
    };
    vec![
        (
            1,
            0,
            F::Call(CallFacts {
                count: 9,
                callees: TargetFacts {
                    state: IcState::Polymorphic,
                    targets: vec![js(2), other, FunctionRef::Builtin((3 << 9) | 5)],
                },
                result: TypeSet::INT32 | TypeSet::DOUBLE,
            }),
        ),
        (
            1,
            1,
            F::LoadNamed(PropertyFacts {
                state: IcState::Polymorphic,
                cases: vec![
                    case(point(), Access::OwnField { field: 1 }),
                    case(point3(), Access::ProtoField { depth: 1, field: 3 }),
                    case(bare(), Access::Missing { depth: 1 }),
                    case(
                        instance(),
                        Access::Getter {
                            depth: 0,
                            getter: js(3),
                        },
                    ),
                ],
                values: TypeSet::INT32 | TypeSet::UNDEFINED,
            }),
        ),
        (
            1,
            2,
            F::LoadKeyed(KeyedFacts {
                property: PropertyFacts {
                    state: IcState::Polymorphic,
                    cases: vec![
                        case(
                            dictionary(),
                            Access::Element {
                                kind: ElementsKind::Dictionary,
                            },
                        ),
                        case(point(), Access::ArrayLength),
                    ],
                    values: TypeSet::ALL,
                },
                keys: KeySet::INDEX | KeySet::NAME,
            }),
        ),
        (
            1,
            3,
            F::StoreNamed(PropertyFacts {
                state: IcState::Monomorphic,
                cases: vec![case(
                    point(),
                    Access::AddField {
                        to: point3(),
                        field: 2,
                    },
                )],
                values: TypeSet::DOUBLE,
            }),
        ),
        (
            1,
            4,
            F::StoreKeyed(KeyedFacts {
                property: PropertyFacts {
                    state: IcState::Monomorphic,
                    cases: vec![case(
                        instance(),
                        Access::Setter {
                            depth: 2,
                            setter: other,
                        },
                    )],
                    values: TypeSet::STRING,
                },
                keys: KeySet::SYMBOL | KeySet::OTHER,
            }),
        ),
        (
            1,
            5,
            F::DefineNamedOwn(PropertyFacts {
                state: IcState::Megamorphic,
                cases: vec![],
                values: TypeSet::FUNCTION,
            }),
        ),
        (
            1,
            6,
            F::DefineKeyedOwn(KeyedFacts {
                property: PropertyFacts::default(),
                keys: KeySet::EMPTY,
            }),
        ),
        (
            2,
            0,
            F::LoadGlobal(GlobalFacts {
                bindings: BindingSet::LEXICAL | BindingSet::DATA,
                values: TypeSet::PLAIN_OBJECT,
            }),
        ),
        (
            2,
            1,
            F::LoadGlobalInsideTypeof(GlobalFacts {
                bindings: BindingSet::MISSING,
                values: TypeSet::UNDEFINED,
            }),
        ),
        (
            2,
            2,
            F::StoreGlobal(GlobalFacts {
                bindings: BindingSet::ACCESSOR,
                values: TypeSet::NULL | TypeSet::BOOLEAN,
            }),
        ),
        (
            2,
            3,
            F::HasKeyed(KeyedFacts {
                property: PropertyFacts {
                    state: IcState::Monomorphic,
                    cases: vec![case(bare(), Access::StringLength)],
                    values: TypeSet::EMPTY,
                },
                keys: KeySet::NAME,
            }),
        ),
        (2, 4, F::BinaryOp(BinaryHint::ALL)),
        (2, 5, F::CompareOp(CompareHint::ALL)),
        (
            2,
            6,
            F::TypeOf(TypeSet::SYMBOL | TypeSet::BIGINT | TypeSet::ARRAY | TypeSet::OTHER_OBJECT),
        ),
        (
            3,
            0,
            F::InstanceOf(TargetFacts {
                state: IcState::Monomorphic,
                targets: vec![js(2)],
            }),
        ),
        (
            3,
            1,
            F::Literal(LiteralFacts {
                count: 3,
                shape: Some(point()),
                elements: Some(ElementsKind::PackedInt32),
            }),
        ),
        (
            3,
            2,
            F::CloneObject(CloneFacts {
                state: IcState::Polymorphic,
                cases: vec![
                    CloneCase {
                        source: point(),
                        result: bare(),
                    },
                    CloneCase {
                        source: instance(),
                        result: dictionary(),
                    },
                ],
            }),
        ),
        (
            3,
            3,
            F::StoreInArrayLiteral(KeyedFacts {
                property: PropertyFacts {
                    state: IcState::Polymorphic,
                    cases: vec![
                        case(dictionary(), Access::Generic),
                        case(
                            point3(),
                            Access::Element {
                                kind: ElementsKind::HoleyDouble,
                            },
                        ),
                    ],
                    values: TypeSet::INT32,
                },
                keys: KeySet::INDEX,
            }),
        ),
        (3, 4, F::ForIn(ForInHint::KEYS_AND_INDICES | ForInHint::ANY)),
        (
            3,
            5,
            F::JumpLoop(LoopFacts {
                back_edges: 0x0102_0304,
            }),
        ),
        (
            3,
            6,
            F::CreateClosure(ClosureFacts {
                closures: ClosureCount::One,
            }),
        ),
        // Extra records for the variants the first pass leaves out.
        (
            4,
            0,
            F::Literal(LiteralFacts {
                count: 0,
                shape: None,
                elements: None,
            }),
        ),
        (
            4,
            1,
            F::CreateClosure(ClosureFacts {
                closures: ClosureCount::Many,
            }),
        ),
        (
            4,
            2,
            F::CreateClosure(ClosureFacts {
                closures: ClosureCount::None,
            }),
        ),
        (4, 3, F::ForIn(ForInHint::KEYS)),
        (
            4,
            4,
            F::Literal(LiteralFacts {
                count: 1,
                shape: Some(of_kind(ObjectKind::Error)),
                elements: Some(ElementsKind::HoleyElements),
            }),
        ),
        (
            4,
            5,
            F::LoadKeyed(KeyedFacts {
                property: PropertyFacts {
                    state: IcState::Polymorphic,
                    cases: vec![
                        case(
                            of_kind(ObjectKind::Function),
                            Access::Element {
                                kind: ElementsKind::HoleyInt32,
                            },
                        ),
                        case(
                            of_kind(ObjectKind::Iterator),
                            Access::Element {
                                kind: ElementsKind::PackedDouble,
                            },
                        ),
                        case(
                            of_kind(ObjectKind::PrimitiveWrapper),
                            Access::Element {
                                kind: ElementsKind::PackedElements,
                            },
                        ),
                        case(of_kind(ObjectKind::BoundFunction), Access::Generic),
                    ],
                    values: TypeSet::INT32,
                },
                keys: KeySet::INDEX,
            }),
        ),
    ]
}

/// The sample: [`sample_records`] inserted in order.
pub fn sample() -> FactSnapshot {
    let mut s = FactSnapshot::new(header());
    for (f, i, facts) in sample_records() {
        s.insert(f, i, facts).expect("sample record is valid");
    }
    s
}

// ---- coverage ----

/// The names of every tag and enum value the records of a snapshot use, as `<type>:<name>`
/// (the header's FactSource is tested on its own). The matches are exhaustive, so a new kind
/// does not compile here until it is handled.
pub fn coverage(s: &FactSnapshot) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |t: &str, n: &str| {
        out.insert(format!("{t}:{n}"));
    };
    for (_, f) in s.records() {
        add("kind", f.kind().name());
        let state = match f {
            SlotFacts::Call(c) => Some(c.callees.state),
            SlotFacts::InstanceOf(t) => Some(t.state),
            SlotFacts::CloneObject(c) => Some(c.state),
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                Some(p.state)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => Some(k.property.state),
            SlotFacts::LoadGlobal(_)
            | SlotFacts::LoadGlobalInsideTypeof(_)
            | SlotFacts::StoreGlobal(_)
            | SlotFacts::BinaryOp(_)
            | SlotFacts::CompareOp(_)
            | SlotFacts::TypeOf(_)
            | SlotFacts::Literal(_)
            | SlotFacts::ForIn(_)
            | SlotFacts::JumpLoop(_)
            | SlotFacts::CreateClosure(_) => None,
        };
        if let Some(st) = state {
            add("IcState", st.name());
        }
        match f {
            SlotFacts::Literal(l) => {
                if let Some(k) = l.elements {
                    add("ElementsKind", k.name());
                }
            }
            SlotFacts::CreateClosure(c) => add("ClosureCount", c.closures.name()),
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                for c in &p.cases {
                    add("Access", Access::TAGS[c.access.tag() as usize]);
                    if let Access::Element { kind } = c.access {
                        add("ElementsKind", kind.name());
                    }
                }
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => {
                for c in &k.property.cases {
                    add("Access", Access::TAGS[c.access.tag() as usize]);
                    if let Access::Element { kind } = c.access {
                        add("ElementsKind", kind.name());
                    }
                }
            }
            _ => {}
        }
        f.for_each_function(&mut |r| add("FunctionRef", FunctionRef::TAGS[r.tag() as usize]));
        f.for_each_shape(&mut |sh| {
            add("ProtoRef", ProtoRef::TAGS[sh.proto.tag() as usize]);
            add("ObjectKind", sh.kind.name());
            for (k, _) in &sh.keys {
                add("KeyRef", KeyRef::TAGS[k.tag() as usize]);
            }
        });
    }
    out
}

/// Everything [`coverage`] can report.
pub fn every_variant() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |t: &str, names: &mut dyn Iterator<Item = &'static str>| {
        for n in names {
            out.insert(format!("{t}:{n}"));
        }
    };
    add("kind", &mut FeedbackSlotKind::ALL.iter().map(|v| v.name()));
    add("IcState", &mut IcState::ALL.iter().map(|v| v.name()));
    add(
        "ElementsKind",
        &mut ElementsKind::ALL.iter().map(|v| v.name()),
    );
    add(
        "ClosureCount",
        &mut ClosureCount::ALL.iter().map(|v| v.name()),
    );
    add("Access", &mut Access::TAGS.iter().copied());
    add("FunctionRef", &mut FunctionRef::TAGS.iter().copied());
    add("ProtoRef", &mut ProtoRef::TAGS.iter().copied());
    add("ObjectKind", &mut ObjectKind::ALL.iter().map(|v| v.name()));
    add("KeyRef", &mut KeyRef::TAGS.iter().copied());
    out
}

// ---- random snapshots ----

fn random_function(rng: &mut Rng) -> FunctionRef {
    match rng.below(3) {
        0 => js(1 + rng.below(6) as u32),
        1 => FunctionRef::Js {
            unit: OTHER_UNIT,
            function: 1 + rng.below(3) as u32,
        },
        _ => FunctionRef::Builtin(rng.next() as u16),
    }
}

fn random_key(rng: &mut Rng) -> KeyRef {
    match rng.below(5) {
        0 => KeyRef::String(UnitString::new(rng.pick(&["a", "b", "c", "length", ""]))),
        1 => KeyRef::String(UnitString::from_utf16(&[
            0x61,
            0xd800 + rng.below(4) as u16,
        ])),
        2 => KeyRef::String(UnitString::new(rng.pick(&["\u{e9}", "x\u{ff}", "0", "1"]))),
        3 => KeyRef::WellKnownSymbol(*rng.pick(WellKnownSymbol::ALL)),
        _ => KeyRef::OtherSymbol,
    }
}

pub fn random_shape(rng: &mut Rng) -> ShapeDesc {
    let proto = match rng.below(4) {
        0 => ProtoRef::Null,
        1 => ProtoRef::Intrinsic(rng.below(4) as u32),
        2 => ProtoRef::FunctionPrototype(random_function(rng)),
        _ => ProtoRef::Unknown,
    };
    let dictionary = rng.below(6) == 0;
    let mut keys: Vec<(KeyRef, PropAttrs)> = Vec::new();
    if !dictionary {
        for _ in 0..rng.below(5) {
            let k = random_key(rng);
            if k == KeyRef::OtherSymbol || !keys.iter().any(|(o, _)| *o == k) {
                keys.push((k, PropAttrs::from_bits(rng.below(16) as u8).unwrap()));
            }
        }
    }
    ShapeDesc {
        proto,
        kind: *rng.pick(ObjectKind::ALL),
        flags: if dictionary {
            ShapeFlags::DICTIONARY | ShapeFlags::from_bits(rng.below(2) as u8).unwrap()
        } else {
            ShapeFlags::EXTENSIBLE
        },
        capacity: rng.below(9) as u16,
        keys,
    }
}

fn random_access(rng: &mut Rng, receiver: &ShapeDesc) -> Access {
    loop {
        let a = match rng.below(10) {
            0 if !receiver.keys.is_empty() => Access::OwnField {
                field: rng.below(receiver.keys.len() as u64) as u16,
            },
            1 => Access::ProtoField {
                depth: rng.below(4) as u8,
                field: rng.below(300) as u16,
            },
            2 => Access::Missing {
                depth: rng.below(4) as u8,
            },
            3 => Access::Getter {
                depth: rng.below(3) as u8,
                getter: random_function(rng),
            },
            4 => Access::Setter {
                depth: rng.below(3) as u8,
                setter: random_function(rng),
            },
            5 => {
                let mut to = receiver.clone();
                to.flags = ShapeFlags::EXTENSIBLE;
                to.keys.push((
                    KeyRef::String(UnitString::new("added")),
                    PropAttrs::WRITABLE,
                ));
                Access::AddField {
                    field: to.keys.len() as u16 - 1,
                    to,
                }
            }
            6 => Access::ArrayLength,
            7 => Access::StringLength,
            8 => Access::Element {
                kind: *rng.pick(ElementsKind::ALL),
            },
            9 => Access::Generic,
            _ => continue,
        };
        return a;
    }
}

fn random_state(rng: &mut Rng) -> (IcState, usize) {
    let state = *rng.pick(IcState::ALL);
    let max = match state {
        IcState::Uninitialized | IcState::Megamorphic => 0,
        IcState::Monomorphic => 1,
        IcState::Polymorphic => MAX_CASES,
    };
    (state, rng.below(max as u64 + 1) as usize)
}

fn random_property(rng: &mut Rng) -> PropertyFacts {
    let (state, n) = random_state(rng);
    let mut cases: Vec<PropertyCase> = Vec::new();
    for _ in 0..n {
        let receiver = random_shape(rng);
        if cases.iter().all(|c| c.receiver != receiver) {
            let access = random_access(rng, &receiver);
            cases.push(PropertyCase { receiver, access });
        }
    }
    PropertyFacts {
        state,
        cases,
        values: TypeSet::from_bits(rng.next() as u16 & TypeSet::ALL.bits()).unwrap(),
    }
}

fn random_targets(rng: &mut Rng) -> TargetFacts {
    let (state, n) = random_state(rng);
    let mut targets: Vec<FunctionRef> = Vec::new();
    for _ in 0..n {
        let r = random_function(rng);
        if !targets.contains(&r) {
            targets.push(r);
        }
    }
    TargetFacts { state, targets }
}

/// Random valid facts for `kind`, in any list order (insert sorts them).
pub fn random_facts(rng: &mut Rng, kind: FeedbackSlotKind) -> SlotFacts {
    use FeedbackSlotKind as K;
    let types =
        |rng: &mut Rng| TypeSet::from_bits(rng.next() as u16 & TypeSet::ALL.bits()).unwrap();
    let keyed = |rng: &mut Rng| KeyedFacts {
        property: random_property(rng),
        keys: KeySet::from_bits(rng.below(16) as u8).unwrap(),
    };
    let global = |rng: &mut Rng| GlobalFacts {
        bindings: BindingSet::from_bits(rng.below(16) as u8).unwrap(),
        values: types(rng),
    };
    match kind {
        K::Call => SlotFacts::Call(CallFacts {
            count: rng.next() as u32,
            callees: random_targets(rng),
            result: types(rng),
        }),
        K::LoadNamed => SlotFacts::LoadNamed(random_property(rng)),
        K::LoadKeyed => SlotFacts::LoadKeyed(keyed(rng)),
        K::StoreNamed => SlotFacts::StoreNamed(random_property(rng)),
        K::StoreKeyed => SlotFacts::StoreKeyed(keyed(rng)),
        K::DefineNamedOwn => SlotFacts::DefineNamedOwn(random_property(rng)),
        K::DefineKeyedOwn => SlotFacts::DefineKeyedOwn(keyed(rng)),
        K::LoadGlobal => SlotFacts::LoadGlobal(global(rng)),
        K::LoadGlobalInsideTypeof => SlotFacts::LoadGlobalInsideTypeof(global(rng)),
        K::StoreGlobal => SlotFacts::StoreGlobal(global(rng)),
        K::HasKeyed => SlotFacts::HasKeyed(keyed(rng)),
        K::BinaryOp => SlotFacts::BinaryOp(BinaryHint::from_bits(rng.below(128) as u8).unwrap()),
        K::CompareOp => {
            SlotFacts::CompareOp(CompareHint::from_bits(rng.below(1024) as u16).unwrap())
        }
        K::TypeOf => SlotFacts::TypeOf(types(rng)),
        K::InstanceOf => SlotFacts::InstanceOf(random_targets(rng)),
        K::Literal => SlotFacts::Literal(LiteralFacts {
            count: rng.next() as u32,
            shape: (rng.below(2) == 0).then(|| random_shape(rng)),
            elements: (rng.below(2) == 0).then(|| *rng.pick(ElementsKind::ALL)),
        }),
        K::CloneObject => {
            let (state, n) = random_state(rng);
            let mut cases: Vec<CloneCase> = Vec::new();
            for _ in 0..n {
                let source = random_shape(rng);
                if cases.iter().all(|c| c.source != source) {
                    cases.push(CloneCase {
                        source,
                        result: random_shape(rng),
                    });
                }
            }
            SlotFacts::CloneObject(CloneFacts { state, cases })
        }
        K::StoreInArrayLiteral => SlotFacts::StoreInArrayLiteral(keyed(rng)),
        K::ForIn => SlotFacts::ForIn(ForInHint::from_bits(rng.below(8) as u8).unwrap()),
        K::JumpLoop => SlotFacts::JumpLoop(LoopFacts {
            back_edges: rng.next() as u32,
        }),
        K::CreateClosure => SlotFacts::CreateClosure(ClosureFacts {
            closures: *rng.pick(ClosureCount::ALL),
        }),
    }
}

/// A random snapshot and its records, so a test can insert them in another order.
pub fn random_records(rng: &mut Rng) -> Vec<(u32, u32, SlotFacts)> {
    let mut out: Vec<(u32, u32, SlotFacts)> = Vec::new();
    for _ in 0..rng.below(12) {
        let (f, i) = (rng.below(4) as u32, rng.below(6) as u32);
        if out.iter().any(|(a, b, _)| (*a, *b) == (f, i)) {
            continue;
        }
        let kind = *rng.pick(FeedbackSlotKind::ALL);
        out.push((f, i, random_facts(rng, kind)));
    }
    out
}

pub fn snapshot_of(records: &[(u32, u32, SlotFacts)], source: FactSource) -> FactSnapshot {
    let mut s = FactSnapshot::new(FactHeader { source, ..header() });
    for (f, i, facts) in records {
        s.insert(*f, *i, facts.clone())
            .expect("random record is valid");
    }
    s
}
