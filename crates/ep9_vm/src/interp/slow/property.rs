//! Globals and property access.
//!
//! ```text
//! 0x21 GetNamedProperty, 0x22 GetNamedPropertyFromSuper,      fast handler: ic::load / ic::store
//! 0x24 SetNamedProperty, 0x25 DefineNamedOwnProperty           miss: get_named, get_super, set_named
//!                                                              = case (ic.rs), generic op, ic::record
//! the other *Property opcodes                                  prop (generic only)
//! 0x10 LdaGlobal, 0x11 LdaGlobalInsideTypeof, 0x12 StaGlobal    fast handler: ic::load_global /
//!                                                              ic::store_global; miss: load_global,
//!                                                              store_global
//! ```

use crate::conv::{self, Res, describe};
use crate::error::{not_implemented, throw_error};
use crate::exotic;
use crate::feedback;
use crate::frame::{HOLE, NULL, UNDEFINED};
use crate::ic;
use crate::interp::Cx;
use crate::key;
use crate::object::{self, attr, is_object, own_get};
use crate::property;
use crate::props;
use crate::runtime::{ErrorKind, Root};
use crate::symbol;

use super::{NameMsg, enter, info, ret, strict, throw_name_now};

// ---- globals and properties ----------------------------------------------------------------

/// `LdaGlobal` / `LdaGlobalInsideTypeof` after an IC miss (or with no vector): the binding's
/// value, then the IC update of feedback slot `slot` (`ic::global_case`, computed before the
/// load). GC point.
///
/// W4-P1-GLOBAL-ONE-SCAN: a found global costs one scan of the global object's table. A case
/// gives the value from its own lookup (`ic::global_case_value`); otherwise
/// [`global_value`] scans once.
#[cold]
#[inline(never)]
pub(crate) fn load_global(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    in_typeof: bool,
    slot: u32,
) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    let vector = feedback::vector(info(fp));
    let case = vector
        .filter(|&v| !ic::skip_global_miss(rt, v, slot))
        .and_then(|_| ic::global_case(rt, name, false));
    let r = match case.as_ref().and_then(|c| ic::global_case_value(rt, c)) {
        Some(v) => v,
        None => global_value(cx, name, in_typeof),
    };
    if r != 0
        && let (Some(v), Some(c)) = (vector, case)
    {
        // GC point: the loaded value lives in `r` (a Rust local).
        ic::record(rt, v, slot, c);
    }
    r
}

/// GetBindingValue of the global environment (§9.1.1.4.6, v0): the script lexical record
/// first, then the global object. Returns the value, or 0 with a pending ReferenceError (a
/// TDZ, or a missing name outside `typeof`).
fn global_value(cx: &mut Cx, name: u64, in_typeof: bool) -> u64 {
    let rt = cx.rt();
    if let Some((v, _)) = own_get(rt.root(Root::Lexical), name) {
        if v == HOLE {
            throw_name_now(cx, NameMsg::Tdz, name);
            return 0;
        }
        return v;
    }
    let g = rt.global();
    // One lookup per object on the chain, and a data property ends the walk. An accessor or
    // an exotic property takes HasProperty and `[[Get]]` (the getter gets the global object as
    // `this`).
    let how = key::probe(name);
    let mut o = g;
    loop {
        if exotic::own_attrs(rt, o, name, how).is_some() {
            break;
        }
        match props::own_get_with(o, name, how) {
            Some((v, a)) if a & attr::ACCESSOR == 0 => return v,
            Some(_) => break,
            None => {
                o = object::proto_of(o);
                if o == NULL {
                    if in_typeof {
                        return UNDEFINED;
                    }
                    throw_name_now(cx, NameMsg::NotDefined, name);
                    return 0;
                }
            }
        }
    }
    let r = (|| -> Res<u64> {
        if !property::has(cx, g, name)? {
            if in_typeof {
                return Ok(UNDEFINED);
            }
            return Err(throw_name_now(cx, NameMsg::NotDefined, name));
        }
        property::get(cx, g, name)
    })();
    ret(r)
}

/// `StaGlobal` after an IC miss with no vector (T0's cold arm; a miss with a vector goes to
/// `ic::store_global_miss`): SetMutableBinding of the global environment (v0), then the IC
/// update of feedback slot `slot` (`ic::global_case`, computed before the store). Returns the
/// stored value. GC point.
#[cold]
#[inline(never)]
pub(crate) fn store_global(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    v: u64,
    slot: u32,
) -> u64 {
    let rt = cx.rt();
    let vector = feedback::vector(info(fp));
    let case = vector
        .filter(|&vec| !ic::skip_global_miss(rt, vec, slot))
        .and_then(|_| ic::global_case(rt, name, true));
    store_global_with(cx, fp, pc, name, v, slot, vector, case)
}

