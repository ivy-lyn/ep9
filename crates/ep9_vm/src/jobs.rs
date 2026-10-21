#![allow(dead_code)] // m2-stub
//! The microtask job queue (owner P4-PROMISE; RT-9). The queue and each promise reaction are
//! heap cells registered as roots, never a Rust `Vec`/`VecDeque` of Values (plan rule 8).
//!
//! P0 stub: no items yet.
