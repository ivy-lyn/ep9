#![allow(dead_code)] // m6-stub
//! Tests of the accessor ICs (`ic.rs` IC_HANDLER kinds 9 GETTER, 10 SETTER, 11
//! PROTO_DICT_LOAD) (owner M6-P1 ACCIC; the H17 "Accessor ICs" part): own and inherited getter
//! and setter, a replaced pair, a holder that turns dictionary, a validity-cell pop, GC with a
//! cached weak holder. Empty until the owner fills it; `tests/mod.rs` is frozen after M6-P0, so
//! the module is declared now.
