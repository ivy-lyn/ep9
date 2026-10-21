//! The fact records: position-independent references, shape descriptions and one payload per
//! feedback slot kind (H16 v0 table).
//!
//! ```text
//!  SlotFacts (one variant per FeedbackSlotKind, exhaustive)
//!    property kinds ── PropertyFacts { state, cases[≤4]: (ShapeDesc, Access), values }
//!    keyed kinds ───── KeyedFacts { PropertyFacts, keys: KeySet }
//!    Call / InstanceOf ─ TargetFacts { state, targets[≤4]: FunctionRef } (+ count, result)
//!    globals ───────── GlobalFacts { bindings, values }
//!    hints ─────────── TypeSet, BinaryHint, CompareHint, ForInHint (bitsets, merge by OR)
//!  ShapeDesc { proto: ProtoRef, kind, flags, capacity, keys[(KeyRef, PropAttrs)] }
//!    held as Arc<ShapeDesc>: a decoded snapshot shares one per shape-table entry
//!  never: a ShapeId, an address, a cell kind id, a closure identity
//! ```
//!
//! Order: every list in a record is sorted by the derived `Ord` (fields in declaration order,
//! variants in tag order) with no duplicates, so one set of facts has one encoding.

use std::sync::Arc;

use ep9_bytecode::{FeedbackSlotKind, UnitString};

use crate::FactError;
use crate::bits::{
    BinaryHint, BindingSet, ClosureCount, CompareHint, ElementsKind, ForInHint, IcState, KeySet,
    ObjectKind, PropAttrs, ShapeFlags, TypeSet, WellKnownSymbol,
};

/// Most cases (or targets) one record holds: the H16 polymorphism limit.
pub const MAX_CASES: usize = 4;

/// A SHA-256 unit content hash (`ep9_bytecode::CompiledUnit::content_hash`).
pub type UnitHash = [u8; 32];

/// A function, by a position-independent name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FunctionRef {
    /// Function `function` (stable preorder id, H2) of the unit with content hash `unit`. Any
    /// closure of it: closure identity is never recorded. `function` is never 0, the entry
    /// function, which no closure has (H02).
    Js {
        /// The unit's content hash.
        unit: UnitHash,
        /// The stable function id.
        function: u32,
    },
    /// A builtin by its stable `BuiltinId` (block in bits 9-15, index in bits 0-8).
    Builtin(u16),
}

impl FunctionRef {
    /// Tag names, indexed by tag byte.
    pub const TAGS: &'static [&'static str] = &["js", "builtin"];

    /// The tag byte.
    pub const fn tag(&self) -> u8 {
        match self {
            FunctionRef::Js { .. } => 0,
            FunctionRef::Builtin(_) => 1,
        }
    }
}

/// A prototype, by a position-independent name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProtoRef {
    /// `null`.
    Null,
    /// A realm intrinsic by stable index (H13 IntrinsicRef; the index list is due in M1).
    Intrinsic(u32),
    /// The initial `prototype` object of a function: the prototype of `new F` instances.
    FunctionPrototype(FunctionRef),
    /// Any other object. A consumer must check the prototype itself.
    Unknown,
}

impl ProtoRef {
    /// Tag names, indexed by tag byte.
    pub const TAGS: &'static [&'static str] =
        &["null", "intrinsic", "function-prototype", "unknown"];

    /// The tag byte.
    pub const fn tag(&self) -> u8 {
        match self {
            ProtoRef::Null => 0,
            ProtoRef::Intrinsic(_) => 1,
            ProtoRef::FunctionPrototype(_) => 2,
            ProtoRef::Unknown => 3,
        }
    }
}

/// A property key, by content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyRef {
    /// A string key (an index string stays a string, plan §2 #20). Encoded Latin-1 or UTF-16
    /// by its canonical form.
    String(UnitString),
    /// A well-known symbol.
    WellKnownSymbol(WellKnownSymbol),
    /// Any other symbol. It has no position-independent name.
    OtherSymbol,
}

