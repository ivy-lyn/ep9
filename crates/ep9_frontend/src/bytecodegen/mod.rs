//! BytecodeGen v0: JS source -> [`CompiledUnit`](ep9_bytecode::CompiledUnit) (bytecode ABI v2).
//!
//! ```text
//! compile_script / compile_module / compile
//!   | with_front_end: parse, depth guard, oxc_semantic (on the sized-stack worker)
//!   | scope::analyze -> ScopeAnalysis (bindings, locations, hops, hole checks)
//!   v
//! UnitGen (this file)
//!   1. one scope descriptor per analysis scope (unit scope id == analysis scope id)
//!      (+ the script context descriptor as unit scope 1 when the script top level needs one)
//!   2. job queue: f0, then every function node in the order the walk meets it
//!   3. FnGen per job (func.rs, stmt.rs, expr.rs, class.rs, pattern.rs) -> FunctionCode
//!   4. UnitBuilder::finish: interns strings, runs the verifier
//!   v
//! VerifiedUnit
//! ```
//!
//! # Design decisions (BytecodeGen v0)
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | G1 | The scope pass decides every binding location; BytecodeGen never re-allocates. Named locals are `r0..register_count`; temporaries follow, stack-allocated with register marks (scope-based, deterministic) | One source of truth; `hops` from the scope pass stay valid |
//! | G2 | BytecodeGen pushes a context exactly for each scope with `needs_context`, in tree order (name scope, function scope, body scope; class scope before the heritage). An access `d` hops out uses the saved-context register of the pushed context, or the entry context register (saved at entry only when a reference needs it) | ABI F11: no `<context>` register name |
//! | G3 | Unit scope id = analysis scope id, except: a script whose top level needs a context (captured `this`, direct eval) gets the script context descriptor as unit scope 1, and later ids move up by one. Scope 0 of a script keeps the global declarations for `DeclareGlobals` and never has a context | Near-identity map; `DeclareGlobals` has no operands; every `NEEDS_CONTEXT` descriptor matches one run-time context and outer scopes come first (verifier rule) |
//! | G4 | Expressions leave their value in the accumulator. A plain local register is used directly as an operand only when the other operand cannot write a local | No register optimizer in v0; avoids `a + (a = 1)` errors |
//! | G5 | `finally` uses a token register + a value register and a `SwitchOnSmi` dispatch (V8 model). Token 0 = rethrow, -1 = fall through. The pending message is saved around the finally block | Handler tables stay static; break/continue/return through finally work |
//! | G6 | `for-of` wraps the whole loop in a try-finally with a `done` register (V8 `BuildFinalizeIteration`): the iterator is closed on break, return and throw from the body, never after `next()` fails | test262 iterator-close order |
//! | G7 | `let` loops copy their context per iteration (CreatePerIterationEnvironment) only when the loop head scope has a context; registers need no copy | Spec semantics, no cost for uncaptured loops |
//! | G8 | TDZ: a binding gets an explicit hole store at scope entry only if some reference checks it (or a dynamic lookup may reach it). Globals and module cells: the VM checks | Fewer stores; V8 rule |
//! | G9 | Runtime calls for what has no opcode: global init, direct eval, classes, super, accessors, object spread, `__proto__` (ABI 2 ids 6-18) | Opcode table unchanged |
//! | G10 | A position entry before every instruction that may throw; statement positions at every statement | M0 exit 7 (`throw` prints file:line:col) |
//! | G11 | Unsupported constructs fail the whole unit with [`CodegenError::Unsupported`] (generators, async, private names, class fields, ...) | No silent wrong code |

use std::collections::VecDeque;
use std::fmt;

use ep9_bytecode::{
    BuildError, CodeKind, FunctionBuilder, FunctionFlags, FunctionMeta, ScopeFlags, UnitBuilder,
    UnitOptions, UnitString, VariableMode, VerifiedUnit,
};
use oxc_ast::ast::{ArrowFunctionExpression, Class, FormalParameters, Function};

use crate::scope::{
    BindingId, BindingKind, BindingLocation, FunctionId, FunctionInfo, FunctionKind, Resolution,
    ScopeAnalysis, ScopeId, ScopeKind,
};
use crate::{
    ByteSpan, CompileError, FrontEnd, Goal, LineMap, Position, SourceOpts, Strictness,
    with_front_end,
};

mod class;
mod expr;
mod func;
mod literal;
mod pattern;
mod stmt;

