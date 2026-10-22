#![allow(dead_code)] // m3-stub
//! The facts exporter (owner M3-P3 FACTS-EXPORT; H16-Q5, the exporter half): a unit's
//! feedback vectors (H16) as an `ep9_facts::FactSnapshot` (FeedbackVector -> FactSnapshot).
//! Public as `ep9_vm::abi::export_facts` (V8-A13); not part of the H20 core (FREEZE-PREP lists
//! it in the H20 part-2 table).
//!
//! M3-P0 stub: the function and its types exist, so the `ep9_vm -> ep9_facts` edge is used;
//! it returns an error.

use crate::Isolate;

/// The facts of the loaded unit whose content hash is `unit_hash`, from its feedback vectors.
///
/// # Errors
///
/// M3-P0: always "not implemented" (M3-P3 FACTS-EXPORT builds it).
pub fn export_facts(
    isolate: &Isolate,
    unit_hash: &ep9_facts::UnitHash,
) -> Result<ep9_facts::FactSnapshot, &'static str> {
    let _ = (isolate, unit_hash);
    Err("not implemented in ep9 v0: facts export (M3-P3 FACTS-EXPORT)")
}
