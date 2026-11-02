#![allow(dead_code)] // m6-stub
//! Tests of Dictionary v1 (`props_index.rs`, PROPS_INDEXED 84) (owner M6-P1 DICT1; the H17
//! "Dictionary v1" part): insert, find and delete in the indexed form, re-index on grow, the GC
//! trace of kind 84, many properties deleted in random order. Empty until the owner fills it;
//! `tests/mod.rs` is frozen after M6-P0, so the module is declared now.
