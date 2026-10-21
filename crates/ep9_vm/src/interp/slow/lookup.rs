//! Dynamic scope lookup: sloppy direct eval and `with` (P2-LOOKUP).
//!
//! ```text
//! LdaLookupSlot name               lda_lookup_slot(.., false)   GetBindingValue
//! LdaLookupSlotInsideTypeof name   lda_lookup_slot(.., true)    (no ReferenceError if unresolvable)
//! StaLookupSlot name, flags        sta_lookup_slot               SetMutableBinding / PutValue
//! CreateWithContext object, scope  create_with_context           a context of length 1: the object
//! CallRuntime DeleteLookupSlot     delete_lookup_slot            (runtime id 8) DeleteBinding
//! CallRuntimeForPair               load_lookup_slot_for_call     (runtime id 26) the function and
//!   LoadLookupSlotForCall                                         its receiver (WithBaseObject)
//! ```
//!
//! Every entry first resolves the name (§9.1.2.1 GetIdentifierReference) with [`resolve`],
//! which walks the context chain of the frame from the current context out:
//!
//! ```text
//! with context (unit scope kind With, slot 0 = the object): an object Environment Record
//!   (§9.1.1.2.1 HasBinding): HasProperty(o, name), then Get(o, @@unscopables) and, if that
//!   is an object, ToBoolean(Get(unscopables, name)) blocks the binding
//! any other context: a declarative record; its unit scope names slot i
//!   (`object::context_scope`: unit atom of `scope.slots[i].name`, compared by bits, H19);
//!   then, in a CALLS_SLOPPY_EVAL var scope, the own properties of the `.eval_ext` object in
//!   slot 0 (the vars of sloppy direct eval code)
//! then the global environment: the script lexical record, then the global object
//! ```
//!
//! A name that no context binds resolves to the global environment, where an unresolvable
//! name (no lexical binding, no property on the global object's chain) gives ReferenceError
//! on a read (undefined inside `typeof`), a ReferenceError on a strict store and a new global
//! property on a sloppy store (§6.2.5.6 PutValue step 3).
//!
//! The opcode entries use the result protocol of `slow` (the new accumulator, or 0 with an
//! exception pending). `name` and `scope` are constant indices of the frame's function; the
//! runtime ids take the name as a string value. Each context of the walk takes one budget
//! unit and polls when the budget runs out (M2-P2-POLL): the walk has no back edge. A walk
//! is a GC point when a getter or a poll runs; the values it holds are Rust locals (the
//! conservative stack scan keeps them, H08).

use ep9_bytecode::{EVAL_EXT_BINDING, ScopeFlags, ScopeKind, UnitString, VariableMode};
use ep9_value::Throw;

use crate::conv::{self, Res};
use crate::error::throw_error;
use crate::frame::{HOLE, UNDEFINED};
use crate::interp::Cx;
use crate::key;
use crate::kinds;
use crate::object::{self, attr, is_context, is_object, own_get};
use crate::property;
use crate::runtime::{ErrorKind, Root};
use crate::symbol::{self, WellKnownSymbol};

use super::literal::{CtxKind, create_context};
use super::{NameMsg, context, enter, info, interrupt, ret, strict, throw_msg_now, throw_name_now};

/// Where a name resolved (the base of the Reference, §6.2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    /// Slot `index` of the declarative context `context`, declared with `mode`.
    Slot {
        context: u64,
        index: u32,
        mode: VariableMode,
    },
    /// A property of the binding object of a `with` context (an object Environment Record
    /// with `withEnvironment` true: WithBaseObject is the object).
    With(u64),
    /// An own property of a `.eval_ext` object (a var of sloppy direct eval code in a
    /// declarative record: no receiver, deletable).
    EvalExt(u64),
    /// The global environment, which also stands for an unresolvable name.
    Global,
}

/// One budget unit per context of the walk; polls when the budget runs out (M2-P2-POLL).
#[inline]
fn poll(cx: &mut Cx) -> Res<()> {
    cx.budget -= 1;
    if cx.budget <= 0 && interrupt(cx, core::ptr::null_mut()) == 0 {
        return Err(Throw);
    }
    Ok(())
}

/// The slot name `id` of unit `u` is the `.eval_ext` binding.
fn is_eval_ext(u: &crate::unit::LoadedUnit, id: usize) -> bool {
    matches!(&u.unit.strings[id], UnitString::Latin1(b) if **b == *EVAL_EXT_BINDING.as_bytes())
}

