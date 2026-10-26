//! The facts exporter (H16-Q5, the exporter half): the feedback vectors of one loaded unit as
//! an `ep9_facts::FactSnapshot` v0 (FeedbackVector -> FactSnapshot). Public as
//! `ep9_vm::abi::export_facts` (V8-A13); not part of the H20 core (FREEZE-PREP lists it in
//! the H20 part-2 table). The `.ep9prof` container that stores a snapshot is H16-Q5b (A3).
//!
//! ```text
//!  export_facts(isolate, unit hash)
//!    the first loaded unit whose content hash is `unit hash` (else Err)
//!    └─ each function with a FEEDBACK cell (feedback.rs), each slot that an IC site uses
//!         LoadNamed, StoreNamed, DefineNamedOwn (0x21, 0x22, 0x24, 0x25) ── PropertyFacts
//!         LoadGlobal, LoadGlobalInsideTypeof, StoreGlobal (0x10-0x12) ──── GlobalFacts
//!         other kinds: no record (T0 records all but InstanceOf since M4-P1, H16-Q2;
//!         InstanceOf from IC2, M4-P3; FACTS2 exports them, M4-P2)
//!  IC entry (ic.rs)   ── state: uninitialized | monomorphic | polymorphic | megamorphic
//!   case (key, handler)
//!     key: a live shape ──root walk──> ShapeDesc {proto, kind, flags, capacity, keys}
//!     handler: own field ─> OwnField     array length ─> ArrayLength    SLOW ─> Generic
//!              PROTO_LOAD ─> ProtoField  MISSING ─> Missing             ADD ─> AddField
//! ```
//!
//! Rules:
//! - The output holds no ShapeId, address, cell kind id or closure identity (the schema has
//!   no field for one). A shape becomes a [`ShapeDesc`] by its root walk: its keys in
//!   property order with their attributes, which are the first `count` entries of its
//!   DESCRIPTORS cell (the prefix rule of `shape.rs`: the same keys as the parent chain from
//!   the root shape, and a unique shape has them too). A prototype becomes a [`ProtoRef`]:
//!   `null`; an intrinsic of the current realm by its `IntrinsicId` (the H13 index list is
//!   H16-Q4, M1); the initial `prototype` of a bytecode class constructor `F`, named by
//!   `F`'s unit hash and function id; else `unknown`. So two runs of one program give the
//!   same bytes.
//! - `function-prototype` needs a proof that the VM keeps, not a backlink that JS can write
//!   (`C.prototype = {constructor: C}`, or an arrow function given a `prototype`). The object's
//!   own `constructor` only names the candidate `F`. The proof: `F` is a class constructor,
//!   its own `prototype` is the object with {w: false, c: false} (DefineClass makes it so, and
//!   no JS can change it), and its `[[HomeObject]]` is the object (DefineClass sets it; no JS
//!   can). The VM keeps no such proof for a `function` declaration's lazy `prototype`
//!   (`exotic::function_prototype`), so that object is `unknown`.
//! - A global case's binding comes from what made it (`ic::global_case`): a field handler is
//!   keyed on the lexical record (`lexical`), a GLOBAL cell on the global object
//!   (`data-property`). A `SLOW` case can be keyed on either. An accessor is on the global
//!   object (a lexical binding is never an accessor: `accessor-property`). So is a data
//!   property (a read-only global, at a store) under a shape whose prototype is not null (the
//!   lexical record's prototype is null for good: no JS can reach it). A null prototype does
//!   not tell them apart, because the global object can have one too: then the key must be
//!   the current shape of the lexical record or of the global object, and not of both; else
//!   the case gives no binding.
//! - A case that has no description is left out (the `ep9_facts` rule: a state may have
//!   fewer cases than the IC had): the string key ([`ic::KEY_STRING`]), a dead key, an object
//!   kind with no [`ObjectKind`] (generators, arguments, objects with internal slots, module
//!   namespaces), a prototype handler whose validity cell is invalid (`validity.rs`: it
//!   misses from now on, and its chain may have changed) or whose holder or target shape is
//!   dead, and
//!   two cases whose receivers have equal descriptions but different accesses.
//! - T0 records no value types at property and global sites (H16-Q2; the kinds it records
//!   since M4-P1 are not exported yet): `values` is empty; global `bindings` come from cases.
//! - The exporter reads the heap and allocates nothing, so it is not a GC point. It takes
//!   `&Isolate`, so no JS runs while it reads. It keeps no table keyed by a heap-cell Value
//!   (H12-Q2 (a)): each shape and prototype is described again where a case names it. A
//!   finished [`ShapeDesc`] is interned by its content (Codex C2-FX1): equal descriptions
//!   share one `Arc`, so the memory kept grows with the distinct descriptions, not with the
//!   sites that name them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ep9_bytecode::{ABI_VERSION, FeedbackSlotKind, Instructions, Opcode, OperandKind, UnitString};
use ep9_facts::{
    Access, BindingSet, FactHeader, FactSnapshot, FactSource, FunctionRef, GlobalFacts, IcState,
    KeyRef, ObjectKind, PropAttrs, PropertyCase, PropertyFacts, ProtoRef, ShapeDesc, ShapeFlags,
    SlotFacts, TypeSet, UnitHash,
};

