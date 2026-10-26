#![allow(dead_code)] // m4-stub
//! Validity cells (owner M4-P1 VALIDITY; OI-02, V9-D7, H16 K5): one VALIDITY_CELL (kind 79,
//! OBJECT_LAYOUT 4) per prototype, kept in its PROTO_INFO +32 word (`shape.rs`). A change of a
//! prototype's shape invalidates its cell and the cells of every prototype below it (a weak
//! users list). The PROTO_LOAD, MISSING, ADD and GLOBAL IC cases keep a cell (IC_HANDLER +16
//! under IC_LAYOUT 2, `ic.rs`), and their hit path loads the cell and tests one word, in place
//! of the global prototype epoch (`shape::ShapeState::proto_epoch`, which saturates).
//!
//! Empty until the owner fills it; `lib.rs` is frozen after M4-P0, so the module is declared
//! now. The owner registers the kind and its trace function (no code allocates one before).
