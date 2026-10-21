//! The binary form of a [`FactSnapshot`]: deterministic, little endian, fixed-width fields.
//!
//! ```text
//!  0   8  magic "\x89EP9FS\r\n"         8  4  schema_version (FACT_SCHEMA_VERSION)
//!  12  4  bytecode_abi                  16 32  unit_hash
//!  48  1  source                        49  3  reserved, 0
//!  52  4  shape_count                   56  4  record_count       60  4  total_size
//!  64     shapes[shape_count]: the distinct ShapeDescs of all records, ascending
//!         records[record_count]: function u32, slot u32, kind u8, payload(kind); ascending
//! ```
//!
//! The decoder accepts exactly one encoding per value: anything [`FactSnapshot::encode`]
//! would not write is an error, so `encode(decode(b)) == b` for every accepted `b`. The full
//! field list is in [`crate::schema_text`].

use std::collections::{BTreeMap, BTreeSet};

use ep9_bytecode::{FeedbackSlotKind, UnitString};

use crate::bits::{
    BinaryHint, BindingSet, ClosureCount, CompareHint, ElementsKind, FactSource, ForInHint,
    IcState, KeySet, ObjectKind, PropAttrs, ShapeFlags, TypeSet, WellKnownSymbol,
};
use crate::facts::{
    Access, CallFacts, CloneCase, CloneFacts, ClosureFacts, FunctionRef, GlobalFacts, KeyRef,
    KeyedFacts, LiteralFacts, LoopFacts, MAX_CASES, PropertyCase, PropertyFacts, ProtoRef,
    ShapeDesc, SlotFacts, TargetFacts, check_shape,
};
use crate::snapshot::{FactError, FactHeader, FactSnapshot, SlotKey};
use crate::{FACT_SCHEMA_VERSION, MAGIC};

/// Size of the fixed header.
pub const HEADER_SIZE: usize = 64;

/// Byte offsets of the header fields.
pub mod offsets {
    /// `schema_version` u32.
    pub const SCHEMA_VERSION: usize = 8;
    /// `bytecode_abi` u32.
    pub const BYTECODE_ABI: usize = 12;
    /// `unit_hash` [u8; 32].
    pub const UNIT_HASH: usize = 16;
    /// `source` u8.
    pub const SOURCE: usize = 48;
    /// 3 reserved bytes, zero.
    pub const RESERVED: usize = 49;
    /// `shape_count` u32.
    pub const SHAPE_COUNT: usize = 52;
    /// `record_count` u32.
    pub const RECORD_COUNT: usize = 56;
    /// `total_size` u32.
    pub const TOTAL_SIZE: usize = 60;
}

// ---- writer ----

struct Writer<'a> {
    out: Vec<u8>,
    shapes: BTreeMap<&'a ShapeDesc, u32>,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).expect("more than u32::MAX items in a fact snapshot")
}

impl<'a> Writer<'a> {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn function(&mut self, r: &FunctionRef) {
        self.u8(r.tag());
        match r {
            FunctionRef::Js { unit, function } => {
                self.out.extend_from_slice(unit);
                self.u32(*function);
            }
            FunctionRef::Builtin(id) => self.u16(*id),
        }
    }

    fn shape_desc(&mut self, s: &ShapeDesc) {
        self.u8(s.proto.tag());
        match &s.proto {
            ProtoRef::Null | ProtoRef::Unknown => {}
            ProtoRef::Intrinsic(i) => self.u32(*i),
            ProtoRef::FunctionPrototype(r) => self.function(r),
        }
        self.u8(s.kind as u8);
        self.u8(s.flags.bits());
        self.u16(s.capacity);
        self.u32(count(s.keys.len()));
        for (k, attrs) in &s.keys {
            self.u8(k.tag());
            match k {
                KeyRef::String(UnitString::Latin1(b)) => {
                    self.u32(count(b.len()));
                    self.out.extend_from_slice(b);
                }
                KeyRef::String(UnitString::Utf16(u)) => {
                    self.u32(count(u.len()));
                    u.iter().for_each(|&c| self.u16(c));
                }
                KeyRef::WellKnownSymbol(w) => self.u8(*w as u8),
                KeyRef::OtherSymbol => {}
            }
            self.u8(attrs.bits());
        }
    }