impl KeyRef {
    /// Tag names, indexed by tag byte.
    pub const TAGS: &'static [&'static str] =
        &["latin1", "utf16", "well-known-symbol", "other-symbol"];

    /// The tag byte. A string's tag is its canonical encoding.
    pub fn tag(&self) -> u8 {
        match self {
            KeyRef::String(UnitString::Latin1(_)) => 0,
            KeyRef::String(UnitString::Utf16(_)) => 1,
            KeyRef::WellKnownSymbol(_) => 2,
            KeyRef::OtherSymbol => 3,
        }
    }
}

/// A shape by its contents: what the VM exporter writes for a live ShapeId (H8: ShapeIds are
/// run-local, so no fact holds one).
///
/// Records hold shapes as `Arc<ShapeDesc>`. The decoder gives every reference to one entry of
/// the shape table the same `Arc`, so a decoded snapshot holds each shape once, however many
/// records use it: its memory grows with the input, not with shape size times references.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShapeDesc {
    /// The prototype.
    pub proto: ProtoRef,
    /// The object kind.
    pub kind: ObjectKind,
    /// Extensible and dictionary bits.
    pub flags: ShapeFlags,
    /// In-object slot capacity. Key `i` lives in-object slot `i` while `i < capacity`, and in
    /// out-of-line slot `i - capacity` after it.
    pub capacity: u16,
    /// The keys in property order with their attributes. Empty for a dictionary shape.
    pub keys: Vec<(KeyRef, PropAttrs)>,
}

/// What a property access did for one receiver shape.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    /// A data property of the receiver: key `field` of its shape.
    OwnField {
        /// Index into the receiver shape's keys.
        field: u16,
    },
    /// A data property of the prototype `depth` steps up (1 = the receiver's prototype).
    ProtoField {
        /// Prototype-chain steps to the holder.
        depth: u8,
        /// Index into the holder shape's keys.
        field: u16,
    },
    /// Not found after `depth` prototypes (the load gives `undefined`; `in` gives false).
    Missing {
        /// Prototypes walked.
        depth: u8,
    },
    /// An accessor whose getter ran.
    Getter {
        /// Prototype-chain steps to the holder (0 = own).
        depth: u8,
        /// The getter.
        getter: FunctionRef,
    },
    /// An accessor whose setter ran.
    Setter {
        /// Prototype-chain steps to the holder (0 = own).
        depth: u8,
        /// The setter.
        setter: FunctionRef,
    },
    /// A store that added a property: the receiver moved to shape `to`.
    AddField {
        /// The shape after the store.
        to: Arc<ShapeDesc>,
        /// Index of the new key in `to`.
        field: u16,
    },
    /// `length` of an Array.
    ArrayLength,
    /// `length` of a string.
    StringLength,
    /// An element access with this H18 elements kind.
    Element {
        /// The elements kind of the receiver.
        kind: ElementsKind,
    },
    /// The generic path ran (dictionary receivers, proxies, anything v0 has no handler for).
    Generic,
}

impl Access {
    /// Tag names, indexed by tag byte.
    pub const TAGS: &'static [&'static str] = &[
        "own-field",
        "proto-field",
        "missing",
        "getter",
        "setter",
        "add-field",
        "array-length",
        "string-length",
        "element",
        "generic",
    ];

    /// The tag byte.
    pub const fn tag(&self) -> u8 {
        match self {
            Access::OwnField { .. } => 0,
            Access::ProtoField { .. } => 1,
            Access::Missing { .. } => 2,
            Access::Getter { .. } => 3,
            Access::Setter { .. } => 4,
            Access::AddField { .. } => 5,
            Access::ArrayLength => 6,
            Access::StringLength => 7,
            Access::Element { .. } => 8,
            Access::Generic => 9,
        }
    }
}

/// One case of a property IC.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PropertyCase {
    /// The receiver's shape (for 0x22, the lookup-start object's shape).
    pub receiver: Arc<ShapeDesc>,
    /// What the access did.
    pub access: Access,
}

