//! The per-function generator: registers, contexts, control flow, variable access and the
//! function prologues.
//!
//! ```text
//! contexts (pushed by this function)      saved register holds
//!   C1 = contexts[0]                       C0 (the entry context)
//!   ...
//!   Ck = contexts[k-1]  <- current         C(k-1)
//! access d hops out:  d == 0 -> *CurrentContextSlot
//!                     d <= k -> LdaContextSlot contexts[k-d].saved, slot, 0
//!                     d >  k -> LdaContextSlot contexts[0].saved (or entry_ctx), slot, d-k
//! ```

use std::collections::HashMap;

use ep9_bytecode::{
    ConstIndex, FeedbackSlot, FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionMeta,
    HandlerKind, Label, Register, RegisterList, RegisterMark, RuntimeId, UnitString,
};
use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, Class, FormalParameters, Function, Program, Statement,
};
use oxc_span::{GetSpan, Span};

use super::{CodegenError, Job, UnitCtx, build_error, new_builder};
use crate::scope::{
    Binding, BindingId, BindingKind, BindingLocation, FrameSlot, FunctionInfo, FunctionKind,
    NameId, PerIteration, Reference, Resolution, ScopeAnalysis, ScopeId, ScopeKind,
};
use crate::{AST_CAP, CompileError, DepthLayer, RangeError};

/// Generator results. The error is boxed: `Result<(), Box<_>>` is one word, which keeps the
/// recursive frames small in debug builds (stack per AST level, `stack_tests.rs`).
pub(crate) type Gen<T = ()> = Result<T, Box<CodegenError>>;

/// A context pushed by this function.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pushed {
    pub(crate) scope: ScopeId,
    /// Holds the previous context.
    pub(crate) saved: Register,
}

/// What a jump target is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetKind {
    Loop,
    Switch,
    Labeled,
}

/// A command that leaves statements: break, continue or return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Break to the target at this control stack index.
    Break(usize),
    /// Continue the loop at this control stack index.
    Continue(usize),
    /// Return the accumulator.
    Return,
}

/// One entry of the control stack (innermost last).
pub(crate) enum Control<'a> {
    /// A loop, a switch or a labeled statement.
    Target {
        labels: Vec<&'a str>,
        kind: TargetKind,
        break_to: Label,
        continue_to: Option<Label>,
    },
    /// A context pushed by this function: leaving it pops the context.
    Context { saved: Register },
    /// A `try` block with a `finally` (G5).
    Finally {
        token: Register,
        value: Register,
        entry: Label,
        commands: Vec<Command>,
    },
}

pub(crate) struct FnGen<'a, 's> {
    pub(crate) cx: &'s UnitCtx<'s>,
    pub(crate) sa: &'s ScopeAnalysis,
    pub(crate) info: &'s FunctionInfo,
    pub(crate) b: FunctionBuilder,
    pub(crate) jobs: Vec<Job<'a>>,
    /// The current analysis scope.
    pub(crate) scope: ScopeId,
    pub(crate) contexts: Vec<Pushed>,
    pub(crate) entry_ctx: Option<Register>,
    pub(crate) control: Vec<Control<'a>>,
    pub(crate) loop_depth: i32,
    /// Target of a short-circuit in the current optional chain.
    pub(crate) chain_end: Option<Label>,
    /// Labels that wrap the next loop statement.
    pub(crate) pending_labels: Vec<&'a str>,
    global_slots: HashMap<(FeedbackSlotKind, u32), FeedbackSlot>,
    depth: u32,
    /// Start offsets of the statements and expressions being generated (innermost last): the
    /// default position of a throwing instruction (G10).
    spans: Vec<u32>,
    /// The generator state of a generator function body (`generator.rs`); `None` elsewhere.
    pub(crate) generator: Option<super::generator::GenState>,
    /// The start offset of the last statement position set ([`FnGen::stmt_pos`]): the
    /// `return` statement that [`FnGen::perform`] completes (`async_gen.rs` AGF4).
    last_stmt: u32,
    /// Function 0 of sloppy eval code: each var binding of an Annex B block function
    /// (B.3.2.3) and the register that holds what `DeclareEvalAnnexBFunction` answered for it
    /// (`gen_eval_program`; `stmt.rs` `store_annex_b_var` reads it).
    pub(crate) eval_annex_b: Vec<(BindingId, Register)>,
}

impl<'a, 's> FnGen<'a, 's> {
    pub(crate) fn new(cx: &'s UnitCtx<'s>, info: &'s FunctionInfo, meta: FunctionMeta) -> Self {
        let mut b = new_builder(info, meta);
        // Prologue code (context set-up, frame bindings) reports the function start.
        b.set_throw_position(Some(info.span.start));
        let entry_ctx = cx.needs_entry_ctx[info.id.index()].then(|| {
            let r = b.new_register();
            b.lda_current_context().star(r);
            r
        });
        Self {
            cx,
            sa: cx.sa,
            info,
            b,
            jobs: Vec::new(),
            scope: info.outer_scope.unwrap_or(info.scope),
            contexts: Vec::new(),
            entry_ctx,
            control: Vec::new(),
            loop_depth: 0,
            chain_end: None,
            pending_labels: Vec::new(),
            global_slots: HashMap::new(),
            depth: 0,
            spans: Vec::new(),
            generator: None,
            last_stmt: u32::MAX,
            eval_annex_b: Vec::new(),
        }
    }

    pub(crate) fn finish(self) -> Gen<(FunctionCode, Vec<Job<'a>>)> {
        let code = self.b.finish().map_err(build_error)?;
        Ok((code, self.jobs))
    }

