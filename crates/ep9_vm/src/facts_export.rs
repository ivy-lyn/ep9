//! The facts exporter (H16-Q5, the exporter half): the feedback vectors of one loaded unit as
//! an `ep9_facts::FactSnapshot` v0 (FeedbackVector -> FactSnapshot). Public as
//! `ep9_vm::abi::export_facts` (V8-A13); not part of the H20 core (FREEZE-PREP lists it in
//! the H20 part-2 table). The `.ep9prof` container that stores a snapshot is H16-Q5b (A3).
//!
//! ```text
//!  export_facts(isolate, unit hash)
//!    the first loaded unit whose content hash is `unit hash` (else Err)
//!    └─ each function with a FEEDBACK cell (feedback.rs), each slot that a T0 site writes
//!         LoadNamed, StoreNamed, DefineNamedOwn (0x21, 0x22, 0x24, 0x25) ── PropertyFacts
//!         LoadGlobal, LoadGlobalInsideTypeof, StoreGlobal (0x10-0x12) ──── GlobalFacts
//!         Call (Call* 0x5e-0x67 | Construct* 0x69, 0x6a; branch on the opcode) ── CallFacts
//!         keyed kinds (2, 4, 6, 10, 17) ─ KeyedFacts    BinaryOp ─ BinaryHint
//!         CompareOp ─ CompareHint    TypeOf ─ TypeSet    InstanceOf ─ TargetFacts
//!         Literal ─ LiteralFacts     CloneObject ─ CloneFacts    ForIn ─ ForInHint
//!         JumpLoop ─ LoopFacts       CreateClosure ─ ClosureFacts   (M4-P2 FACTS2)
//!  IC entry (ic.rs)   ── state: uninitialized | monomorphic | polymorphic | megamorphic
//!   case (key, handler)
//!     key: a live shape ──root walk──> ShapeDesc {proto, kind, flags, capacity, keys}
//!     handler: own field ─> OwnField     array length ─> ArrayLength    SLOW ─> Generic
//!              PROTO_LOAD ─> ProtoField  MISSING ─> Missing             ADD ─> AddField
//!  call target (a FunctionInfo word, or a FUNCTION cell) ──address──> FunctionRef
//!     a unit function ─> Js {unit hash, id}   a table builtin ─> Builtin(id)   else: none
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
//! - T0 records no value types at property and global sites, and no call result types
//!   (H16-Q2): `values` and `CallFacts::result` are empty; global `bindings` come from cases.
//! - **Which slots get a record (M4-P2 FACTS2).** A slot that a T0 site writes: the named and
//!   global sites (as before), every `Call*` and `Construct*` site, and every site of the
//!   other kinds except four that T0 leaves empty: the two slots of 0x8f `GetIterator`, the
//!   Call slot of 0x6b `TailCall` (reserved), the Literal slot of `GetTemplateObject` and the
//!   Literal slot of `CreateRegExpLiteral` (`regexp::create_literal` writes no slot).
//!   InstanceOf (14) gets a record only once its slot left the start state (undefined, int32
//!   0): `instance_ic.rs` (M4-P3 IC2) records a run only when it gives a result (a rhs that
//!   throws records nothing), so a slot in its start state has no rhs to name.
//! - **Implicit bits (H16-Q2 (1); MODES-D10, F-R9).** T0 writes no bit for an int32 pair with
//!   an int32 result at a BinaryOp site (also an int32 operand of a unary site), an int32 pair
//!   at a CompareOp site, and a present int32 index of an Array at `GetKeyedProperty`. The
//!   reader adds them to every slot of the kind in a function that has a vector: every
//!   BinaryOp record has `BinaryHint::INT32`, every CompareOp record `CompareHint::INT32`,
//!   every LoadKeyed record `KeySet::INDEX`. So a record whose only bit is that one is an
//!   empty slot, and it means **"int32 or never ran"**: T0 cannot tell a cold site from an
//!   int32 site (a slot starts as int32 0, `feedback.rs`). AOT treats it so, and the H32 site
//!   budget bounds the cost when the site later sees other values. A bit next to it is a
//!   prediction too (T0 may also have seen int32 there): a consumer guards (H16
//!   Consequences).
//! - **Count units (F-R7, MC-w13-2).** Each count names its unit, and a merge adds counts of
//!   one unit only (H16-Q5b):
//!
//!   | Field | Unit | Writer |
//!   |---|---|---|
//!   | `CallFacts::count`, `Call*` site | **sampled calls**: a call in the first `CALL_WINDOW` (64) budget units after a budget reset | `profile.rs` (`record_call`) |
//!   | `CallFacts::count`, `Construct*` site | **constructs of the cold path** (a hit writes nothing) | `construct_ic.rs` (`miss`) |
//!   | `CallFacts::count`, a slot both share | none: 0 (the slot mixes the two units) | - |
//!   | `LoopFacts::back_edges` | **sampled back edges**: a back edge that polls (it used up the budget) | `profile.rs` (`record_jump_loop`) |
//!   | `LiteralFacts::count` | **cold-path literal results**: a lower bound of the allocations | `literal_site.rs` (`miss`) |
//!
//!   All saturate at `i32::MAX`. A sampled count is a share of the budget units, not of the
//!   executions: the windows come at a fixed period (`BUDGET`), so a call pattern with that
//!   period can keep a site out of every window (`tests::facts_export`, MC-w13-17; the
//!   jitter of the window start is month 5).
//! - **Call keys are never written out (F-R8, MC-w13-8).** A `Call*` site keys the native word
//!   of the callee's FunctionInfo (`profile.rs`), a `Construct*` site the constructor cell
//!   (weak, `construct_ic.rs`). The exporter branches on the site opcode and maps the
//!   FunctionInfo by its address: inside a loaded unit's dense info array, it is
//!   `FunctionRef::Js {unit hash, id}` (a unit function but function 0); in the table of this
//!   export's builtin infos, `FunctionRef::Builtin(id)` (a table builtin of the current realm,
//!   `builtins::existing_function`). Anything else (a host function, a bound function, a
//!   proxy, a word in a slot that both kinds share) makes the site megamorphic with no target.
//!   The word is never read as a pointer.
//! - The exporter reads the heap and allocates nothing, so it is not a GC point. It takes
//!   `&Isolate`, so no JS runs while it reads. It keeps no table keyed by a heap-cell Value
//!   (H12-Q2 (a)): each shape and prototype is described again where a case names it (the
//!   builtin table is keyed by FunctionInfo addresses: Rust memory that lives as long as the
//!   isolate and never moves). A finished [`ShapeDesc`] is interned by its content
//!   (Codex C2-FX1): equal descriptions share one `Arc`, so the memory kept grows with the
//!   distinct descriptions, not with the sites that name them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ep9_bytecode::{ABI_VERSION, FeedbackSlotKind, Instructions, Opcode, OperandKind, UnitString};
use ep9_facts::{
    Access, BinaryHint, BindingSet, CallFacts, CloneCase, CloneFacts, ClosureCount, ClosureFacts,
    CompareHint, FactHeader, FactSnapshot, FactSource, ForInHint, FunctionRef, GlobalFacts,
    IcState, KeyRef, KeySet, KeyedFacts, LiteralFacts, LoopFacts, ObjectKind, PropAttrs,
    PropertyCase, PropertyFacts, ProtoRef, ShapeDesc, ShapeFlags, SlotFacts, TargetFacts, TypeSet,
    UnitHash,
};
use ep9_value::encoding::DOUBLE_ENCODE_OFFSET;