/// [`store_global`] with the feedback vector of the frame and the case that the caller
/// computed before the store (`None` if the entry needs none, [`ic::skip_global_miss`], or if
/// there is no case):
/// SetMutableBinding of the global environment, then the case is recorded in slot `slot` if
/// the store succeeded. `ic::store_global_miss` calls it, so a miss checks the entry and
/// computes the case once (P4F-R1). Returns the stored value, or 0 with a pending exception.
/// GC point: `vector` is a root of its unit, and the case's key and references live in Rust
/// locals until `ic::record` stores them (conservative scan, H10).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn store_global_with(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    v: u64,
    slot: u32,
    vector: Option<u64>,
    case: Option<ic::Case>,
) -> u64 {
    enter(cx, fp, pc);
    let r = set_global(cx, fp, name, v);
    if r != 0
        && let (Some(vec), Some(c)) = (vector, case)
    {
        ic::record(cx.rt(), vec, slot, c);
    }
    r
}

/// SetMutableBinding of the global environment (v0): `v`, or 0 with a pending exception.
fn set_global(cx: &mut Cx, fp: *mut u64, name: u64, v: u64) -> u64 {
    let rt = cx.rt();
    let lexical = rt.root(Root::Lexical);
    if let Some((old, a)) = own_get(lexical, name) {
        if old == HOLE {
            throw_name_now(cx, NameMsg::Tdz, name);
            return 0;
        }
        if a & attr::WRITABLE == 0 {
            throw_error(cx, ErrorKind::TypeError, "Assignment to constant variable.");
            return 0;
        }
        object::own_replace(rt, lexical, name, v);
        return v;
    }
    let g = rt.global();
    // One scan for the common case: a writable own data property of the global object
    // (what `[[Set]]` does first, property::set_with_receiver).
    if props::own_store_if_writable(rt, g, name, key::probe(name), v) == Some(true) {
        return v;
    }
    let s = strict(fp);
    let r = (|| -> Res<u64> {
        if s && !property::has(cx, g, name)? {
            return Err(throw_name_now(cx, NameMsg::NotDefined, name));
        }
        property::set(cx, g, name, v, s)?;
        Ok(v)
    })();
    ret(r)
}

/// Property operations of the `*Property` opcodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PropOp {
    /// `o[k]`, any key.
    GetKeyed,
    /// `o[k] = v` (strict mode of the function).
    SetKeyed,
    /// CreateDataPropertyOrThrow (`StaInArrayLiteral`).
    DefineKeyed,
    /// `DefineKeyedOwnProperty` and `DefineKeyedOwnPropertyInLiteral` with their flags (bit 0:
    /// set the function name; bit 1: private add, ABI 3): CreateDataPropertyOrThrow, or a
    /// private field add for a private key.
    DefineKeyedOwn(u8),
    /// `delete o[k]` in strict code.
    DeleteStrict,
    DeleteSloppy,
}

/// `super[key]` (§13.3.7.1 MakeSuperPropertyReference, then GetValue): `[[Get]]` on the
/// prototype of the home object with `receiver` (the `this` value).
pub(crate) fn super_get_with_receiver(cx: &mut Cx, home: u64, key: u64, receiver: u64) -> Res<u64> {
    let p = super_base(cx, home, key, "reading")?;
    property::get_with_receiver(cx, p, key, receiver)
}

/// `super[key] = v` (§13.3.7.1, then PutValue: `[[Set]]` on the prototype of the home object
/// with `receiver`, §10.1.9.2). A failed assignment throws in strict code.
pub(crate) fn super_set(
    cx: &mut Cx,
    home: u64,
    key: u64,
    v: u64,
    receiver: u64,
    strict: bool,
) -> Res<()> {
    let p = super_base(cx, home, key, "setting")?;
    if property::set_with_receiver(cx, p, key, v, receiver)? || !strict {
        return Ok(());
    }
    let msg = format!(
        "Cannot assign to read only property '{}' of object",
        crate::key::key_text(cx.rt(), key)
    );
    Err(throw_error(cx, ErrorKind::TypeError, &msg))
}