    fn shape(&mut self, s: &ShapeDesc) {
        let i = self.shapes[s];
        self.u32(i);
    }

    fn targets(&mut self, t: &TargetFacts) {
        self.u8(t.state as u8);
        self.u32(count(t.targets.len()));
        for r in &t.targets {
            self.function(r);
        }
    }

    fn property(&mut self, p: &PropertyFacts) {
        self.u8(p.state as u8);
        self.u32(count(p.cases.len()));
        for c in &p.cases {
            self.shape(&c.receiver);
            self.u8(c.access.tag());
            match &c.access {
                Access::OwnField { field } => self.u16(*field),
                Access::ProtoField { depth, field } => {
                    self.u8(*depth);
                    self.u16(*field);
                }
                Access::Missing { depth } => self.u8(*depth),
                Access::Getter { depth, getter: r } | Access::Setter { depth, setter: r } => {
                    self.u8(*depth);
                    self.function(r);
                }
                Access::AddField { to, field } => {
                    self.shape(to);
                    self.u16(*field);
                }
                Access::Element { kind } => self.u8(*kind as u8),
                Access::ArrayLength | Access::StringLength | Access::Generic => {}
            }
        }
        self.u16(p.values.bits());
    }

    fn keyed(&mut self, k: &KeyedFacts) {
        self.property(&k.property);
        self.u8(k.keys.bits());
    }

    fn global(&mut self, g: &GlobalFacts) {
        self.u8(g.bindings.bits());
        self.u16(g.values.bits());
    }

    fn payload(&mut self, f: &SlotFacts) {
        match f {
            SlotFacts::Call(c) => {
                self.u32(c.count);
                self.targets(&c.callees);
                self.u16(c.result.bits());
            }
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                self.property(p)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => self.keyed(k),
            SlotFacts::LoadGlobal(g)
            | SlotFacts::LoadGlobalInsideTypeof(g)
            | SlotFacts::StoreGlobal(g) => self.global(g),
            SlotFacts::BinaryOp(h) => self.u8(h.bits()),
            SlotFacts::CompareOp(h) => self.u16(h.bits()),
            SlotFacts::TypeOf(t) => self.u16(t.bits()),
            SlotFacts::InstanceOf(t) => self.targets(t),
            SlotFacts::Literal(l) => {
                self.u32(l.count);
                match &l.shape {
                    Some(s) => {
                        let i = self.shapes[s];
                        self.u32(i + 1);
                    }
                    None => self.u32(0),
                }
                self.u8(l.elements.map_or(0, |k| k as u8 + 1));
            }
            SlotFacts::CloneObject(c) => {
                self.u8(c.state as u8);
                self.u32(count(c.cases.len()));
                for case in &c.cases {
                    self.shape(&case.source);
                    self.shape(&case.result);
                }
            }
            SlotFacts::ForIn(h) => self.u8(h.bits()),
            SlotFacts::JumpLoop(l) => self.u32(l.back_edges),
            SlotFacts::CreateClosure(c) => self.u8(c.closures as u8),
        }
    }
}

impl FactSnapshot {
    /// The shape table: every distinct shape description of every record, ascending.
    pub(crate) fn shape_table(&self) -> Vec<&ShapeDesc> {
        let mut table: BTreeSet<&ShapeDesc> = BTreeSet::new();
        for (_, f) in self.records() {
            f.for_each_shape(&mut |s| {
                table.insert(s);
            });
        }
        table.into_iter().collect()
    }

    /// The canonical bytes. Equal snapshots give equal bytes on every target.
    pub fn encode(&self) -> Vec<u8> {
        let table = self.shape_table();
        let mut w = Writer {
            out: Vec::with_capacity(HEADER_SIZE + 64 * self.len()),
            shapes: table
                .iter()
                .enumerate()
                .map(|(i, s)| (*s, count(i)))
                .collect(),
        };
        let h = self.header();
        w.out.extend_from_slice(&MAGIC);
        w.u32(FACT_SCHEMA_VERSION);
        w.u32(h.bytecode_abi);
        w.out.extend_from_slice(&h.unit_hash);
        w.u8(h.source as u8);
        w.out.extend_from_slice(&[0; 3]);
        w.u32(count(table.len()));
        w.u32(count(self.len()));
        w.u32(0); // total_size, set below
        debug_assert_eq!(w.out.len(), HEADER_SIZE);
        for s in &table {
            w.shape_desc(s);
        }
        for (key, f) in self.records() {
            w.u32(key.function);
            w.u32(key.slot);
            w.u8(f.kind() as u8);
            w.payload(f);
        }
        let total = count(w.out.len()).to_le_bytes();
        w.out[offsets::TOTAL_SIZE..offsets::TOTAL_SIZE + 4].copy_from_slice(&total);
        w.out
    }

