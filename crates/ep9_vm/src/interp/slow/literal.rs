//! Closures, contexts and literals.

use ep9_bytecode::{Constant, ScopeKind};

use crate::conv::{self, Res};
use crate::error::{not_implemented, stack_overflow};
use crate::frame::{HOLE, UNDEFINED};
use crate::interp::Cx;
use crate::object::{self, own_define};
use crate::property;
use crate::runtime::{Atom, Root, val};
use crate::unit::FunctionInfo;

use super::runtime::copy_data_properties;
use super::{context, enter, info, ret, throw_msg_now};

// ---- closures, contexts, literals ----------------------------------------------------------

/// `CreateClosure(function)`: a function cell for nested function `k` with the current context.
#[cold]
#[inline(never)]
pub(crate) fn create_closure(cx: &mut Cx, fp: *mut u64, pc: *const u8, k: u32) -> u64 {
    enter(cx, fp, pc);
    let fi = info(fp);
    let Constant::Function(id) = fi.constant_entry(k) else {
        return throw_msg_now(cx, "CreateClosure constant is not a function");
    };
    let u = fi.loaded_unit().expect("a bytecode function");
    let target: *const FunctionInfo = &u.infos[id.index()];
    object::new_function(cx.rt(), target, context(fp))
}

/// The kinds of `Create*Context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CtxKind {
    Function,
    Block,
    Catch,
    Eval,
    /// `CreateWithContext` (slow/lookup.rs): one slot, the object. Its scope descriptor (kind
    /// With) has no slots, so a name walk never reads names for it (an object environment).
    With,
}

/// `Create{Function,Block,Catch,Eval,With}Context`: a new context whose previous context is
/// the current one. `slots` is the slot count operand (0 for block, catch and with: the scope
/// or the kind says). `first` goes to slot 0 of a catch context (the exception) or a with
/// context (the object).
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_context(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    kind: CtxKind,
    k: u32,
    slots: u32,
    first: u64,
) -> u64 {
    enter(cx, fp, pc);
    let fi = info(fp);
    let Constant::Scope(id) = fi.constant_entry(k) else {
        return throw_msg_now(cx, "context constant is not a scope");
    };
    let u = fi.loaded_unit().expect("a bytecode function");
    let unit_scope = &u.unit.scopes[id.index()];
    // The name walk reads slot 0 of a With scope's context as the binding object.
    if (kind == CtxKind::With) != (unit_scope.kind == ScopeKind::With) {
        return throw_msg_now(cx, "CreateWithContext and With scopes go together");
    }
    let scope_len = unit_scope.slots.len();
    let len = match kind {
        CtxKind::Function | CtxKind::Eval => slots as usize,
        CtxKind::Block => scope_len,
        CtxKind::Catch | CtxKind::With => scope_len.max(1),
    };
    let scope = &u.context_scopes[id.index()];
    let c = object::new_context(cx.rt(), context(fp), scope, len);
    if matches!(kind, CtxKind::Catch | CtxKind::With) {
        object::context_set(cx.rt(), c, 0, first);
    }
    c
}

/// Literal kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lit {
    Array,
    Object,
    Template,
    EmptyArray,
    EmptyObject,
}

/// `Create{Empty,}ArrayLiteral`, `Create{Empty,}ObjectLiteral`, `GetTemplateObject` (the
/// boilerplate or description is constant `k`).
#[cold]
#[inline(never)]
pub(crate) fn literal(cx: &mut Cx, fp: *mut u64, pc: *const u8, kind: Lit, k: u32) -> u64 {
    enter(cx, fp, pc);
    let r = match kind {
        Lit::Array => array_literal(cx, fp, k),
        Lit::Object => object_literal(cx, fp, k),
        Lit::Template => template_object(cx, fp, k),
        Lit::EmptyArray => Ok(object::new_array(cx.rt(), 0)),
        Lit::EmptyObject => Ok(object::new_plain_object(cx.rt())),
    };
    ret(r)
}

fn array_literal(cx: &mut Cx, fp: *mut u64, k: u32) -> Res<u64> {
    array_from_boilerplate(cx, info(fp), k)
}

fn object_literal(cx: &mut Cx, fp: *mut u64, k: u32) -> Res<u64> {
    object_from_boilerplate(cx, info(fp), k)
}

