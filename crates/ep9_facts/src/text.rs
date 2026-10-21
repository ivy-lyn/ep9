//! The text dump of a snapshot and the schema text.
//!
//! `schema_text()` describes the whole format: header, every type's fields, the kind →
//! payload table, every enum value and bit, and the rules. `tests/golden/schema-history.txt`
//! maps each `FACT_SCHEMA_VERSION` to the SHA-256 of this text plus the sample snapshot's
//! bytes, so a format change without a version bump fails the history test.

use core::fmt::Write as _;

use ep9_bytecode::FeedbackSlotKind;

use crate::bits::{
    BinaryHint, BindingSet, ClosureCount, CompareHint, ElementsKind, FactSource, ForInHint,
    IcState, KeySet, ObjectKind, PropAttrs, ShapeFlags, TypeSet, WellKnownSymbol, bit_names,
};
use crate::codec::HEADER_SIZE;
use crate::facts::{
    Access, FunctionRef, KeyRef, MAX_CASES, Payload, PropertyFacts, ProtoRef, ShapeDesc, SlotFacts,
    TargetFacts,
};
use crate::snapshot::FactSnapshot;

macro_rules! bits_text {
    ($set:expr, $ty:ty) => {
        bit_names(<$ty>::BITS, |b| $set.contains(b))
    };
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Dump<'a> {
    snap: &'a FactSnapshot,
    shapes: Vec<&'a ShapeDesc>,
}

impl Dump<'_> {
    fn function(&self, r: &FunctionRef) -> String {
        match r {
            FunctionRef::Js { unit, function } if *unit == self.snap.header().unit_hash => {
                format!("js:self#{function}")
            }
            FunctionRef::Js { unit, function } => format!("js:{}#{function}", hex(unit)),
            FunctionRef::Builtin(id) => format!("builtin:{}.{}", id >> 9, id & 0x1ff),
        }
    }

    fn shape_ref(&self, s: &ShapeDesc) -> String {
        match self.shapes.binary_search(&s) {
            Ok(i) => format!("s{i}"),
            Err(_) => unreachable!("every shape of a record is in the table"),
        }
    }

    fn shape_desc(&self, s: &ShapeDesc) -> String {
        let proto = match &s.proto {
            ProtoRef::Null => "null".to_owned(),
            ProtoRef::Intrinsic(i) => format!("intrinsic#{i}"),
            ProtoRef::FunctionPrototype(r) => format!("prototype-of({})", self.function(r)),
            ProtoRef::Unknown => "unknown".to_owned(),
        };
        let keys: Vec<String> = s
            .keys
            .iter()
            .map(|(k, a)| {
                let k = match k {
                    KeyRef::String(u) => u.quoted(),
                    KeyRef::WellKnownSymbol(w) => format!("@@{}", w.name()),
                    KeyRef::OtherSymbol => "<symbol>".to_owned(),
                };
                format!("{k} {}", bits_text!(a, PropAttrs))
            })
            .collect();
        format!(
            "proto={proto} kind={} flags={} capacity={} keys=[{}]",
            s.kind.name(),
            bits_text!(s.flags, ShapeFlags),
            s.capacity,
            keys.join(", ")
        )
    }

    fn access(&self, a: &Access) -> String {
        match a {
            Access::OwnField { field } => format!("own-field {field}"),
            Access::ProtoField { depth, field } => format!("proto-field depth={depth} {field}"),
            Access::Missing { depth } => format!("missing depth={depth}"),
            Access::Getter { depth, getter } => {
                format!("getter depth={depth} {}", self.function(getter))
            }
            Access::Setter { depth, setter } => {
                format!("setter depth={depth} {}", self.function(setter))
            }
            Access::AddField { to, field } => format!("add-field {} {field}", self.shape_ref(to)),
            Access::ArrayLength => "array-length".to_owned(),
            Access::StringLength => "string-length".to_owned(),
            Access::Element { kind } => format!("element {}", kind.name()),
            Access::Generic => "generic".to_owned(),
        }
    }

    fn property(&self, p: &PropertyFacts) -> String {
        let cases: Vec<String> = p
            .cases
            .iter()
            .map(|c| format!("{} {}", self.shape_ref(&c.receiver), self.access(&c.access)))
            .collect();
        format!(
            "{} [{}] values={}",
            p.state.name(),
            cases.join(", "),
            bits_text!(p.values, TypeSet)
        )
    }

    fn targets(&self, t: &TargetFacts) -> String {
        let targets: Vec<String> = t.targets.iter().map(|r| self.function(r)).collect();
        format!("{} [{}]", t.state.name(), targets.join(", "))
    }

    fn payload(&self, f: &SlotFacts) -> String {
        match f {
            SlotFacts::Call(c) => format!(
                "count={} {} result={}",
                c.count,
                self.targets(&c.callees),
                bits_text!(c.result, TypeSet)
            ),
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) | SlotFacts::DefineNamedOwn(p) => {
                self.property(p)
            }
            SlotFacts::LoadKeyed(k)
            | SlotFacts::StoreKeyed(k)
            | SlotFacts::DefineKeyedOwn(k)
            | SlotFacts::HasKeyed(k)
            | SlotFacts::StoreInArrayLiteral(k) => format!(
                "{} keys={}",
                self.property(&k.property),
                bits_text!(k.keys, KeySet)
            ),
            SlotFacts::LoadGlobal(g)
            | SlotFacts::LoadGlobalInsideTypeof(g)
            | SlotFacts::StoreGlobal(g) => format!(
                "bindings={} values={}",
                bits_text!(g.bindings, BindingSet),
                bits_text!(g.values, TypeSet)
            ),
            SlotFacts::BinaryOp(h) => bits_text!(h, BinaryHint),
            SlotFacts::CompareOp(h) => bits_text!(h, CompareHint),
            SlotFacts::TypeOf(t) => bits_text!(t, TypeSet),
            SlotFacts::InstanceOf(t) => self.targets(t),
            SlotFacts::Literal(l) => format!(
                "count={} shape={} elements={}",
                l.count,
                l.shape
                    .as_ref()
                    .map_or("none".to_owned(), |s| self.shape_ref(s)),
                l.elements.map_or("none", |k| k.name())
            ),
            SlotFacts::CloneObject(c) => {
                let cases: Vec<String> = c
                    .cases
                    .iter()
                    .map(|c| {
                        format!(
                            "{} -> {}",
                            self.shape_ref(&c.source),
                            self.shape_ref(&c.result)
                        )
                    })
                    .collect();
                format!("{} [{}]", c.state.name(), cases.join(", "))
            }
            SlotFacts::ForIn(h) => bits_text!(h, ForInHint),
            SlotFacts::JumpLoop(l) => format!("back-edges={}", l.back_edges),
            SlotFacts::CreateClosure(c) => format!("closures={}", c.closures.name()),
        }
    }
}

