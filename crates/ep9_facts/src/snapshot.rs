//! [`FactSnapshot`]: the facts of one unit, keyed by (unit hash, function id, slot index),
//! and the validator that checks them against the unit.

use std::collections::BTreeMap;

use ep9_bytecode::{ABI_VERSION, CompiledUnit, FeedbackSlotKind};

use crate::bits::FactSource;
use crate::facts::{FunctionRef, SlotFacts, UnitHash};

/// Why a snapshot or a record was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactError {
    /// The input ends inside a field.
    Truncated,
    /// The first 8 bytes are not [`crate::MAGIC`].
    Magic,
    /// Another schema version: the reader drops the whole snapshot (H35: a unit then loads
    /// without its hints).
    SchemaVersion(u32),
    /// A reserved byte or bit is set.
    Reserved,
    /// `total_size` is not the input length.
    TotalSize,
    /// An undefined tag or enum value.
    BadValue(&'static str),
    /// A second encoding of the same facts: a list out of order, a duplicate, an unreferenced
    /// shape, a UTF-16 string that fits Latin-1.
    NonCanonical(&'static str),
    /// A record breaks a format rule (too many cases, a field outside its shape, ...).
    Invalid(&'static str),
}

impl core::fmt::Display for FactError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FactError::Truncated => f.write_str("truncated fact snapshot"),
            FactError::Magic => f.write_str("not a fact snapshot (bad magic)"),
            FactError::SchemaVersion(v) => write!(f, "fact schema version {v} is not supported"),
            FactError::Reserved => f.write_str("a reserved byte or bit is set"),
            FactError::TotalSize => f.write_str("total_size does not match the input length"),
            FactError::BadValue(what) => write!(f, "undefined value: {what}"),
            FactError::NonCanonical(what) => write!(f, "non-canonical encoding: {what}"),
            FactError::Invalid(what) => write!(f, "invalid record: {what}"),
        }
    }
}

impl std::error::Error for FactError {}

/// The snapshot header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FactHeader {
    /// The bytecode ABI of the unit (`ep9_bytecode::ABI_VERSION` when the facts were taken).
    pub bytecode_abi: u32,
    /// The unit's content hash. With the function id and slot index of a record it forms the
    /// record key.
    pub unit_hash: UnitHash,
    /// Where the facts come from.
    pub source: FactSource,
}

/// A record key inside one snapshot: the function's stable id and the slot index. The full
/// key adds the header's unit hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotKey {
    /// Stable function id (preorder, H2).
    pub function: u32,
    /// Index into the function's feedback slot kinds.
    pub slot: u32,
}

/// The facts of one unit. Every value is canonical: records are sorted by key and checked,
/// so equal facts encode to equal bytes whatever order they were added in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FactSnapshot {
    header: FactHeader,
    records: BTreeMap<SlotKey, SlotFacts>,
}

/// Why the validator dropped a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The unit has no function with this id.
    NoFunction,
    /// The function has no slot with this index.
    NoSlot,
    /// The record's kind is not the slot's kind in the unit.
    KindMismatch {
        /// The slot's kind in the unit.
        unit: FeedbackSlotKind,
        /// The record's kind.
        record: FeedbackSlotKind,
    },
    /// A function reference into this unit names a function it does not have.
    NoTarget,
}

/// A record the validator dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dropped {
    /// The record's key.
    pub key: SlotKey,
    /// Why.
    pub reason: DropReason,
}

/// Why the validator refused a whole snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    /// The snapshot was taken under another bytecode ABI.
    BytecodeAbi {
        /// The snapshot's ABI.
        snapshot: u32,
        /// This build's ABI.
        current: u32,
    },
    /// The snapshot is about another unit (or another version of this source).
    UnitHash,
}

impl FactSnapshot {
    /// An empty snapshot.
    pub fn new(header: FactHeader) -> FactSnapshot {
        FactSnapshot {
            header,
            records: BTreeMap::new(),
        }
    }

    /// The header.
    pub fn header(&self) -> &FactHeader {
        &self.header
    }

    /// Adds (or replaces) the record of slot `slot` of function `function`. The facts are
    /// put in canonical order first ([`SlotFacts::normalize`]); a record that breaks a format
    /// rule is refused and the snapshot does not change.
    pub fn insert(
        &mut self,
        function: u32,
        slot: u32,
        mut facts: SlotFacts,
    ) -> Result<(), FactError> {
        facts.normalize()?;
        self.records.insert(SlotKey { function, slot }, facts);
        Ok(())
    }

    /// The record of a slot.
    pub fn get(&self, function: u32, slot: u32) -> Option<&SlotFacts> {
        self.records.get(&SlotKey { function, slot })
    }

    /// Removes the record of a slot.
    pub fn remove(&mut self, function: u32, slot: u32) -> Option<SlotFacts> {
        self.records.remove(&SlotKey { function, slot })
    }

    /// The records in key order.
    pub fn records(&self) -> impl Iterator<Item = (SlotKey, &SlotFacts)> {
        self.records.iter().map(|(k, v)| (*k, v))
    }

    /// The number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True if there are no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// A record from the decoder, which checks the order of keys itself.
    pub(crate) fn push_decoded(&mut self, key: SlotKey, facts: SlotFacts) {
        self.records.insert(key, facts);
    }

    /// Checks the snapshot against `unit`, the unit it claims to describe.
    ///
    /// - The whole snapshot is refused ([`Stale`]) if its bytecode ABI is not this build's,
    ///   or its unit hash is not `unit`'s content hash.
    /// - A record is dropped, never trusted, if its function or slot does not exist, if its
    ///   kind is not `unit.functions[f].feedback_slots[i]`, or if it names a function of this
    ///   unit that does not exist.
    ///
    /// Computing the content hash serializes the unit once.
    pub fn validate(mut self, unit: &CompiledUnit) -> Result<(FactSnapshot, Vec<Dropped>), Stale> {
        if self.header.bytecode_abi != ABI_VERSION {
            return Err(Stale::BytecodeAbi {
                snapshot: self.header.bytecode_abi,
                current: ABI_VERSION,
            });
        }
        if self.header.unit_hash != unit.content_hash() {
            return Err(Stale::UnitHash);
        }
        let mut dropped = Vec::new();
        let unit_hash = self.header.unit_hash;
        self.records.retain(|&key, facts| {
            let reason = match unit.functions.get(key.function as usize) {
                None => Some(DropReason::NoFunction),
                Some(f) => match f.feedback_slots.get(key.slot as usize) {
                    None => Some(DropReason::NoSlot),
                    Some(&k) if k != facts.kind() => Some(DropReason::KindMismatch {
                        unit: k,
                        record: facts.kind(),
                    }),
                    Some(_) => {
                        let mut bad = false;
                        facts.for_each_function(&mut |r| {
                            if let FunctionRef::Js { unit: u, function } = r
                                && *u == unit_hash
                                && *function as usize >= unit.functions.len()
                            {
                                bad = true;
                            }
                        });
                        bad.then_some(DropReason::NoTarget)
                    }
                },
            };
            match reason {
                Some(reason) => {
                    dropped.push(Dropped { key, reason });
                    false
                }
                None => true,
            }
        });
        Ok((self, dropped))
    }
}
