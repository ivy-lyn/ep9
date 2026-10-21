//! FactSnapshot: the feedback schema that the AOT compiler reads (H16). A fact never holds a
//! ShapeId or an address, so two runs of one program encode the same bytes.
//!
//! Its only dependency is `ep9_bytecode` (`.github/scripts/dep-guards.sh` check 7).
#![forbid(unsafe_code)]

/// Version of the FactSnapshot encoding. A unit header records it (H35), and a change to the
/// schema bumps it.
pub const FACT_SCHEMA_VERSION: u32 = 1;
