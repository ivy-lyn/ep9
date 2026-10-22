#![allow(dead_code)] // m3-stub
//! Modules (§16.2; owner M3-P2 MODULE, then M3-P3 IMPORT-TLA): the MODULE record cell (kind
//! 76, `object::modf`), module namespace exotic objects (kind 26, `object::nsf`, §10.4.6) and
//! their internal methods.
//!
//! ```text
//! exotic.rs hooks (get, own_property, own_attrs) ──────┐
//! property.rs hooks (set, define, delete, own keys) ───┴──> the namespace hook fns below
//! LdaModuleVariable / StaModuleVariable (ops.rs cold) ────> interp/slow/module.rs
//! ```
//!
//! M3-P0 stub: the record and namespace constructors (for the layout tests), and hooks that
//! answer "not exotic" (`None`), so the ordinary path runs. MODULE fills this file.

use crate::conv::Res;
use crate::descriptor::Desc;
use crate::exotic::probe;
use crate::frame::NULL;
use crate::interp::Cx;
use crate::key::Probe;
use crate::kinds as kind;
use crate::object::{self, init, nsf};
use crate::runtime::Runtime;

/// A new MODULE record: every field undefined. GC point.
pub(crate) fn new_module_record(rt: &Runtime) -> u64 {
    object::new_record(rt, kind::MODULE)
}

/// A new MODULE_NAMESPACE object for `module` (a MODULE cell) with the export names
/// `exports` (an ELEMENTS cell): prototype null, its in-object slot undefined. M3-P0 does not
/// make it non-extensible or add @@toStringTag (MODULE does, §10.4.6.12). GC point.
pub(crate) fn new_namespace(rt: &Runtime, module: u64, exports: u64) -> u64 {
    let c = object::new_js_object(rt, kind::MODULE_NAMESPACE, NULL, nsf::CAPACITY);
    init(c, nsf::MODULE, module);
    init(c, nsf::EXPORTS, exports);
    c.to_value().to_bits()
}

// ---- namespace exotic hooks (`exotic.rs`): `None` = no exotic own property ----

/// `[[Get]]` of an export (§10.4.6.8): `Some(Err)` for a binding in its TDZ (a
/// ReferenceError). Stub: none (test builds: the probe's throw, `probe::set_throw`).
pub(crate) fn get(cx: &mut Cx, _o: u64, _key: u64, _how: Probe) -> Option<Res<u64>> {
    probe::note("module get");
    probe_throw(cx)
}

/// `[[GetOwnProperty]]` of an export (§10.4.6.5): (value, attribute bits); `Some(Err)` for a
/// binding in its TDZ (step 4 reads it). Stub: none (test builds: the probe's throw).
pub(crate) fn own_property(
    cx: &mut Cx,
    _o: u64,
    _key: u64,
    _how: Probe,
) -> Option<Res<(u64, i32)>> {
    probe::note("module own_property");
    probe_throw(cx)
}

/// The value stubs' answer: none; in test builds, a ReferenceError ("probe") while
/// `probe::set_throw` is on (the routing tests, VF-TDZ).
fn probe_throw<T>(cx: &mut Cx) -> Option<Res<T>> {
    #[cfg(test)]
    if probe::throwing() {
        let e = crate::error::throw_error(cx, crate::runtime::ErrorKind::ReferenceError, "probe");
        return Some(Err(e));
    }
    let _ = cx;
    None
}

/// The attributes of an export, without reading its binding. Not a GC point. Stub: none.
pub(crate) fn own_attrs(_rt: &Runtime, _o: u64, _key: u64, _how: Probe) -> Option<i32> {
    probe::note("module own_attrs");
    None
}

// ---- namespace internal-method hooks (`property.rs`): `None` = the ordinary method ----

/// `[[Set]]` (§10.4.6.9: always false). Stub: ordinary.
pub(crate) fn set(_cx: &mut Cx, _o: u64, _key: u64, _v: u64, _receiver: u64) -> Option<Res<bool>> {
    probe::note("module set");
    None
}

/// `[[DefineOwnProperty]]` (§10.4.6.6). Stub: ordinary.
pub(crate) fn define_own_property(
    _cx: &mut Cx,
    _o: u64,
    _key: u64,
    _desc: Desc,
) -> Option<Res<bool>> {
    probe::note("module define_own_property");
    None
}

/// `[[Delete]]` (§10.4.6.10). Stub: ordinary.
pub(crate) fn delete(_cx: &mut Cx, _o: u64, _key: u64) -> Option<Res<bool>> {
    probe::note("module delete");
    None
}

/// `[[OwnPropertyKeys]]` (§10.4.6.11) as a new array. Stub: ordinary.
pub(crate) fn own_property_keys(_cx: &mut Cx, _o: u64) -> Option<u64> {
    probe::note("module own_property_keys");
    None
}
