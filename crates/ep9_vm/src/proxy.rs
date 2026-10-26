#![allow(dead_code)] // m4-stub
//! Proxy exotic objects (owner M4-P4 PROXY1; LB-6, V9-D19): the kinds PROXY (27) and
//! PROXY_CALLABLE (28) of OBJECT_LAYOUT 4 (`kinds.rs`), the internal methods of §10.5 and their
//! routing from `exotic.rs` and `property.rs`, revocation, and `[[Call]]`/`[[Construct]]` of a
//! callable proxy. `[[ProxyTarget]]` and `[[ProxyHandler]]` live in heap cells (plan rule 8).
//!
//! Empty until the owner fills it; `lib.rs` is frozen after M4-P0, so the module is declared
//! now. The owner registers both kinds and their trace function.