use crate::Isolate;
use crate::atoms::Atom;
use crate::builtins::{self, IntrinsicId};
use crate::feedback;
use crate::frame::{HOLE, NULL, UNDEFINED, int32_of};
use crate::ic::{self, State, hk, smi};
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, attr, cell, obj, rd, rd_i32};
use crate::props;
use crate::runtime::{Root, Runtime, val};
use crate::shape::{self, slf};
use crate::string::{self, JsStr};
use crate::symbol::{self, WellKnownSymbol};
use crate::unit::{FunctionInfo, LoadedUnit};
use crate::validity;

/// The fields of an IC_HANDLER cell (the `ic.rs` layout table: +8 kind, +16 the validity cell
/// (or undefined for a 0x25 add), +24 holder or target shape (weak), +32 location).
mod hc {
    pub const KIND: usize = 8;
    pub const CELL: usize = 16;
    pub const REF: usize = 24;
    pub const LOCATION: usize = 32;
}

/// The case keys of a POLY_IC cell (the `ic.rs` layout table: 4 cases from +8, each a key
/// (weak) and a handler (strong)). The exporter reads each handler with [`ic::find`].
const POLY_KEY: usize = 8;
const POLY_CASE: usize = 16;

/// The facts of the loaded unit whose content hash is `unit_hash`, from its feedback vectors
/// (H16-Q5). The snapshot's header names this build's bytecode ABI and the source `runtime`.
/// One record per feedback slot that an IC site of T0 uses (the named-property and global
/// sites), in each function that has a feedback vector; a function that has none (it ran
/// fewer times than `IsolateConfig::feedback_threshold`) gives no record. If the isolate
/// loaded two units with this hash (one source run twice), the first one loaded is read.
///
/// The bytes of `snapshot.encode()` depend only on the program and what it did: no ShapeId,
/// address or closure identity is in them. Not a GC point; runs no JS.
///
/// ```
/// use ep9_vm::{Isolate, IsolateConfig, abi::export_facts};
/// let iso = Isolate::new(IsolateConfig::default()).unwrap();
/// // No unit with this hash is loaded.
/// assert!(export_facts(&iso, &[0; 32]).is_err());
/// ```
///
/// # Errors
///
/// "no loaded unit has this content hash" if the isolate has no such unit.
pub fn export_facts(
    isolate: &Isolate,
    unit_hash: &ep9_facts::UnitHash,
) -> Result<ep9_facts::FactSnapshot, &'static str> {
    let rt: &Runtime = &isolate.rt;
    let mut ex = Exporter::new(rt);
    let units = rt.units.borrow();
    let mut target = None;
    for u in units.iter() {
        if ex.unit_hash(u) == *unit_hash {
            target = Some(&**u);
            break;
        }
    }
    let unit = target.ok_or("no loaded unit has this content hash")?;
    let mut snapshot = FactSnapshot::new(FactHeader {
        bytecode_abi: ABI_VERSION,
        unit_hash: *unit_hash,
        source: FactSource::Runtime,
    });
    for (f, info) in unit.infos.iter().enumerate() {
        let Some(v) = feedback::vector(info) else {
            continue;
        };
        let code = &unit.unit.functions[f].bytecode;
        let kinds = &unit.unit.functions[f].feedback_slots;
        let sites = site_names(info, code, kinds.len());
        debug_assert_eq!(feedback::slot_count(v) as usize, kinds.len());
        for (i, (&k, site)) in kinds.iter().zip(&sites).enumerate() {
            let Some(name) = *site else {
                continue;
            };
            let i = i as u32;
            let facts = match k {
                FeedbackSlotKind::LoadNamed => SlotFacts::LoadNamed(ex.property(v, i, true)),
                FeedbackSlotKind::StoreNamed => SlotFacts::StoreNamed(ex.property(v, i, true)),
                // 0x25 add cases keep no validity cell (ic.rs).
                FeedbackSlotKind::DefineNamedOwn => {
                    SlotFacts::DefineNamedOwn(ex.property(v, i, false))
                }
                FeedbackSlotKind::LoadGlobal => SlotFacts::LoadGlobal(ex.global(v, i, name)),
                FeedbackSlotKind::LoadGlobalInsideTypeof => {
                    SlotFacts::LoadGlobalInsideTypeof(ex.global(v, i, name))
                }
                FeedbackSlotKind::StoreGlobal => SlotFacts::StoreGlobal(ex.global(v, i, name)),
                // Not exported yet (FACTS2, M4-P2), though T0 records every other kind but
                // InstanceOf (H16-Q2): no record, not an empty one.
                _ => continue,
            };
            let r = snapshot.insert(f as u32, i, facts);
            // The cases are distinct and checked above: a refusal is an exporter bug.
            debug_assert!(r.is_ok(), "facts export: record ({f}, {i}) refused: {r:?}");
        }
    }
    Ok(snapshot)
}

