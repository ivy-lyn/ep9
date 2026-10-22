#![allow(dead_code)] // m3-stub
//! Realms on the builtin ABI (owner M3-P1 REALMS; H22): making a realm, the current realm, and
//! the realm of a function (`object::function_realm`, the FUNCTION `[[Realm]]` field of
//! OBJECT_LAYOUT 3; `Cx.realm` in the interpreter). Empty in M3-P0.

use super::Cx;

impl<'s> Cx<'s> {}
