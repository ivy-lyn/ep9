//! Direct eval in the VM (owner P3-EVAL-VM): runtime id 7 `ResolvePossiblyDirectEval`,
//! (ABI 3) ids 27-28 `DeclareEvalVar` and `DeclareEvalFunction`, and (ABI 5, M3-P2 VM-ARGS)
//! ids 46-47 `DeclareEvalAnnexBFunction` and `StoreEvalAnnexBFunction`. The compiler is reached
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
//!   then DeclareEvalAnnexBFunction(name) per Annex B block function name -> true or false
//! a block function declaration whose name got true, when it runs:
//!   StoreEvalAnnexBFunction(name, function) -> the variable environment
//! ```
//!
//! The [`EvalContext`] of a direct eval (PerformEval steps 4-11, §19.2.1.1):
//!
//! | Field | From |
//! |---|---|
//! | `strict` | `ScopeFlags::STRICT` of the `scope` operand: the unit scope at the call site, so class code inside a sloppy function is strict (M2-P2-L6) |
//! | `in_function`, `in_method`, `in_derived_ctor`, `in_class_field_init` (and the `*_ok` flags) | the kind of the `function` operand's unit function, past arrows along `parent` (GetThisEnvironment). Function 0 of a script or module: global code, all false. Function 0 of eval code: the flags the eval unit was compiled with (`LoadedUnit::eval_receiver`, set from the caller's `EvalContext` by `direct_eval`; all false for indirect eval), so a class field initializer's `in_class_field_init` holds through nested eval |
//! | `private_names` | the `#x` slots of the Class scopes on the `scope` operand's `outer` chain (without the `#`), and for eval code also those of the Class scope contexts on the runtime chain past the eval unit's own contexts |
//!
//! EvalDeclarationInstantiation (§19.2.1.3) is split over the two runtime ids, one name per
//! call (the order of `functionsToInitialize`, then the var names). The spec checks every name
//! (steps 3, 9, 11) before it creates any binding, so a conflict of a later name must leave
//! no binding made. The first call of a run therefore reads the whole prologue of function 0
//! (`gen_eval_program`: `LdaConstant name; Star r; CallRuntime DeclareEval*, r..`, straight-line
//! code before the body) and checks every name; then each call creates its own binding.
//!
//! Not built yet (needs ABI 5): step 3.d over the parameter environment of a sloppy function
//! with parameter expressions. FunctionDeclarationInstantiation step 20 puts its parameters in
//! an environment of their own, between an eval in the parameters and the variable
//! environment, so `function f(a = eval('var a')) {}` is a SyntaxError. The front end merges
//! that environment into the function's var scope, and no unit field tells this scope from
//! the one of `function f({a}) { eval('var a'); { let z } }` (same scopes, same flags), where
//! the var is allowed. So such a var or function name is not a conflict here.

use std::collections::HashSet;

use ep9_bytecode::{
    CodeKind, EVAL_EXT_BINDING, EvalContext, FunctionKind, Instructions, Opcode, Register,
    RuntimeId, ScopeFlags, ScopeKind, UnitString, VariableMode,
};
use ep9_value::Throw;

use crate::abi::{self, Local};
use crate::conv::Res;
use crate::descriptor::Desc;
use crate::error::{FrameRef, throw_error};
use crate::frame::{FALSE, HOLE, TRUE, UNDEFINED, int32_of};
use crate::interp::Cx;
use crate::key::{self, key_text};
use crate::object::{
    self, attr, is_context, is_function, is_object, is_string, own_define, own_get,
};
use crate::property;
use crate::runtime::{ErrorKind, Root};
use crate::unit::{EvalPrologue, FunctionInfo, LoadedUnit};

use super::{NameMsg, context, info, interrupt, throw_msg_now, throw_name_now};

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
    match unit.functions[id].kind {
        FunctionKind::Script | FunctionKind::Module | FunctionKind::Arrow => {}
        // Eval code: the same this environment as the eval's own caller.
        FunctionKind::Eval => {
            let r = u.eval_receiver;
            ctx.in_function = r.in_function;
            ctx.in_method = r.in_method;
            ctx.in_derived_ctor = r.in_derived_ctor;
            ctx.in_class_field_init = r.in_class_field_init;
        }
        k => set_receiver(&mut ctx, k),
    }
    if unit.header.code_kind == CodeKind::Eval {
        // Past this unit's own contexts, the chain is the eval caller's (and further out).
        outer_private_names(cx, fp, u, &mut ctx)?;
    }
    ctx.new_target_ok = ctx.in_function;
    ctx.super_property_ok = ctx.in_method;
    ctx.super_call_ok = ctx.in_derived_ctor;
    Ok(ctx)
}