/// The value of boilerplate element constant `ci`: a primitive constant, or a new array or
/// object for a nested boilerplate (`[[1, 2]]`, `{a: {b: 1}}`; the pool refers to the inner
/// boilerplate by index). `None` = an elision (the hole).
fn boilerplate_value(cx: &mut Cx, fi: &FunctionInfo, ci: u32) -> Res<Option<u64>> {
    match fi.constant_entry(ci) {
        Constant::ArrayBoilerplate(_) => array_from_boilerplate(cx, fi, ci).map(Some),
        Constant::ObjectBoilerplate(_) => object_from_boilerplate(cx, fi, ci).map(Some),
        Constant::BigInt(_) => Err(not_implemented(cx, "BigInt values")),
        _ => {
            let v = fi.constant(ci);
            Ok((v != HOLE).then_some(v))
        }
    }
}

fn array_from_boilerplate(cx: &mut Cx, fi: &FunctionInfo, k: u32) -> Res<u64> {
    let rt = cx.rt();
    // Nested boilerplates recurse once per literal level (bounded by the front-end depth cap).
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let Constant::ArrayBoilerplate(elems) = fi.constant_entry(k) else {
        return Err(not_implemented(cx, "array literal constant"));
    };
    let n = elems.len();
    // `a` is a Rust local: the conservative stack scan keeps it alive across the nested
    // allocations below.
    let a = object::new_array(rt, n);
    for (i, e) in elems.iter().enumerate() {
        let Some(ci) = e else { continue };
        if let Some(v) = boilerplate_value(cx, fi, ci.0)? {
            object::array_set(rt, a, i as u32, v);
        }
    }
    object::array_set_length(rt, a, n as u32);
    Ok(a)
}

fn object_from_boilerplate(cx: &mut Cx, fi: &FunctionInfo, k: u32) -> Res<u64> {
    let rt = cx.rt();
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let Constant::ObjectBoilerplate(props) = fi.constant_entry(k) else {
        return Err(not_implemented(cx, "object literal constant"));
    };
    // One in-object slot per named key (H17 v0); index keys go to dictionary mode anyway.
    let named = props
        .iter()
        .filter(|(kc, _)| {
            let c = fi.constant(kc.0);
            object::is_string(c) && crate::key::array_index_of(c).is_none()
        })
        .count();
    let o = if named == 0 {
        object::new_plain_object(rt)
    } else {
        object::new_object_with_capacity(rt, rt.root(Root::ObjectPrototype), named)
    };
    for (kc, vc) in props.iter() {
        let key = conv::to_property_key(cx, fi.constant(kc.0))?;
        let v = match vc {
            Some(c) => boilerplate_value(cx, fi, c.0)?.unwrap_or(UNDEFINED),
            None => UNDEFINED,
        };
        property::define(cx, o, key, v)?;
    }
    Ok(o)
}

/// GetTemplateObject (§13.2.8.4): one frozen array per site (v0: not frozen), cached per
/// function and constant.
fn template_object(cx: &mut Cx, fp: *mut u64, k: u32) -> Res<u64> {
    let rt = cx.rt();
    let fi = info(fp);
    let key = (core::ptr::from_ref(fi).addr(), k);
    if let Some(h) = rt.templates.borrow().get(&key) {
        return Ok(rt.heap.global(*h).to_bits());
    }
    let Constant::TemplateObject(parts) = fi.constant_entry(k) else {
        return Err(not_implemented(cx, "template object constant"));
    };
    let u = fi.loaded_unit().expect("a bytecode function");
    let cooked = object::new_array(rt, parts.len());
    for (i, (_, c)) in parts.iter().enumerate() {
        let v = c.map_or(UNDEFINED, |s| u.string(s.index()));
        object::array_set(rt, cooked, i as u32, v);
    }
    object::array_set_length(rt, cooked, parts.len() as u32);
    let raw = object::new_array(rt, parts.len());
    for (i, (r, _)) in parts.iter().enumerate() {
        object::array_set(rt, raw, i as u32, u.string(r.index()));
    }
    object::array_set_length(rt, raw, parts.len() as u32);
    own_define(rt, cooked, rt.atom(Atom::Raw), raw, 0);
    let h = rt.heap.new_global(val(cooked));
    rt.templates.borrow_mut().insert(key, h);
    Ok(cooked)
}

/// `CloneObject(source)`: `{...source}`.
#[cold]
#[inline(never)]
pub(crate) fn clone_object(cx: &mut Cx, fp: *mut u64, pc: *const u8, source: u64) -> u64 {
    enter(cx, fp, pc);
    let o = object::new_plain_object(cx.rt());
    ret(copy_data_properties(cx, o, source, UNDEFINED).map(|_| o))
}