#[cfg(test)]
mod corpus;
#[cfg(test)]
mod ctx_check;
#[cfg(test)]
mod stack_tests;
#[cfg(test)]
mod tests;

use func::FnGen;

/// Options for the produced unit (not the source).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    /// Engine build id written into the header. Not covered by the content hash.
    pub engine_build_id: [u8; 16],
    /// Embed the source text in the unit.
    pub embed_source: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            engine_build_id: [0; 16],
            embed_source: true,
        }
    }
}

/// A construct that BytecodeGen v0 does not compile yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// Short name of the construct (stable; tests and reports group by it).
    pub feature: &'static str,
    /// Where it is.
    pub span: ByteSpan,
    /// Line and UTF-16 column of `span.start`.
    pub position: Position,
}

/// Why a source text did not compile to a unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// Early errors or a depth limit (the same as [`crate::check_early_errors`]).
    Compile(CompileError),
    /// A construct BytecodeGen v0 does not support yet. The engine throws a `SyntaxError`.
    Unsupported(Unsupported),
    /// A size limit of the bytecode format (registers, code size). The engine throws a
    /// `RangeError`.
    Limit(String),
    /// The builder or the verifier rejected the output: a BytecodeGen bug.
    Internal(String),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(e) => e.fmt(f),
            Self::Unsupported(u) => write!(
                f,
                "{}:{}: SyntaxError: ep9 BytecodeGen v0 does not support {} yet",
                u.position.line, u.position.column, u.feature
            ),
            Self::Limit(m) => write!(f, "RangeError: {m}"),
            Self::Internal(m) => write!(f, "internal BytecodeGen error: {m}"),
        }
    }
}

impl std::error::Error for CodegenError {}

impl From<CompileError> for CodegenError {
    fn from(e: CompileError) -> Self {
        Self::Compile(e)
    }
}

/// Compile a Script.
///
/// # Errors
///
/// See [`CodegenError`].
pub fn compile_script(source: &str, strictness: Strictness) -> Result<VerifiedUnit, CodegenError> {
    compile(
        source,
        SourceOpts::script(strictness),
        CompileOptions::default(),
    )
}

/// Compile a Module. v0 has no module record section (H02-Q3): imports and exports are module
/// cells. Exported top-level bindings are cells `1, 2, ...` and imports `-1, -2, ...`, both in
/// binding order.
///
/// # Errors
///
/// See [`CodegenError`].
pub fn compile_module(source: &str) -> Result<VerifiedUnit, CodegenError> {
    compile(source, SourceOpts::module(), CompileOptions::default())
}

/// Compile `source` with explicit options.
///
/// # Errors
///
/// See [`CodegenError`].
pub fn compile(
    source: &str,
    opts: SourceOpts,
    options: CompileOptions,
) -> Result<VerifiedUnit, CodegenError> {
    with_front_end(source, opts, move |fe| {
        let analysis = crate::scope::analyze(fe)?;
        Ok(generate(fe, &analysis, options))
    })?
}

/// The result of [`check_determinism`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Determinism {
    /// Both builds gave the same bytes. The content hash (SHA-256) of the unit.
    Same([u8; 32]),
    /// The builds differ: the two content hashes.
    Different([u8; 32], [u8; 32]),
}

/// Compile `source` twice (the second time on a new thread, so hash-map seeds and the worker
/// differ) and compare the serialized bytes (M0 exit 12, H2 determinism rule).
///
/// # Errors
///
/// The first compile error.
pub fn check_determinism(source: &str, opts: SourceOpts) -> Result<Determinism, CodegenError> {
    let first = compile(source, opts, CompileOptions::default())?;
    let owned = source.to_owned();
    let second = std::thread::spawn(move || compile(&owned, opts, CompileOptions::default()))
        .join()
        .map_err(|_| CodegenError::Internal("second build panicked".into()))??;
    let (a, b) = (first.to_bytes(), second.to_bytes());
    let (ha, hb) = (first.content_hash(), second.content_hash());
    if a == b && ha == hb {
        Ok(Determinism::Same(ha))
    } else {
        Ok(Determinism::Different(ha, hb))
    }
}

// ---------------------------------------------------------------------------------------------
// Unit generation

