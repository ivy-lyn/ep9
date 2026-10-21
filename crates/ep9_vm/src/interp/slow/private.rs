#![allow(dead_code)] // m2-stub
//! Private names on the keyed property paths (C7; owner P2-CLS-VM). A private name is a
//! symbol with the private flag (`symbol::new_private`); `#x` access uses the keyed opcodes
//! with the private symbol as the key:
//!
//! ```text
//! GetKeyedProperty   o[#x]      slow::prop   -> keyed_get     PrivateGet (§7.3.31)
//! SetKeyedProperty   o[#x] = v  slow::prop   -> keyed_set     PrivateSet (§7.3.32)
//! DefineKeyedOwn*    flags      slow::prop   -> keyed_define  bit 1: PrivateFieldAdd (§7.3.28)
//! TestIn             #x in o    slow::compare -> keyed_has    PrivateElementFind (§7.3.27)
//! ```
//!
//! P0 stubs: each forwards to the path that a private key took before (the ordinary keyed
//! operation), so no behaviour changes. T0 records no keyed IC; the IC code asserts that it
//! never sees a private key (ic.rs).

use crate::conv::Res;
use crate::interp::Cx;
use crate::property;

/// `o[#x]`. Stub: the ordinary `[[Get]]`.
pub(crate) fn keyed_get(cx: &mut Cx, o: u64, k: u64) -> Res<u64> {
    property::get_keyed(cx, o, k)
}

/// `o[#x] = v`. Stub: the ordinary `[[Set]]`.
pub(crate) fn keyed_set(cx: &mut Cx, o: u64, k: u64, v: u64, strict: bool) -> Res<()> {
    property::set_keyed(cx, o, k, v, strict)
}

/// A private field add (`flags` bit 1) or another own define with a private key. Stub:
/// CreateDataPropertyOrThrow.
pub(crate) fn keyed_define(cx: &mut Cx, o: u64, k: u64, v: u64, _flags: u8) -> Res<()> {
    super::property::define_checked(cx, o, k, v, true)
}

/// `#x in o`. Stub: the ordinary `in`.
pub(crate) fn keyed_has(cx: &mut Cx, o: u64, k: u64) -> Res<bool> {
    property::has(cx, o, k)
}