/// GetSuperBase (§9.1.1.3.5): the prototype of the home object, which must be an object.
fn super_base(cx: &mut Cx, home: u64, key: u64, doing: &str) -> Res<u64> {
    if !is_object(home) {
        return Err(not_implemented(
            cx,
            "super property access without a home object",
        ));
    }
    let p = object::proto_of(home);
    if p == NULL {
        let msg = format!(
            "Cannot read properties of null ({doing} '{}')",
            describe(key).trim_matches('"')
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    Ok(p)
}

/// Runs `op`. Returns the accumulator: the loaded value, the stored value or the boolean.
#[cold]
#[inline(never)]
pub(crate) fn prop(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    op: PropOp,
    o: u64,
    k: u64,
    v: u64,
) -> u64 {
    enter(cx, fp, pc);
    // A private name (`#x`, symbol.rs) goes to the private paths (C7).
    let private = symbol::is_private_key(k);
    let r = match op {
        PropOp::GetKeyed if private => super::private::keyed_get(cx, o, k),
        PropOp::GetKeyed => property::get_keyed(cx, o, k),
        PropOp::SetKeyed if private => {
            super::private::keyed_set(cx, o, k, v, strict(fp)).map(|_| v)
        }
        PropOp::SetKeyed => property::set_keyed(cx, o, k, v, strict(fp)).map(|_| v),
        PropOp::DefineKeyed => define_checked(cx, o, k, v, true).map(|_| v),
        PropOp::DefineKeyedOwn(flags) if private => {
            super::private::keyed_define(cx, o, k, v, flags).map(|_| v)
        }
        // Bit 0 (set the function name) is applied by class_fields.rs (D15).
        PropOp::DefineKeyedOwn(flags) => {
            super::class_fields::define_keyed_own(cx, o, k, v, flags).map(|_| v)
        }
        PropOp::DeleteStrict => property::delete(cx, o, k, true).map(conv::boolean),
        PropOp::DeleteSloppy => property::delete(cx, o, k, false).map(conv::boolean),
    };
    ret(r)
}

// ---- named property ICs (H16, ic.rs): the miss paths -----------------------------------------

/// `GetNamedProperty` (0x21) after an IC miss: `o[key]`, then the IC update of feedback slot
/// `slot`. The case is computed before the load (no GC point, no JS) and recorded only if the
/// load succeeded. GC point.
#[cold]
#[inline(never)]
pub(crate) fn get_named(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    o: u64,
    key: u64,
    slot: u32,
) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    let vector = feedback::vector(info(fp));
    let case = vector
        .filter(|&v| !ic::skip_miss(v, slot, o))
        .and_then(|_| ic::load_case(rt, o, key));
    let r = property::get(cx, o, key);
    if r.is_ok()
        && let (Some(v), Some(c)) = (vector, case)
    {
        // GC point: the loaded value lives in `r` (a Rust local).
        ic::record(rt, v, slot, c);
    }
    ret(r)
}

/// `GetNamedPropertyFromSuper` (0x22) after an IC miss: `super[key]` with the home object
/// `home` and `receiver` as `this` (W4-P3-SUPER-SPLIT), then the IC update. The IC is keyed
/// on the lookup start, the prototype of `home`. GC point.
#[cold]
#[inline(never)]
pub(crate) fn get_super(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    home: u64,
    key: u64,
    receiver: u64,
    slot: u32,
) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    let start = if is_object(home) {
        object::proto_of(home)
    } else {
        NULL
    };
    let vector = feedback::vector(info(fp));
    let case = vector
        .filter(|&v| !ic::skip_miss(v, slot, start))
        .and_then(|_| ic::load_case(rt, start, key));
    let r = super_get_with_receiver(cx, home, key, receiver);
    if r.is_ok()
        && let (Some(v), Some(c)) = (vector, case)
    {
        ic::record(rt, v, slot, c);
    }
    ret(r)
}

/// `SetNamedProperty` (0x24, `define` false: `o[key] = v`, a TypeError on failure in strict
/// code) or `DefineNamedOwnProperty` (0x25, `define` true: CreateDataPropertyOrThrow) after
/// an IC miss, then the IC update. An add-property case is checked against the shape the
/// store produced (`ic::store_case_after`). Returns `v`. GC point.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_named(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    o: u64,
    key: u64,
    v: u64,
    slot: u32,
    define: bool,
) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    let vector = feedback::vector(info(fp));
    let pre = vector
        .filter(|&vec| !ic::skip_miss(vec, slot, o))
        .and_then(|_| ic::store_pre(rt, o, key, define));
    let r = if define {
        define_checked(cx, o, key, v, false)
    } else {
        property::set(cx, o, key, v, strict(fp))
    };
    if r.is_ok()
        && let (Some(vec), Some(pre)) = (vector, pre)
        && let Some(c) = ic::store_case_after(rt, pre, o, key)
    {
        ic::record(rt, vec, slot, c);
    }
    ret(r.map(|()| v))
}

pub(super) fn define_checked(cx: &mut Cx, o: u64, k: u64, v: u64, keyed: bool) -> Res<()> {
    if !is_object(o) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cannot define a property on a non-object",
        ));
    }
    if keyed {
        property::define_keyed(cx, o, k, v)
    } else {
        property::define(cx, o, k, v)
    }
}