    // ---- Errors, depth, positions ----

    pub(crate) fn unsupported<T>(&self, feature: &'static str, span: Span) -> Gen<T> {
        Err(Box::new(self.cx.unsupported(feature, span)))
    }

    /// Layer-3 depth guard at every recursion cycle point (the scope pass checked the same tree).
    /// Also makes `span.start` the default position of throwing instructions until `leave`.
    pub(crate) fn enter(&mut self, span: Span) -> Gen {
        self.depth += 1;
        self.spans.push(span.start);
        self.b.set_throw_position(Some(span.start));
        if self.depth > AST_CAP {
            return Err(Box::new(CodegenError::Compile(CompileError::Range(
                RangeError::too_deep(DepthLayer::Pass, self.cx.source, Some(span.start)),
            ))));
        }
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
        self.spans.pop();
        let at = self.spans.last().copied().unwrap_or(self.info.span.start);
        self.b.set_throw_position(Some(at));
    }

    pub(crate) fn stmt_pos(&mut self, span: Span) {
        self.last_stmt = span.start;
        self.b.set_statement_position(span.start);
    }

    pub(crate) fn expr_pos(&mut self, offset: u32) {
        self.b.set_expression_position(offset);
    }

    // ---- Registers ----

    pub(crate) fn temp(&mut self) -> Register {
        self.b.new_register()
    }

    pub(crate) fn temps(&mut self, n: u32) -> RegisterList {
        self.b.new_register_list(n)
    }

    pub(crate) fn mark(&self) -> RegisterMark {
        self.b.register_mark()
    }

    pub(crate) fn release(&mut self, m: RegisterMark) {
        self.b.release_registers(m);
    }

    // ---- Constants and slots ----

    pub(crate) fn str_const(&mut self, s: &str) -> ConstIndex {
        self.b.string_constant(UnitString::new(s))
    }

    pub(crate) fn name_const(&mut self, n: NameId) -> ConstIndex {
        let text = self.sa.name(n);
        self.b.string_constant(UnitString::new(text))
    }

    pub(crate) fn slot(&mut self, kind: FeedbackSlotKind) -> FeedbackSlot {
        self.b.feedback_slot(kind)
    }

    /// One slot per (kind, name) for global loads and stores (V8 shares them per function).
    pub(crate) fn global_slot(&mut self, kind: FeedbackSlotKind, name: ConstIndex) -> FeedbackSlot {
        if let Some(&s) = self.global_slots.get(&(kind, name.0)) {
            return s;
        }
        let s = self.b.feedback_slot(kind);
        self.global_slots.insert((kind, name.0), s);
        s
    }

    pub(crate) fn runtime(&mut self, id: RuntimeId, args: RegisterList) {
        self.b.call_runtime(id, args, args.len());
    }

    /// Calls `id` with the given registers (copied into a fresh list when they are not one).
    pub(crate) fn runtime_with(&mut self, id: RuntimeId, args: &[Register]) {
        self.runtime_with_at(id, args, None);
    }

    /// [`Self::runtime_with`] with an expression position on the call (after the moves).
    pub(crate) fn runtime_with_at(&mut self, id: RuntimeId, args: &[Register], at: Option<u32>) {
        let m = self.mark();
        let consecutive = args.windows(2).all(|w| {
            matches!((w[0].local_index(), w[1].local_index()), (Some(a), Some(b)) if b == a + 1)
        }) && args.first().is_some_and(|r| r.local_index().is_some());
        let list = if args.is_empty() {
            RegisterList::EMPTY
        } else if consecutive {
            RegisterList::new(args[0].local_index().expect("checked"), args.len() as u32)
        } else {
            let list = self.temps(args.len() as u32);
            for (i, &r) in args.iter().enumerate() {
                self.b.mov(r, list.get(i as u32));
            }
            list
        };
        if let Some(at) = at {
            self.expr_pos(at);
        }
        self.runtime(id, list);
        self.release(m);
    }

    // ---- Scope bookkeeping ----