/// What one feedback slot of a function serves: `None` if no IC site of T0 uses it (a slot of
/// another kind, or the load slot of 0x8f `GetIterator`, which T0 keeps empty); else the
/// site's name constant (HOLE if two sites with different names share it: `ic::init_vector`
/// made it megamorphic).
fn site_names(fi: &FunctionInfo, code: &[u8], n: usize) -> Vec<Option<u64>> {
    let mut names = vec![None; n];
    for insn in Instructions::new(code) {
        let Ok(insn) = insn else {
            debug_assert!(false, "verified code decodes");
            break;
        };
        if !matches!(
            insn.opcode,
            Opcode::GetNamedProperty
                | Opcode::GetNamedPropertyFromSuper
                | Opcode::SetNamedProperty
                | Opcode::DefineNamedOwnProperty
                | Opcode::LdaGlobal
                | Opcode::LdaGlobalInsideTypeof
                | Opcode::StaGlobal
        ) {
            continue;
        }
        let operand = |want| (0..insn.operand_count()).find(|&j| insn.operand_kind(j) == want);
        let (Some(s), Some(k)) = (operand(OperandKind::Slot), operand(OperandKind::Const)) else {
            continue;
        };
        let Some(entry) = names.get_mut(insn.unsigned(s) as usize) else {
            continue;
        };
        let name = fi.constant(insn.unsigned(k));
        *entry = match *entry {
            None => Some(name),
            Some(seen) if seen == name => Some(name),
            Some(_) => Some(HOLE),
        };
    }
    names
}

/// One export: the runtime, the content hashes of the units made so far, by unit index, and
/// the shape descriptions made so far, by their content (no heap address is a key, H12-Q2).
struct Exporter<'a> {
    rt: &'a Runtime,
    hashes: HashMap<u32, UnitHash>,
    descs: HashSet<Arc<ShapeDesc>>,
}

