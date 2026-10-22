#![allow(dead_code)] // m3-stub
//! Arguments exotic objects (§10.4.4; owner M3-P2 VM-ARGS): the ARGUMENTS kind (25,
//! `object::argsf`), `CreateMappedArguments` and `CreateUnmappedArguments`, and the internal
//! methods of a mapped arguments object.
//!
//! ```text
//! CreateMappedArguments   (ops.rs cold body) ──> create(.., mapped = true)
//! CreateUnmappedArguments (ops.rs cold body) ──> create(.., mapped = false)
//! exotic.rs hooks (get, own_property, own_attrs) ──────┐
//! property.rs hooks (set, define, delete, own keys) ───┴──> the hook fns below
//! ```
//!
//! M3-P0 stub: `create` makes today's array (`slow::arguments(.., false)`, V8-A15), and every
//! hook answers "not exotic" (`None`), so the ordinary path runs. VM-ARGS fills this file; the
//! `ordinary_*` entries of `property.rs` are the OrdinaryX steps a hook can call.

use crate::conv::Res;
use crate::descriptor::Desc;
use crate::exotic::probe;
use crate::frame::{UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::key::Probe;
use crate::kinds as kind;
use crate::object::{self, argsf, init};
use crate::runtime::Runtime;

/// `CreateMappedArguments` (`mapped`) and `CreateUnmappedArguments` (§10.4.4.6, §10.4.4.7).
/// M3-P0: both make today's arguments array (`slow::arguments` with `rest` false). Returns
/// the new accumulator, or 0 if an exception is pending (the `slow` entry convention).
#[cold]
#[inline(never)]
pub(crate) fn create(cx: &mut Cx, fp: *mut u64, pc: *const u8, mapped: bool) -> u64 {
    probe::note(if mapped {
        "arguments create mapped"
    } else {
        "arguments create unmapped"
    });
    crate::interp::slow::arguments(cx, fp, pc, false)
}

/// A new unmapped ARGUMENTS object with prototype `proto`: no parameter map, mapped count 0,
/// no elements, its 3 in-object slots undefined. For the layout tests until VM-ARGS builds
/// `create`. GC point.
pub(crate) fn new_arguments_object(rt: &Runtime, proto: u64) -> u64 {
    let c = object::new_js_object(rt, kind::ARGUMENTS, proto, argsf::CAPACITY);
    init(c, argsf::MAP, UNDEFINED);
    init(c, argsf::MAPPED_COUNT, int32_bits(0));
    c.to_value().to_bits()
}

// ---- exotic hooks (`exotic.rs`): `None` = no exotic own property, take the ordinary path ----

/// `[[Get]]` of an exotic own property (§10.4.4.3). Stub: none.
pub(crate) fn get(_cx: &mut Cx, _o: u64, _key: u64, _how: Probe) -> Option<Res<u64>> {
    probe::note("arguments get");
    None
}

/// `[[GetOwnProperty]]` of an exotic own property (§10.4.4.1): (value, attribute bits).
/// Stub: none.
pub(crate) fn own_property(
    _cx: &mut Cx,
    _o: u64,
    _key: u64,
    _how: Probe,
) -> Option<Res<(u64, i32)>> {
    probe::note("arguments own_property");
    None
}

/// The attributes of an exotic own property, without making anything. Not a GC point.
/// Stub: none.
pub(crate) fn own_attrs(_rt: &Runtime, _o: u64, _key: u64, _how: Probe) -> Option<i32> {
    probe::note("arguments own_attrs");
    None
}

// ---- internal-method hooks (`property.rs`): `None` = the ordinary method ----

/// `[[Set]]` (§10.4.4.4). Stub: ordinary.
pub(crate) fn set(_cx: &mut Cx, _o: u64, _key: u64, _v: u64, _receiver: u64) -> Option<Res<bool>> {
    probe::note("arguments set");
    None
}

/// `[[DefineOwnProperty]]` (§10.4.4.2). Stub: ordinary.
pub(crate) fn define_own_property(
    _cx: &mut Cx,
    _o: u64,
    _key: u64,
    _desc: Desc,
) -> Option<Res<bool>> {
    probe::note("arguments define_own_property");
    None
}

/// `[[Delete]]` (§10.4.4.5). Stub: ordinary.
pub(crate) fn delete(_cx: &mut Cx, _o: u64, _key: u64) -> Option<Res<bool>> {
    probe::note("arguments delete");
    None
}

/// `[[OwnPropertyKeys]]` as a new array. Stub: ordinary.
pub(crate) fn own_property_keys(_cx: &mut Cx, _o: u64) -> Option<u64> {
    probe::note("arguments own_property_keys");
    None
}
