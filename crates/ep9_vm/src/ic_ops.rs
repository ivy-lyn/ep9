#![allow(dead_code)] // m6-stub
//! The CacheIR-like op lists of the T0 inline caches (owner M6-P1 CBC; plan v11 V11-D20, the
//! M1 scope "CacheIR-like ICs for ... call/binary/compare", doc 09:86): `ops_of`, one total
//! function from every T0 IC case (named, keyed, global, InstanceOf, Call, BinaryOp,
//! CompareOp) to the H16 op list (`docs/adr/H16-ics-and-feedback.md`, the "Op (v0 list)"
//! table and the `ops_of` handler table), with a test per handler kind that the ops match the
//! handler's checks (`tests/ic_ops.rs`).
//!
//! ```text
//!  T0 IC case (handler kind, feedback slot) --ops_of--> [guards..., action]
//!  e.g. FIELD_IN_OBJECT -> GuardShape, LoadField      SLOW / MEGA -> Generic
//! ```
//!
//! No IC_LAYOUT change: the op list is a reading of the existing handler and slot forms.
//!
//! M6-P0 stub: no op list yet; nothing calls this module.