/// One function to generate. Function nodes are queued when the walk of the parent meets them.
pub(crate) enum Job<'a> {
    /// Function 0.
    Program,
    /// A `function` node: declaration, expression, method, getter, setter, constructor.
    Function {
        node: &'a Function<'a>,
        id: FunctionId,
        name: Option<UnitString>,
    },
    /// An arrow function.
    Arrow {
        node: &'a ArrowFunctionExpression<'a>,
        id: FunctionId,
        name: Option<UnitString>,
    },
    /// The synthetic default constructor of a class.
    DefaultConstructor {
        class: &'a Class<'a>,
        id: FunctionId,
        name: Option<UnitString>,
    },
}

/// Unit-wide data shared by every function generator.
pub(crate) struct UnitCtx<'s> {
    pub(crate) source: &'s str,
    pub(crate) sa: &'s ScopeAnalysis,
    pub(crate) module: bool,
    /// Contexts from the root down to each scope (inclusive).
    pub(crate) depth: Vec<u32>,
    /// Per binding: a TDZ binding that needs an explicit hole store at scope entry.
    pub(crate) needs_hole: Vec<bool>,
    /// Per function: the entry context must be saved in a register.
    pub(crate) needs_entry_ctx: Vec<bool>,
    /// Per binding: module cell index (> 0 export, < 0 import), 0 if none.
    pub(crate) cells: Vec<i32>,
    /// The descriptor of the script top-level context (G3): unit scope 1, when it exists.
    pub(crate) script_ctx_scope: Option<ep9_bytecode::ScopeId>,
    pub(crate) lines: std::cell::OnceCell<LineMap>,
}

impl UnitCtx<'_> {
    /// The unit scope id of an analysis scope (G3): the identity, except that ids from 1 up
    /// move up by one when the script context descriptor (unit scope 1) exists.
    pub(crate) fn unit_scope(&self, s: ScopeId) -> ep9_bytecode::ScopeId {
        unit_scope_id(s, self.script_ctx_scope.is_some())
    }

    /// The unit scope whose context is current inside analysis scope `s` at its top: scope 0
    /// of a script maps to the script context descriptor when it exists.
    pub(crate) fn context_scope(&self, s: ScopeId) -> ep9_bytecode::ScopeId {
        match self.script_ctx_scope {
            Some(ctx) if s.0 == 0 => ctx,
            _ => self.unit_scope(s),
        }
    }

    pub(crate) fn position(&self, offset: u32) -> Position {
        self.lines
            .get_or_init(|| LineMap::new(self.source.as_bytes()))
            .position(offset)
    }

    pub(crate) fn unsupported(&self, feature: &'static str, span: oxc_span::Span) -> CodegenError {
        CodegenError::Unsupported(Unsupported {
            feature,
            span: ByteSpan::of(span),
            position: self.position(span.start),
        })
    }
}

/// Module cell numbering (v0, until the module record section exists, H02-Q3): exported
/// top-level bindings are `1, 2, ...` and imports `-1, -2, ...`, both in binding order.
pub(crate) fn module_cells(sa: &ScopeAnalysis) -> Vec<i32> {
    let mut cells = vec![0; sa.bindings().len()];
    let (mut exports, mut imports) = (0i32, 0i32);
    for (i, b) in sa.bindings().iter().enumerate() {
        if b.location != BindingLocation::Module {
            continue;
        }
        if b.kind == BindingKind::Import {
            imports += 1;
            cells[i] = -imports;
        } else {
            exports += 1;
            cells[i] = exports;
        }
    }
    cells
}

fn generate(
    fe: &FrontEnd<'_>,
    sa: &ScopeAnalysis,
    options: CompileOptions,
) -> Result<VerifiedUnit, CodegenError> {
    let module = fe.opts.goal() == Goal::Module;
    let cx = UnitCtx {
        source: fe.source,
        sa,
        module,
        depth: context_depths(sa),
        needs_hole: hole_bindings(sa),
        needs_entry_ctx: Vec::new(),
        cells: module_cells(sa),
        script_ctx_scope: None,
        lines: std::cell::OnceCell::new(),
    };
    let mut cx = cx;
    cx.needs_entry_ctx = entry_contexts(sa, &cx.depth);
    let strict = sa.functions()[0].strict;
    let mut unit = UnitBuilder::new(
        fe.source,
        UnitOptions {
            code_kind: if module {
                CodeKind::Module
            } else {
                CodeKind::Script
            },
            strict,
            typescript: None,
            engine_build_id: options.engine_build_id,
            embed_source: options.embed_source,
        },
    );
    cx.script_ctx_scope = add_scopes(&mut unit, sa, module);

    let mut done = vec![false; sa.functions().len()];
    let mut queue: VecDeque<Job<'_>> = VecDeque::new();
    queue.push_back(Job::Program);
    while let Some(job) = queue.pop_front() {
        let (id, code, jobs) = gen_job(&cx, fe, job).map_err(|e| *e)?;
        if std::mem::replace(&mut done[id.index()], true) {
            return Err(CodegenError::Internal(format!(
                "function {} generated twice",
                id.0
            )));
        }
        unit.set_function(ep9_bytecode::FunctionId(id.0), code);
        queue.extend(jobs);
    }
    if let Some(missing) = done.iter().position(|d| !d) {
        let f = &sa.functions()[missing];
        return Err(CodegenError::Internal(format!(
            "function {missing} ({:?} at {}) was never generated",
            f.kind, f.span.start
        )));
    }
    unit.finish().map_err(build_error)
}