/// Facts of a named property site (and the base of a keyed one).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PropertyFacts {
    /// The IC state.
    pub state: IcState,
    /// Up to [`MAX_CASES`] cases with distinct receivers. A case whose shape has no
    /// description is left out, so a state may have fewer cases than the IC had.
    pub cases: Vec<PropertyCase>,
    /// Loaded values (loads) or stored values (stores).
    pub values: TypeSet,
}

/// Facts of a keyed property site.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyedFacts {
    /// The property facts.
    pub property: PropertyFacts,
    /// The keys seen.
    pub keys: KeySet,
}

/// Call targets or `instanceof` constructors seen at a site.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetFacts {
    /// The IC state.
    pub state: IcState,
    /// Up to [`MAX_CASES`] distinct functions; ones without a name are left out.
    pub targets: Vec<FunctionRef>,
}

/// Facts of a call site.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallFacts {
    /// Calls counted (saturating).
    pub count: u32,
    /// The callees.
    pub callees: TargetFacts,
    /// Result types.
    pub result: TypeSet,
}

/// Facts of a global load or store.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalFacts {
    /// What the name resolved to.
    pub bindings: BindingSet,
    /// Loaded or stored values.
    pub values: TypeSet,
}

/// Facts of a literal site (allocation site).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LiteralFacts {
    /// Objects allocated (saturating).
    pub count: u32,
    /// The shape of the last result, if it has a description.
    pub shape: Option<Arc<ShapeDesc>>,
    /// The elements kind of the last array result.
    pub elements: Option<ElementsKind>,
}

/// One case of a `CloneObject` IC.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CloneCase {
    /// The source object's shape.
    pub source: Arc<ShapeDesc>,
    /// The result object's shape.
    pub result: Arc<ShapeDesc>,
}

/// Facts of a `CloneObject` site.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CloneFacts {
    /// The IC state.
    pub state: IcState,
    /// Up to [`MAX_CASES`] cases with distinct sources.
    pub cases: Vec<CloneCase>,
}

/// Facts of a loop back edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LoopFacts {
    /// Back edges taken (saturating).
    pub back_edges: u32,
}

/// Facts of a `CreateClosure` site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClosureFacts {
    /// None, one or many closures.
    pub closures: ClosureCount,
}

// `byte_enum!` derives a fixed trait list, so the default is written out here.
#[allow(clippy::derivable_impls)]
impl Default for IcState {
    fn default() -> IcState {
        IcState::Uninitialized
    }
}

/// The facts of one feedback slot: one variant per [`FeedbackSlotKind`], so the kind byte and
/// the payload always agree.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotFacts {
    /// [`FeedbackSlotKind::Call`]
    Call(CallFacts),
    /// [`FeedbackSlotKind::LoadNamed`]
    LoadNamed(PropertyFacts),
    /// [`FeedbackSlotKind::LoadKeyed`]
    LoadKeyed(KeyedFacts),
    /// [`FeedbackSlotKind::StoreNamed`]
    StoreNamed(PropertyFacts),
    /// [`FeedbackSlotKind::StoreKeyed`]
    StoreKeyed(KeyedFacts),
    /// [`FeedbackSlotKind::DefineNamedOwn`]
    DefineNamedOwn(PropertyFacts),
    /// [`FeedbackSlotKind::DefineKeyedOwn`]
    DefineKeyedOwn(KeyedFacts),
    /// [`FeedbackSlotKind::LoadGlobal`]
    LoadGlobal(GlobalFacts),
    /// [`FeedbackSlotKind::LoadGlobalInsideTypeof`]
    LoadGlobalInsideTypeof(GlobalFacts),
    /// [`FeedbackSlotKind::StoreGlobal`]
    StoreGlobal(GlobalFacts),
    /// [`FeedbackSlotKind::HasKeyed`]
    HasKeyed(KeyedFacts),
    /// [`FeedbackSlotKind::BinaryOp`]
    BinaryOp(BinaryHint),
    /// [`FeedbackSlotKind::CompareOp`]
    CompareOp(CompareHint),
    /// [`FeedbackSlotKind::TypeOf`]
    TypeOf(TypeSet),
    /// [`FeedbackSlotKind::InstanceOf`]
    InstanceOf(TargetFacts),
    /// [`FeedbackSlotKind::Literal`]
    Literal(LiteralFacts),
    /// [`FeedbackSlotKind::CloneObject`]
    CloneObject(CloneFacts),
    /// [`FeedbackSlotKind::StoreInArrayLiteral`]
    StoreInArrayLiteral(KeyedFacts),
    /// [`FeedbackSlotKind::ForIn`]
    ForIn(ForInHint),
    /// [`FeedbackSlotKind::JumpLoop`]
    JumpLoop(LoopFacts),
    /// [`FeedbackSlotKind::CreateClosure`]
    CreateClosure(ClosureFacts),
}

