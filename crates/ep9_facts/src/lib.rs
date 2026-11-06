//! FactSnapshot v0: the feedback schema that the optimizing compilers and AOT read (H16,
//! `docs/adr/H16-ics-and-feedback.md`). One schema serves RUNTIME feedback, PROFILED
//! training runs and HINTED TS facts (H30), so the three are interchangeable inputs.
//!
//! ```text
//!  FactSnapshot ── header: schema version, bytecode ABI, unit hash, source (H30)
//!     └─ records keyed by (unit hash, function id, slot index), each with its slot kind
//!          └─ SlotFacts: one payload per FeedbackSlotKind (exhaustive match)
//!  encode() ──> canonical bytes ──> decode()      dump() ──> stable text
//!  validate(unit): drop the snapshot on another ABI or unit hash; drop a record whose kind
//!                  is not the unit's feedback_slots[i]
//! ```
//!
//! Rules:
//! - No type holds a ShapeId, an address, a cell kind id or a closure identity. Shapes are
//!   [`ShapeDesc`] values; functions are (unit hash, function id) or a builtin id. So two runs
//!   of one program give the same bytes.
//! - Bitsets only grow and merge by OR.
//! - Any change to the format bumps [`FACT_SCHEMA_VERSION`]: `tests/golden/schema-history.txt`
//!   pins the SHA-256 of [`schema_text`] and the sample snapshot's bytes per version.
//!
//! Its only dependency is `ep9_bytecode` (`.github/scripts/dep-guards.sh` check 7). The VM
//! exporter (FeedbackVector to FactSnapshot) is `ep9_vm::abi::export_facts` (H16-Q5).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bits;
mod codec;
mod facts;
mod snapshot;
mod text;

pub use bits::{
    BinaryHint, BindingSet, ClosureCount, CompareHint, ElementsKind, ElementsKindSet, FactSource,
    FieldHint, FieldRep, ForInHint, IcState, KeySet, KeyedHint, ObjectKind, PropAttrs, RecordFlags,
    ShapeFlags, TypeSet, WellKnownSymbol,
};
pub use codec::{HEADER_SIZE, offsets};
pub use facts::{
    Access, CallFacts, CloneCase, CloneFacts, ClosureFacts, FunctionRef, GlobalFacts, KeyRef,
    KeyedFacts, LiteralFacts, LoopFacts, MAX_CASES, Payload, PropertyCase, PropertyFacts, ProtoRef,
    ShapeDesc, SlotFacts, TargetFacts, UnitHash,
};
pub use snapshot::{
    DropReason, Dropped, FactError, FactHeader, FactSnapshot, RAN_KINDS, RecordMeta, SlotKey, Stale,
};
pub use text::{schema_format_text, schema_text};

/// Version of the FactSnapshot schema. A unit header records it (H35); a unit whose version
/// differs loads without its hints. Any change to [`schema_text`] or to the encoding bumps it
/// (`tests/golden/schema-history.txt`).
///
/// | Version | Change |
/// |---|---|
/// | 1 | FactSnapshot v0 (H16 v0 slot kinds, M0 week 4) |
/// | 2 | The one month-6 bump (M6-P0; M6-PLAN-D10, V10-D58): per-record flags (H29-Q5 exit flag, MODES-D15 "ran" bit at the MODES-D17 sites) and the H30-Q1 confidence byte; `ShapeDesc` elements kind and per-key rep/const hints (M6-PLAN-D8); the keyed hint (bits 4-9) and kinds set (M6-PLAN-D4); `ElementsKind` in the VM order; `ObjectKind` arguments and typed array (M6-PLAN-D5); the open lists (MC-w23-8) |
pub const FACT_SCHEMA_VERSION: u32 = 2;

/// The first 8 bytes of an encoded snapshot: `"\x89EP9FS\r\n"` (PNG style, like the unit and
/// CompiledUnit magics).
pub const MAGIC: [u8; 8] = *b"\x89EP9FS\r\n";