    /// Reads a snapshot. Returns an error for any input that [`FactSnapshot::encode`] would
    /// not write; it never panics.
    pub fn decode(bytes: &[u8]) -> Result<FactSnapshot, FactError> {
        let mut r = Reader {
            b: bytes,
            at: 0,
            shapes: Vec::new(),
            used: Vec::new(),
        };
        if r.take(8)? != MAGIC {
            return Err(FactError::Magic);
        }
        // The version comes before every other field, so another schema always ends here,
        // even if it moved the rest of the header.
        let version = r.u32()?;
        if version != FACT_SCHEMA_VERSION {
            return Err(FactError::SchemaVersion(version));
        }
        let bytecode_abi = r.u32()?;
        let mut unit_hash = [0u8; 32];
        unit_hash.copy_from_slice(r.take(32)?);
        let source = FactSource::from_u8(r.u8()?).ok_or(FactError::BadValue("fact source"))?;
        if r.take(3)? != [0; 3] {
            return Err(FactError::Reserved);
        }
        let shape_count = r.u32()?;
        let record_count = r.u32()?;
        if r.u32()? as usize != bytes.len() {
            return Err(FactError::TotalSize);
        }
        // Each shape takes at least 9 bytes and each record at least 10.
        r.room(shape_count, 9)?;
        for _ in 0..shape_count {
            let s = r.shape_desc()?;
            check_shape(&s)?;
            if r.shapes.last().is_some_and(|prev| *prev >= s) {
                return Err(FactError::NonCanonical("shapes not strictly ascending"));
            }
            r.shapes.push(s);
        }
        r.used = vec![false; r.shapes.len()];
        let mut snap = FactSnapshot::new(FactHeader {
            bytecode_abi,
            unit_hash,
            source,
        });
        r.room(record_count, 10)?;
        let mut last: Option<SlotKey> = None;
        for _ in 0..record_count {
            let key = SlotKey {
                function: r.u32()?,
                slot: r.u32()?,
            };
            if last.is_some_and(|l| l >= key) {
                return Err(FactError::NonCanonical("records not strictly ascending"));
            }
            last = Some(key);
            let kind =
                FeedbackSlotKind::from_u8(r.u8()?).ok_or(FactError::BadValue("slot kind"))?;
            let facts = r.payload(kind)?;
            facts.check()?;
            snap.push_decoded(key, facts);
        }
        if r.at != bytes.len() {
            return Err(FactError::NonCanonical("bytes after the last record"));
        }
        if r.used.iter().any(|u| !u) {
            return Err(FactError::NonCanonical("a shape no record uses"));
        }
        Ok(snap)
    }
}