/// The payload type of each kind, for the schema text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// [`CallFacts`]
    Call,
    /// [`PropertyFacts`]
    Property,
    /// [`KeyedFacts`]
    Keyed,
    /// [`GlobalFacts`]
    Global,
    /// [`BinaryHint`]
    Binary,
    /// [`CompareHint`]
    Compare,
    /// [`TypeSet`]
    Types,
    /// [`TargetFacts`]
    Targets,
    /// [`LiteralFacts`]
    Literal,
    /// [`CloneFacts`]
    Clone,
    /// [`ForInHint`]
    ForIn,
    /// [`LoopFacts`]
    Loop,
    /// [`ClosureFacts`]
    Closure,
}

impl Payload {
    /// The payload of a slot kind. An exhaustive match: a new [`FeedbackSlotKind`] does not
    /// compile until it has a payload (and the schema text then changes).
    pub const fn of(kind: FeedbackSlotKind) -> Payload {
        use FeedbackSlotKind as K;
        match kind {
            K::Call => Payload::Call,
            K::LoadNamed | K::StoreNamed | K::DefineNamedOwn => Payload::Property,
            K::LoadKeyed | K::StoreKeyed | K::DefineKeyedOwn | K::HasKeyed => Payload::Keyed,
            K::StoreInArrayLiteral => Payload::Keyed,
            K::LoadGlobal | K::LoadGlobalInsideTypeof | K::StoreGlobal => Payload::Global,
            K::BinaryOp => Payload::Binary,
            K::CompareOp => Payload::Compare,
            K::TypeOf => Payload::Types,
            K::InstanceOf => Payload::Targets,
            K::Literal => Payload::Literal,
            K::CloneObject => Payload::Clone,
            K::ForIn => Payload::ForIn,
            K::JumpLoop => Payload::Loop,
            K::CreateClosure => Payload::Closure,
        }
    }

    /// The payload type's name.
    pub const fn name(self) -> &'static str {
        match self {
            Payload::Call => "CallFacts",
            Payload::Property => "PropertyFacts",
            Payload::Keyed => "KeyedFacts",
            Payload::Global => "GlobalFacts",
            Payload::Binary => "BinaryHint",
            Payload::Compare => "CompareHint",
            Payload::Types => "TypeSet",
            Payload::Targets => "TargetFacts",
            Payload::Literal => "LiteralFacts",
            Payload::Clone => "CloneFacts",
            Payload::ForIn => "ForInHint",
            Payload::Loop => "LoopFacts",
            Payload::Closure => "ClosureFacts",
        }
    }
}