/// For a direct eval inside eval code `u`: the private names of the Class scope contexts past
/// the contexts of `u` on the runtime chain.
fn outer_private_names(
    cx: &mut Cx,
    fp: *mut u64,
    u: &LoadedUnit,
    ctx: &mut EvalContext,
) -> Res<()> {
    let mut c = context(fp);
    while is_context(c) {
        poll(cx)?;
        let (cu, scope) = object::context_scope(c);
        if !core::ptr::eq(cu, u) {
            add_private_names(cu, scope, &mut ctx.private_names);
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

#[cfg(test)]
thread_local! {
    /// Tests: how many times [`prologue`] decoded a function.
    pub(crate) static PROLOGUE_DECODES: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// The declarations of the sloppy eval prologue of `fi` (function 0 of eval code), in call
/// order: (the constant index of the name, a function). Also the byte range of the first
/// `CallRuntime DeclareEval*`. The names are unit constants (atoms that the unit roots), kept
/// here as constant indices.
fn prologue(fi: &FunctionInfo) -> EvalPrologue {
    #[cfg(test)]
    PROLOGUE_DECODES.with(|n| n.set(n.get() + 1));
    let mut first = None;
    let mut decls = Vec::new();
    let Some(u) = fi.loaded_unit() else {
        return EvalPrologue { first, decls };
    };
    let code = &u.unit.functions[fi.function_id as usize].bytecode;
    // The constant index in the accumulator and in each register, along straight-line code.
    let mut acc: Option<u32> = None;
    let mut regs: Vec<(Register, u32)> = Vec::new();
    for insn in Instructions::new(code) {
        let Ok(insn) = insn else { break };
        match insn.opcode {
            Opcode::LdaConstant => acc = Some(insn.unsigned(0)),
            Opcode::Star => {
                let r = insn.register(0);
                regs.retain(|&(q, _)| q != r);
                if let Some(k) = acc {
                    regs.push((r, k));
                }
            }
            Opcode::CallRuntime => {
                let function = match RuntimeId::from_u16(insn.unsigned(0) as u16) {
                    Some(RuntimeId::DeclareEvalFunction) => true,
                    Some(RuntimeId::DeclareEvalVar) => false,
                    _ => {
                        acc = None;
                        continue;
                    }
                };
                let r = insn.register(1);
                if let Some(&(_, k)) = regs.iter().find(|&&(q, _)| q == r) {
                    first.get_or_insert((insn.offset, insn.next_offset()));
                    decls.push((k, function));
                }
                acc = None;
            }
            _ => acc = None,
        }
    }
    EvalPrologue { first, decls }
}

/// At the first `DeclareEval*` call of a run of the eval code in frame `fp`: the checks of
/// EvalDeclarationInstantiation for every name of the prologue (step 3: SyntaxError; steps 9
/// and 11 at the global level: CanDeclareGlobalFunction, CanDeclareGlobalVar: TypeError),
/// before any binding is made. Later calls of the run skip it. The prologue of function 0 is
/// decoded once per unit (`LoadedUnit::eval_prologue`), so a run with D names costs O(D), not
/// D decodes of the whole function. GC point (a poll).
fn check_all(cx: &mut Cx, fp: *mut u64) -> Res<()> {
    let fi = info(fp);
    let fresh;
    let EvalPrologue { first, decls } = match fi.loaded_unit() {
        Some(u) if fi.function_id == 0 => u.eval_prologue.get_or_init(|| prologue(fi)),
        // Only function 0 of eval code has a prologue (`gen_eval_program`); a hand-built unit
        // may call DeclareEval* elsewhere.
        _ => {
            fresh = prologue(fi);
            &fresh
        }
    };
    let at = FrameRef(fp).saved_offset();
    if !first.is_some_and(|(start, end)| (start..end).contains(&at)) {
        return Ok(());
    }
    let mut global = false;
    for &(k, _) in decls {
        let name = key::to_property_key(cx, fi.constant(k))?;
        global = matches!(var_env(cx, fp, name)?, VarEnv::Global);
    }
    if !global {
        return Ok(());
    }
    let g = cx.rt().global();
    for &(k, function) in decls.iter().rev() {
        if !function {
            continue;
        }
        let name = fi.constant(k);
        let definable = match property::get_own_property(cx, g, name)? {
            None => property::is_extensible(g),
            Some(d) => {
                d.configurable == Some(true)
                    || !d.is_accessor() && d.writable == Some(true) && d.enumerable == Some(true)
            }
        };
        if !definable {
            let exists = property::has_own_property(cx, g, name);
            return Err(cannot_declare(cx, name, exists));
        }
    }
    // The function names (atoms): step 11 skips a var name that is one of them.
    let functions: HashSet<u64> = decls
        .iter()
        .filter(|&&(_, function)| function)
        .map(|&(k, _)| fi.constant(k))
        .collect();
    for &(k, _) in decls {
        let name = fi.constant(k);
        if functions.contains(&name) {
            continue;
        }
        if !property::has_own_property(cx, g, name) && !property::is_extensible(g) {
            return Err(cannot_declare(cx, name, false));
        }
    }
    Ok(())
}

/// `DeclareEvalVar(name)` (runtime id 27): a `var` of sloppy eval code (§19.2.1.3 steps 3,
/// 11 and 18). In a function: a new binding in the `.eval_ext` object of its var scope
/// context unless the scope or that object binds `name`; at the global level: CanDeclareGlobalVar
/// (TypeError), then CreateGlobalVarBinding(name, true), a configurable property. GC point.
pub(crate) fn declare_eval_var(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<u64> {
    check_all(cx, fp)?;
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

// ---- Annex B.3.2.3: block functions of sloppy eval code (ABI 5 ids 46-47) ------------------

/// The first context of the frame `fp`'s chain that is not a context of the running eval
/// unit (its eval scope and its blocks): the eval caller's LexicalEnvironment. Not a GC point.
fn caller_context(fp: *mut u64) -> u64 {
    let own = info(fp).loaded_unit().map(core::ptr::from_ref);
    let mut c = context(fp);
    while is_context(c) {
        let (u, _) = object::context_scope(c);
        if Some(core::ptr::from_ref(u)) != own {
            break;
        }
        c = object::context_previous(c);
    }
    c
}

/// B.3.2.3 steps ii.2.a-f for the block function name `name` of the eval code in frame `fp`:
/// the variable environment if the function gets a var binding (bindingExists is false and
/// fnDefinable is true), else `None`. The walk goes from the caller's LexicalEnvironment out to
/// the VariableEnvironment: a declarative context on the way that binds `name` (a lexical slot
/// of the var scope too: a sloppy function's top-level lexical declarations have their own
/// environment) means bindingExists; a `with` object and, as in V8 and [`var_env`] (B.3.4), a
/// catch clause do not. At the global environment, fnDefinable is false for a lexical
/// declaration of `name`, else CanDeclareGlobalVar. GC point (a poll).
fn annex_b_env(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<Option<VarEnv>> {
    let mut c = caller_context(fp);
    while is_context(c) {
        poll(cx)?;
        let (u, scope) = object::context_scope(c);
        if scope.kind == ScopeKind::With {
            c = object::context_previous(c);
            continue;
        }
        if is_var_scope(u, scope) {
            if let Some((_, mode)) = slot_of(c, name)
                && is_lexical(mode)
            {
                return Ok(None);
            }
            return Ok(Some(VarEnv::Function(c)));
        }
        if scope.kind != ScopeKind::Catch && slot_of(c, name).is_some() {
            return Ok(None);
        }
        c = object::context_previous(c);
    }
    let rt = cx.rt();
    if own_get(rt.root(Root::Lexical), name).is_some() {
        return Ok(None);
    }
    let g = rt.global();
    let definable = property::has_own_property(cx, g, name) || property::is_extensible(g);
    Ok(definable.then_some(VarEnv::Global))
}

/// The VariableEnvironment of the eval code in frame `fp` (no checks: the walk of
/// [`annex_b_env`] without its conflicts). GC point (a poll).
fn plain_var_env(cx: &mut Cx, fp: *mut u64) -> Res<VarEnv> {
    let mut c = caller_context(fp);
    while is_context(c) {
        poll(cx)?;
        let (u, scope) = object::context_scope(c);
        if is_var_scope(u, scope) {
            return Ok(VarEnv::Function(c));
        }
        c = object::context_previous(c);
    }
    Ok(VarEnv::Global)
}

/// `DeclareEvalAnnexBFunction(name)` (runtime id 46, ABI 5): B.3.2.3 step ii for a block
/// function of sloppy eval code, at EvalDeclarationInstantiation (after the checks and the
/// bindings of the eval code's own functions and vars: `gen_eval_program` calls it after them,
/// so F of declaredFunctionOrVarNames already has its binding). If the function gets a var
/// binding (see [`annex_b_env`]): in a function, a new `.eval_ext` binding initialized to
/// undefined unless the variable environment binds `name`; at the global level,
/// CreateGlobalVarBinding(name, true). Returns true in that case (the code then copies the
/// function into the var binding when its declaration runs, id 47), else false. GC point.
pub(crate) fn declare_eval_annex_b_function(cx: &mut Cx, fp: *mut u64, name: u64) -> Res<u64> {
    let name = key::to_property_key(cx, name)?;
    match annex_b_env(cx, fp, name)? {
        None => return Ok(FALSE),
        Some(VarEnv::Function(c)) => {
            if slot_of(c, name).is_none() {
                let ext = eval_ext(cx, c);
                if !property::has_own_property(cx, ext, name) {
                    define_ext(cx, ext, name, UNDEFINED);
                }
            }
        }
        Some(VarEnv::Global) => {
            let g = cx.rt().global();
            if !property::has_own_property(cx, g, name) && property::is_extensible(g) {
                let desc = Desc::data(UNDEFINED, attr::DEFAULT);
                if !property::define_own_property(cx, g, name, desc)? {
                    return Err(cannot_declare(cx, name, true));
                }
            }
        }
    }
    Ok(TRUE)
}

/// `StoreEvalAnnexBFunction(name, value)` (runtime id 47, ABI 5, V8-A2): B.3.2.3 step
/// ii.2.g.ii, when a block function declaration of sloppy eval code whose name got a var
/// binding (id 46 gave true) is evaluated: genv.SetMutableBinding(name, value, false) on the
/// eval code's VariableEnvironment, not on the block binding. In a function: the var scope's
/// slot or its `.eval_ext` binding (made again if `delete` removed it, §9.1.1.1.5 step 1); at
/// the global level: a script lexical binding (a TDZ is a ReferenceError, a constant is left
/// alone), else Set(global object, name, value, false). GC point (a setter).
pub(crate) fn store_eval_annex_b_function(
    cx: &mut Cx,
    fp: *mut u64,
    name: u64,
    value: u64,
) -> Res<u64> {
    let name = key::to_property_key(cx, name)?;
    match plain_var_env(cx, fp)? {
        VarEnv::Function(c) => {
            if let Some((i, _)) = slot_of(c, name) {
                object::context_set(cx.rt(), c, i, value);
            } else {
                let ext = eval_ext(cx, c);
                if property::has_own_property(cx, ext, name) {
                    property::set(cx, ext, name, value, false)?;
                } else {
                    define_ext(cx, ext, name, value);
                }
            }
        }
        VarEnv::Global => {
            let rt = cx.rt();
            let lexical = rt.root(Root::Lexical);
            if let Some((old, a)) = own_get(lexical, name) {
                if old == HOLE {
                    return Err(throw_name_now(cx, NameMsg::Tdz, name));
                }
                if a & attr::WRITABLE != 0 {
                    object::own_replace(rt, lexical, name, value);
                }
            } else {
                let g = rt.global();
                property::set(cx, g, name, value, false)?;
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
    check_all(cx, fp)?;
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
            let desc = match property::get_own_property(cx, g, name)? {
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

#[cfg(test)]
mod tests {
    //! Ids 46-47 on hand-built units at the global level (indirect eval). The JS-level checks
    //! (direct and indirect, function and global variable environments, the conflicts) are the
    //! test262 files under `annexB/language/eval-code`, which need the front end.

    use ep9_bytecode::{
        CodeKind, EvalContext, FunctionCtorKind, FunctionId, FunctionMeta, RegisterList, RuntimeId,
        UnitBuilder, UnitOptions, VerifiedUnit,
    };

    use crate::tests::{Asm, config, meta, on_big_stack, opts, run_in};
    use crate::{CompileFailure, CompileResult, CompilerHooks, Dispatch, IsolateConfig};

    /// Sloppy eval code `{ function af() { return 7 } }` as the front end makes it (E7):
    /// `DeclareEvalAnnexBFunction("af")` into a flag, then, where the declaration runs,
    /// `StoreEvalAnnexBFunction("af", closure)` if the flag is true. Returns the flag.
    fn annex_unit() -> VerifiedUnit {
        let mut s = Asm::new(FunctionMeta::entry(CodeKind::Eval, false, 0));
        let flag = s.b.new_register();
        let args = s.b.new_register_list(2);
        let one = RegisterList::new(args.first().local_index().unwrap(), 1);
        s.lda_str("af");
        s.b.star(args.get(0));
        s.b.call_runtime(RuntimeId::DeclareEvalAnnexBFunction, one, 1);
        s.b.star(flag);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(args.get(1));
        let skip = s.b.new_label();
        s.b.ldar(flag).jump_if_false(skip);
        s.lda_str("af");
        s.b.star(args.get(0));
        s.b.call_runtime(RuntimeId::StoreEvalAnnexBFunction, args, 2);
        s.b.bind(skip);
        s.b.ldar(flag).return_();
        let mut f = Asm::new(meta("af", 0, 0, 0, 0));
        f.b.lda_smi(7).return_();
        let mut u = UnitBuilder::new(
            "",
            UnitOptions {
                code_kind: CodeKind::Eval,
                ..opts()
            },
        );
        u.set_function(FunctionId(0), s.finish());
        u.set_function(FunctionId(1), f.finish());
        u.finish().unwrap()
    }

    fn hook_eval(source: &str, _: &EvalContext) -> CompileResult {
        match source {
            "annex" => Ok(annex_unit()),
            _ => Err(CompileFailure::Syntax(format!("unknown {source:?}"))),
        }
    }

    fn hook_function(_: FunctionCtorKind, _: &str, _: &str) -> CompileResult {
        Err(CompileFailure::Unsupported("functions".into()))
    }

    fn hook_script(_: &str) -> CompileResult {
        Err(CompileFailure::Unsupported("scripts".into()))
    }

    static HOOKS: CompilerHooks = CompilerHooks {
        compile_eval: hook_eval,
        compile_function: hook_function,
        compile_script: hook_script,
    };

    /// At the global level: id 46 makes the configurable global `af` (CreateGlobalVarBinding
    /// with D = true) and answers true; id 47 stores the function there. After `delete af`, a
    /// second run makes it again. Every dispatch, with and without GC stress.
    #[test]
    fn annex_b_ids_make_and_set_a_global_var() {
        fn make() -> VerifiedUnit {
            let mut s = Asm::script(0);
            let g = s.b.new_register();
            for _ in 0..2 {
                // print(eval("annex")); print(af()); print(delete globalThis.af)
                s.lda_str("annex");
                let a0 = s.a.get(0);
                s.b.star(a0);
                s.lda_global("eval");
                let f = s.f;
                s.b.star(f);
                s.b.call_undefined_receiver1(f, a0, s.call);
                s.print_acc();
                s.lda_global("af");
                s.b.star(f);
                s.b.call_undefined_receiver0(f, s.call);
                s.print_acc();
                s.lda_global("globalThis");
                s.b.star(g);
                s.lda_str("af");
                s.b.delete_property_sloppy(g);
                s.print_acc();
            }
            s.b.lda_undefined().return_();
            crate::tests::unit("", vec![s.finish()])
        }
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let r = on_big_stack(move || {
                    let c = IsolateConfig {
                        compiler: Some(&HOOKS),
                        ..config(d, stress)
                    };
                    run_in(c, make())
                });
                assert!(r.result.is_ok(), "{r:?}");
                assert_eq!(r.output, ["true", "7", "true", "true", "7", "true"]);
            }
        }
    }
}