use crate::Isolate;
use crate::atoms::Atom;
use crate::builtins::{self, IntrinsicId};
use crate::feedback;
use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of};
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
/// (or undefined for a 0x25 add; an int32 count for CONSTRUCT_SITE and CLONE), +24 holder,
/// target or result shape (weak), +32 location).
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
/// One record per feedback slot that a T0 site writes (the module docs list them), in each
/// function that has a feedback vector; a function that has none (it ran fewer times than
/// `IsolateConfig::feedback_threshold` and polled at no loop) gives no record. If the
/// isolate loaded two units with this hash (one source run twice), the first one loaded is
/// read.
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
        let uses = slot_uses(info, code, kinds.len());
        debug_assert_eq!(feedback::slot_count(v) as usize, kinds.len());
        for (i, (&k, u)) in kinds.iter().zip(&uses).enumerate() {
            let i = i as u32;
            let Some(facts) = ex.slot(&units, v, i, k, u) else {
                continue;
            };
            let r = snapshot.insert(f as u32, i, facts);
            // The cases are distinct and checked above: a refusal is an exporter bug.
            debug_assert!(r.is_ok(), "facts export: record ({f}, {i}) refused: {r:?}");
        }
    }
    Ok(snapshot)
}

/// What the instructions of one function do with one of its feedback slots ([`slot_uses`]).
#[derive(Clone, Copy, Debug, Default)]
struct SlotUse {
    /// The name constant of the named-property and global sites that use the slot (HOLE if
    /// two sites with different names share it: `ic::init_vector` made it megamorphic).
    name: Option<u64>,
    /// A `Call*` site (0x5e-0x67) uses it: T0 keeps a FunctionInfo word and sampled calls.
    call: bool,
    /// A `Construct*` site (0x69, 0x6a) uses it: the constructor (weak) and the constructs of
    /// the cold path.
    construct: bool,
    /// A site of another kind whose slot T0 writes uses it.
    other: bool,
}