impl SlotFacts {
    /// The facts of a slot that never ran: uninitialized states, empty sets, zero counts.
    pub fn empty(kind: FeedbackSlotKind) -> SlotFacts {
        use FeedbackSlotKind as K;
        match kind {
            K::Call => SlotFacts::Call(CallFacts::default()),
            K::LoadNamed => SlotFacts::LoadNamed(PropertyFacts::default()),
            K::LoadKeyed => SlotFacts::LoadKeyed(KeyedFacts::default()),
            K::StoreNamed => SlotFacts::StoreNamed(PropertyFacts::default()),
            K::StoreKeyed => SlotFacts::StoreKeyed(KeyedFacts::default()),
            K::DefineNamedOwn => SlotFacts::DefineNamedOwn(PropertyFacts::default()),
            K::DefineKeyedOwn => SlotFacts::DefineKeyedOwn(KeyedFacts::default()),
            K::LoadGlobal => SlotFacts::LoadGlobal(GlobalFacts::default()),
            K::LoadGlobalInsideTypeof => SlotFacts::LoadGlobalInsideTypeof(GlobalFacts::default()),
            K::StoreGlobal => SlotFacts::StoreGlobal(GlobalFacts::default()),
            K::HasKeyed => SlotFacts::HasKeyed(KeyedFacts::default()),
            K::BinaryOp => SlotFacts::BinaryOp(BinaryHint::EMPTY),
            K::CompareOp => SlotFacts::CompareOp(CompareHint::EMPTY),
            K::TypeOf => SlotFacts::TypeOf(TypeSet::EMPTY),
            K::InstanceOf => SlotFacts::InstanceOf(TargetFacts::default()),
            K::Literal => SlotFacts::Literal(LiteralFacts::default()),
            K::CloneObject => SlotFacts::CloneObject(CloneFacts::default()),
            K::StoreInArrayLiteral => SlotFacts::StoreInArrayLiteral(KeyedFacts::default()),
            K::ForIn => SlotFacts::ForIn(ForInHint::EMPTY),
            K::JumpLoop => SlotFacts::JumpLoop(LoopFacts::default()),
            K::CreateClosure => SlotFacts::CreateClosure(ClosureFacts {
                closures: ClosureCount::None,
            }),
        }
    }

    /// The slot kind these facts are for.
    pub const fn kind(&self) -> FeedbackSlotKind {
        use FeedbackSlotKind as K;
        match self {
            SlotFacts::Call(_) => K::Call,
            SlotFacts::LoadNamed(_) => K::LoadNamed,
            SlotFacts::LoadKeyed(_) => K::LoadKeyed,
            SlotFacts::StoreNamed(_) => K::StoreNamed,
            SlotFacts::StoreKeyed(_) => K::StoreKeyed,
            SlotFacts::DefineNamedOwn(_) => K::DefineNamedOwn,
            SlotFacts::DefineKeyedOwn(_) => K::DefineKeyedOwn,
            SlotFacts::LoadGlobal(_) => K::LoadGlobal,
            SlotFacts::LoadGlobalInsideTypeof(_) => K::LoadGlobalInsideTypeof,
            SlotFacts::StoreGlobal(_) => K::StoreGlobal,
            SlotFacts::HasKeyed(_) => K::HasKeyed,
            SlotFacts::BinaryOp(_) => K::BinaryOp,
            SlotFacts::CompareOp(_) => K::CompareOp,
            SlotFacts::TypeOf(_) => K::TypeOf,
            SlotFacts::InstanceOf(_) => K::InstanceOf,
            SlotFacts::Literal(_) => K::Literal,
            SlotFacts::CloneObject(_) => K::CloneObject,
            SlotFacts::StoreInArrayLiteral(_) => K::StoreInArrayLiteral,
            SlotFacts::ForIn(_) => K::ForIn,
            SlotFacts::JumpLoop(_) => K::JumpLoop,
            SlotFacts::CreateClosure(_) => K::CreateClosure,
        }
    }

    /// The property facts of a property or keyed kind.
    fn property_mut(&mut self) -> Option<&mut PropertyFacts> {
        match self {
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                Some(p)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => Some(&mut k.property),
            _ => None,
        }
    }

    /// Sorts the lists into canonical order, then checks the record ([`SlotFacts::check`]).
    pub fn normalize(&mut self) -> Result<(), FactError> {
        if let Some(p) = self.property_mut() {
            p.cases.sort();
        }
        match self {
            SlotFacts::Call(c) => c.callees.targets.sort(),
            SlotFacts::InstanceOf(t) => t.targets.sort(),
            SlotFacts::CloneObject(c) => c.cases.sort(),
            _ => {}
        }
        self.check()
    }

    /// Checks the format rules of a record: case and target counts against the IC state,
    /// strictly ascending lists (no duplicates, no two cases for one receiver or source),
    /// field indices inside their shape, and every shape and function reference.
    pub fn check(&self) -> Result<(), FactError> {
        self.check_rules(None)
    }

