#![allow(dead_code)] // m6-stub
//! `select()` (owner M6-P0 MODES6; doc 13 §3.2 and C-15, MODES-D13, MODES-D14): the one rule
//! that picks the code object a function's dispatch slot caches, from its code set, the
//! execution policy and the session. A pure function with no caller yet, tested with fake code
//! objects and no codegen (`tests/select.rs`: the T1-T11 table, the append-only lists, JIT and
//! hybrid tier-up, PIN).
//!
//! ```text
//!  select(code set, policy, session) -> code object    (slot := select(..) after every event)
//! ```
//!
//! This is the selector, so the rule 40 policy grep exempts it (plan v11 rule 40).
//!
//! M6-P0 stub: empty until its owner fills it.
