//! Dynamic scope lookup: sloppy direct eval and `with` (P2-LOOKUP).
//!
//! ```text
//! LdaLookupSlot name               lda_lookup_slot(.., false)   GetBindingValue
//! LdaLookupSlotInsideTypeof name   lda_lookup_slot(.., true)    (no ReferenceError if unresolvable)
//! StaLookupSlot name, flags        sta_lookup_slot               SetMutableBinding / PutValue
//! CreateWithContext object, scope  create_with_context           a context of length 1: the object
//! CallRuntime DeleteLookupSlot     delete_lookup_slot            (runtime id 8) DeleteBinding
//! CallRuntimeForPair               load_lookup_slot_for_call     (runtime id 26) the function and
//!   LoadLookupSlotForCall, strict                                 its receiver (WithBaseObject)
//! CallRuntime ResolveLookupSlot    resolve_lookup_slot           (runtime id 29) ResolveBinding only
//! CallRuntime                      load_lookup_slot_resolved     (runtime id 30) GetValue of it
//!   LoadLookupSlotResolved, strict
//! CallRuntime                      store_lookup_slot_resolved    (runtime id 31) PutValue to it
//!   StoreLookupSlotResolved
//! ```
//!
//! Every entry first resolves the name (§9.1.2.1 GetIdentifierReference) with [`resolve`],
//! which walks the context chain of the frame from the current context out (a private name
//! `#x` of direct eval code is the exception: [`resolve_private`], CX2-E9-1):
//!
//! ```text
//! with context (unit scope kind With, slot 0 = the object): an object Environment Record
//!   (§9.1.1.2.1 HasBinding): HasProperty(o, name), then Get(o, @@unscopables) and, if that
//!   is an object, ToBoolean(Get(unscopables, name)) blocks the binding
//! any other context: a declarative record; its unit scope names slot i
//!   (`object::context_scope`: unit atom of `scope.slots[i].name`, compared by bits, H19);
//!   then, in a CALLS_SLOPPY_EVAL var scope, the own properties of the `.eval_ext` object in
//!   slot 0 (the vars of sloppy direct eval code)
//! at the end of the chain, a MODULE record (direct eval in module code): the module's
//!   cells, by local name (`module::cell_of_name`: imports and exported bindings)
//! then the global environment: the script lexical record, then the global object
//! ```
//!
//! A module cell is a box that the module Environment Record shares with other modules
//! (`module.rs`). A read gives a ReferenceError in its TDZ (the hole), as
//! `LdaModuleVariable` + `ThrowReferenceErrorIfHole`; a store to an import is a TypeError,
//! as `StaModuleVariable` (eval code in a module is strict). A store to an exported binding
//! throws the v0 not-implemented Error after the TDZ check: the module records (ABI 5) do not
//! say which exported binding is `const`, so the store could not give the TypeError of a
//! `const`.
//!
//! A name that no context binds resolves to the global environment, where an unresolvable
//! name (no lexical binding, no property on the global object's chain) gives ReferenceError
//! on a read (undefined inside `typeof`), a ReferenceError on a strict store and a new global
//! property on a sloppy store (§6.2.5.6 PutValue step 3).
//!
//! Direct eval code reads `this`, `new.target`, `super.x` (`.home_object`) and the callee
//! (`.callee`) by these names (ep9_bytecode `eval_context.rs`). They are not identifier
//! references, so only the declarative slots that hold them answer: the walk skips `with`
//! objects and `.eval_ext` objects for them, and the global environment gives the global
//! this for `this` and undefined for the others, never a global property. `arguments` is an
//! ordinary name. The only store to an implicit name is `StaLookupSlot "this"`, which
//! BytecodeGen emits for `super()` in direct eval code (E6, M3-P4): it is BindThisValue
//! (§9.1.1.3.1) on the derived constructor's `this` slot (a ReferenceError if `this` is
//! already bound), never a TDZ error.
//!
//! A reference that is used after other code runs (`x = v`, `x op= v`, `x++`, `var x = v`,
//! a destructuring target: §13.15.2, §13.4, §14.3.2.1, §13.15.5, §8.6.3-4 evaluate the
//! reference first) is resolved once by
//! `ResolveLookupSlot`, which returns its environment as a value: the declarative context
//! that binds it (by a slot, or by its `.eval_ext` object), the `with` object, the MODULE
//! record of a module cell, undefined for the global environment, or null for an
//! unresolvable name. `LoadLookupSlotResolved` and
//! `StoreLookupSlotResolved` turn it back into the binding without a walk (`binding_of_env`),
//! so the right-hand side cannot move the reference, and a strict store to a `with` binding
//! that is gone, or to a name that was unresolvable, throws a ReferenceError (§9.1.1.2.5,
//! §6.2.5.6 step 3.a). The environment value lives only in a register of the frame
//! (BytecodeGen temporaries), and no JS code sees it.
//!
//! GetValue of a `with` binding that is gone by then gives undefined in sloppy code and a
//! ReferenceError in strict code (§9.1.1.2.6 step 2), and the strictness is that of the use
//! site (§9.4.2 ResolveBinding step 3), not of the function: a class heritage and a computed
//! key are strict code inside sloppy code (§11.2.2). So the runtime ids take it as an
//! argument (26 and 30, like 31). The opcodes `LdaLookupSlot` and `LdaLookupSlotInsideTypeof`
//! use the strictness of the function (`strict(fp)`), and BytecodeGen emits them only where
//! the two agree; a strict read in a sloppy function (only in a class heritage or computed
//! key) takes ids 29 and 30 instead (`bytecodegen/lookup.rs`).
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
use crate::error::{not_implemented, throw_error};
use crate::frame::{HOLE, NULL, TRUE, UNDEFINED};
use crate::interp::Cx;
use crate::key;
use crate::kinds;
use crate::module;
use crate::object::{self, attr, is_context, is_object, own_get};
use crate::property;
use crate::runtime::{ErrorKind, Root};
use crate::string;
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
    /// An own property of the `.eval_ext` object `ext` in slot 0 of the declarative context
    /// `context` (a var of sloppy direct eval code in a declarative record: no receiver,
    /// deletable).
    EvalExt { context: u64, ext: u64 },
    /// Cell `cell` of the MODULE record `module` (direct eval in module code): an import
    /// (`cell` < 0) or an exported binding.
    Module { module: u64, cell: i32 },
    /// The global environment, which also stands for an unresolvable name.
    Global,
    /// A name that was unresolvable when `ResolveLookupSlot` ran (only from
    /// [`binding_of_env`]): a read throws, a strict store throws, a sloppy store sets a
    /// property of the global object (§6.2.5.6 step 3).
    Unresolvable,
    /// An implicit name (see the module doc) that no context slot binds: the global
    /// environment's answer (GetThisBinding of the global record for `this`).
    GlobalImplicit,
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

