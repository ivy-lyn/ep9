//! Direct eval in the VM (owner P3-EVAL-VM): runtime id 7 `ResolvePossiblyDirectEval`, and
//! (ABI 3) ids 27-28 `DeclareEvalVar` and `DeclareEvalFunction`. The compiler is reached
//! through `IsolateConfig::compiler` (`crate::CompilerHooks`, C2); the shared steps (source
//! text, compile failures, loading the unit) are `abi/script.rs`.
//!
//! ```text
//! eval(src) in code:  [LoadLookupSlotForCall eval -> callee, receiver]  (a Dynamic `eval`, L5)
//!                     CallRuntime ResolvePossiblyDirectEval(callee, src, function, scope, pos)
//!   callee is not %eval%, or src is not a string -> callee: the call goes on as written
//!   else: EvalContext (below) -> compile_eval hook -> load the unit
//!         -> a closure of function 0 over the caller's current context
//!   Call(closure or callee, receiver, args)
//! sloppy eval code, before its body:
//!   DeclareEvalFunction(name, closure) per function, DeclareEvalVar(name) per var
//!     -> the variable environment (below): a `.eval_ext` object, or the global object
//! ```
//!
//! The [`EvalContext`] of a direct eval (PerformEval steps 4-11, §19.2.1.1):
//!
//! | Field | From |
//! |---|---|
//! | `strict` | `ScopeFlags::STRICT` of the `scope` operand: the unit scope at the call site, so class code inside a sloppy function is strict (M2-P2-L6) |
//! | `in_function`, `in_method`, `in_derived_ctor`, `in_class_field_init` (and the `*_ok` flags) | the kind of the `function` operand's unit function, past arrows along `parent` (GetThisEnvironment). Function 0 of a script or module: global code, all false. Function 0 of eval code: the eval's own caller, by the names of the receiver function's context slots (`this`, `new.target`, `.home_object`, `.callee`), which the front end puts in a context for every direct eval site (scope/resolve.rs) |
//! | `private_names` | the `#x` slots of the Class scopes on the `scope` operand's `outer` chain (without the `#`), and for eval code also those of the Class scope contexts on the runtime chain past the eval unit's own contexts |
//!
//! One rule is approximate: a direct eval in eval code that a class field initializer called
//! sees `in_class_field_init` false (slot names cannot tell a field initializer from a
//! method). `arguments` in such nested eval code is then not an early error.
//!
//! EvalDeclarationInstantiation (§19.2.1.3) is split over the two runtime ids, one name per
//! call (the order of `functionsToInitialize`, then the var names). Each call does the checks
//! of its name and then creates its binding, so a conflict of a later name leaves the earlier
//! bindings made (the spec checks every name first; the names of the other calls are not
//! available here: an ABI 5 candidate if a test needs it).

use ep9_bytecode::{
    CodeKind, EVAL_EXT_BINDING, EvalContext, FunctionKind, ScopeFlags, ScopeKind, UnitString,
    VariableMode,
};
use ep9_value::Throw;

use crate::abi::{self, Local};
use crate::conv::Res;
use crate::descriptor::Desc;
use crate::error::throw_error;
use crate::frame::{UNDEFINED, int32_of};
use crate::interp::Cx;
use crate::key::{self, key_text};
use crate::object::{
    self, attr, is_context, is_function, is_object, is_string, own_define, own_get,
};
use crate::property;
use crate::runtime::{ErrorKind, Root};
use crate::unit::LoadedUnit;

use super::{context, info, interrupt, throw_msg_now};

/// One budget unit per context of a walk; polls when the budget runs out (M2-P2-POLL): the
/// walks have no back edge.
#[inline]
fn poll(cx: &mut Cx) -> Res<()> {
    cx.budget -= 1;
    if cx.budget <= 0 && interrupt(cx, core::ptr::null_mut()) == 0 {
        return Err(Throw);
    }
    Ok(())
}

/// Throws the internal error "ep9 internal error: `msg`".
fn internal(cx: &mut Cx, msg: &str) -> Throw {
    throw_msg_now(cx, msg);
    Throw
}

/// The text of slot `i` of `scope` in the unit `u`, if it is Latin-1 (every name this file
/// compares is).
fn slot_name<'u>(u: &'u LoadedUnit, scope: &ep9_bytecode::Scope, i: usize) -> Option<&'u [u8]> {
    match &u.unit.strings[scope.slots[i].name.index()] {
        UnitString::Latin1(b) => Some(b),
        UnitString::Utf16(_) => None,
    }
}

/// The slot of `scope` named `text`.
fn has_slot(u: &LoadedUnit, scope: &ep9_bytecode::Scope, text: &[u8]) -> bool {
    (0..scope.slots.len()).any(|i| slot_name(u, scope, i) == Some(text))
}