/// GetIdentifierReference (§9.1.2.1) of the atom `name` from the current context of the
/// frame `fp`, out to the global environment (see the module doc). GC point (an
/// `@@unscopables` getter, a poll).
fn resolve(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<Binding> {
    let mut c = context(fp);
    while is_context(c) {
        poll(cx)?;
        let (u, scope) = object::context_scope(c);
        if scope.kind == ScopeKind::With {
            // Only CreateWithContext makes a context of a With scope (`create_context`
            // refuses the other kinds), and slot 0 holds its object; the checks keep the
            // walk safe all the same.
            if let Some(o) = object::context_get(c, 0)
                && is_object(o)
                && with_has_binding(cx, o, name)?
            {
                return Ok(Binding::With(o));
            }
        } else {
            for (i, s) in scope.slots.iter().enumerate() {
                if u.string(s.name.index()) == name {
                    return Ok(Binding::Slot {
                        context: c,
                        index: i as u32,
                        mode: s.mode,
                    });
                }
            }
            if scope.flags.contains(ScopeFlags::CALLS_SLOPPY_EVAL)
                && scope.kind != ScopeKind::Script
                && scope
                    .slots
                    .first()
                    .is_some_and(|s| is_eval_ext(u, s.name.index()))
                && let Some(ext) = object::context_get(c, 0)
                && is_object(ext)
                && property::has_own_property(cx, ext, name)
            {
                return Ok(Binding::EvalExt(ext));
            }
        }
        c = object::context_previous(c);
    }
    Ok(Binding::Global)
}

/// HasBinding of an object Environment Record with `withEnvironment` true (§9.1.1.2.1).
fn with_has_binding(cx: &mut Cx, o: u64, name: u64) -> Res<bool> {
    if !property::has_property(cx, o, name) {
        return Ok(false);
    }
    let key = symbol::well_known(cx.rt(), WellKnownSymbol::Unscopables);
    if !kinds::is_symbol(key) {
        // An isolate without the symbol table installed has no `@@unscopables` either.
        return Ok(true);
    }
    let unscopables = property::get(cx, o, key)?;
    if is_object(unscopables) {
        let blocked = property::get(cx, unscopables, name)?;
        if conv::to_boolean(blocked) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Slot `index` of `context`: an internal error if the context is shorter than its scope.
fn slot_value(cx: &mut Cx, context: u64, index: u32) -> Res<u64> {
    match object::context_get(context, index) {
        Some(v) => Ok(v),
        None => {
            throw_msg_now(cx, "context slot out of range");
            Err(Throw)
        }
    }
}

/// GetBindingValue (§9.1.1.1.6, §9.1.1.2.6, §9.1.1.4.6) of a resolved `name`. `strict` is
/// the strictness of the code (an object binding that is gone by now reads as undefined in
/// sloppy code); `inside_typeof`: an unresolvable name is undefined. GC point.
fn get_binding_value(
    cx: &mut Cx,
    b: Binding,
    name: u64,
    strict: bool,
    inside_typeof: bool,
) -> Res<u64> {
    match b {
        Binding::Slot { context, index, .. } => {
            let v = slot_value(cx, context, index)?;
            if v == HOLE {
                return Err(throw_name_now(cx, NameMsg::Tdz, name));
            }
            Ok(v)
        }
        Binding::With(o) => {
            if !property::has_property(cx, o, name) {
                if strict {
                    return Err(throw_name_now(cx, NameMsg::NotDefined, name));
                }
                return Ok(UNDEFINED);
            }
            property::get(cx, o, name)
        }
        Binding::EvalExt(ext) => property::get(cx, ext, name),
        Binding::Global => {
            let rt = cx.rt();
            if let Some((v, _)) = own_get(rt.root(Root::Lexical), name) {
                if v == HOLE {
                    return Err(throw_name_now(cx, NameMsg::Tdz, name));
                }
                return Ok(v);
            }
            let g = rt.global();
            if !property::has_property(cx, g, name) {
                if inside_typeof {
                    return Ok(UNDEFINED);
                }
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            property::get(cx, g, name)
        }
    }
}

/// PutValue (§6.2.5.6) of `v` to a resolved `name`: SetMutableBinding of its record
/// (§9.1.1.1.5, §9.1.1.2.5, §9.1.1.4.5), or a global property for an unresolvable name in
/// sloppy code. GC point (a setter).
fn set_binding_value(cx: &mut Cx, b: Binding, name: u64, v: u64, strict: bool) -> Res<()> {
    match b {
        Binding::Slot {
            context,
            index,
            mode,
        } => {
            // The TDZ check (ReferenceError) comes before the TypeError of a `const`.
            if slot_value(cx, context, index)? == HOLE {
                return Err(throw_name_now(cx, NameMsg::Tdz, name));
            }
            if mode == VariableMode::Const {
                return Err(const_assign(cx));
            }
            object::context_set(cx.rt(), context, index, v);
            Ok(())
        }
        Binding::With(o) => {
            // The binding may be gone since the reference was resolved (a getter of
            // `@@unscopables` deleted it): strict code throws, sloppy code sets it again.
            if strict && !property::has_property(cx, o, name) {
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            property::set(cx, o, name, v, strict)
        }
        Binding::EvalExt(ext) => property::set(cx, ext, name, v, strict),
        Binding::Global => {
            let rt = cx.rt();
            let lexical = rt.root(Root::Lexical);
            if let Some((old, a)) = own_get(lexical, name) {
                if old == HOLE {
                    return Err(throw_name_now(cx, NameMsg::Tdz, name));
                }
                if a & attr::WRITABLE == 0 {
                    return Err(const_assign(cx));
                }
                object::own_replace(rt, lexical, name, v);
                return Ok(());
            }
            let g = rt.global();
            if strict && !property::has_property(cx, g, name) {
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            property::set(cx, g, name, v, strict)
        }
    }
}

fn const_assign(cx: &mut Cx) -> Throw {
    throw_error(cx, ErrorKind::TypeError, "Assignment to constant variable.")
}

/// The atom of a runtime-id name argument (a string constant of the bytecode).
fn name_key(cx: &mut Cx, name: u64) -> Res<u64> {
    key::to_property_key(cx, name)
}

/// `LdaLookupSlot(name)` / `LdaLookupSlotInsideTypeof(name)` (`inside_typeof`): the value of
/// the binding `name` (ResolveBinding, then GetValue).
#[cold]
#[inline(never)]
pub(crate) fn lda_lookup_slot(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u32,
    inside_typeof: bool,
) -> u64 {
    enter(cx, fp, pc);
    let name = info(fp).constant(name);
    let r = (|| -> Res<u64> {
        let b = resolve(cx, fp, name)?;
        get_binding_value(cx, b, name, strict(fp), inside_typeof)
    })();
    ret(r)
}

/// `StaLookupSlot(name, flags)` with the value `v` (the accumulator, which it keeps). Bit 0
/// of `flags`: the store is in strict code.
#[cold]
#[inline(never)]
pub(crate) fn sta_lookup_slot(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u32,
    flags: u8,
    v: u64,
) -> u64 {
    enter(cx, fp, pc);
    let name = info(fp).constant(name);
    let strict = flags & 1 != 0;
    let r = (|| -> Res<u64> {
        let b = resolve(cx, fp, name)?;
        set_binding_value(cx, b, name, v, strict)?;
        Ok(v)
    })();
    ret(r)
}

/// `CreateWithContext(object, scope)`: a new context of the With scope `scope` whose slot 0
/// holds `object` (bytecode converts it with `ToObject` first; a primitive is converted here
/// too), and whose previous context is the current one.
#[cold]
#[inline(never)]
pub(crate) fn create_with_context(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    object: u64,
    scope: u32,
) -> u64 {
    enter(cx, fp, pc);
    let o = match conv::to_object(cx, object) {
        Ok(o) => o,
        Err(Throw) => return 0,
    };
    create_context(cx, fp, pc, CtxKind::With, scope, 0, o)
}

/// `DeleteLookupSlot(name)` (runtime id 8): sloppy `delete name` (§13.5.1.2) of a name that is
/// not statically bound, or of a global name. DeleteBinding of the record that binds it: a
/// context slot cannot be deleted (false); a `with` object or a `.eval_ext` object deletes
/// its property; the global environment refuses a lexical binding (false) and deletes an own
/// property of the global object (false for a non-configurable one, such as `NaN`). An
/// unresolvable name gives true.
pub(crate) fn delete_lookup_slot(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<u64> {
    let name = name_key(cx, name)?;
    let deleted = match resolve(cx, fp, name)? {
        Binding::Slot { .. } => false,
        Binding::With(o) | Binding::EvalExt(o) => property::delete(cx, o, name, false)?,
        Binding::Global => {
            let rt = cx.rt();
            if own_get(rt.root(Root::Lexical), name).is_some() {
                false
            } else {
                property::delete(cx, rt.global(), name, false)?
            }
        }
    };
    Ok(conv::boolean(deleted))
}

/// `LoadLookupSlotForCall(name)` (runtime id 26, a pair; `slow::call_runtime_pair` writes the
/// two results to the `CallRuntimeForPair` output registers): the function bound to `name`
/// (GetValue), and the receiver of the call, which is the `with` object that holds `name`,
/// else undefined (§13.3.6.1 step 1.b, §9.1.1.2.10 WithBaseObject).
pub(crate) fn load_lookup_slot_for_call(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<(u64, u64)> {
    let name = name_key(cx, name)?;
    let b = resolve(cx, fp, name)?;
    let f = get_binding_value(cx, b, name, strict(fp), false)?;
    let receiver = match b {
        Binding::With(o) => o,
        _ => UNDEFINED,
    };
    Ok((f, receiver))
}