impl<'a> Exporter<'a> {
    fn new(rt: &'a Runtime) -> Exporter<'a> {
        Exporter {
            rt,
            hashes: HashMap::new(),
            descs: HashSet::new(),
        }
    }

    /// The content hash of a loaded unit (one serialization per unit and export).
    fn unit_hash(&mut self, u: &LoadedUnit) -> UnitHash {
        *self
            .hashes
            .entry(u.index)
            .or_insert_with(|| u.unit.content_hash())
    }

    /// The case keys of the IC entry in slot `i` of the vector `v`, with its state.
    fn entry(&self, v: u64, i: u32) -> (IcState, Vec<u64>) {
        match ic::state(v, i) {
            State::Uninitialized => (IcState::Uninitialized, Vec::new()),
            State::Megamorphic => (IcState::Megamorphic, Vec::new()),
            State::Monomorphic => (IcState::Monomorphic, vec![feedback::key(v, i)]),
            State::Polymorphic(_) => {
                let p = cell(feedback::value(v, i));
                let keys = (0..ic::POLY_CASES)
                    .map(|c| rd(p, POLY_KEY + POLY_CASE * c))
                    .filter(|&k| k != UNDEFINED)
                    .collect();
                (IcState::Polymorphic, keys)
            }
        }
    }

    /// The facts of a named-property slot. `check_chain`: an add case checks its validity cell
    /// (0x24, not 0x25).
    fn property(&mut self, v: u64, i: u32, check_chain: bool) -> PropertyFacts {
        let (state, keys) = self.entry(v, i);
        let mut cases: Vec<PropertyCase> = Vec::new();
        for k in keys {
            let Some(h) = ic::find(v, i, k) else {
                debug_assert!(false, "a case key with no handler");
                continue;
            };
            if let Some(c) = self.case(k, h, check_chain) {
                cases.push(c);
            }
        }
        // Two live shapes may have one description (for example under an `unknown`
        // prototype). Equal cases merge; a receiver with two different accesses is left out.
        cases.sort();
        cases.dedup();
        let mut keep = vec![true; cases.len()];
        for j in 1..cases.len() {
            if cases[j - 1].receiver == cases[j].receiver {
                keep[j - 1] = false;
                keep[j] = false;
            }
        }
        let mut it = keep.into_iter();
        cases.retain(|_| it.next().unwrap_or(false));
        PropertyFacts {
            state,
            cases,
            values: TypeSet::EMPTY,
        }
    }

    /// The case for key `k` (a shape) and handler `h`, if both have a description.
    fn case(&mut self, k: u64, h: u64, check_chain: bool) -> Option<PropertyCase> {
        if k == ic::KEY_STRING || !is_cell_bits(k) || !shape::is_shape(k) {
            return None;
        }
        let receiver = self.shape_desc(k)?;
        let access = if val(h).is_int32() {
            let x = int32_of(h);
            match x & smi::KIND_MASK {
                smi::FIELD_IN_OBJECT | smi::FIELD_OUT_OF_LINE => Access::OwnField {
                    field: key_index(k, x)?,
                },
                smi::ARRAY_LENGTH => Access::ArrayLength,
                smi::SLOW => Access::Generic,
                // STRING_LENGTH comes only with KEY_STRING; MEGA is not a case handler.
                _ => return None,
            }
        } else {
            let c = cell(h);
            // The stale rule (H16 K5): a prototype case holds only while its cell is valid.
            let current = validity::is_valid(rd(c, hc::CELL));
            let r = rd(c, hc::REF);
            let location = rd_i32(c, hc::LOCATION);
            match rd_i32(c, hc::KIND) {
                hk::PROTO_LOAD if current && r != UNDEFINED => Access::ProtoField {
                    depth: chain_depth(shape::proto(k), Some(r))?,
                    field: key_index(shape::shape_of(r), location)?,
                },
                hk::MISSING if current => Access::Missing {
                    depth: chain_depth(shape::proto(k), None)?,
                },
                hk::ADD if (current || !check_chain) && r != UNDEFINED => Access::AddField {
                    to: self.shape_desc(r)?,
                    field: key_index(r, location)?,
                },
                _ => return None,
            }
        };
        Some(PropertyCase { receiver, access })
    }

    /// The facts of a global slot: the bindings its cases found. `name` is the site's name
    /// constant (HOLE for a shared slot, which is megamorphic and has no case).
    fn global(&mut self, v: u64, i: u32, name: u64) -> GlobalFacts {
        let (_, keys) = self.entry(v, i);
        let mut bindings = BindingSet::EMPTY;
        for k in keys {
            let Some(h) = ic::find(v, i, k) else {
                continue;
            };
            if !is_cell_bits(k) || !shape::is_shape(k) {
                continue;
            }
            if !val(h).is_int32() {
                if rd_i32(cell(h), hc::KIND) == hk::GLOBAL {
                    bindings |= BindingSet::DATA;
                }
                continue;
            }
            let x = int32_of(h);
            match x & smi::KIND_MASK {
                // A field case is keyed on the lexical record's shape (`ic::global_case`).
                smi::FIELD_IN_OBJECT | smi::FIELD_OUT_OF_LINE => bindings |= BindingSet::LEXICAL,
                // `SLOW`: a dictionary-mode record (the name may be anywhere: nothing known),
                // a `const` (store), or an accessor or read-only property of the global
                // object.
                smi::SLOW if name != HOLE && !shape::is_dictionary(k) => {
                    bindings |= self.slow_global(k, name);
                }
                _ => {}
            }
        }
        GlobalFacts {
            bindings,
            values: TypeSet::EMPTY,
        }
    }

    /// The binding of a `SLOW` global case for `name` keyed on the fast shape `k`: the lexical
    /// record's or the global object's shape when the case was made (`ic::global_case`). An
    /// accessor is on the global object (a lexical binding is a data property: `realm.rs`
    /// defines them {} or {w}). A data property (a store to a `const` or to a read-only
    /// global) is on the global object if `k`'s prototype is not null (the lexical record is
    /// made with a null prototype and no JS can reach it to change that). Under a null
    /// prototype it is lexical only if `k` is the current shape of the lexical record and not
    /// of the global object, and the reverse; else (an old shape, or both) nothing is known:
    /// the global object can have a null prototype too (Codex C-FX1).
    fn slow_global(&self, k: u64, name: u64) -> BindingSet {
        let Some(f) = shape::lookup(self.rt, k, name) else {
            return BindingSet::EMPTY;
        };
        if f.details.attrs() & attr::ACCESSOR != 0 {
            return BindingSet::ACCESSOR;
        }
        if shape::proto(k) != NULL {
            return BindingSet::DATA;
        }
        let lexical = shape::shape_of(self.rt.root(Root::Lexical)) == k;
        let global = shape::shape_of(self.rt.global()) == k;
        match (lexical, global) {
            (true, false) => BindingSet::LEXICAL,
            (false, true) => BindingSet::DATA,
            _ => BindingSet::EMPTY,
        }
    }

    /// The description of the live shape `s`, or `None` if its object kind has no
    /// [`ObjectKind`]. An equal description made before is given again (interned by content,
    /// Codex C2-FX1): many sites that name one shape keep one copy of its keys.
    fn shape_desc(&mut self, s: u64) -> Option<Arc<ShapeDesc>> {
        let d = self.describe(s)?;
        if let Some(a) = self.descs.get(&d) {
            return Some(Arc::clone(a));
        }
        let a = Arc::new(d);
        self.descs.insert(Arc::clone(&a));
        Some(a)
    }

    fn describe(&mut self, s: u64) -> Option<ShapeDesc> {
        let k = shape::object_kind(s);
        let kind = [
            (kind::OBJECT, ObjectKind::Ordinary),
            (kind::ARRAY, ObjectKind::Array),
            (kind::FUNCTION, ObjectKind::Function),
            (kind::ERROR, ObjectKind::Error),
            (kind::ITERATOR, ObjectKind::Iterator),
            (kind::PRIMITIVE_WRAPPER, ObjectKind::PrimitiveWrapper),
            (kind::BOUND_FUNCTION, ObjectKind::BoundFunction),
        ]
        .into_iter()
        .find(|(id, _)| id.get() == k)
        .map(|(_, o)| o)?;
        let fl = shape::flags(s);
        let mut flags = ShapeFlags::EMPTY;
        if fl & shape::flag::EXTENSIBLE != 0 {
            flags |= ShapeFlags::EXTENSIBLE;
        }
        let dictionary = fl & shape::flag::DICTIONARY != 0;
        if dictionary {
            flags |= ShapeFlags::DICTIONARY;
        }
        let keys = if dictionary {
            Vec::new()
        } else {
            (0..shape::count(s))
                .map(|i| {
                    let a = shape::details_at(s, i).attrs();
                    (self.key_ref(shape::key_at(s, i)), prop_attrs(a))
                })
                .collect()
        };
        Some(ShapeDesc {
            proto: self.proto_ref(shape::proto(s)),
            kind,
            flags,
            capacity: u16::from(shape::capacity(s)),
            keys,
        })
    }

    /// A shape key (an atom or a symbol) by content.
    fn key_ref(&self, k: u64) -> KeyRef {
        if kind::is_string(k) {
            return KeyRef::String(string::with_text(k, |t| match t {
                JsStr::Latin1(b) => UnitString::from_latin1(b),
                JsStr::Utf16(u) => UnitString::from_utf16(u),
            }));
        }
        WellKnownSymbol::ALL
            .into_iter()
            .find(|&w| symbol::well_known(self.rt, w) == k)
            .and_then(|w| ep9_facts::WellKnownSymbol::from_u8(w as u8))
            .map_or(KeyRef::OtherSymbol, KeyRef::WellKnownSymbol)
    }

    /// The prototype `p` (an object or null) by a position-independent name.
    fn proto_ref(&mut self, p: u64) -> ProtoRef {
        if p == NULL {
            return ProtoRef::Null;
        }
        IntrinsicId::ALL
            .into_iter()
            .find(|&id| builtins::intrinsic(self.rt, id) == p)
            .map(|id| ProtoRef::Intrinsic(id as u32))
            .or_else(|| self.function_prototype(p).map(ProtoRef::FunctionPrototype))
            .unwrap_or(ProtoRef::Unknown)
    }

    /// The bytecode class constructor `F` whose initial prototype `p` is, with a proof that the
    /// VM keeps (Codex C-FX2). `p`'s own data property `constructor` names the candidate `F`
    /// (JS can write it, so it proves nothing). The proof: `F` is a class constructor, its own
    /// `prototype` is `p` with {w: false, c: false}, and its `[[HomeObject]]` is `p`
    /// (DefineClass, `interp/slow/runtime.rs`, makes both, and no JS can change them). A
    /// `function` declaration's `prototype` is writable and the VM keeps no proof of the
    /// initial one, so it gives `None`.
    fn function_prototype(&mut self, p: u64) -> Option<FunctionRef> {
        let (f, at) = props::own_get(p, self.rt.atom(Atom::Constructor))?;
        if at & attr::ACCESSOR != 0 || !kind::is_function(f) {
            return None;
        }
        let fi = object::function_info(f);
        if !fi.is_class_constructor() || object::home_object(f) != p {
            return None;
        }
        let (proto, at) = props::own_get(f, self.rt.atom(Atom::Prototype))?;
        if proto != p || at & (attr::ACCESSOR | attr::WRITABLE | attr::CONFIGURABLE) != 0 {
            return None;
        }
        let unit = fi.loaded_unit()?;
        // Function 0 (a unit's entry) has no closure (H02).
        if fi.function_id == 0 {
            return None;
        }
        Some(FunctionRef::Js {
            unit: self.unit_hash(unit),
            function: fi.function_id,
        })
    }
}

/// The schema's attribute bits for VM attributes (`object::attr`, `shape::ACCESSOR`).
fn prop_attrs(a: i32) -> PropAttrs {
    let mut r = PropAttrs::EMPTY;
    for (bit, p) in [
        (attr::WRITABLE, PropAttrs::WRITABLE),
        (attr::ENUMERABLE, PropAttrs::ENUMERABLE),
        (attr::CONFIGURABLE, PropAttrs::CONFIGURABLE),
        (attr::ACCESSOR, PropAttrs::ACCESSOR),
    ] {
        if a & bit != 0 {
            r |= p;
        }
    }
    r
}

/// The index into the keys of the fast shape `s` of the property that the smi field handler
/// `loc` names (`ic::field_location`: in-object at `obj::FIXED + 8 (fixed + f)`, out of line
/// at `slf::DATA + 8 (f - capacity)`).
fn key_index(s: u64, loc: i32) -> Option<u16> {
    if shape::is_dictionary(s) {
        return None;
    }
    let off = (loc >> smi::SHIFT) as usize;
    let (cap, k) = shape::field_layout(s);
    let field = match loc & smi::KIND_MASK {
        smi::FIELD_IN_OBJECT => {
            let words = off.checked_sub(obj::FIXED)?;
            if words % 8 != 0 {
                return None;
            }
            (words / 8).checked_sub(object::fixed_count(k))?
        }
        smi::FIELD_OUT_OF_LINE => {
            let words = off.checked_sub(slf::DATA)?;
            if words % 8 != 0 {
                return None;
            }
            cap + words / 8
        }
        _ => return None,
    };
    (0..shape::count(s))
        .find(|&i| usize::from(shape::details_at(s, i).field()) == field)
        .and_then(|i| u16::try_from(i).ok())
}

/// The prototype-chain steps from the prototype `p` of a receiver (1 = `p` itself) to
/// `holder`, or, with `None`, the number of prototypes up to null. `None` if the chain does
/// not reach the holder or is longer than 255.
fn chain_depth(mut p: u64, holder: Option<u64>) -> Option<u8> {
    let mut depth: u8 = 0;
    while p != NULL {
        depth = depth.checked_add(1)?;
        if Some(p) == holder {
            return Some(depth);
        }
        p = object::proto_of(p);
    }
    match holder {
        None => Some(depth),
        Some(_) => None,
    }
}