impl FactSnapshot {
    /// A stable text form: one line per shape (`s<i>`, in table order) and per record.
    pub fn dump(&self) -> String {
        let d = Dump {
            snap: self,
            shapes: self.shape_table(),
        };
        let h = self.header();
        let mut s = String::new();
        let _ = writeln!(
            s,
            "FactSnapshot schema {} bytecode-abi {} source {}",
            crate::FACT_SCHEMA_VERSION,
            h.bytecode_abi,
            h.source.name()
        );
        let _ = writeln!(s, "unit {}", hex(&h.unit_hash));
        let _ = writeln!(s, "shapes {}", d.shapes.len());
        for (i, shape) in d.shapes.iter().enumerate() {
            let _ = writeln!(s, "  s{i} {}", d.shape_desc(shape));
        }
        let _ = writeln!(s, "records {}", self.len());
        for (key, f) in self.records() {
            let _ = writeln!(
                s,
                "  f{} slot {} {}: {}",
                key.function,
                key.slot,
                f.kind().name(),
                d.payload(f)
            );
        }
        s
    }
}

/// One line per tag: `<tag> <name> <fields>`.
fn tags(out: &mut String, ty: &str, names: &[&str], fields: &[&str]) {
    assert_eq!(names.len(), fields.len(), "{ty}: a field text per tag");
    let _ = writeln!(out, "  {ty} = tag u8:");
    for (i, (n, f)) in names.iter().zip(fields).enumerate() {
        let _ = writeln!(out, "    {i} {n}{}{f}", if f.is_empty() { "" } else { " " });
    }
}

macro_rules! enum_lines {
    ($out:expr, $($ty:ident),*) => {$(
        let names: Vec<String> =
            $ty::ALL.iter().map(|v| format!("{} {}", *v as u8, v.name())).collect();
        let _ = writeln!($out, "  {} u8: {}", stringify!($ty), names.join(", "));
    )*};
}

macro_rules! bitset_lines {
    ($out:expr, $($ty:ident),*) => {$(
        let names: Vec<String> = $ty::BITS
            .iter()
            .enumerate()
            .map(|(i, (_, n))| format!("{i} {n}"))
            .collect();
        let width = core::mem::size_of_val(&$ty::EMPTY.bits()) * 8;
        let _ = writeln!($out, "  {} u{}: {}", stringify!($ty), width, names.join(", "));
    )*};
}