/// The names of the implicit bindings of direct eval code (see the module doc).
const IMPLICIT_NAMES: [&[u8]; 4] = [b"this", b"new.target", b".home_object", b".callee"];

/// `name` is one of [`IMPLICIT_NAMES`].
fn is_implicit_name(name: u64) -> bool {
    kinds::is_string(name)
        && IMPLICIT_NAMES
            .iter()
            .any(|t| string::equals_text(name, string::JsStr::Latin1(t)))
}

/// The slot name `id` of unit `u` is the `.eval_ext` binding.
fn is_eval_ext(u: &crate::unit::LoadedUnit, id: usize) -> bool {
    matches!(&u.unit.strings[id], UnitString::Latin1(b) if **b == *EVAL_EXT_BINDING.as_bytes())
}

/// The slot named `name` of the declarative context `c`.
fn slot_binding(c: u64, name: u64) -> Option<Binding> {
    let (u, scope) = object::context_scope(c);
    let i = scope
        .slots
        .iter()
        .position(|s| u.string(s.name.index()) == name)?;
    Some(Binding::Slot {
        context: c,
        index: i as u32,
        mode: scope.slots[i].mode,
    })
}

/// The `.eval_ext` object of the declarative context `c`: slot 0 of a CALLS_SLOPPY_EVAL var
/// scope that is not a script scope, once the first `DeclareEvalVar` or `DeclareEvalFunction`
/// made it.
fn eval_ext(c: u64) -> Option<u64> {
    let (u, scope) = object::context_scope(c);
    if scope.flags.contains(ScopeFlags::CALLS_SLOPPY_EVAL)
        && scope.kind != ScopeKind::Script
        && scope
            .slots
            .first()
            .is_some_and(|s| is_eval_ext(u, s.name.index()))
        && let Some(ext) = object::context_get(c, 0)
        && is_object(ext)
    {
        return Some(ext);
    }
    None
}