/// Adds the private names of a Class scope (`#x` slots, without the `#`; the synthetic
/// `#.brand`-style names are left out: no source text can spell them).
fn add_private_names(u: &LoadedUnit, scope: &ep9_bytecode::Scope, out: &mut Vec<String>) {
    if scope.kind != ScopeKind::Class {
        return;
    }
    for i in 0..scope.slots.len() {
        let text = match &u.unit.strings[scope.slots[i].name.index()] {
            UnitString::Latin1(b) => b.iter().map(|&c| char::from(c)).collect::<String>(),
            UnitString::Utf16(w) => String::from_utf16_lossy(w),
        };
        if let Some(name) = text.strip_prefix('#')
            && !name.starts_with('.')
            && !out.iter().any(|n| n == name)
        {
            out.push(name.to_owned());
        }
    }
}

/// The receiver flags of a function kind (PerformEval step 8.b): `in_function`, `in_method`,
/// `in_derived_ctor`, `in_class_field_init`.
fn set_receiver(ctx: &mut EvalContext, kind: FunctionKind) {
    ctx.in_function = true;
    ctx.in_method = matches!(
        kind,
        FunctionKind::Method
            | FunctionKind::Getter
            | FunctionKind::Setter
            | FunctionKind::ClassConstructor
            | FunctionKind::DerivedConstructor
            | FunctionKind::ClassFieldsInitializer
    );
    ctx.in_derived_ctor = kind == FunctionKind::DerivedConstructor;
    ctx.in_class_field_init = kind == FunctionKind::ClassFieldsInitializer;
}

/// The [`EvalContext`] of a direct eval at the call site of the frame `fp`: `function` is the
/// running closure and `scope` the unit scope id of the site (see the module doc). GC point
/// (a poll).
fn eval_context(cx: &mut Cx, fp: *mut u64, function: u64, scope: u64) -> Res<EvalContext> {
    if !is_function(function) {
        return Err(internal(cx, "ResolvePossiblyDirectEval: not a function"));
    }
    let fi = object::function_info(function);
    let Some(u) = fi.loaded_unit() else {
        return Err(internal(
            cx,
            "ResolvePossiblyDirectEval: not a bytecode function",
        ));
    };
    let unit = &u.unit;
    let Some(site) = usize::try_from(int32_of(scope))
        .ok()
        .and_then(|i| unit.scopes.get(i))
    else {
        return Err(internal(
            cx,
            "ResolvePossiblyDirectEval: scope out of range",
        ));
    };
    let mut ctx = EvalContext {
        strict: site.flags.contains(ScopeFlags::STRICT),
        direct: true,
        ..EvalContext::default()
    };
    // The static scope chain of the site, in its own unit.
    let mut s = Some(site);
    while let Some(sc) = s {
        add_private_names(u, sc, &mut ctx.private_names);
        s = sc.outer.and_then(|o| unit.scopes.get(o.index()));
    }
    // GetThisEnvironment: the nearest function that is not an arrow.
    let mut id = fi.function_id as usize;
    while unit.functions[id].kind == FunctionKind::Arrow {
        match unit.functions[id].parent {
            Some(p) => id = p.index(),
            None => break,
        }
    }
    let kind = unit.functions[id].kind;
    let eval_code = unit.header.code_kind == CodeKind::Eval;
    match kind {
        FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval | FunctionKind::Arrow => {}
        k => set_receiver(&mut ctx, k),
    }
    if eval_code {
        // Past this unit's own contexts, the chain is the eval caller's (and further out).
        let receiver_known = ctx.in_function;
        outer_context(cx, fp, u, &mut ctx, receiver_known)?;
    }
    ctx.new_target_ok = ctx.in_function;
    ctx.super_property_ok = ctx.in_method;
    ctx.super_call_ok = ctx.in_derived_ctor;
    Ok(ctx)
}

/// For a direct eval inside eval code `u`: the private names of the Class scope contexts past
/// the contexts of `u` on the runtime chain, and (unless `receiver_known`) the receiver flags
/// from the first context there that binds `this` (see the module doc).
fn outer_context(
    cx: &mut Cx,
    fp: *mut u64,
    u: &LoadedUnit,
    ctx: &mut EvalContext,
    mut receiver_known: bool,
) -> Res<()> {
    let mut c = context(fp);
    while is_context(c) {
        poll(cx)?;
        let (cu, scope) = object::context_scope(c);
        if !core::ptr::eq(cu, u) {
            add_private_names(cu, scope, &mut ctx.private_names);
            if !receiver_known && has_slot(cu, scope, b"this") {
                receiver_known = true;
                // A function's own `this`; a script context or indirect eval code binds
                // `this` too, but never `new.target` (global code).
                if scope.kind == ScopeKind::Function && has_slot(cu, scope, b"new.target") {
                    ctx.in_function = true;
                    ctx.in_method = has_slot(cu, scope, b".home_object");
                    ctx.in_derived_ctor = has_slot(cu, scope, b".callee");
                }
            }
        }
        c = object::context_previous(c);
    }
    Ok(())
}