/// The whole schema as stable text. The schema history fingerprints it.
pub fn schema_text() -> String {
    let mut s = String::new();
    s.push_str("ep9 FactSnapshot schema (H16)\n");
    s.push_str(
        "encoding: little endian; fixed-width u8 u16 u32; a count is a u32; an optional shape \
         index or enum stores value + 1 (0 = none)\n",
    );
    let _ = writeln!(s, "header {HEADER_SIZE} bytes:");
    for line in [
        "0 8 magic 89 45 50 39 46 53 0d 0a",
        "8 4 schema_version",
        "12 4 bytecode_abi",
        "16 32 unit_hash",
        "48 1 source FactSource",
        "49 3 reserved",
        "52 4 shape_count",
        "56 4 record_count",
        "60 4 total_size",
    ] {
        let _ = writeln!(s, "  {line}");
    }
    s.push_str("body: shapes[shape_count] ShapeDesc, then records[record_count] Record\n");
    s.push_str("types:\n");
    for line in [
        "ShapeDesc = proto ProtoRef, kind u8 ObjectKind, flags u8 ShapeFlags, capacity u16, \
         key_count u32, keys[key_count] (KeyRef, attrs u8 PropAttrs)",
        "ShapeIndex = u32 index into shapes",
        "Record = function u32, slot u32, kind u8 FeedbackSlotKind, payload by kind",
        "CallFacts = count u32, callees TargetFacts, result u16 TypeSet",
        "TargetFacts = state u8 IcState, count u32, FunctionRef[count]",
        "PropertyFacts = state u8 IcState, count u32, (receiver ShapeIndex, Access)[count], \
         values u16 TypeSet",
        "KeyedFacts = PropertyFacts, keys u8 KeySet",
        "GlobalFacts = bindings u8 BindingSet, values u16 TypeSet",
        "LiteralFacts = count u32, shape u32 (ShapeIndex + 1), elements u8 (ElementsKind + 1)",
        "CloneFacts = state u8 IcState, count u32, (source ShapeIndex, result ShapeIndex)[count]",
        "LoopFacts = back_edges u32",
        "ClosureFacts = closures u8 ClosureCount",
        "BinaryHint = u8 bits; CompareHint = u16 bits; TypeSet = u16 bits; ForInHint = u8 bits",
    ] {
        let _ = writeln!(s, "  {line}");
    }
    tags(
        &mut s,
        "ProtoRef",
        ProtoRef::TAGS,
        &["", "index u32", "FunctionRef", ""],
    );
    tags(
        &mut s,
        "KeyRef",
        KeyRef::TAGS,
        &[
            "len u32, u8[len]",
            "len u32, u16[len]",
            "u8 WellKnownSymbol",
            "",
        ],
    );
    tags(
        &mut s,
        "FunctionRef",
        FunctionRef::TAGS,
        &["unit_hash u8[32], function u32", "builtin_id u16"],
    );
    tags(
        &mut s,
        "Access",
        Access::TAGS,
        &[
            "field u16",
            "depth u8, field u16",
            "depth u8",
            "depth u8, FunctionRef",
            "depth u8, FunctionRef",
            "to ShapeIndex, field u16",
            "",
            "",
            "kind u8 ElementsKind",
            "",
        ],
    );
    s.push_str("kinds (FeedbackSlotKind byte, name, payload):\n");
    for &k in FeedbackSlotKind::ALL {
        let _ = writeln!(s, "  {} {} {}", k as u8, k.name(), Payload::of(k).name());
    }
    s.push_str("enums:\n");
    enum_lines!(
        s,
        FactSource,
        IcState,
        ObjectKind,
        ElementsKind,
        WellKnownSymbol,
        ClosureCount
    );
    s.push_str("bitsets (reserved bits are 0):\n");
    bitset_lines!(
        s,
        TypeSet,
        BinaryHint,
        CompareHint,
        ForInHint,
        KeySet,
        BindingSet,
        PropAttrs,
        ShapeFlags
    );
    s.push_str("rules:\n");
    for line in [
        "value order: fields in declaration order, variants in tag order, strings Latin-1 \
         before UTF-16 then by code units"
            .to_owned(),
        "shapes strictly ascending in value order; every shape is used by a record".to_owned(),
        "records strictly ascending by (function, slot); nothing after the last record".to_owned(),
        format!(
            "cases and targets strictly ascending in value order, at most {MAX_CASES}; \
             uninitialized and megamorphic hold 0, monomorphic at most 1"
        ),
        "property cases have distinct receivers; clone cases have distinct sources".to_owned(),
        "own-field and add-field: field < key count of the shape".to_owned(),
        "a dictionary shape lists no keys; keys are distinct except other-symbol; a utf16 key \
         holds a code unit above 0xff"
            .to_owned(),
        "a js FunctionRef never names function 0".to_owned(),
        "key i is in-object slot i below capacity, else out-of-line slot i - capacity".to_owned(),
        "bitsets merge by OR".to_owned(),
        "validator: drop the snapshot if bytecode_abi or unit_hash differ from the unit; drop \
         a record if its function or slot does not exist, its kind is not the slot's kind, or \
         a js ref into this unit names no function"
            .to_owned(),
    ] {
        let _ = writeln!(s, "  {line}");
    }
    s
}