/// (M3-P4, CX2-E9-1) `name` is a private name `#x` (or a synthetic `#.` name): no identifier
/// starts with `#`, so only the private names of direct eval code (E9) are looked up by name.
/// `name` can be a rope that is not flattened (a rope that `atoms::intern` made an atom, M4-P1
/// STR1 F3; `key::intern_key` still does, `Cx::intern` atomizes since M4-P2), so the first
/// unit is read with the rope-safe `string::unit_at`. Not a GC point.
fn is_private_name(name: u64) -> bool {
    kinds::is_string(name)
        && string::str_len(name) != 0
        && string::unit_at(name, 0) == u16::from(b'#')
}

/// (CX2-E9-1) The binding of the private name `name` (`#x`) of direct eval code: the slot of
/// the class context that the VM found when it compiled the eval code
/// (`LoadedUnit::eval_private`, `slow/eval.rs` `private_environment`): past the eval code's
/// own contexts (the site context), then `depth` contexts out. A search by name from the
/// current context would find a class that the reference must not see: a class of the eval
/// code whose heritage holds the reference, or the caller's class whose heritage holds the
/// eval (§15.7.14 step 8.b), and its slot (undefined while the heritage runs) would become an
/// ordinary property key. A name with no entry, or a chain that does not reach a context
/// with the slot, throws a TypeError. GC point (a poll).
fn resolve_private(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<Binding> {
    let text = string::to_rust_string(name);
    let entry = info(fp).loaded_unit().and_then(|u| {
        u.eval_private
            .iter()
            .find(|e| text.strip_prefix('#') == Some(e.name.as_str()))
            .map(|e| (u, e.depth))
    });
    if let Some((u, depth)) = entry {
        let mut c = context(fp);
        while is_context(c) && core::ptr::eq(object::context_scope(c).0, u) {
            poll(cx)?;
            c = object::context_previous(c);
        }
        for _ in 0..depth {
            if !is_context(c) {
                break;
            }
            poll(cx)?;
            c = object::context_previous(c);
        }
        if is_context(c)
            && let Some(b) = slot_binding(c, name)
        {
            return Ok(b);
        }
    }
    Err(private_not_found(cx, &text))
}

/// (CX2-E9-1) The TypeError of a private name that no class around the code gives.
fn private_not_found(cx: &mut Cx, text: &str) -> Throw {
    let msg = format!("Private name {text} is not declared by a class around this code");
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// (CX2-E9-1) A private name symbol from a class context slot: a field's `#x` or a brand. Any
/// other value (undefined while the class is not set up) would be an ordinary property key,
/// so it is a TypeError.
fn private_symbol(cx: &mut Cx, name: u64, v: u64) -> Res<u64> {
    if symbol::is_private_key(v) {
        return Ok(v);
    }
    let text = string::to_rust_string(name);
    Err(private_not_found(cx, &text))
}

/// GetIdentifierReference (§9.1.2.1) of the atom `name` from the current context of the
/// frame `fp`, out to the global environment (see the module doc). GC point (an
/// `@@unscopables` getter, a poll).
fn resolve(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<Binding> {
    if is_private_name(name) {
        return resolve_private(cx, fp, name);
    }
    let implicit = is_implicit_name(name);
    let mut c = context(fp);
    while is_context(c) {
        poll(cx)?;
        if object::context_scope(c).1.kind == ScopeKind::With {
            // Only CreateWithContext makes a context of a With scope (`create_context`
            // refuses the other kinds), and slot 0 holds its object; the checks keep the
            // walk safe all the same. An implicit name never asks the object.
            if !implicit
                && let Some(o) = object::context_get(c, 0)
                && is_object(o)
                && with_has_binding(cx, o, name)?
            {
                return Ok(Binding::With(o));
            }
        } else {
            if let Some(b) = slot_binding(c, name) {
                return Ok(b);
            }
            if !implicit
                && let Some(ext) = eval_ext(c)
                && property::has_own_property(cx, ext, name)
            {
                return Ok(Binding::EvalExt { context: c, ext });
            }
        }
        c = object::context_previous(c);
    }
    if !implicit && let Some((module, cell)) = module::cell_of_name(c, name) {
        return Ok(Binding::Module { module, cell });
    }
    Ok(if implicit {
        Binding::GlobalImplicit
    } else {
        Binding::Global
    })
}

/// HasBinding of an object Environment Record with `withEnvironment` true (§9.1.1.2.1).
fn with_has_binding(cx: &mut Cx, o: u64, name: u64) -> Res<bool> {
    if !property::try_has_property(cx, o, name)? {
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
            if !property::try_has_property(cx, o, name)? {
                if strict {
                    return Err(throw_name_now(cx, NameMsg::NotDefined, name));
                }
                return Ok(UNDEFINED);
            }
            property::get(cx, o, name)
        }
        Binding::EvalExt { ext, .. } => property::get(cx, ext, name),
        Binding::Module { module, cell } => match module::cell_get(module, cell) {
            Some(HOLE) => Err(throw_name_now(cx, NameMsg::Tdz, name)),
            Some(v) => Ok(v),
            None => {
                throw_msg_now(cx, "module cell out of range");
                Err(Throw)
            }
        },
        Binding::Unresolvable => Err(throw_name_now(cx, NameMsg::NotDefined, name)),
        Binding::Global => {
            let rt = cx.rt();
            if let Some((v, _)) = own_get(rt.root(Root::Lexical), name) {
                if v == HOLE {
                    return Err(throw_name_now(cx, NameMsg::Tdz, name));
                }
                return Ok(v);
            }
            let g = rt.global();
            if !property::try_has_property(cx, g, name)? {
                if inside_typeof {
                    return Ok(UNDEFINED);
                }
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            property::get(cx, g, name)
        }
        Binding::GlobalImplicit => Ok(
            if string::equals_text(name, string::JsStr::Latin1(IMPLICIT_NAMES[0])) {
                cx.global_this
            } else {
                UNDEFINED
            },
        ),
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
            // BindThisValue (§9.1.1.3.1): `super()` in direct eval code (E6) binds the
            // derived constructor's `this` slot by its name. No source text can assign to
            // `this`, so this is the only store to that name.
            if string::equals_text(name, string::JsStr::Latin1(IMPLICIT_NAMES[0])) {
                if slot_value(cx, context, index)? != HOLE {
                    return Err(throw_error(
                        cx,
                        ErrorKind::ReferenceError,
                        "Super constructor may only be called once",
                    ));
                }
                object::context_set(cx.rt(), context, index, v);
                return Ok(());
            }
            // The TDZ check (ReferenceError) comes before the TypeError of a `const`.
            if slot_value(cx, context, index)? == HOLE {
                return Err(throw_name_now(cx, NameMsg::Tdz, name));
            }
            if mode == VariableMode::Const {
                return Err(const_assign(cx));
            }
            // A named function expression's own name: immutable, not strict (§9.1.1.1.5
            // step 5.b): ignored in sloppy code.
            if mode == VariableMode::FunctionName {
                return if strict {
                    Err(const_assign(cx))
                } else {
                    Ok(())
                };
            }
            object::context_set(cx.rt(), context, index, v);
            Ok(())
        }
        Binding::With(o) => {
            // The binding may be gone since the reference was resolved (a getter of
            // `@@unscopables` deleted it): strict code throws, sloppy code sets it again.
            if strict && !property::try_has_property(cx, o, name)? {
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            property::set(cx, o, name, v, strict)
        }
        Binding::EvalExt { ext, .. } => property::set(cx, ext, name, v, strict),
        Binding::Module { module, cell } => {
            // An import is immutable (CreateImportBinding), as `StaModuleVariable`.
            if cell < 0 {
                return Err(const_assign(cx));
            }
            match module::cell_get(module, cell) {
                Some(HOLE) => Err(throw_name_now(cx, NameMsg::Tdz, name)),
                Some(_) => Err(not_implemented(
                    cx,
                    "assignment to an exported module binding in direct eval code",
                )),
                None => {
                    throw_msg_now(cx, "module cell out of range");
                    Err(Throw)
                }
            }
        }
        Binding::Unresolvable => {
            if strict {
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            let g = cx.rt().global();
            property::set(cx, g, name, v, false)
        }
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
            if strict && !property::try_has_property(cx, g, name)? {
                return Err(throw_name_now(cx, NameMsg::NotDefined, name));
            }
            property::set(cx, g, name, v, strict)
        }
        Binding::GlobalImplicit => {
            // No bytecode stores to an implicit name.
            throw_msg_now(cx, "store to an implicit binding");
            Err(Throw)
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
/// the binding `name` (ResolveBinding, then GetValue), in code as strict as the function (see
/// the module doc).
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
        let v = get_binding_value(cx, b, name, strict(fp), inside_typeof)?;
        // (CX2-E9-1) Only a private field's key is read this way (E9).
        if is_private_name(name) {
            return private_symbol(cx, name, v);
        }
        Ok(v)
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
        Binding::Slot { .. } | Binding::Module { .. } | Binding::GlobalImplicit => false,
        Binding::With(o) | Binding::EvalExt { ext: o, .. } => property::delete(cx, o, name, false)?,
        // `resolve` never gives it.
        Binding::Unresolvable => true,
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

/// `LoadLookupSlotForCall(name, strict)` (runtime id 26, a pair; `slow::call_runtime_pair`
/// writes the two results to the `CallRuntimeForPair` output registers): the function bound
/// to `name` (GetValue; `strict` is the boolean strictness of the call site), and the receiver
/// of the call, which is the `with` object that holds `name`, else undefined (§13.3.6.1 step
/// 1.b, §9.1.1.2.10 WithBaseObject).
pub(crate) fn load_lookup_slot_for_call(
    cx: &mut Cx,
    fp: *mut u64,
    name: u64,
    strict: u64,
) -> Res<(u64, u64)> {
    let name = name_key(cx, name)?;
    let b = resolve(cx, fp, name)?;
    let f = get_binding_value(cx, b, name, strict == TRUE, false)?;
    let receiver = match b {
        Binding::With(o) => o,
        _ => UNDEFINED,
    };
    Ok((f, receiver))
}

/// `ResolveLookupSlot(name)` (runtime id 29): ResolveBinding (§9.4.2) of `name` without
/// GetValue. Returns the environment of the reference (see the module doc): the declarative
/// context that binds it, the `with` object, the MODULE record of a module cell, undefined
/// for the global environment (the script lexical record has it, or HasProperty of the
/// global object, §9.1.1.4.1), or null for an unresolvable name. GC point (an `@@unscopables` getter, a poll, a proxy trap).
pub(crate) fn resolve_lookup_slot(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<u64> {
    let name = name_key(cx, name)?;
    Ok(match resolve(cx, fp, name)? {
        Binding::Slot { context, .. } | Binding::EvalExt { context, .. } => context,
        Binding::Module { module, .. } => module,
        Binding::With(o) => o,
        Binding::GlobalImplicit => UNDEFINED,
        Binding::Global | Binding::Unresolvable => {
            let rt = cx.rt();
            let g = rt.global();
            if own_get(rt.root(Root::Lexical), name).is_some()
                || property::try_has_property(cx, g, name)?
            {
                UNDEFINED
            } else {
                NULL
            }
        }
    })
}

/// The binding of `name` in the environment `env` that [`resolve_lookup_slot`] gave, without
/// a walk. A declarative context binds `name` by a slot or, failing that, by its `.eval_ext`
/// object (whose property may be gone since: then a read gives undefined and a store makes
/// it again, as the walk would have found it there).
fn binding_of_env(cx: &mut Cx, env: u64, name: u64) -> Res<Binding> {
    if env == UNDEFINED {
        return Ok(if is_implicit_name(name) {
            Binding::GlobalImplicit
        } else {
            Binding::Global
        });
    }
    if env == NULL {
        return Ok(Binding::Unresolvable);
    }
    if let Some((module, cell)) = module::cell_of_name(env, name) {
        return Ok(Binding::Module { module, cell });
    }
    if is_context(env) {
        if let Some(b) = slot_binding(env, name) {
            return Ok(b);
        }
        if let Some(ext) = eval_ext(env) {
            return Ok(Binding::EvalExt { context: env, ext });
        }
    } else if is_object(env) {
        return Ok(Binding::With(env));
    }
    throw_msg_now(cx, "not an environment of ResolveLookupSlot");
    Err(Throw)
}

/// `LoadLookupSlotResolved(env, name, strict)` (runtime id 30): GetValue (§6.2.5.5) of the
/// reference that [`resolve_lookup_slot`] resolved to `env`; `strict` is the boolean
/// strictness of the read. GC point.
pub(crate) fn load_lookup_slot_resolved(cx: &mut Cx, env: u64, name: u64, strict: u64) -> Res<u64> {
    let name = name_key(cx, name)?;
    let b = binding_of_env(cx, env, name)?;
    let v = get_binding_value(cx, b, name, strict == TRUE, false)?;
    // (CX2-E9-1) A brand that eval code reads (E9) must be a private name symbol.
    if [BRAND, STATIC_BRAND]
        .iter()
        .any(|t| string::equals_text(name, string::JsStr::Latin1(t)))
    {
        return private_symbol(cx, name, v);
    }
    Ok(v)
}

/// The brand slots of a class context (`ep9_frontend` `bytecodegen/class_fields.rs`), which
/// eval code reads by name (E9, ep9_bytecode `eval_context.rs`).
const BRAND: &[u8] = b"#.brand";
const STATIC_BRAND: &[u8] = b"#.static-brand";

/// `StoreLookupSlotResolved(env, name, value, strict)` (runtime id 31): PutValue (§6.2.5.6) of
/// `value` to the reference that [`resolve_lookup_slot`] resolved to `env`; `strict` is the
/// boolean strictness of the store. Returns `value`. GC point (a setter).
pub(crate) fn store_lookup_slot_resolved(
    cx: &mut Cx,
    env: u64,
    name: u64,
    value: u64,
    strict: u64,
) -> Res<u64> {
    let name = name_key(cx, name)?;
    let b = binding_of_env(cx, env, name)?;
    set_binding_value(cx, b, name, value, strict == TRUE)?;
    Ok(value)
}
