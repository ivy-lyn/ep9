//! Globals and property access.

use crate::conv::{self, Res, describe};
use crate::error::{not_implemented, throw_error};
use crate::frame::{HOLE, NULL, UNDEFINED};
use crate::interp::Cx;
use crate::object::{self, attr, is_object, own_get};
use crate::property;
use crate::runtime::{ErrorKind, Root};

use super::{NameMsg, enter, ret, strict, throw_name_now};

// ---- globals and properties ----------------------------------------------------------------

/// `LdaGlobal` / `LdaGlobalInsideTypeof`: the script lexical record first, then the global
/// object (§9.1.1.4.6 GetBindingValue of the global environment, v0).
#[cold]
#[inline(never)]
pub(crate) fn load_global(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    in_typeof: bool,
) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if let Some((v, _)) = own_get(rt.root(Root::Lexical), name) {
        if v == HOLE {
            throw_name_now(cx, NameMsg::Tdz, name);
            return 0;
        }
        return v;
    }
    let g = rt.global();
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

/// `StaGlobal`: SetMutableBinding of the global environment (v0). Returns the stored value.
#[cold]
#[inline(never)]
pub(crate) fn store_global(cx: &mut Cx, fp: *mut u64, pc: *const u8, name: u64, v: u64) -> u64 {
    enter(cx, fp, pc);
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
    /// `o[k]`, `k` a property-key string.
    GetNamed,
    /// `o[k]`, any key.
    GetKeyed,
    /// `super[k]` with the home object `o` (v0: no receiver for accessors, none exist).
    GetSuper,
    /// `o[k] = v` (strict mode of the function).
    SetNamed,
    SetKeyed,
    /// CreateDataPropertyOrThrow.
    DefineNamed,
    DefineKeyed,
    /// `delete o[k]` in strict code.
    DeleteStrict,
    DeleteSloppy,
}

/// `super[key]` with home object `home` (§13.3.7.3 GetSuperBase, then [[Get]]). The callers
/// pass no receiver yet (opcode 0x22 and `LoadKeyedFromSuper`): v0 has no getters, so the
/// result is the same. P3 passes `this` through [`super_get_with_receiver`].
pub(super) fn super_get(cx: &mut Cx, home: u64, key: u64) -> Res<u64> {
    super_get_with_receiver(cx, home, key, UNDEFINED)
}

/// `super[key]` (§13.3.7.1 MakeSuperPropertyReference, then GetValue): `[[Get]]` on the
/// prototype of the home object with `receiver` (the `this` value).
pub(crate) fn super_get_with_receiver(cx: &mut Cx, home: u64, key: u64, receiver: u64) -> Res<u64> {
    let p = super_base(cx, home, key, "reading")?;
    property::get_with_receiver(cx, p, key, receiver)
}

/// `super[key] = v` (§13.3.7.1, then PutValue: `[[Set]]` on the prototype of the home object
/// with `receiver`, §10.1.9.2). A failed assignment throws in strict code.
#[allow(dead_code)] // P3-ACC moves `StoreToSuper` (interp/slow/runtime.rs) here.
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
    let r = match op {
        PropOp::GetNamed => property::get(cx, o, k),
        PropOp::GetKeyed => property::get_keyed(cx, o, k),
        PropOp::GetSuper => super_get(cx, o, k),
        PropOp::SetNamed => property::set(cx, o, k, v, strict(fp)).map(|_| v),
        PropOp::SetKeyed => property::set_keyed(cx, o, k, v, strict(fp)).map(|_| v),
        PropOp::DefineNamed => define_checked(cx, o, k, v, false).map(|_| v),
        PropOp::DefineKeyed => define_checked(cx, o, k, v, true).map(|_| v),
        PropOp::DeleteStrict => property::delete(cx, o, k, true).map(conv::boolean),
        PropOp::DeleteSloppy => property::delete(cx, o, k, false).map(conv::boolean),
    };
    ret(r)
}

fn define_checked(cx: &mut Cx, o: u64, k: u64, v: u64, keyed: bool) -> Res<()> {
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