/// What each feedback slot of a function serves, from its code. No IC site of T0 uses the two
/// slots of 0x8f `GetIterator` (it keeps them empty), the Call slot of 0x6b `TailCall`
/// (reserved: `not_impl`), the Literal slot of `GetTemplateObject` (`literal_site::miss`
/// records no template) or the Literal slot of `CreateRegExpLiteral` (`regexp::create_literal`
/// ignores it), so they give no use.
fn slot_uses(fi: &FunctionInfo, code: &[u8], n: usize) -> Vec<SlotUse> {
    let mut uses = vec![SlotUse::default(); n];
    for insn in Instructions::new(code) {
        let Ok(insn) = insn else {
            debug_assert!(false, "verified code decodes");
            break;
        };
        let operand = |want| (0..insn.operand_count()).find(|&j| insn.operand_kind(j) == want);
        let Some(s) = operand(OperandKind::Slot) else {
            continue;
        };
        let Some(u) = uses.get_mut(insn.unsigned(s) as usize) else {
            continue;
        };
        match insn.opcode {
            Opcode::GetNamedProperty
            | Opcode::GetNamedPropertyFromSuper
            | Opcode::SetNamedProperty
            | Opcode::DefineNamedOwnProperty
            | Opcode::LdaGlobal
            | Opcode::LdaGlobalInsideTypeof
            | Opcode::StaGlobal => {
                let Some(k) = operand(OperandKind::Const) else {
                    continue;
                };
                let name = fi.constant(insn.unsigned(k));
                u.name = match u.name {
                    None => Some(name),
                    Some(seen) if seen == name => Some(name),
                    Some(_) => Some(HOLE),
                };
            }
            Opcode::CallAnyReceiver
            | Opcode::CallProperty
            | Opcode::CallProperty0
            | Opcode::CallProperty1
            | Opcode::CallProperty2
            | Opcode::CallUndefinedReceiver
            | Opcode::CallUndefinedReceiver0
            | Opcode::CallUndefinedReceiver1
            | Opcode::CallUndefinedReceiver2
            | Opcode::CallWithSpread => u.call = true,
            Opcode::Construct | Opcode::ConstructWithSpread => u.construct = true,
            Opcode::GetIterator
            | Opcode::TailCall
            | Opcode::GetTemplateObject
            | Opcode::CreateRegExpLiteral => {}
            _ => u.other = true,
        }
    }
    uses
}