    pub(crate) fn binding(&self, b: BindingId) -> &'s Binding {
        self.sa.binding(b)
    }

    /// Contexts between the current scope and `scope` (an ancestor or the same scope).
    pub(crate) fn hops_to(&self, scope: ScopeId) -> u32 {
        self.cx.depth[self.scope.index()] - self.cx.depth[scope.index()]
    }

    /// Push a context: `create` leaves the new context in the accumulator.
    pub(crate) fn push_context(&mut self, scope: ScopeId, create: impl FnOnce(&mut Self)) {
        create(self);
        let saved = self.temp();
        self.b.push_context(saved);
        self.contexts.push(Pushed { scope, saved });
        self.control.push(Control::Context { saved });
    }

    pub(crate) fn pop_context(&mut self) {
        let pushed = self.contexts.pop().expect("a pushed context");
        match self.control.pop() {
            Some(Control::Context { saved }) => debug_assert_eq!(saved, pushed.saved),
            _ => panic!("control stack out of order"),
        }
        self.b.pop_context(pushed.saved);
    }

    /// The standard context creation for a scope kind (catch and with have their own).
    fn create_context(&mut self, scope: ScopeId) {
        let info = self.sa.scope(scope);
        let k = self.b.scope_constant(self.cx.unit_scope(scope));
        match info.kind {
            ScopeKind::Function | ScopeKind::FunctionBody | ScopeKind::Module => {
                let n = info.context_slots.len() as u32;
                self.b.create_function_context(k, n);
            }
            _ => {
                self.b.create_block_context(k);
            }
        }
    }

    /// Enter `scope`: push its context if it needs one, then store the hole into its TDZ
    /// bindings that need it (G8). Returns whether a context was pushed.
    pub(crate) fn enter_scope(&mut self, scope: ScopeId) -> bool {
        debug_assert_eq!(
            self.sa.scope(scope).parent,
            Some(self.scope),
            "scopes are entered in tree order"
        );
        self.scope = scope;
        let pushed = self.sa.scope(scope).needs_context;
        if pushed {
            self.push_context(scope, |g| g.create_context(scope));
        }
        self.init_holes(scope);
        pushed
    }

    pub(crate) fn exit_scope(&mut self, scope: ScopeId, pushed: bool) {
        debug_assert_eq!(self.scope, scope);
        if pushed {
            debug_assert_eq!(self.contexts.last().map(|p| p.scope), Some(scope));
            self.pop_context();
        }
        self.scope = self.sa.scope(scope).parent.expect("a non-root scope");
    }

    /// Hole stores for the TDZ bindings of `scope` that need them.
    pub(crate) fn init_holes(&mut self, scope: ScopeId) {
        for &b in &self.sa.scope(scope).bindings {
            if self.cx.needs_hole[b.index()] {
                self.b.lda_the_hole();
                self.store_binding_init(b);
            }
        }
    }

    /// Closures for the function declarations hoisted to `scope` (block entry or prologue).
    pub(crate) fn hoist_functions(&mut self, scope: ScopeId) -> Gen {
        for &(b, f) in &self.sa.scope(scope).hoisted_functions {
            let info = self.sa.function(f);
            let span = oxc_span::Span::new(info.span.start, info.span.end);
            if info.is_generator {
                self.check_generator(info.is_async, span)?;
            } else if info.is_async {
                self.async_closure(span)?;
            }
            self.enter(span)?;
            let k = self.b.function_constant(ep9_bytecode::FunctionId(f.0));
            let s = self.slot(FeedbackSlotKind::CreateClosure);
            self.b.create_closure(k, s, 0);
            self.store_binding_init(b);
            self.leave();
        }
        Ok(())
    }

    // ---- Context slot access ----

    /// The register and depth to reach the context `d` hops out (d >= 1).
    fn ctx_reg(&self, d: u32) -> (Register, u32) {
        let k = self.contexts.len() as u32;
        if d <= k {
            (self.contexts[(k - d) as usize].saved, 0)
        } else if k >= 1 {
            (self.contexts[0].saved, d - k)
        } else {
            (
                self.entry_ctx
                    .expect("the scope pass marked this function for an entry context"),
                d,
            )
        }
    }

    pub(crate) fn ctx_load(&mut self, slot: u32, d: u32, immutable: bool) {
        if d == 0 {
            if immutable {
                self.b.lda_immutable_current_context_slot(slot);
            } else {
                self.b.lda_current_context_slot(slot);
            }
        } else {
            let (r, depth) = self.ctx_reg(d);
            if immutable {
                self.b.lda_immutable_context_slot(r, slot, depth);
            } else {
                self.b.lda_context_slot(r, slot, depth);
            }
        }
    }

    pub(crate) fn ctx_store(&mut self, slot: u32, d: u32) {
        if d == 0 {
            self.b.sta_current_context_slot(slot);
        } else {
            let (r, depth) = self.ctx_reg(d);
            self.b.sta_context_slot(r, slot, depth);
        }
    }

    // ---- Variable access ----

    /// A binding never changes after its initialization.
    fn immutable(&self, b: &Binding) -> bool {
        match b.kind {
            BindingKind::Const
            | BindingKind::Using
            | BindingKind::ClassInner
            | BindingKind::FunctionName
            | BindingKind::HomeObject
            | BindingKind::Callee
            | BindingKind::NewTarget => true,
            BindingKind::This => !self
                .sa
                .function(self.sa.scope(b.scope).function)
                .kind
                .is_derived_constructor(),
            _ => false,
        }
    }

    /// `acc = new.target` from the frame. In a generator or async function body `new.target`
    /// is undefined (neither is ever constructed), and fp-3 of a resumed frame holds the
    /// generator object (FRAME 2), so the body loads undefined instead.
    fn load_new_target(&mut self) {
        if self.info.is_generator || self.info.is_async {
            self.b.lda_undefined();
        } else {
            self.b.lda_new_target();
        }
    }

    /// `acc = home object` from the frame (`GetHomeObject(running function)`).
    fn load_frame_home_object(&mut self) {
        let m = self.mark();
        let r = self.temp();
        self.b.lda_current_function().star(r);
        self.runtime(
            RuntimeId::GetHomeObject,
            RegisterList::new(r.local_index().unwrap(), 1),
        );
        self.release(m);
    }

    /// `acc = binding b`, `d` hops out if it is in a context. No hole check.
    pub(crate) fn load_binding_raw(&mut self, b: BindingId, d: u32, inside_typeof: bool) {
        let binding = self.binding(b);
        match binding.location {
            BindingLocation::Register(r) => {
                self.b.ldar(Register::local(r));
            }
            BindingLocation::Parameter(i) => {
                self.b.ldar(Register::parameter(i));
            }
            BindingLocation::Frame(FrameSlot::Receiver) => {
                self.b.ldar(Register::RECEIVER);
            }
            BindingLocation::Frame(FrameSlot::NewTarget) => self.load_new_target(),
            BindingLocation::Frame(FrameSlot::Callee) => {
                self.b.lda_current_function();
            }
            BindingLocation::Frame(FrameSlot::HomeObject) => self.load_frame_home_object(),
            BindingLocation::Context { slot, .. } => {
                let imm = self.immutable(binding);
                self.ctx_load(slot, d, imm);
            }
            BindingLocation::Global | BindingLocation::GlobalLexical => {
                let k = self.name_const(binding.name);
                if inside_typeof {
                    let s = self.global_slot(FeedbackSlotKind::LoadGlobalInsideTypeof, k);
                    self.b.lda_global_inside_typeof(k, s);
                } else {
                    let s = self.global_slot(FeedbackSlotKind::LoadGlobal, k);
                    self.b.lda_global(k, s);
                }
            }
            BindingLocation::Module => {
                let cell = self.cx.cells[b.index()];
                let d = self.hops_to(ScopeId(0));
                self.b.lda_module_variable(cell, d);
            }
        }
    }

    /// The hole check after a load of `b`.
    pub(crate) fn hole_check(&mut self, b: BindingId) {
        let binding = self.binding(b);
        if binding.kind == BindingKind::This {
            self.b.throw_super_not_called_if_hole();
        } else {
            let k = self.name_const(binding.name);
            self.b.throw_reference_error_if_hole(k);
        }
    }

    /// `acc = value of reference r`.
    pub(crate) fn load_ref(&mut self, r: &Reference, inside_typeof: bool) {
        match r.resolution {
            Resolution::Binding {
                binding,
                hops,
                hole_check,
            } => {
                self.load_binding_raw(binding, hops, inside_typeof);
                if hole_check && !self.vm_checks_hole(binding) {
                    self.expr_pos(r.span.start);
                    self.hole_check(binding);
                }
            }
            Resolution::Global => {
                let k = self.name_const(r.name);
                self.expr_pos(r.span.start);
                if inside_typeof {
                    let s = self.global_slot(FeedbackSlotKind::LoadGlobalInsideTypeof, k);
                    self.b.lda_global_inside_typeof(k, s);
                } else {
                    let s = self.global_slot(FeedbackSlotKind::LoadGlobal, k);
                    self.b.lda_global(k, s);
                }
            }
            // Strict code in a sloppy function (a class heritage or computed key): the
            // opcodes read with the function's strictness (lookup.rs).
            Resolution::Dynamic { .. } if self.strict_at(r) && !self.info.strict => {
                self.load_dynamic_strict(r, inside_typeof);
            }
            Resolution::Dynamic { .. } => {
                let k = self.name_const(r.name);
                self.expr_pos(r.span.start);
                if inside_typeof {
                    self.b.lda_lookup_slot_inside_typeof(k);
                } else {
                    self.b.lda_lookup_slot(k);
                }
            }
        }
    }

    /// Global lexical bindings are looked up by name: the VM checks their TDZ.
    fn vm_checks_hole(&self, b: BindingId) -> bool {
        matches!(
            self.binding(b).location,
            BindingLocation::Global | BindingLocation::GlobalLexical
        )
    }

    /// Store the accumulator into `b`, `d` hops out, as an initialization. The accumulator is
    /// unchanged afterwards (register and context stores keep it; others are wrapped).
    pub(crate) fn store_binding_raw(&mut self, b: BindingId, d: u32) {
        let binding = self.binding(b);
        match binding.location {
            BindingLocation::Register(r) => {
                self.b.star(Register::local(r));
            }
            BindingLocation::Parameter(i) => {
                self.b.star(Register::parameter(i));
            }
            BindingLocation::Context { slot, .. } => self.ctx_store(slot, d),
            BindingLocation::Global | BindingLocation::GlobalLexical => {
                // Initialization of a global binding that DeclareGlobals made.
                let m = self.mark();
                let args = self.temps(2);
                self.b.star(args.get(1));
                let k = self.name_const(binding.name);
                self.b.lda_constant(k).star(args.get(0));
                self.runtime(RuntimeId::InitializeGlobal, args);
                self.b.ldar(args.get(1));
                self.release(m);
            }
            BindingLocation::Module => {
                let cell = self.cx.cells[b.index()];
                let d = self.hops_to(ScopeId(0));
                self.b.sta_module_variable(cell, d);
            }
            BindingLocation::Frame(_) => {
                // `this` of a base function, new.target, callee, home object: never stored.
                debug_assert!(false, "store to a frame slot");
            }
        }
    }

    /// Initialize `b` (declared in the current scope or an ancestor in this function).
    pub(crate) fn store_binding_init(&mut self, b: BindingId) {
        let d = match self.binding(b).location {
            BindingLocation::Context { scope, .. } => self.hops_to(scope),
            _ => 0,
        };
        self.store_binding_raw(b, d);
    }

    /// `acc = binding b` from the current scope, with a hole check if `check`.
    pub(crate) fn load_binding_here(&mut self, b: BindingId, check: bool) {
        let d = match self.binding(b).location {
            BindingLocation::Context { scope, .. } => self.hops_to(scope),
            _ => 0,
        };
        self.load_binding_raw(b, d, false);
        if check && self.binding(b).tdz && !self.vm_checks_hole(b) {
            self.hole_check(b);
        }
    }

    /// PutValue of the accumulator to reference `r` (an assignment, not an initialization).
    /// The accumulator holds the value afterwards when `keep` is true.
    pub(crate) fn store_ref(&mut self, r: &Reference, keep: bool) {
        match r.resolution {
            Resolution::Binding {
                binding,
                hops,
                hole_check,
            } => {
                let bd = self.binding(binding);
                let global = matches!(
                    bd.location,
                    BindingLocation::Global | BindingLocation::GlobalLexical
                );
                if global {
                    // The VM checks the TDZ and `const` of global lexical bindings.
                    self.sta_global(bd.name, r.span.start, keep);
                    return;
                }
                if bd.kind.is_immutable() {
                    // Sloppy: an assignment to the function's own name is ignored.
                    if bd.kind == BindingKind::FunctionName && !self.strict_at(r) {
                        return;
                    }
                    // The TDZ check (ReferenceError) comes before the TypeError.
                    if hole_check {
                        self.check_hole_keeping_acc(binding, hops, r.span.start);
                    }
                    self.expr_pos(r.span.start);
                    self.runtime(RuntimeId::ThrowConstAssignError, RegisterList::EMPTY);
                    return;
                }
                if hole_check {
                    self.check_hole_keeping_acc(binding, hops, r.span.start);
                }
                self.store_binding_raw(binding, hops);
            }
            Resolution::Global => self.sta_global(r.name, r.span.start, keep),
            Resolution::Dynamic { .. } => {
                let k = self.name_const(r.name);
                let flags = u8::from(self.strict_at(r));
                self.expr_pos(r.span.start);
                if keep {
                    let m = self.mark();
                    let t = self.temp();
                    self.b.star(t);
                    self.b.sta_lookup_slot(k, flags);
                    self.b.ldar(t);
                    self.release(m);
                } else {
                    self.b.sta_lookup_slot(k, flags);
                }
            }
        }
    }

    /// Throw a ReferenceError if `binding` is the hole; the accumulator is kept.
    fn check_hole_keeping_acc(&mut self, binding: BindingId, hops: u32, at: u32) {
        let m = self.mark();
        let t = self.temp();
        self.b.star(t);
        self.load_binding_raw(binding, hops, false);
        self.expr_pos(at);
        self.hole_check(binding);
        self.b.ldar(t);
        self.release(m);
    }

    fn sta_global(&mut self, name: NameId, at: u32, keep: bool) {
        let k = self.name_const(name);
        let s = self.global_slot(FeedbackSlotKind::StoreGlobal, k);
        self.expr_pos(at);
        if keep {
            let m = self.mark();
            let t = self.temp();
            self.b.star(t);
            self.b.sta_global(k, s);
            self.b.ldar(t);
            self.release(m);
        } else {
            self.b.sta_global(k, s);
        }
    }

    /// Strict mode at a reference.
    pub(crate) fn strict_at(&self, r: &Reference) -> bool {
        self.sa.scope(r.scope).strict
    }

    pub(crate) fn strict(&self) -> bool {
        self.sa.scope(self.scope).strict
    }

    /// The reference of an identifier, by its oxc reference id.
    pub(crate) fn reference_of(
        &self,
        id: &oxc_ast::ast::IdentifierReference<'_>,
    ) -> Gen<&'s Reference> {
        id.reference_id
            .get()
            .and_then(|r| self.sa.reference(r))
            .ok_or_else(|| {
                super::internal(format!(
                    "no reference for `{}` at {}",
                    id.name.as_str(),
                    id.span.start
                ))
            })
    }

    pub(crate) fn implicit(
        &self,
        start: u32,
        kind: crate::scope::ImplicitRef,
    ) -> Gen<&'s Reference> {
        self.sa
            .implicit(start, kind)
            .ok_or_else(|| super::internal(format!("no implicit {kind:?} reference at {start}")))
    }

    pub(crate) fn declared(&self, id: &oxc_ast::ast::BindingIdentifier<'_>) -> Gen<BindingId> {
        self.sa.declared_at(id.span.start).ok_or_else(|| {
            super::internal(format!(
                "no binding for `{}` at {}",
                id.name.as_str(),
                id.span.start
            ))
        })
    }

    pub(crate) fn scope_of(&self, oxc: Option<oxc_semantic::ScopeId>, span: Span) -> Gen<ScopeId> {
        oxc.and_then(|s| self.sa.scope_for(s))
            .ok_or_else(|| super::internal(format!("no scope for the node at {}", span.start)))
    }

    pub(crate) fn function_id(&self, span: Span) -> Gen<crate::scope::FunctionId> {
        self.sa
            .function_at(span.start)
            .ok_or_else(|| super::internal(format!("no function at {}", span.start)))
    }

    // ---- Control flow ----

    pub(crate) fn find_break(&self, label: Option<&str>) -> Option<usize> {
        self.control.iter().rposition(|c| match c {
            Control::Target { labels, kind, .. } => match label {
                Some(l) => labels.contains(&l),
                None => *kind != TargetKind::Labeled,
            },
            _ => false,
        })
    }

    pub(crate) fn find_continue(&self, label: Option<&str>) -> Option<usize> {
        self.control.iter().rposition(|c| match c {
            Control::Target { labels, kind, .. } => {
                *kind == TargetKind::Loop && label.is_none_or(|l| labels.contains(&l))
            }
            _ => false,
        })
    }

    /// Leave statements by `cmd` for a `break`, `continue` or `return` statement: in an async
    /// generator, `return Expression;` first awaits the value in the accumulator (§14.10.1
    /// step 3; `async_gen.rs` AGF4); then [`FnGen::perform_now`].
    pub(crate) fn perform(&mut self, cmd: Command) {
        if cmd == Command::Return {
            self.await_return_statement(self.last_stmt);
        }
        self.perform_now(cmd);
    }

    /// Leave statements by `cmd`: pop contexts, run finally blocks, then jump or return. The
    /// completion value of a return is in the accumulator, as it is (no await).
    pub(crate) fn perform_now(&mut self, cmd: Command) {
        let stop = match cmd {
            Command::Break(i) | Command::Continue(i) => i + 1,
            Command::Return => 0,
        };
        let mut pop: Option<Register> = None;
        let mut k = self.control.len();
        while k > stop {
            k -= 1;
            match &mut self.control[k] {
                Control::Context { saved } => pop = Some(*saved),
                Control::Finally {
                    token,
                    value,
                    entry,
                    commands,
                } => {
                    let index = match commands.iter().position(|&c| c == cmd) {
                        Some(i) => i,
                        None => {
                            commands.push(cmd);
                            commands.len() - 1
                        }
                    };
                    let (token, value, entry) = (*token, *value, *entry);
                    if cmd == Command::Return {
                        self.b.star(value);
                    }
                    if let Some(r) = pop.take() {
                        self.b.pop_context(r);
                    }
                    self.b
                        .lda_smi(i32::try_from(index + 1).unwrap_or(i32::MAX))
                        .star(token)
                        .jump(entry);
                    return;
                }
                Control::Target { .. } => {}
            }
        }
        match cmd {
            Command::Break(i) | Command::Continue(i) => {
                if let Some(r) = pop {
                    self.b.pop_context(r);
                }
                let Control::Target {
                    break_to,
                    continue_to,
                    ..
                } = &self.control[i]
                else {
                    unreachable!("break target is a Target");
                };
                let to = if matches!(cmd, Command::Break(_)) {
                    *break_to
                } else {
                    continue_to.expect("continue target is a loop")
                };
                self.b.jump(to);
            }
            // The frame goes away: contexts need no pop.
            Command::Return => self.emit_return(),
        }
    }

    /// Return the accumulator from this function (derived constructors: `undefined` returns
    /// the checked `this`; an async function resolves its promise first, `async_fn.rs` AF4;
    /// an async generator completes its first request, `async_gen.rs`).
    pub(crate) fn emit_return(&mut self) {
        if let Some(object) = self.async_object() {
            self.async_return(object);
            return;
        }
        if let Some(object) = self.async_generator_object() {
            self.async_generator_return(object);
            return;
        }
        if self.info.kind.is_derived_constructor() {
            let this_check = self.b.new_label();
            self.b.jump_if_undefined(this_check);
            self.b.return_();
            self.b.bind(this_check);
            match self.info.this {
                Some(t) => {
                    self.load_binding_here(t, false);
                    self.b.throw_super_not_called_if_hole();
                }
                None => {
                    self.b.lda_the_hole().throw_super_not_called_if_hole();
                }
            }
        }
        self.b.return_();
    }

    /// `try { body } finally { finalizer }` (G5). `finalizer` sees no Finally entry.
    pub(crate) fn try_finally(
        &mut self,
        body: impl FnOnce(&mut Self) -> Gen,
        finalizer: impl FnOnce(&mut Self, Register) -> Gen,
    ) -> Gen {
        let m = self.mark();
        let token = self.temp();
        let value = self.temp();
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler, entry) = (
            self.b.new_label(),
            self.b.new_label(),
            self.b.new_label(),
            self.b.new_label(),
        );
        self.control.push(Control::Finally {
            token,
            value,
            entry,
            commands: Vec::new(),
        });
        let before = self.b.instruction_count();
        self.b.bind(start);
        // On an error the unit is abandoned: the control stack no longer matters.
        body(self)?;
        let Some(Control::Finally { commands, .. }) = self.control.pop() else {
            panic!("control stack out of order");
        };
        self.b.bind(end);
        let has_range = self.b.instruction_count() > before;
        if has_range {
            self.b
                .add_handler(start, end, handler, ctx, HandlerKind::Uncaught);
        }
        self.b.lda_smi(-1).star(token).jump(entry);
        self.b.bind(handler);
        self.b.star(value).lda_zero().star(token);
        self.b.bind(entry);
        let message = self.temp();
        self.b.lda_the_hole().set_pending_message().star(message);
        finalizer(self, token)?;
        self.b.ldar(message).set_pending_message();
        let (table, labels) = self.b.jump_table(1 + commands.len() as u32);
        let after = self.b.new_label();
        self.b.ldar(token).switch_on_smi(table, 0);
        self.b.jump(after);
        self.b.bind(labels[0]);
        self.b.ldar(value).re_throw();
        for (i, &cmd) in commands.iter().enumerate() {
            self.b.bind(labels[i + 1]);
            if cmd == Command::Return {
                self.b.ldar(value);
            }
            self.perform_now(cmd);
        }
        self.b.bind(after);
        self.release(m);
        Ok(())
    }

    /// `JumpLoop` to `header` with this function's loop depth.
    pub(crate) fn jump_loop(&mut self, header: Label) {
        let s = self.slot(FeedbackSlotKind::JumpLoop);
        let depth = (self.loop_depth - 1).max(0);
        self.b.jump_loop(header, depth, s);
    }

    // ---- Function bodies ----

    /// Function 0 of a script (a module's is `module.rs` `gen_module_program`).
    pub(crate) fn gen_program(&mut self, program: &'a Program<'a>) -> Gen {
        let root = ScopeId(0);
        self.scope = root;
        let root_info = self.sa.scope(root);
        let has_decls = root_info.bindings.iter().any(|&b| {
            matches!(
                self.binding(b).location,
                BindingLocation::Global | BindingLocation::GlobalLexical
            )
        });
        if has_decls {
            self.b.set_statement_position(0);
            self.runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY);
        }
        if let Some(extra) = self.cx.script_ctx_scope {
            let n = root_info.context_slots.len() as u32;
            self.push_context(root, |g| {
                let k = g.b.scope_constant(extra);
                g.b.create_function_context(k, n);
            });
        }
        self.init_frame_bindings()?;
        self.init_holes(root);
        self.hoist_functions(root)?;
        self.statements(&program.body)?;
        self.b.lda_undefined().return_();
        Ok(())
    }

    /// Copy `this`, `new.target`, the callee and the home object into their context slots,
    /// and store the hole into the `this` of a derived constructor.
    fn init_frame_bindings(&mut self) -> Gen {
        let info = self.info;
        let derived = info.kind.is_derived_constructor();
        for (b, load) in [
            (info.this, 0u8),
            (info.new_target, 1),
            (info.callee, 2),
            (info.home_object, 3),
        ] {
            let Some(b) = b else { continue };
            let binding = self.binding(b);
            // `this` of a derived constructor starts as the hole (init_holes stores it).
            if (load == 0 && derived)
                || !matches!(binding.location, BindingLocation::Context { .. })
            {
                continue;
            }
            match load {
                0 => {
                    self.b.ldar(Register::RECEIVER);
                }
                1 => self.load_new_target(),
                2 => {
                    self.b.lda_current_function();
                }
                _ => self.load_frame_home_object(),
            }
            self.store_binding_init(b);
        }
        Ok(())
    }

    /// ABI 5 (P0-D16): the mapped-parameter list of a function with a mapped arguments object:
    /// for formal parameter `i`, its context slot in the function scope `fs`, or none when a
    /// later parameter of the same name binds that name (CreateMappedArgumentsObject maps only
    /// the last one). Mapped parameters always live in `fs`'s context
    /// (`ContextReason::MappedArguments`). The VM reads the list from M3-P2 (VM-ARGS).
    fn set_mapped_parameters(&mut self, fs: ScopeId) -> Gen {
        let mut slots = Vec::with_capacity(self.info.params.len());
        for p in &self.info.params {
            slots.push(match p.binding.map(|b| self.binding(b).location) {
                None => None,
                Some(BindingLocation::Context { scope, slot }) if scope == fs => Some(slot),
                Some(other) => {
                    return Err(super::internal(format!(
                        "mapped parameter not in its function context: {other:?}"
                    )));
                }
            });
        }
        let unit_scope = self.cx.unit_scope(fs);
        self.b.set_mapped_parameters(unit_scope, slots);
        Ok(())
    }

    /// FunctionDeclarationInstantiation, then the body. `params` and `body` are the AST.
    pub(crate) fn prologue_and_params(&mut self, params: &'a FormalParameters<'a>) -> Gen {
        let info = self.info;
        // 1. Name scope (named function expression): its own context with the callee.
        if let Some(ns) = info.name_scope {
            let pushed = self.enter_scope(ns);
            let _ = pushed;
            if let Some(b) = info.self_name
                && matches!(self.binding(b).location, BindingLocation::Context { .. })
            {
                self.b.lda_current_function();
                self.store_binding_init(b);
            }
        }
        // 2. Function scope.
        let fs = info.scope;
        self.scope = fs;
        if self.sa.scope(fs).needs_context {
            self.push_context(fs, |g| g.create_context(fs));
        }
        // 3. Simple parameters captured into the context.
        if info.simple_params {
            for (i, p) in info.params.iter().enumerate() {
                if let Some(b) = p.binding
                    && let BindingLocation::Context { slot, .. } = self.binding(b).location
                {
                    self.b.ldar(Register::parameter(i as u32));
                    self.ctx_store(slot, 0);
                }
            }
        }
        // 4. this / new.target / callee / home object.
        self.init_frame_bindings()?;
        // 5. The arguments object (before parameter initialization).
        if let Some(a) = info.arguments {
            if info.mapped_arguments {
                self.set_mapped_parameters(fs)?;
                self.b.create_mapped_arguments();
            } else {
                self.b.create_unmapped_arguments();
            }
            // The binding is in the function scope; a split body's `var arguments` copies it
            // below (init_from).
            self.store_binding_init(a);
        }
        // 6. TDZ holes of the function scope (non-simple parameters, top-level lexicals).
        self.init_holes(fs);
        // 7. Non-simple parameters, left to right.
        if !info.simple_params {
            for (i, item) in params.items.iter().enumerate() {
                let value = Register::parameter(i as u32);
                if let Some(init) = &item.initializer {
                    let m = self.mark();
                    let t = self.temp();
                    let has_value = self.b.new_label();
                    let done = self.b.new_label();
                    self.b.ldar(value);
                    self.b.jump_if_not_undefined(has_value);
                    self.named_value(init, pattern_name(&item.pattern))?;
                    self.b.star(t).jump(done);
                    self.b.bind(has_value);
                    self.b.ldar(value).star(t);
                    self.b.bind(done);
                    self.bind_pattern(&item.pattern, t, super::pattern::BindMode::Init)?;
                    self.release(m);
                } else {
                    self.bind_pattern(&item.pattern, value, super::pattern::BindMode::Init)?;
                }
            }
            if let Some(rest) = &params.rest {
                let m = self.mark();
                let t = self.temp();
                self.b.create_rest_parameter().star(t);
                self.bind_pattern(&rest.rest.argument, t, super::pattern::BindMode::Init)?;
                self.release(m);
            }
        }
        // 8. Split body scope: body vars start with their parameter's value or undefined.
        if info.body_scope != fs {
            let bs = info.body_scope;
            self.enter_scope(bs);
            for &b in &self.sa.scope(bs).bindings {
                let binding = self.binding(b);
                if binding.kind != BindingKind::Var {
                    continue;
                }
                // Without init_from a var starts undefined: registers and new context slots
                // are undefined (VM contract).
                if let Some(from) = binding.init_from {
                    self.load_binding_here(from, false);
                    self.store_binding_init(b);
                }
            }
        }
        // 9. Function declarations.
        self.hoist_functions(info.body_scope)?;
        Ok(())
    }

    /// A `function` node (declaration, expression, method, accessor, constructor).
    pub(crate) fn gen_function(&mut self, node: &'a Function<'a>) -> Gen {
        if node.generator {
            return self.gen_generator(node);
        }
        if node.r#async {
            return self.gen_async_function(node);
        }
        self.scope = self.info.outer_scope.unwrap_or(self.info.scope);
        // A base class constructor runs the field initializers before its parameters.
        self.class_constructor_prologue()?;
        self.prologue_and_params(&node.params)?;
        if let Some(body) = &node.body {
            self.statements(&body.statements)?;
        }
        // The implicit return reports the closing brace.
        self.b
            .set_throw_position(Some(node.span.end.saturating_sub(1)));
        self.b.lda_undefined();
        self.emit_return();
        Ok(())
    }

    /// An arrow function.
    pub(crate) fn gen_arrow(&mut self, node: &'a ArrowFunctionExpression<'a>) -> Gen {
        if node.r#async {
            return self.gen_async_arrow(node);
        }
        self.scope = self.info.outer_scope.unwrap_or(self.info.scope);
        self.prologue_and_params(&node.params)?;
        if let Some(expr) = node.get_expression() {
            self.b.set_statement_position(expr.span().start);
            self.expr(expr)?;
            self.b.return_();
        } else if let Some(body) = node.get_function_body() {
            self.statements(&body.statements)?;
            self.b.lda_undefined().return_();
        }
        Ok(())
    }

    /// The synthetic functions of `class`: the default constructor, or (dispatched on the
    /// function kind) the instance or static members initializer (`class_fields.rs`).
    pub(crate) fn gen_default_constructor(&mut self, class: &'a Class<'a>) -> Gen {
        self.scope = self.info.outer_scope.unwrap_or(self.info.scope);
        let fs = self.info.scope;
        self.scope = fs;
        if self.sa.scope(fs).needs_context {
            self.push_context(fs, |g| g.create_context(fs));
        }
        self.b.set_statement_position(class.span.start);
        self.b.set_throw_position(Some(class.span.start));
        if matches!(
            self.info.kind,
            FunctionKind::InstanceMembersInitializer | FunctionKind::StaticMembersInitializer
        ) {
            self.init_frame_bindings()?;
            self.init_holes(fs);
            return self.members_initializer(class);
        }
        if self.info.kind == FunctionKind::DefaultDerivedConstructor {
            // constructor(...args) { super(...args); }, but with no `@@iterator` call
            // (§15.7.14 NOTE): ConstructWithSpread spreads the rest array as it is.
            let m = self.mark();
            let sup = self.temp();
            let args = self.temps(1);
            self.b.lda_current_function().get_super_constructor(sup);
            self.b.throw_if_not_super_constructor(sup);
            self.b.create_rest_parameter().star(args.get(0));
            self.b.lda_new_target();
            let s = self.slot(FeedbackSlotKind::Call);
            self.expr_pos(class.span.start);
            self.b.construct_with_spread(sup, args, 1, s);
            if self.has_instance_initializer() {
                let this = self.temp();
                self.b.star(this);
                self.default_derived_fields(this, class.span.start);
                self.b.ldar(this);
            }
            self.b.return_();
            self.release(m);
        } else {
            self.class_constructor_prologue()?;
            self.b.lda_undefined().return_();
        }
        Ok(())
    }

    /// Statements of a body.
    pub(crate) fn statements(&mut self, list: &'a oxc_allocator::Vec<'a, Statement<'a>>) -> Gen {
        for s in list {
            self.statement(s)?;
        }
        Ok(())
    }

    /// The `this` binding of a derived constructor, for `super()`.
    pub(crate) fn per_iteration(&self, scope: ScopeId) -> PerIteration {
        self.sa.scope(scope).per_iteration
    }

    /// Queue a function node for generation and emit `CreateClosure` for it.
    pub(crate) fn closure(
        &mut self,
        node: &'a Function<'a>,
        name: Option<UnitString>,
    ) -> Gen<crate::scope::FunctionId> {
        if node.generator {
            self.check_generator(node.r#async, node.span)?;
        } else if node.r#async {
            self.async_closure(node.span)?;
        }
        let id = self.function_id(node.span)?;
        self.jobs.push(Job::Function { node, id, name });
        Ok(id)
    }

    pub(crate) fn create_closure(&mut self, id: crate::scope::FunctionId) {
        let k = self.b.function_constant(ep9_bytecode::FunctionId(id.0));
        let s = self.slot(FeedbackSlotKind::CreateClosure);
        self.b.create_closure(k, s, 0);
    }
}

/// The name a binding pattern gives to an anonymous function default value.
pub(crate) fn pattern_name(p: &BindingPattern<'_>) -> Option<UnitString> {
    match p {
        BindingPattern::BindingIdentifier(id) => Some(UnitString::new(id.name.as_str())),
        _ => None,
    }
}