fn gen_job<'a>(
    cx: &UnitCtx<'_>,
    fe: &FrontEnd<'a>,
    job: Job<'a>,
) -> func::Gen<(FunctionId, ep9_bytecode::FunctionCode, Vec<Job<'a>>)> {
    let sa = cx.sa;
    match job {
        Job::Program => {
            let info = &sa.functions()[0];
            let meta = FunctionMeta::entry(
                if cx.module {
                    CodeKind::Module
                } else {
                    CodeKind::Script
                },
                info.strict,
                u32::try_from(fe.source.len()).unwrap_or(u32::MAX),
            );
            let mut g = FnGen::new(cx, info, meta);
            g.gen_program(fe.program)?;
            let (code, jobs) = g.finish()?;
            Ok((info.id, code, jobs))
        }
        Job::Function { node, id, name } => {
            let info = sa.function(id);
            let meta = function_meta(cx, info, Some(&node.params), name);
            let mut g = FnGen::new(cx, info, meta);
            g.gen_function(node)?;
            let (code, jobs) = g.finish()?;
            Ok((id, code, jobs))
        }
        Job::Arrow { node, id, name } => {
            let info = sa.function(id);
            let meta = function_meta(cx, info, Some(&node.params), name);
            let mut g = FnGen::new(cx, info, meta);
            g.gen_arrow(node)?;
            let (code, jobs) = g.finish()?;
            Ok((id, code, jobs))
        }
        Job::DefaultConstructor { class, id, name } => {
            let info = sa.function(id);
            let meta = function_meta(cx, info, None, name);
            let mut g = FnGen::new(cx, info, meta);
            g.gen_default_constructor(class)?;
            let (code, jobs) = g.finish()?;
            Ok((id, code, jobs))
        }
    }
}

/// Builder errors: a size limit is a [`CodegenError::Limit`]; anything else (including a
/// verifier failure, prefixed `verifier:`) is a BytecodeGen bug.
/// A BytecodeGen bug, as a generator error.
pub(crate) fn internal(message: impl Into<String>) -> Box<CodegenError> {
    Box::new(CodegenError::Internal(message.into()))
}

pub(crate) fn build_error(e: BuildError) -> CodegenError {
    match e {
        BuildError::Limit(what) => CodegenError::Limit(format!("bytecode limit: {what}")),
        BuildError::Verify(v) => CodegenError::Internal(format!("verifier: {v}")),
        other => CodegenError::Internal(format!("builder: {other:?}")),
    }
}