/// `ResolvePossiblyDirectEval(callee, source, function, scope, position)` (runtime id 7,
/// §13.3.6.1 step 6): if `callee` is %eval% and `source` a string, compiles the direct eval
/// code (PerformEval with `direct` true) and returns a closure of its function 0 over the
/// current context of the frame `fp`; else returns `callee`. GC point.
pub(crate) fn resolve_possibly_direct_eval(
    cx: &mut Cx,
    fp: *mut u64,
    callee: u64,
    source: u64,
    function: u64,
    scope: u64,
    _position: u64,
) -> Res<u64> {
    let eval = abi::Cx::new(cx).eval_intrinsic();
    if callee != eval || callee == UNDEFINED || !is_string(source) {
        return Ok(callee);
    }
    let ctx = eval_context(cx, fp, function, scope)?;
    let current = context(fp);
    let mut acx = abi::Cx::new(cx);
    let f = acx.direct_eval(Local::from_bits(source), &ctx, current)?;
    Ok(f.bits())
}

// ---- EvalDeclarationInstantiation ------------------------------------------------------------

/// The variable environment of running sloppy eval code (`varEnv`).
#[derive(Clone, Copy)]
enum VarEnv {
    /// The global environment: the script lexical record and the global object.
    Global,
    /// A function var scope with a sloppy direct eval: its context, whose slot 0 holds the
    /// `.eval_ext` object (or undefined before the first declaration).
    Function(u64),
}

/// The `.eval_ext` slot 0 of the declarative context `c` (a CALLS_SLOPPY_EVAL var scope that
/// is not a script scope), as the lookup walk reads it (`slow/lookup.rs` `eval_ext`).
fn is_var_scope(u: &LoadedUnit, scope: &ep9_bytecode::Scope) -> bool {
    scope.flags.contains(ScopeFlags::CALLS_SLOPPY_EVAL)
        && scope.kind != ScopeKind::Script
        && !scope.slots.is_empty()
        && slot_name(u, scope, 0) == Some(EVAL_EXT_BINDING.as_bytes())
}

/// The slot of the declarative context `c` named `name` (an atom): its index and mode.
fn slot_of(c: u64, name: u64) -> Option<(u32, VariableMode)> {
    let (u, scope) = object::context_scope(c);
    let i = scope
        .slots
        .iter()
        .position(|s| u.string(s.name.index()) == name)?;
    Some((i as u32, scope.slots[i].mode))
}

/// A lexical mode (`let`, `const`, `class`): a var of the same name conflicts with it.
fn is_lexical(m: VariableMode) -> bool {
    matches!(
        m,
        VariableMode::Let | VariableMode::Const | VariableMode::Class
    )
}

/// The SyntaxError of a var or function name that a lexical binding on the way holds.
fn redeclared(cx: &mut Cx, name: u64) -> Throw {
    let msg = format!(
        "Identifier '{}' has already been declared",
        key_text(cx.rt(), name)
    );
    throw_error(cx, ErrorKind::SyntaxError, &msg)
}