// ---- reader ----

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    shapes: Vec<ShapeDesc>,
    used: Vec<bool>,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FactError> {
        let end = self.at.checked_add(n).ok_or(FactError::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(FactError::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, FactError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, FactError> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, FactError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    /// Fails unless `n` items of at least `size` bytes each fit in the rest of the input, so
    /// a corrupt count never allocates.
    fn room(&self, n: u32, size: usize) -> Result<(), FactError> {
        let rest = self.b.len() - self.at;
        if (n as usize)
            .checked_mul(size)
            .is_none_or(|need| need > rest)
        {
            return Err(FactError::Truncated);
        }
        Ok(())
    }

    fn cases(&mut self) -> Result<usize, FactError> {
        let n = self.u32()?;
        if n as usize > MAX_CASES {
            return Err(FactError::Invalid("more than 4 cases"));
        }
        Ok(n as usize)
    }

    fn state(&mut self) -> Result<IcState, FactError> {
        IcState::from_u8(self.u8()?).ok_or(FactError::BadValue("IC state"))
    }

    fn types(&mut self) -> Result<TypeSet, FactError> {
        TypeSet::from_bits(self.u16()?).ok_or(FactError::Reserved)
    }

    fn function(&mut self) -> Result<FunctionRef, FactError> {
        Ok(match self.u8()? {
            0 => {
                let mut unit = [0u8; 32];
                unit.copy_from_slice(self.take(32)?);
                FunctionRef::Js {
                    unit,
                    function: self.u32()?,
                }
            }
            1 => FunctionRef::Builtin(self.u16()?),
            _ => return Err(FactError::BadValue("function ref tag")),
        })
    }

    fn shape_desc(&mut self) -> Result<ShapeDesc, FactError> {
        let proto = match self.u8()? {
            0 => ProtoRef::Null,
            1 => ProtoRef::Intrinsic(self.u32()?),
            2 => ProtoRef::FunctionPrototype(self.function()?),
            3 => ProtoRef::Unknown,
            _ => return Err(FactError::BadValue("proto ref tag")),
        };
        let kind = ObjectKind::from_u8(self.u8()?).ok_or(FactError::BadValue("object kind"))?;
        let flags = ShapeFlags::from_bits(self.u8()?).ok_or(FactError::Reserved)?;
        let capacity = self.u16()?;
        let n = self.u32()?;
        self.room(n, 2)?;
        let mut keys = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let key = match self.u8()? {
                0 => {
                    let len = self.u32()? as usize;
                    KeyRef::String(UnitString::Latin1(self.take(len)?.into()))
                }
                1 => {
                    let len = self.u32()?;
                    self.room(len, 2)?;
                    let units = (0..len)
                        .map(|_| self.u16())
                        .collect::<Result<Vec<u16>, _>>()?;
                    let s = UnitString::Utf16(units.into());
                    if !s.is_canonical() {
                        return Err(FactError::NonCanonical("UTF-16 key that fits Latin-1"));
                    }
                    KeyRef::String(s)
                }
                2 => KeyRef::WellKnownSymbol(
                    WellKnownSymbol::from_u8(self.u8()?)
                        .ok_or(FactError::BadValue("well-known symbol"))?,
                ),
                3 => KeyRef::OtherSymbol,
                _ => return Err(FactError::BadValue("key tag")),
            };
            let attrs = PropAttrs::from_bits(self.u8()?).ok_or(FactError::Reserved)?;
            keys.push((key, attrs));
        }
        Ok(ShapeDesc {
            proto,
            kind,
            flags,
            capacity,
            keys,
        })
    }

    fn shape(&mut self) -> Result<ShapeDesc, FactError> {
        let i = self.u32()? as usize;
        let s = self
            .shapes
            .get(i)
            .ok_or(FactError::BadValue("shape index"))?
            .clone();
        self.used[i] = true;
        Ok(s)
    }

    fn targets(&mut self) -> Result<TargetFacts, FactError> {
        let state = self.state()?;
        let n = self.cases()?;
        let targets = (0..n).map(|_| self.function()).collect::<Result<_, _>>()?;
        Ok(TargetFacts { state, targets })
    }

    fn property(&mut self) -> Result<PropertyFacts, FactError> {
        let state = self.state()?;
        let n = self.cases()?;
        let mut cases = Vec::with_capacity(n);
        for _ in 0..n {
            let receiver = self.shape()?;
            let access = match self.u8()? {
                0 => Access::OwnField { field: self.u16()? },
                1 => Access::ProtoField {
                    depth: self.u8()?,
                    field: self.u16()?,
                },
                2 => Access::Missing { depth: self.u8()? },
                3 => Access::Getter {
                    depth: self.u8()?,
                    getter: self.function()?,
                },
                4 => Access::Setter {
                    depth: self.u8()?,
                    setter: self.function()?,
                },
                5 => Access::AddField {
                    to: self.shape()?,
                    field: self.u16()?,
                },
                6 => Access::ArrayLength,
                7 => Access::StringLength,
                8 => Access::Element {
                    kind: ElementsKind::from_u8(self.u8()?)
                        .ok_or(FactError::BadValue("elements kind"))?,
                },
                9 => Access::Generic,
                _ => return Err(FactError::BadValue("access tag")),
            };
            cases.push(PropertyCase { receiver, access });
        }
        let values = self.types()?;
        Ok(PropertyFacts {
            state,
            cases,
            values,
        })
    }

    fn keyed(&mut self) -> Result<KeyedFacts, FactError> {
        let property = self.property()?;
        let keys = KeySet::from_bits(self.u8()?).ok_or(FactError::Reserved)?;
        Ok(KeyedFacts { property, keys })
    }

    fn global(&mut self) -> Result<GlobalFacts, FactError> {
        let bindings = BindingSet::from_bits(self.u8()?).ok_or(FactError::Reserved)?;
        Ok(GlobalFacts {
            bindings,
            values: self.types()?,
        })
    }

    /// The payload of `kind`: an exhaustive match, like [`SlotFacts::empty`].
    fn payload(&mut self, kind: FeedbackSlotKind) -> Result<SlotFacts, FactError> {
        use FeedbackSlotKind as K;
        Ok(match kind {
            K::Call => SlotFacts::Call(CallFacts {
                count: self.u32()?,
                callees: self.targets()?,
                result: self.types()?,
            }),
            K::LoadNamed => SlotFacts::LoadNamed(self.property()?),
            K::LoadKeyed => SlotFacts::LoadKeyed(self.keyed()?),
            K::StoreNamed => SlotFacts::StoreNamed(self.property()?),
            K::StoreKeyed => SlotFacts::StoreKeyed(self.keyed()?),
            K::DefineNamedOwn => SlotFacts::DefineNamedOwn(self.property()?),
            K::DefineKeyedOwn => SlotFacts::DefineKeyedOwn(self.keyed()?),
            K::LoadGlobal => SlotFacts::LoadGlobal(self.global()?),
            K::LoadGlobalInsideTypeof => SlotFacts::LoadGlobalInsideTypeof(self.global()?),
            K::StoreGlobal => SlotFacts::StoreGlobal(self.global()?),
            K::HasKeyed => SlotFacts::HasKeyed(self.keyed()?),
            K::BinaryOp => {
                SlotFacts::BinaryOp(BinaryHint::from_bits(self.u8()?).ok_or(FactError::Reserved)?)
            }
            K::CompareOp => SlotFacts::CompareOp(
                CompareHint::from_bits(self.u16()?).ok_or(FactError::Reserved)?,
            ),
            K::TypeOf => SlotFacts::TypeOf(self.types()?),
            K::InstanceOf => SlotFacts::InstanceOf(self.targets()?),
            K::Literal => {
                let count = self.u32()?;
                let shape = match self.u32()? {
                    0 => None,
                    i => {
                        let i = i as usize - 1;
                        let s = self
                            .shapes
                            .get(i)
                            .ok_or(FactError::BadValue("shape index"))?;
                        let s = s.clone();
                        self.used[i] = true;
                        Some(s)
                    }
                };
                let elements = match self.u8()? {
                    0 => None,
                    k => Some(
                        ElementsKind::from_u8(k - 1).ok_or(FactError::BadValue("elements kind"))?,
                    ),
                };
                SlotFacts::Literal(LiteralFacts {
                    count,
                    shape,
                    elements,
                })
            }
            K::CloneObject => {
                let state = self.state()?;
                let n = self.cases()?;
                let mut cases = Vec::with_capacity(n);
                for _ in 0..n {
                    cases.push(CloneCase {
                        source: self.shape()?,
                        result: self.shape()?,
                    });
                }
                SlotFacts::CloneObject(CloneFacts { state, cases })
            }
            K::StoreInArrayLiteral => SlotFacts::StoreInArrayLiteral(self.keyed()?),
            K::ForIn => {
                SlotFacts::ForIn(ForInHint::from_bits(self.u8()?).ok_or(FactError::Reserved)?)
            }
            K::JumpLoop => SlotFacts::JumpLoop(LoopFacts {
                back_edges: self.u32()?,
            }),
            K::CreateClosure => SlotFacts::CreateClosure(ClosureFacts {
                closures: ClosureCount::from_u8(self.u8()?)
                    .ok_or(FactError::BadValue("closure count"))?,
            }),
        })
    }
}