/// Function metadata from the scope pass and the parameter list.
fn function_meta(
    cx: &UnitCtx<'_>,
    info: &FunctionInfo,
    params: Option<&FormalParameters<'_>>,
    name: Option<UnitString>,
) -> FunctionMeta {
    use ep9_bytecode::FunctionKind as K;
    let kind = match info.kind {
        FunctionKind::Script => K::Script,
        FunctionKind::Module => K::Module,
        FunctionKind::Normal => K::Normal,
        FunctionKind::Arrow => K::Arrow,
        FunctionKind::Method => K::Method,
        FunctionKind::Getter => K::Getter,
        FunctionKind::Setter => K::Setter,
        FunctionKind::ClassConstructor | FunctionKind::DefaultConstructor => K::ClassConstructor,
        FunctionKind::DerivedConstructor | FunctionKind::DefaultDerivedConstructor => {
            K::DerivedConstructor
        }
        FunctionKind::InstanceMembersInitializer | FunctionKind::StaticMembersInitializer => {
            K::ClassFieldsInitializer
        }
    };
    let mut flags = FunctionFlags::default();
    if info.strict {
        flags = flags | FunctionFlags::STRICT;
    }
    if info.is_generator {
        flags = flags | FunctionFlags::GENERATOR;
    }
    if info.is_async {
        flags = flags | FunctionFlags::ASYNC;
    }
    if info.simple_params {
        flags = flags | FunctionFlags::SIMPLE_PARAMETERS;
    }
    let (parameter_count, length) = match params {
        Some(p) => {
            let count = u32::try_from(p.items.len()).unwrap_or(u32::MAX);
            let length = p
                .items
                .iter()
                .take_while(|item| {
                    item.initializer.is_none()
                        && !matches!(
                            item.pattern,
                            oxc_ast::ast::BindingPattern::AssignmentPattern(_)
                        )
                })
                .count();
            (count, u32::try_from(length).unwrap_or(u32::MAX))
        }
        None => (0, 0),
    };
    FunctionMeta {
        kind,
        flags,
        name,
        parent: info.parent.map(|p| ep9_bytecode::FunctionId(p.0)),
        outer_scope: info.outer_scope.map(|s| cx.context_scope(s)),
        source_start: info.span.start,
        source_end: info.span.end,
        parameter_count,
        length,
    }
}

/// Contexts from the root down to each scope, inclusive (the scope pass uses the same rule
/// for `hops`).
fn context_depths(sa: &ScopeAnalysis) -> Vec<u32> {
    let mut depth = vec![0u32; sa.scopes().len()];
    for s in sa.scopes() {
        let parent = s.parent.map_or(0, |p| depth[p.index()]);
        depth[s.id.index()] = parent + u32::from(s.needs_context);
    }
    depth
}

/// Bindings that need a hole store at scope entry (G8).
fn hole_bindings(sa: &ScopeAnalysis) -> Vec<bool> {
    let mut needs = vec![false; sa.bindings().len()];
    for r in sa.references() {
        match r.resolution {
            Resolution::Binding {
                binding,
                hole_check: true,
                ..
            } => needs[binding.index()] = true,
            Resolution::Dynamic {
                fallback: Some(binding),
            } => needs[binding.index()] = true,
            _ => {}
        }
    }
    for (i, b) in sa.bindings().iter().enumerate() {
        let scope = sa.scope(b.scope);
        // A direct eval below the scope can read the binding before its initialization.
        if scope.contains_eval {
            needs[i] = true;
        }
        // `this` of a derived constructor: `super()` checks that it is still the hole.
        if b.kind == BindingKind::This && sa.function(scope.function).kind.is_derived_constructor()
        {
            needs[i] = true;
        }
        if !b.tdz
            || !matches!(
                b.location,
                BindingLocation::Register(_) | BindingLocation::Context { .. }
            )
        {
            needs[i] = false;
        }
    }
    needs
}

/// Functions that must save their entry context: a reference reaches more than 0 contexts out
/// of the function while the function has pushed none (G2).
fn entry_contexts(sa: &ScopeAnalysis, depth: &[u32]) -> Vec<bool> {
    let mut needs = vec![false; sa.functions().len()];
    for r in sa.references() {
        let Resolution::Binding { binding, hops, .. } = r.resolution else {
            continue;
        };
        if hops == 0
            || !matches!(
                sa.binding(binding).location,
                BindingLocation::Context { .. }
            )
        {
            continue;
        }
        let f = sa.function(r.function);
        let entry = f.outer_scope.map_or(0, |s| depth[s.index()]);
        if depth[r.scope.index()] == entry {
            needs[r.function.index()] = true;
        }
    }
    needs
}

/// The runtime scope kind of an analysis scope.
fn unit_scope_kind(kind: ScopeKind) -> ep9_bytecode::ScopeKind {
    use ep9_bytecode::ScopeKind as K;
    match kind {
        ScopeKind::Script => K::Script,
        ScopeKind::Module => K::Module,
        ScopeKind::Function => K::Function,
        ScopeKind::Catch => K::Catch,
        ScopeKind::With => K::With,
        ScopeKind::Class => K::Class,
        ScopeKind::FunctionName
        | ScopeKind::FunctionBody
        | ScopeKind::Block
        | ScopeKind::For
        | ScopeKind::Switch
        | ScopeKind::StaticBlock => K::Block,
    }
}