/// The int32 value half of slot `i` of `v` as bits, or `None` (a VM bug: the slot kinds that
/// `profile.rs` writes hold an int32, H16-Q2).
fn int_value(v: u64, i: u32) -> Option<u32> {
    let x = feedback::value(v, i);
    if val(x).is_int32() {
        Some(int32_of(x) as u32)
    } else {
        debug_assert!(false, "slot {i} holds no int32: {x:#x}");
        None
    }
}

/// A keyed slot (2, 4, 6, 10, 17; `profile.rs` writes the value half, KEYED (M4-P3) the key
/// half): the `KeySet` bits 0-3 of the value, with `implicit` added (H16-Q2 (1)). The state
/// is the key half's: undefined (no keyed IC case yet: T0 v0 has none, KEYED), a receiver
/// shape (monomorphic, no case: v0 has no element access to describe) or the hole
/// (megamorphic). The receiver bits 4-7 and the out-of-bounds and hole bits 8-9 have no field
/// in schema 1 (MC-w13-9, the month-6 bump).
fn keyed(v: u64, i: u32, implicit: KeySet) -> Option<KeyedFacts> {
    let bits = int_value(v, i)?;
    let state = match feedback::key(v, i) {
        UNDEFINED => IcState::Uninitialized,
        HOLE => IcState::Megamorphic,
        _ => IcState::Monomorphic,
    };
    Some(KeyedFacts {
        property: PropertyFacts {
            state,
            cases: Vec::new(),
            values: TypeSet::EMPTY,
        },
        keys: known(KeySet::from_bits((bits & KeySet::ALL.bits() as u32) as u8))?.union(implicit),
    })
}

/// `bits` as a set of a schema bitset, or `None` if a bit is not in it (a VM bug: `profile.rs`
/// writes the `ep9_facts` bit numbers).
fn known<T>(bits: Option<T>) -> Option<T> {
    debug_assert!(
        bits.is_some(),
        "a recorded bit that the schema does not have"
    );
    bits
}

/// The address of the FunctionInfo that the native word `w` names, if `w` is a native word
/// (`frame::native`: an address below 2^49 plus the double offset). Not a pointer read.
fn native_address(w: u64) -> Option<usize> {
    let addr = w.checked_sub(DOUBLE_ENCODE_OFFSET)?;
    (addr < 1 << 49).then_some(addr as usize)
}

/// One export: the runtime, the content hashes of the units made so far, by unit index, the
/// shape descriptions made so far, by their content (no heap address is a key, H12-Q2), and
/// the builtin FunctionInfo table, made at the first call target that is no unit function
/// ([`Exporter::builtin_infos`]).
struct Exporter<'a> {
    rt: &'a Runtime,
    hashes: HashMap<u32, UnitHash>,
    descs: HashSet<Arc<ShapeDesc>>,
    targets: Option<HashMap<usize, FunctionRef>>,
}