    /// [`SlotFacts::check`] for a record the decoder just read. Each of its shapes is an entry
    /// of the shape table, which the decoder checked once and found strictly ascending, and
    /// `order` holds the table index of each case's receiver (or clone source). So no shape is
    /// checked again and the case order is checked on the indices: the work per record does
    /// not grow with the size of its shapes.
    pub(crate) fn check_decoded(&self, order: &[u32]) -> Result<(), FactError> {
        self.check_rules(Some(order))
    }

    /// `table` is `None` for [`SlotFacts::check`] and the case order for
    /// [`SlotFacts::check_decoded`].
    fn check_rules(&self, table: Option<&[u32]>) -> Result<(), FactError> {
        match self {
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                check_property(p, table)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => check_property(&k.property, table),
            SlotFacts::Call(c) => check_targets(&c.callees),
            SlotFacts::InstanceOf(t) => check_targets(t),
            SlotFacts::Literal(l) => match (&l.shape, table) {
                (Some(s), None) => check_shape(s),
                _ => Ok(()),
            },
            SlotFacts::CloneObject(c) => {
                check_count(c.state, c.cases.len())?;
                match table {
                    Some(order) => ascending(order, |a, b| a < b),
                    None => {
                        ascending(&c.cases, |a, b| a.source < b.source)?;
                        c.cases.iter().try_for_each(|case| {
                            check_shape(&case.source)?;
                            check_shape(&case.result)
                        })
                    }
                }
            }
            SlotFacts::LoadGlobal(_)
            | SlotFacts::LoadGlobalInsideTypeof(_)
            | SlotFacts::StoreGlobal(_)
            | SlotFacts::BinaryOp(_)
            | SlotFacts::CompareOp(_)
            | SlotFacts::TypeOf(_)
            | SlotFacts::ForIn(_)
            | SlotFacts::JumpLoop(_)
            | SlotFacts::CreateClosure(_) => Ok(()),
        }
    }

    /// Calls `f` on every shape description the record holds (case receivers, `AddField`
    /// targets, clone sources and results, the literal shape), in record order.
    pub fn for_each_shape<'a>(&'a self, f: &mut impl FnMut(&'a ShapeDesc)) {
        match self {
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                property_shapes(p, f)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => property_shapes(&k.property, f),
            SlotFacts::Literal(l) => {
                if let Some(s) = &l.shape {
                    f(s);
                }
            }
            SlotFacts::CloneObject(c) => {
                for case in &c.cases {
                    f(&case.source);
                    f(&case.result);
                }
            }
            SlotFacts::Call(_)
            | SlotFacts::InstanceOf(_)
            | SlotFacts::LoadGlobal(_)
            | SlotFacts::LoadGlobalInsideTypeof(_)
            | SlotFacts::StoreGlobal(_)
            | SlotFacts::BinaryOp(_)
            | SlotFacts::CompareOp(_)
            | SlotFacts::TypeOf(_)
            | SlotFacts::ForIn(_)
            | SlotFacts::JumpLoop(_)
            | SlotFacts::CreateClosure(_) => {}
        }
    }

    /// Calls `f` on every function reference: targets, accessors, and the prototypes of every
    /// shape.
    pub fn for_each_function(&self, f: &mut impl FnMut(&FunctionRef)) {
        match self {
            SlotFacts::Call(c) => c.callees.targets.iter().for_each(&mut *f),
            SlotFacts::InstanceOf(t) => t.targets.iter().for_each(&mut *f),
            _ => {}
        }
        if let Some(p) = match self {
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                Some(p)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => Some(&k.property),
            _ => None,
        } {
            for case in &p.cases {
                if let Access::Getter { getter: r, .. } | Access::Setter { setter: r, .. } =
                    &case.access
                {
                    f(r);
                }
            }
        }
        self.for_each_shape(&mut |s| {
            if let ProtoRef::FunctionPrototype(r) = &s.proto {
                f(r);
            }
        });
    }
}

fn property_shapes<'a>(p: &'a PropertyFacts, f: &mut impl FnMut(&'a ShapeDesc)) {
    for case in &p.cases {
        f(&case.receiver);
        if let Access::AddField { to, .. } = &case.access {
            f(to);
        }
    }
}