pub(crate) fn variable_mode(kind: BindingKind) -> VariableMode {
    match kind {
        BindingKind::Let => VariableMode::Let,
        BindingKind::Const
        | BindingKind::Using
        | BindingKind::ClassInner
        | BindingKind::Import
        | BindingKind::PrivateName { .. } => VariableMode::Const,
        BindingKind::Function | BindingKind::BlockFunction => VariableMode::Function,
        BindingKind::Class => VariableMode::Class,
        BindingKind::Var
        | BindingKind::Parameter
        | BindingKind::CatchParameter
        | BindingKind::FunctionName
        | BindingKind::This
        | BindingKind::NewTarget
        | BindingKind::Arguments
        | BindingKind::HomeObject
        | BindingKind::Callee => VariableMode::Var,
    }
}

/// One descriptor per analysis scope (same ids), plus the script context descriptor (G3).
/// One descriptor per analysis scope, in analysis order (G3). A script whose top level needs a
/// context (captured `this`, direct eval) gets the script context descriptor as unit scope 1
/// (kind Function, outer scope 0); every later scope moves up by one, and the children of
/// scope 0 get it as their outer scope. Scope 0 of a script lists the global declarations for
/// `DeclareGlobals` and never has a context.
fn add_scopes(
    unit: &mut UnitBuilder,
    sa: &ScopeAnalysis,
    module: bool,
) -> Option<ep9_bytecode::ScopeId> {
    let name = |b: BindingId| UnitString::new(sa.name(sa.binding(b).name));
    let root = &sa.scopes()[0];
    let script_ctx = (!module && root.needs_context).then_some(ep9_bytecode::ScopeId(1));
    let shifted = script_ctx.is_some();
    for s in sa.scopes() {
        let mut flags = ScopeFlags::default();
        if s.strict {
            flags = flags | ScopeFlags::STRICT;
        }
        if s.sloppy_eval {
            flags = flags | ScopeFlags::CALLS_SLOPPY_EVAL;
        }
        let script_top = s.id.0 == 0 && !module;
        if s.needs_context && !script_top {
            flags = flags | ScopeFlags::NEEDS_CONTEXT;
        }
        let slots: Vec<(UnitString, VariableMode)> = if script_top {
            // GlobalDeclarationInstantiation reads the declarations from scope 0.
            s.bindings
                .iter()
                .filter(|&&b| {
                    matches!(
                        sa.binding(b).location,
                        BindingLocation::Global | BindingLocation::GlobalLexical
                    )
                })
                .map(|&b| (name(b), variable_mode(sa.binding(b).kind)))
                .collect()
        } else {
            s.context_slots
                .iter()
                .map(|&b| (name(b), variable_mode(sa.binding(b).kind)))
                .collect()
        };
        let parent = s.parent.map(|p| match script_ctx {
            Some(ctx) if p.0 == 0 => ctx,
            _ => unit_scope_id(p, shifted),
        });
        let id = unit.add_scope(unit_scope_kind(s.kind), flags, parent, slots);
        debug_assert_eq!(id, unit_scope_id(s.id, shifted));
        if script_top && let Some(ctx) = script_ctx {
            let slots = s
                .context_slots
                .iter()
                .map(|&b| (name(b), variable_mode(sa.binding(b).kind)))
                .collect();
            let mut flags = ScopeFlags::NEEDS_CONTEXT;
            if s.strict {
                flags = flags | ScopeFlags::STRICT;
            }
            let id = unit.add_scope(
                ep9_bytecode::ScopeKind::Function,
                flags,
                Some(ep9_bytecode::ScopeId(0)),
                slots,
            );
            debug_assert_eq!(id, ctx);
        }
    }
    script_ctx
}

/// Unit scope id of analysis scope `s` (ids from 1 up move by one when `shifted`).
fn unit_scope_id(s: ScopeId, shifted: bool) -> ep9_bytecode::ScopeId {
    ep9_bytecode::ScopeId(if s.0 == 0 || !shifted { s.0 } else { s.0 + 1 })
}

/// A fresh function builder with the named locals reserved.
pub(crate) fn new_builder(info: &FunctionInfo, meta: FunctionMeta) -> FunctionBuilder {
    let mut b = FunctionBuilder::new(meta);
    if info.register_count > 0 {
        b.new_register_list(info.register_count);
    }
    b
}