impl<'a> Exporter<'a> {
    fn new(rt: &'a Runtime) -> Exporter<'a> {
        Exporter {
            rt,
            hashes: HashMap::new(),
            descs: HashSet::new(),
            targets: None,
        }
    }

    /// The record of slot `i` (kind `k`, used as `u`) of the vector `v`, or `None` if no T0
    /// site writes it (the module docs).
    fn slot(
        &mut self,
        units: &[Box<LoadedUnit>],
        v: u64,
        i: u32,
        k: FeedbackSlotKind,
        u: &SlotUse,
    ) -> Option<SlotFacts> {
        use FeedbackSlotKind as K;
        let facts = match (k, u.name) {
            (K::LoadNamed, Some(_)) => SlotFacts::LoadNamed(self.property(v, i, true)),
            (K::StoreNamed, Some(_)) => SlotFacts::StoreNamed(self.property(v, i, true)),
            // 0x25 add cases keep no validity cell (ic.rs).
            (K::DefineNamedOwn, Some(_)) => SlotFacts::DefineNamedOwn(self.property(v, i, false)),
            (K::LoadGlobal, Some(name)) => SlotFacts::LoadGlobal(self.global(v, i, name)),
            (K::LoadGlobalInsideTypeof, Some(name)) => {
                SlotFacts::LoadGlobalInsideTypeof(self.global(v, i, name))
            }
            (K::StoreGlobal, Some(name)) => SlotFacts::StoreGlobal(self.global(v, i, name)),
            (
                K::LoadNamed
                | K::StoreNamed
                | K::DefineNamedOwn
                | K::LoadGlobal
                | K::LoadGlobalInsideTypeof
                | K::StoreGlobal,
                None,
            ) => return None,
            // F-R8: the opcode says what the key half holds.
            (K::Call, _) => SlotFacts::Call(match (u.call, u.construct) {
                (true, false) => self.call(units, v, i),
                (false, true) => self.construct(units, v, i),
                // The key is a word or a cell, the count of either unit (F-R7): nothing of it
                // is known.
                (true, true) => CallFacts {
                    count: 0,
                    callees: TargetFacts {
                        state: IcState::Megamorphic,
                        targets: Vec::new(),
                    },
                    result: TypeSet::EMPTY,
                },
                (false, false) => return None,
            }),
            _ if !u.other => return None,
            // The implicit present int32 index of an Array (`GetKeyedProperty`, H16-Q2 (1)).
            (K::LoadKeyed, _) => SlotFacts::LoadKeyed(keyed(v, i, KeySet::INDEX)?),
            (K::StoreKeyed, _) => SlotFacts::StoreKeyed(keyed(v, i, KeySet::EMPTY)?),
            (K::DefineKeyedOwn, _) => SlotFacts::DefineKeyedOwn(keyed(v, i, KeySet::EMPTY)?),
            (K::HasKeyed, _) => SlotFacts::HasKeyed(keyed(v, i, KeySet::EMPTY)?),
            (K::StoreInArrayLiteral, _) => {
                SlotFacts::StoreInArrayLiteral(keyed(v, i, KeySet::EMPTY)?)
            }
            // The implicit int32 bits (H16-Q2 (1)): "int32 or never ran" (MODES-D10).
            (K::BinaryOp, _) => SlotFacts::BinaryOp(
                known(BinaryHint::from_bits(int_value(v, i)? as u8))?.union(BinaryHint::INT32),
            ),
            (K::CompareOp, _) => SlotFacts::CompareOp(
                known(CompareHint::from_bits(int_value(v, i)? as u16))?.union(CompareHint::INT32),
            ),
            (K::TypeOf, _) => {
                SlotFacts::TypeOf(known(TypeSet::from_bits(int_value(v, i)? as u16))?)
            }
            (K::InstanceOf, _) => {
                // A slot in its start state (no run gave a result, `instance_ic.rs`): no record.
                // The value half may be a handler or a POLY_IC cell, so no `int_value`.
                if feedback::key(v, i) == UNDEFINED && feedback::value(v, i) == int32_bits(0) {
                    return None;
                }
                SlotFacts::InstanceOf(self.instance_of(units, v, i))
            }
            (K::Literal, _) => SlotFacts::Literal(self.literal(v, i)?),
            (K::CloneObject, _) => SlotFacts::CloneObject(self.clone_site(v, i)),
            (K::ForIn, _) => SlotFacts::ForIn(known(ForInHint::from_bits(int_value(v, i)? as u8))?),
            (K::JumpLoop, _) => SlotFacts::JumpLoop(LoopFacts {
                back_edges: int_value(v, i)?,
            }),
            (K::CreateClosure, _) => SlotFacts::CreateClosure(ClosureFacts {
                closures: match int_value(v, i)? {
                    0 => ClosureCount::None,
                    1 => ClosureCount::One,
                    _ => ClosureCount::Many,
                },
            }),
        };
        Some(facts)
    }

    /// The name of the FunctionInfo at address `addr`: `Js` for a unit function (but
    /// function 0, which no closure has, H02), found by the address range of its unit's
    /// dense info array (only that unit is hashed); `Builtin` for a table builtin
    /// ([`Exporter::builtin_infos`]); else `None` (a host function, a stub, no info).
    fn target_at(&mut self, units: &[Box<LoadedUnit>], addr: usize) -> Option<FunctionRef> {
        let size = size_of::<FunctionInfo>();
        for u in units {
            let base = u.infos.as_ptr().addr();
            let Some(off) = addr.checked_sub(base) else {
                continue;
            };
            if off < size * u.infos.len() {
                let f = (off % size == 0)
                    .then_some(off / size)
                    .filter(|&f| f != 0)?;
                return Some(FunctionRef::Js {
                    unit: self.unit_hash(u),
                    function: f as u32,
                });
            }
        }
        self.builtin_infos().get(&addr).copied()
    }

    /// The FunctionInfo address of each table builtin whose function object exists in the
    /// current realm (`builtins::existing_function`; the install makes every one), with its
    /// id. Made at the first lookup of an export.
    fn builtin_infos(&mut self) -> &HashMap<usize, FunctionRef> {
        if self.targets.is_none() {
            let mut t = HashMap::new();
            let tables =
                core::iter::once(&crate::natives::VM_TABLE).chain(self.rt.builtins.table());
            for m in tables.flat_map(|t| t.modules.iter()) {
                for d in 0..m.defs.len() {
                    let id = m.id(d as u16);
                    if let Some(f) = builtins::existing_function(self.rt, id) {
                        let info = core::ptr::from_ref(object::function_info(f)).addr();
                        t.insert(info, FunctionRef::Builtin(id.bits()));
                    }
                }
            }
            self.targets = Some(t);
        }
        self.targets.as_ref().expect("made above")
    }

    /// The name of the function cell `f` (a FUNCTION cell of a unit function or a table
    /// builtin), else `None` (a bound function, a proxy, a host function, a dead key).
    fn function_ref(&mut self, units: &[Box<LoadedUnit>], f: u64) -> Option<FunctionRef> {
        if !is_cell_bits(f) || !kind::is_function(f) {
            return None;
        }
        let addr = core::ptr::from_ref(object::function_info(f)).addr();
        self.target_at(units, addr)
    }

    /// The callee state of one key: undefined (no target yet), the hole (megamorphic), or
    /// a target that has a name (monomorphic) or none (megamorphic).
    fn callees(key: u64, target: Option<FunctionRef>) -> TargetFacts {
        let (state, targets) = match (key, target) {
            (UNDEFINED, _) => (IcState::Uninitialized, Vec::new()),
            (_, Some(r)) if key != HOLE => (IcState::Monomorphic, vec![r]),
            _ => (IcState::Megamorphic, Vec::new()),
        };
        TargetFacts { state, targets }
    }

    /// A Call slot that only `Call*` sites use (`profile.rs`): the key is the native word of
    /// the one sampled callee's FunctionInfo, the hole or undefined; the value is the count
    /// of the **sampled calls** (F-R7).
    fn call(&mut self, units: &[Box<LoadedUnit>], v: u64, i: u32) -> CallFacts {
        let key = feedback::key(v, i);
        let target = match key {
            UNDEFINED | HOLE => None,
            // F-R8: a word never leaves the process; its FunctionInfo's address is looked up.
            w if !is_cell_bits(w) => native_address(w).and_then(|a| self.target_at(units, a)),
            // A cell here is not what `profile.rs` writes: no name.
            _ => None,
        };
        // A value half that is not an int32 cannot be here (a construct handler of a shared
        // slot): count 0.
        let n = feedback::value(v, i);
        CallFacts {
            count: if val(n).is_int32() {
                int32_of(n) as u32
            } else {
                0
            },
            callees: Self::callees(key, target),
            result: TypeSet::EMPTY,
        }
    }

    /// A Call slot that only `Construct*` sites use (`construct_ic.rs`): the key is the
    /// constructor (weak; undefined after it died) or the hole; the value is the count of the
    /// **constructs of the cold path** (F-R7), an int32 or the +16 of a `CONSTRUCT_SITE`
    /// handler.
    fn construct(&mut self, units: &[Box<LoadedUnit>], v: u64, i: u32) -> CallFacts {
        let key = feedback::key(v, i);
        let target = self.function_ref(units, key);
        let h = feedback::value(v, i);
        let count = if val(h).is_int32() {
            int32_of(h)
        } else if is_cell_bits(h) && rd_i32(cell(h), hc::KIND) == hk::CONSTRUCT_SITE {
            rd_i32(cell(h), hc::CELL)
        } else {
            0
        };
        CallFacts {
            count: count as u32,
            callees: Self::callees(key, target),
            result: TypeSet::EMPTY,
        }
    }

    /// An InstanceOf slot (IC_LAYOUT 2: the rhs function (weak) or the hole, and a handler, a
    /// POLY_IC cell or an int32 state; `instance_ic.rs`, IC2 M4-P3): the state and the
    /// functions that have a name (the others are left out).
    fn instance_of(&mut self, units: &[Box<LoadedUnit>], v: u64, i: u32) -> TargetFacts {
        let (state, keys) = self.entry(v, i);
        let mut targets: Vec<FunctionRef> = keys
            .into_iter()
            .filter_map(|k| self.function_ref(units, k))
            .collect();
        targets.sort();
        targets.dedup();
        TargetFacts { state, targets }
    }

    /// A Literal slot (`literal_site.rs`): the count of the **cold-path results** (F-R7) and
    /// the shape of the last one (weak key; none if it died or has no description). v0 has no
    /// elements kinds (H18), so `elements` is none.
    fn literal(&mut self, v: u64, i: u32) -> Option<LiteralFacts> {
        let count = int_value(v, i)?;
        let key = feedback::key(v, i);
        let shape = if is_cell_bits(key) && shape::is_shape(key) {
            self.shape_desc(key)
        } else {
            None
        };
        Some(LiteralFacts {
            count,
            shape,
            elements: None,
        })
    }

    /// A CloneObject slot (`literal_site.rs`, an IC entry keyed by the source shape): the state
    /// and one case per live source shape whose `CLONE` handler's result shape is alive and
    /// both have a description. Two sources with one description but different results are
    /// left out, as at a property site.
    fn clone_site(&mut self, v: u64, i: u32) -> CloneFacts {
        let (state, keys) = self.entry(v, i);
        let mut cases = Vec::new();
        for k in keys {
            let Some(h) = ic::find(v, i, k) else {
                debug_assert!(false, "a case key with no handler");
                continue;
            };
            if !is_cell_bits(k) || !shape::is_shape(k) || !is_cell_bits(h) {
                continue;
            }
            let c = cell(h);
            let r = rd(c, hc::REF);
            if rd_i32(c, hc::KIND) != hk::CLONE || !is_cell_bits(r) {
                continue;
            }
            if let (Some(source), Some(result)) = (self.shape_desc(k), self.shape_desc(r)) {
                cases.push(CloneCase { source, result });
            }
        }
        cases.sort();
        cases.dedup();
        let mut keep = vec![true; cases.len()];
        for j in 1..cases.len() {
            if cases[j - 1].source == cases[j].source {
                keep[j - 1] = false;
                keep[j] = false;
            }
        }
        let mut it = keep.into_iter();
        cases.retain(|_| it.next().unwrap_or(false));
        CloneFacts { state, cases }
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