/// The number of cases a state allows: none while uninitialized or megamorphic, at most one
/// while monomorphic, at most [`MAX_CASES`] while polymorphic.
fn check_count(state: IcState, n: usize) -> Result<(), FactError> {
    let max = match state {
        IcState::Uninitialized | IcState::Megamorphic => 0,
        IcState::Monomorphic => 1,
        IcState::Polymorphic => MAX_CASES,
    };
    if n > max {
        return Err(FactError::Invalid("more cases than the IC state allows"));
    }
    Ok(())
}

fn ascending<T>(items: &[T], less: impl Fn(&T, &T) -> bool) -> Result<(), FactError> {
    if items.windows(2).all(|w| less(&w[0], &w[1])) {
        Ok(())
    } else {
        Err(FactError::NonCanonical("list not strictly ascending"))
    }
}

/// `table`: see [`SlotFacts::check_rules`]. With the case order of a decoded record, the
/// shapes are table entries the decoder checked already.
fn check_property(p: &PropertyFacts, table: Option<&[u32]>) -> Result<(), FactError> {
    check_count(p.state, p.cases.len())?;
    match table {
        Some(order) => ascending(order, |a, b| a < b)?,
        None => ascending(&p.cases, |a, b| a.receiver < b.receiver)?,
    }
    for case in &p.cases {
        if table.is_none() {
            check_shape(&case.receiver)?;
        }
        match &case.access {
            Access::OwnField { field } => check_field(&case.receiver, *field)?,
            Access::AddField { to, field } => {
                if table.is_none() {
                    check_shape(to)?;
                }
                check_field(to, *field)?;
            }
            Access::Getter { getter: r, .. } | Access::Setter { setter: r, .. } => {
                check_function(r)?;
            }
            Access::ProtoField { .. }
            | Access::Missing { .. }
            | Access::ArrayLength
            | Access::StringLength
            | Access::Element { .. }
            | Access::Generic => {}
        }
    }
    Ok(())
}

fn check_field(shape: &ShapeDesc, field: u16) -> Result<(), FactError> {
    if usize::from(field) < shape.keys.len() {
        Ok(())
    } else {
        Err(FactError::Invalid("field index outside the shape's keys"))
    }
}

fn check_targets(t: &TargetFacts) -> Result<(), FactError> {
    check_count(t.state, t.targets.len())?;
    ascending(&t.targets, |a, b| a < b)?;
    t.targets.iter().try_for_each(check_function)
}

pub(crate) fn check_function(r: &FunctionRef) -> Result<(), FactError> {
    match r {
        FunctionRef::Js { function: 0, .. } => Err(FactError::Invalid(
            "function 0 (the entry function) has no closure",
        )),
        FunctionRef::Js { .. } | FunctionRef::Builtin(_) => Ok(()),
    }
}

/// The rules of one shape description: a dictionary shape lists no keys; keys are distinct;
/// a string key is in canonical form; a function prototype names a closure-able function.
pub(crate) fn check_shape(s: &ShapeDesc) -> Result<(), FactError> {
    if s.flags.contains(ShapeFlags::DICTIONARY) && !s.keys.is_empty() {
        return Err(FactError::Invalid("a dictionary shape lists keys"));
    }
    if let ProtoRef::FunctionPrototype(r) = &s.proto {
        check_function(r)?;
    }
    // A set, not a pairwise scan: a decoded shape may hold any number of keys, and the check
    // must stay O(n log n) on hostile input.
    let mut seen = std::collections::BTreeSet::new();
    for (k, _) in &s.keys {
        if let KeyRef::String(u) = k
            && !u.is_canonical()
        {
            return Err(FactError::NonCanonical("UTF-16 key that fits Latin-1"));
        }
        // OtherSymbol keys have no name, so two of them may be different symbols.
        if *k != KeyRef::OtherSymbol && !seen.insert(k) {
            return Err(FactError::Invalid("a key appears twice in a shape"));
        }
    }
    Ok(())
}