/// EvalDeclarationInstantiation step 3 for `name`, and the variable environment: walks from
/// the eval code's lexical environment (its caller's current context; the eval code's own
/// context, made by `CreateEvalContext` before this call, is skipped) out to the variable
/// environment. A declarative context on the way that binds `name` is a SyntaxError, except
/// a catch clause's (B.3.4); a `with` object is not checked. At the global environment, a
/// lexical declaration of `name` is a SyntaxError (step 3.a). GC point (a poll).
fn var_env(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<VarEnv> {
    let own = info(fp).loaded_unit().map(core::ptr::from_ref);
    let mut c = context(fp);
    if is_context(c) {
        let (u, scope) = object::context_scope(c);
        if scope.kind == ScopeKind::Eval && Some(core::ptr::from_ref(u)) == own {
            c = object::context_previous(c);
        }
    }
    while is_context(c) {
        poll(cx)?;
        let (u, scope) = object::context_scope(c);
        if scope.kind == ScopeKind::With {
            c = object::context_previous(c);
            continue;
        }
        if is_var_scope(u, scope) {
            // The var scope itself: its lexical slots conflict too (a function's top-level
            // `let` shares the scope with its vars).
            if let Some((_, mode)) = slot_of(c, name)
                && is_lexical(mode)
            {
                return Err(redeclared(cx, name));
            }
            return Ok(VarEnv::Function(c));
        }
        if scope.kind != ScopeKind::Catch && slot_of(c, name).is_some() {
            return Err(redeclared(cx, name));
        }
        c = object::context_previous(c);
    }
    if own_get(cx.rt().root(Root::Lexical), name).is_some() {
        return Err(redeclared(cx, name));
    }
    Ok(VarEnv::Global)
}

/// The `.eval_ext` object of the var scope context `c`, made on the first call (a null-proto
/// object, eval_context.rs). GC point.
fn eval_ext(cx: &mut Cx, c: u64) -> u64 {
    if let Some(ext) = object::context_get(c, 0)
        && is_object(ext)
    {
        return ext;
    }
    let rt = cx.rt();
    let ext = object::new_object(rt, crate::frame::NULL);
    object::context_set(rt, c, 0, ext);
    ext
}

/// A var binding in the `.eval_ext` object `ext`: CreateMutableBinding(N, true) +
/// InitializeBinding(N, `v`) of a declarative record (a deletable binding).
fn define_ext(cx: &mut Cx, ext: u64, name: u64, v: u64) {
    own_define(cx.rt(), ext, name, v, attr::DEFAULT);
}

/// The TypeError of CanDeclareGlobalVar / CanDeclareGlobalFunction (`redefine`: the property
/// exists).
fn cannot_declare(cx: &mut Cx, name: u64, redefine: bool) -> Throw {
    let n = key_text(cx.rt(), name);
    let msg = if redefine {
        format!("Cannot redefine property: {n}")
    } else {
        format!("Cannot add property {n}, object is not extensible")
    };
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// `DeclareEvalVar(name)` (runtime id 27): a `var` of sloppy eval code (§19.2.1.3 steps 3,
/// 11 and 18). In a function: a new binding in the `.eval_ext` object of its var scope
/// context unless the scope or that object binds `name`; at the global level: CanDeclareGlobalVar
/// (TypeError), then CreateGlobalVarBinding(name, true), a configurable property. GC point.
pub(crate) fn declare_eval_var(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<u64> {
    let name = key::to_property_key(cx, name)?;
    match var_env(cx, fp, name)? {
        VarEnv::Function(c) => {
            if slot_of(c, name).is_none() {
                let ext = eval_ext(cx, c);
                if !property::has_own_property(cx, ext, name) {
                    define_ext(cx, ext, name, UNDEFINED);
                }
            }
        }
        VarEnv::Global => {
            let g = cx.rt().global();
            let has = property::has_own_property(cx, g, name);
            if !has {
                if !property::is_extensible(g) {
                    return Err(cannot_declare(cx, name, false));
                }
                let desc = Desc::data(UNDEFINED, attr::DEFAULT);
                if !property::define_own_property(cx, g, name, desc)? {
                    return Err(cannot_declare(cx, name, true));
                }
            }
        }
    }
    Ok(UNDEFINED)
}

/// `DeclareEvalFunction(name, function)` (runtime id 28): a function declaration of sloppy
/// eval code (§19.2.1.3 steps 3, 9 and 17). In a function: SetMutableBinding of an existing
/// var binding (a slot of the var scope or a property of its `.eval_ext` object), else a new
/// `.eval_ext` binding; at the global level: CanDeclareGlobalFunction (TypeError), then
/// CreateGlobalFunctionBinding(name, function, true). GC point (a setter on the global
/// object).
pub(crate) fn declare_eval_function(
    cx: &mut Cx,
    fp: *mut u64,
    name: u64,
    function: u64,
) -> Res<u64> {
    let name = key::to_property_key(cx, name)?;
    match var_env(cx, fp, name)? {
        VarEnv::Function(c) => {
            if let Some((i, _)) = slot_of(c, name) {
                object::context_set(cx.rt(), c, i, function);
            } else {
                let ext = eval_ext(cx, c);
                if property::has_own_property(cx, ext, name) {
                    property::set(cx, ext, name, function, false)?;
                } else {
                    define_ext(cx, ext, name, function);
                }
            }
        }
        VarEnv::Global => {
            let g = cx.rt().global();
            // CanDeclareGlobalFunction (§9.1.1.4.16), then CreateGlobalFunctionBinding
            // (§9.1.1.4.18) with D = true.
            let desc = match property::get_own_property(cx, g, name) {
                None => {
                    if !property::is_extensible(g) {
                        return Err(cannot_declare(cx, name, false));
                    }
                    Desc::data(function, attr::DEFAULT)
                }
                Some(d) if d.configurable == Some(true) => Desc::data(function, attr::DEFAULT),
                Some(d)
                    if !d.is_accessor()
                        && d.writable == Some(true)
                        && d.enumerable == Some(true) =>
                {
                    Desc {
                        value: Some(function),
                        ..Desc::default()
                    }
                }
                Some(_) => return Err(cannot_declare(cx, name, true)),
            };
            if !property::define_own_property(cx, g, name, desc)? {
                return Err(cannot_declare(cx, name, true));
            }
            property::set(cx, g, name, function, false)?;
        }
    }
    Ok(UNDEFINED)
}
