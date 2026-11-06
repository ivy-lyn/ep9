//! The register optimizer (M7-P1 REGOPT; plan v12 §4.2, V12-D32, V12-D46): fewer `Ldar`,
//! `Star` and `Mov` instructions, with no new opcode and no ABI change. The model is V8
//! `src/interpreter/bytecode-register-optimizer.cc`: registers that hold the same value form an
//! equivalence set, and a transfer to a temporary is done only when a later instruction needs
//! the value in that place.
//!
//! ```text
//! FnGen --typed emitters--> Emitter: records events --finish--> replay --> FunctionBuilder
//!   (instructions, labels, positions,                  (knows every handler range and every
//!    register allocation, handlers)                     label that a jump uses)
//! ```
//!
//! The replay needs the whole function because BytecodeGen adds a handler after its body
//! (`func.rs` `try_finally`), and because only the whole function tells which labels a jump
//! uses (a loop header is bound before its `JumpLoop`).
//!
//! | # | Rule | Why |
//! |---|---|---|
//! | O1 | A register is *observable* if it is the receiver, a parameter or a named local (a binding, G1). Every write to it is emitted | the frame, the debugger and the arguments object read it |
//! | O2 | Every load from a binding register (`Ldar` or `Mov` from a parameter or a named local) is emitted | H24 R4 "no binding folds": each read of a binding loads the binding |
//! | O3 | A transfer between the accumulator and temporaries is deferred: the destination joins the equivalence set of the source and is not materialized | BytecodeGen copies values to temporaries freely (G4) |
//! | O4 | A register operand that is a temporary reads a materialized equivalent (never the accumulator). A call operand reads only a local, or a parameter or the receiver that the function never writes. A read of the accumulator, of a list, a pair or an in-out register materializes it | V8 `GetInputRegister`; lists and pairs must stay consecutive locals; JIT code passes call operands from frame slots, and a parameter's slot keeps its incoming value (Spike C `tier.rs`) |
//! | O5 | Before a materialized register or the accumulator gets a new value, one other member of its set gets the value if no other member holds it and the member's value is not dead (a scan to its next write or release in the block) | no value is lost; this keeps the G4 hazard `a + (a = 1)` correct; a dead copy costs a `Mov` |
//! | O6 | Flush (materialize every live register, then end every set) before a jump, a switch, `Debugger`, `SuspendGenerator` and `ResumeGenerator`, and at each label that a jump, a jump table or a handler uses | each edge into a join point has the same state |
//! | O7 | Inside a handler range, before an instruction that may throw, materialize every temporary that was allocated at the range start | the handler reads only registers allocated before its range (stack allocation, G1) |
//! | O8 | At a statement position, materialize every temporary that is in a set with an observable register | a debugger write at a breakpoint does not change a temporary copy |
//! | O9 | A transfer with a pending statement position is emitted. An elided transfer drops a pending expression position | each statement keeps its position (H24 R5); an expression position names its own instruction |
//! | O10 | After a terminator the state is "flushed" with no code: only a label (O6) reaches the next instruction | the state at that label is the flushed state of its jumps |
//! | O11 | A new register whose set holds no value is materialized (its value is dead). A read of a register whose set has no materialized member is an internal error | a BytecodeGen bug (a read after release) must not compile |
//!
//! With the optimizer off (`optimize = false`), the replay emits every event as recorded, so the
//! output is the same as the output of a plain [`FunctionBuilder`].

use std::collections::{BTreeMap, HashMap, HashSet};

use ep9_bytecode::{
    BigIntConst, ConstIndex, FeedbackSlot, FeedbackSlotKind, FunctionBuilder, FunctionCode,
    FunctionId, FunctionMeta, HandlerKind, IntoOperand, Label, MAX_OPERANDS, Oddball, Opcode,
    OpcodeFlags, OperandKind, PendingOperand, Register, RegisterList, RegisterMark, RuntimeId,
    ScopeId, TypeOfLiteral, UnitString,
};

use super::{CodegenError, build_error};

/// One recorded operand.
#[derive(Clone, Copy, Debug)]
enum Arg {
    Reg(Register),
    List(RegisterList),
    Label(Label),
    Lit(PendingOperand),
}

impl Arg {
    fn pending(self) -> PendingOperand {
        match self {
            Self::Reg(r) => r.into_pending(),
            Self::List(l) => l.into_pending(),
            Self::Label(l) => l.into_pending(),
            Self::Lit(p) => p,
        }
    }
}

/// One recorded builder call, in program order.
#[derive(Clone, Copy, Debug)]
enum Ev {
    /// An instruction: its operands are `args[at..at + n]`.
    Insn {
        op: Opcode,
        at: u32,
        n: u8,
    },
    Bind(Label),
    Stmt(u32),
    Expr(u32),
    Throw(Option<u32>),
    /// Locals `first..first + count` are allocated.
    Alloc {
        first: u32,
        count: u32,
    },
    /// Every local from this index up is free.
    Release(u32),
}

#[derive(Clone, Copy, Debug)]
struct Handler {
    start: Label,
    end: Label,
    target: Label,
    context: Register,
    kind: HandlerKind,
}

/// A register allocation state (the [`RegisterMark`] of the builder and the local count).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    inner: RegisterMark,
    top: u32,
}

/// The function builder of BytecodeGen: the [`FunctionBuilder`] API, with the register
/// optimizer between BytecodeGen and the builder.
#[derive(Debug)]
pub(crate) struct Emitter {
    b: FunctionBuilder,
    optimize: bool,
    /// Named locals `r0..named` (bindings, G1). Temporaries follow.
    named: u32,
    events: Vec<Ev>,
    args: Vec<Arg>,
    /// Labels of jump tables (a switch uses each).
    table_labels: Vec<Label>,
    handlers: Vec<Handler>,
    instructions: u32,
    /// Locals allocated now.
    top: u32,
}

impl Emitter {
    /// A builder for a function with `named` named locals, which it allocates.
    pub(crate) fn new(meta: FunctionMeta, named: u32, optimize: bool) -> Self {
        let mut e = Self {
            b: FunctionBuilder::new(meta),
            optimize,
            named,
            events: Vec::with_capacity(64),
            args: Vec::with_capacity(64),
            table_labels: Vec::new(),
            handlers: Vec::new(),
            instructions: 0,
            top: 0,
        };
        if named > 0 {
            e.new_register_list(named);
        }
        e
    }

    /// See [`FunctionBuilder::set_mapped_parameters`].
    pub(crate) fn set_mapped_parameters(&mut self, scope: ScopeId, slots: Vec<Option<u32>>) {
        self.b.set_mapped_parameters(scope, slots);
    }

    // ---- Registers ----

    /// Allocates the next local register.
    pub(crate) fn new_register(&mut self) -> Register {
        self.new_register_list(1).first()
    }

    /// Allocates `count` consecutive local registers.
    pub(crate) fn new_register_list(&mut self, count: u32) -> RegisterList {
        let before = self.b.register_mark();
        let list = self.b.new_register_list(count);
        // On a limit error the builder allocates nothing (its `finish` fails).
        if count > 0 && self.b.register_mark() != before {
            let first = list.first().local_index().unwrap_or(0);
            self.events.push(Ev::Alloc { first, count });
            self.top = first + count;
        }
        list
    }

    /// The current allocation state.
    pub(crate) fn register_mark(&self) -> Mark {
        Mark {
            inner: self.b.register_mark(),
            top: self.top,
        }
    }

    /// Frees every register allocated after `mark`.
    pub(crate) fn release_registers(&mut self, mark: Mark) {
        self.b.release_registers(mark.inner);
        if mark.top <= self.top {
            self.events.push(Ev::Release(mark.top));
            self.top = mark.top;
        }
    }

    // ---- Constants and feedback (no code; passed through) ----

    pub(crate) fn number_constant(&mut self, v: f64) -> ConstIndex {
        self.b.number_constant(v)
    }

    pub(crate) fn string_constant(&mut self, s: impl Into<UnitString>) -> ConstIndex {
        self.b.string_constant(s)
    }

    pub(crate) fn bigint_constant(&mut self, v: BigIntConst) -> ConstIndex {
        self.b.bigint_constant(v)
    }

    pub(crate) fn function_constant(&mut self, id: FunctionId) -> ConstIndex {
        self.b.function_constant(id)
    }

    pub(crate) fn scope_constant(&mut self, id: ScopeId) -> ConstIndex {
        self.b.scope_constant(id)
    }

    pub(crate) fn oddball_constant(&mut self, v: Oddball) -> ConstIndex {
        self.b.oddball_constant(v)
    }

    pub(crate) fn array_boilerplate(&mut self, elements: Vec<Option<ConstIndex>>) -> ConstIndex {
        self.b.array_boilerplate(elements)
    }

    pub(crate) fn object_boilerplate(
        &mut self,
        properties: Vec<(ConstIndex, Option<ConstIndex>)>,
    ) -> ConstIndex {
        self.b.object_boilerplate(properties)
    }

    pub(crate) fn template_object(
        &mut self,
        strings: Vec<(UnitString, Option<UnitString>)>,
    ) -> ConstIndex {
        self.b.template_object(strings)
    }

    /// A jump table; a switch uses each of its labels.
    pub(crate) fn jump_table(&mut self, len: u32) -> (ConstIndex, Vec<Label>) {
        let (k, labels) = self.b.jump_table(len);
        self.table_labels.extend(labels.iter().copied());
        (k, labels)
    }

    pub(crate) fn feedback_slot(&mut self, kind: FeedbackSlotKind) -> FeedbackSlot {
        self.b.feedback_slot(kind)
    }

    // ---- Labels, handlers, positions (recorded) ----

    pub(crate) fn new_label(&mut self) -> Label {
        self.b.new_label()
    }

    /// Binds `label` to the next instruction (see [`FunctionBuilder::bind`]).
    pub(crate) fn bind(&mut self, label: Label) {
        self.events.push(Ev::Bind(label));
    }

    /// A new label bound to the next instruction.
    pub(crate) fn bound_label(&mut self) -> Label {
        let l = self.new_label();
        self.bind(l);
        l
    }

    /// See [`FunctionBuilder::add_handler`]. A range with no instruction after the optimizer
    /// gets no table entry (no instruction in it can throw).
    pub(crate) fn add_handler(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        context: Register,
        kind: HandlerKind,
    ) {
        self.handlers.push(Handler {
            start,
            end,
            target,
            context,
            kind,
        });
    }

    pub(crate) fn set_statement_position(&mut self, offset: u32) {
        self.events.push(Ev::Stmt(offset));
    }

    pub(crate) fn set_expression_position(&mut self, offset: u32) {
        self.events.push(Ev::Expr(offset));
    }

    pub(crate) fn set_throw_position(&mut self, offset: Option<u32>) {
        self.events.push(Ev::Throw(offset));
    }

    /// Instructions recorded so far (before the optimizer).
    pub(crate) fn instruction_count(&self) -> u32 {
        self.instructions
    }

    fn push(&mut self, op: Opcode, args: &[Arg]) {
        self.events.push(Ev::Insn {
            op,
            at: self.args.len() as u32,
            n: args.len() as u8,
        });
        self.args.extend_from_slice(args);
        self.instructions += 1;
    }

    // ---- Finish ----

    /// Replays the events into the builder (through the optimizer when it is on), then
    /// finishes the function.
    ///
    /// # Errors
    ///
    /// The builder error ([`build_error`]), or an internal optimizer error (O11).
    pub(crate) fn finish(self) -> Result<FunctionCode, CodegenError> {
        let Self {
            mut b,
            optimize,
            named,
            events,
            args,
            table_labels,
            handlers,
            ..
        } = self;
        let inner_at = if optimize {
            let mut r = Replay::new(&mut b, named, &events, &args, &table_labels, &handlers);
            r.run();
            if let Some(e) = r.error {
                return Err(CodegenError::Internal(e));
            }
            r.inner_at
        } else {
            plain_replay(&mut b, &events, &args)
        };
        for h in &handlers {
            match (inner_at.get(&h.start), inner_at.get(&h.end)) {
                (Some(s), Some(e)) if s >= e => continue,
                _ => b.add_handler(h.start, h.end, h.target, h.context, h.kind),
            }
        }
        b.finish().map_err(build_error)
    }
}

/// The replay with the optimizer off: every event as recorded. Returns the instruction index
/// of each bound label.
fn plain_replay(b: &mut FunctionBuilder, events: &[Ev], args: &[Arg]) -> HashMap<Label, u32> {
    let mut inner_at = HashMap::new();
    let mut ops = [0u32.into_pending(); MAX_OPERANDS];
    for ev in events {
        match *ev {
            Ev::Insn { op, at, n } => {
                let a = &args[at as usize..at as usize + n as usize];
                for (o, a) in ops.iter_mut().zip(a) {
                    *o = a.pending();
                }
                b.emit(op, &ops[..n as usize]);
            }
            Ev::Bind(l) => {
                inner_at.insert(l, b.instruction_count());
                b.bind(l);
            }
            Ev::Stmt(o) => b.set_statement_position(o),
            Ev::Expr(o) => b.set_expression_position(o),
            Ev::Throw(o) => b.set_throw_position(o),
            Ev::Alloc { .. } | Ev::Release(_) => {}
        }
    }
    inner_at
}

// ---- Typed emitters (the FunctionBuilder names; one per emittable opcode) ----

macro_rules! regopt_operand_type {
    (Reg) => {
        Register
    };
    (RegOut) => {
        Register
    };
    (RegInOut) => {
        Register
    };
    (RegList) => {
        RegisterList
    };
    (RegOutList) => {
        RegisterList
    };
    (RegPair) => {
        RegisterList
    };
    (RegOutTriple) => {
        RegisterList
    };
    (RegOutPair) => {
        RegisterList
    };
    (RegCount) => {
        u32
    };
    (Imm) => {
        i32
    };
    (UImm) => {
        u32
    };
    (Const) => {
        ConstIndex
    };
    (Slot) => {
        FeedbackSlot
    };
    (CtxSlot) => {
        u32
    };
    (CtxDepth) => {
        u32
    };
    (JumpFwd) => {
        Label
    };
    (JumpBack) => {
        Label
    };
    (Flag8) => {
        u8
    };
    (Flag16) => {
        u16
    };
    (Runtime) => {
        RuntimeId
    };
    (TypeOfLiteral) => {
        TypeOfLiteral
    };
}

macro_rules! regopt_arg {
    (Reg, $v:ident) => {
        Arg::Reg($v)
    };
    (RegOut, $v:ident) => {
        Arg::Reg($v)
    };
    (RegInOut, $v:ident) => {
        Arg::Reg($v)
    };
    (RegList, $v:ident) => {
        Arg::List($v)
    };
    (RegOutList, $v:ident) => {
        Arg::List($v)
    };
    (RegPair, $v:ident) => {
        Arg::List($v)
    };
    (RegOutTriple, $v:ident) => {
        Arg::List($v)
    };
    (RegOutPair, $v:ident) => {
        Arg::List($v)
    };
    (JumpFwd, $v:ident) => {
        Arg::Label($v)
    };
    (JumpBack, $v:ident) => {
        Arg::Label($v)
    };
    ($k:ident, $v:ident) => {
        Arg::Lit(IntoOperand::into_pending($v))
    };
}

/// Only emittable opcodes get a typed emitter (as in `ep9_bytecode`'s builder).
macro_rules! regopt_if_emittable {
    ([] $($body:tt)*) => { $($body)* };
    ([Reserved $(, $rest:ident)*] $($body:tt)*) => {};
    ([Prefix $(, $rest:ident)*] $($body:tt)*) => {};
    ([$first:ident $(, $rest:ident)*] $($body:tt)*) => {
        regopt_if_emittable!([$($rest),*] $($body)*);
    };
}

macro_rules! regopt_emitters {
    ( []
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        impl Emitter {
            $(
                regopt_if_emittable! { [$($flag),*]
                    // The names are the opcode names (`to_name`, `to_object`, ...).
                    #[allow(dead_code, clippy::too_many_arguments, clippy::wrong_self_convention)]
                    pub(crate) fn $snake(
                        &mut self $(, $oname: regopt_operand_type!($okind))*
                    ) -> &mut Self {
                        self.push(Opcode::$name, &[$( regopt_arg!($okind, $oname) ),*]);
                        self
                    }
                }
            )*
        }
    };
}

ep9_bytecode::for_each_opcode!(regopt_emitters []);

// ---- The replay with the optimizer ----

/// The scan window of [`Replay::dead`], in events. A temporary copy of a value is almost
/// always written again or released within a few instructions.
const DEAD_SCAN: usize = 32;

/// "No set": the register is alone in its set.
const NONE: u32 = u32::MAX;
/// Key of the accumulator.
const ACC: usize = 0;
/// Key of the receiver. Parameter `a<k>` is key `2 + k`; local `r<i>` is key `base + i`.
const RECEIVER: usize = 1;

struct Replay<'b> {
    b: &'b mut FunctionBuilder,
    events: &'b [Ev],
    args: &'b [Arg],
    /// The event that the replay is at.
    at: usize,
    named: usize,
    /// Key of `r0`.
    base: usize,
    /// Per key: its set (an index into `sets`), or [`NONE`].
    set: Vec<u32>,
    /// Per key: the register holds its value now.
    mat: Vec<bool>,
    /// Per key: allocated (the accumulator, the receiver, parameters and named locals always).
    live: Vec<bool>,
    /// Keys that may be unmaterialized (a key may repeat; `reset` reads them).
    unmat: Vec<usize>,
    /// Locals allocated now (the replay copy of the allocation state).
    top: usize,
    /// Sets of two or more keys, each sorted. Empty entries are free (`free`) and keep their
    /// allocations.
    sets: Vec<Vec<usize>>,
    free: Vec<u32>,
    /// A position for the next emitted instruction of the function (builder rules).
    pending: Option<(u32, bool)>,
    /// Per key below `base`: the function writes this parameter or the receiver (O4).
    written: Vec<bool>,
    /// Labels that a jump, a jump table or a handler target uses (O6).
    used: HashSet<Label>,
    /// Per event: the local count at the start of the innermost handler range around it
    /// (O7), or `u32::MAX`; empty when the function has no handler.
    handler_top: Vec<u32>,
    /// The instruction index of each bound label.
    inner_at: HashMap<Label, u32>,
    error: Option<String>,
    /// Scratch buffers that keep their allocations (`save_where`, `save_binding_copies`).
    todo: Vec<usize>,
    with_obs: Vec<bool>,
}

impl<'b> Replay<'b> {
    fn new(
        b: &'b mut FunctionBuilder,
        named: u32,
        events: &'b [Ev],
        args: &'b [Arg],
        table_labels: &[Label],
        handlers: &[Handler],
    ) -> Self {
        // Key space: every register that the code names.
        let mut params = b.meta().parameter_count as usize;
        let mut locals = (b.register_count() as usize).max(named as usize);
        let mut used: HashSet<Label> = table_labels.iter().copied().collect();
        used.extend(handlers.iter().map(|h| h.target));
        let mut see = |r: Register| {
            if let Some(i) = r.local_index() {
                locals = locals.max(i as usize + 1);
            } else if let Some(k) = r.parameter_index() {
                params = params.max(k as usize + 1);
            }
        };
        for a in args {
            match *a {
                Arg::Reg(r) => see(r),
                Arg::List(l) if !l.is_empty() => see(l.get(l.len() - 1)),
                Arg::Label(l) => {
                    used.insert(l);
                }
                _ => {}
            }
        }
        let handler_top = handler_tops(events, handlers);
        let base = 2 + params;
        let mut written = vec![false; base];
        let mut write = |r: Register| match (r.local_index(), r.parameter_index()) {
            (Some(_), _) => {}
            (_, Some(k)) => written[2 + k as usize] = true,
            _ => written[RECEIVER] = true,
        };
        for ev in events {
            let Ev::Insn { op, at, n } = *ev else {
                continue;
            };
            let a = &args[at as usize..at as usize + n as usize];
            for (spec, a) in op.info().operands.iter().zip(a) {
                if let (OperandKind::RegOut | OperandKind::RegInOut, Arg::Reg(r)) = (spec.kind, *a)
                {
                    write(r);
                }
            }
        }
        let keys = base + locals;
        let mut live = vec![false; keys];
        live[..base + named as usize].fill(true);
        Self {
            b,
            events,
            args,
            at: 0,
            named: named as usize,
            base,
            set: vec![NONE; keys],
            written,
            mat: vec![true; keys],
            live,
            unmat: Vec::new(),
            top: named as usize,
            sets: Vec::new(),
            free: Vec::new(),
            pending: None,
            used,
            handler_top,
            inner_at: HashMap::new(),
            error: None,
            todo: Vec::new(),
            with_obs: Vec::new(),
        }
    }

    fn run(&mut self) {
        let (events, args) = (self.events, self.args);
        for (i, ev) in events.iter().enumerate() {
            self.at = i;
            match *ev {
                Ev::Insn { op, at, n } => {
                    let a = &args[at as usize..at as usize + n as usize];
                    match (op, a) {
                        (Opcode::Ldar, [Arg::Reg(r)]) => {
                            let k = self.key(*r);
                            self.transfer(k, ACC);
                        }
                        (Opcode::Star, [Arg::Reg(r)]) => {
                            let k = self.key(*r);
                            self.transfer(ACC, k);
                        }
                        (Opcode::Mov, [Arg::Reg(s), Arg::Reg(d)]) => {
                            let (s, d) = (self.key(*s), self.key(*d));
                            self.transfer(s, d);
                        }
                        _ => self.instruction(i, op, a),
                    }
                }
                Ev::Bind(l) => {
                    if self.used.contains(&l) {
                        self.flush();
                    }
                    self.inner_at.insert(l, self.b.instruction_count());
                    self.b.bind(l);
                }
                Ev::Stmt(o) => {
                    self.save_binding_copies();
                    self.pending = Some((o, true));
                }
                Ev::Expr(o) => {
                    if !matches!(self.pending, Some((_, true))) {
                        self.pending = Some((o, false));
                    }
                }
                Ev::Throw(o) => self.b.set_throw_position(o),
                Ev::Alloc { first, count } => {
                    let k = self.base + first as usize;
                    for x in k..k + count as usize {
                        self.live[x] = true;
                        // A new register whose old value is gone holds no value of the code
                        // (V8 `AllocateRegister`); one whose value is in its set keeps it.
                        if !self.mat[x] && self.holder(x, x, false).is_none() {
                            self.leave(x);
                            self.mat[x] = true;
                        }
                    }
                    self.top = (first + count) as usize;
                }
                Ev::Release(top) => {
                    let from = (top as usize).max(self.named);
                    if from < self.top {
                        self.live[self.base + from..self.base + self.top].fill(false);
                    }
                    self.top = from;
                }
            }
        }
    }

    // ---- Keys ----

    fn key(&self, r: Register) -> usize {
        match (r.local_index(), r.parameter_index()) {
            (Some(i), _) => self.base + i as usize,
            (_, Some(k)) => 2 + k as usize,
            _ => RECEIVER,
        }
    }

    fn reg(&self, k: usize) -> Register {
        debug_assert_ne!(k, ACC);
        if k >= self.base {
            Register::local((k - self.base) as u32)
        } else if k >= 2 {
            Register::parameter((k - 2) as u32)
        } else {
            Register::RECEIVER
        }
    }

    /// O1: the receiver, a parameter or a named local.
    fn observable(&self, k: usize) -> bool {
        k != ACC && k < self.base + self.named
    }

    /// O2: a parameter or a named local.
    fn binding(&self, k: usize) -> bool {
        k >= 2 && k < self.base + self.named
    }

    fn fail(&mut self, what: &str) {
        if self.error.is_none() {
            self.error = Some(format!("register optimizer: {what}"));
        }
    }

    // ---- Sets ----

    fn same(&self, a: usize, b: usize) -> bool {
        a == b || (self.set[a] != NONE && self.set[a] == self.set[b])
    }

    /// The first materialized member of `k`'s set other than `except` (and other than the
    /// accumulator if `not_acc`), in key order.
    fn holder(&self, k: usize, except: usize, not_acc: bool) -> Option<usize> {
        let ok = |m: usize| self.mat[m] && m != except && !(not_acc && m == ACC);
        match self.set[k] {
            NONE => ok(k).then_some(k),
            s => self.sets[s as usize].iter().copied().find(|&m| ok(m)),
        }
    }

    fn leave(&mut self, k: usize) {
        let s = self.set[k];
        if s == NONE {
            return;
        }
        self.set[k] = NONE;
        let v = &mut self.sets[s as usize];
        v.retain(|&m| m != k);
        if v.len() == 1 {
            let other = v[0];
            v.clear();
            self.set[other] = NONE;
            self.free.push(s);
        }
    }

    /// Adds `k` (alone in its set) to the set of `to`.
    fn join(&mut self, k: usize, to: usize) {
        debug_assert!(self.set[k] == NONE && k != to);
        let s = match self.set[to] {
            NONE => {
                let s = match self.free.pop() {
                    Some(s) => s,
                    None => {
                        self.sets.push(Vec::new());
                        (self.sets.len() - 1) as u32
                    }
                };
                self.sets[s as usize].push(to);
                self.set[to] = s;
                s
            }
            s => s,
        };
        let v = &mut self.sets[s as usize];
        let at = v.partition_point(|&m| m < k);
        v.insert(at, k);
        self.set[k] = s;
    }

    // ---- Code ----

    /// Emits the transfer `dst = src` (no position).
    fn emit_transfer(&mut self, src: usize, dst: usize) {
        if src == ACC {
            let d = self.reg(dst);
            self.b.star(d);
        } else if dst == ACC {
            let s = self.reg(src);
            self.b.ldar(s);
        } else {
            let (s, d) = (self.reg(src), self.reg(dst));
            self.b.mov(s, d);
        }
    }

    /// Gives the pending position to the next instruction the builder emits.
    fn apply_pending(&mut self) {
        match self.pending.take() {
            Some((o, true)) => self.b.set_statement_position(o),
            Some((o, false)) => self.b.set_expression_position(o),
            None => {}
        }
    }

    /// O5: `k` holds its value and is about to change. If no other member of its set holds
    /// the value, the first live member gets it.
    fn save_value(&mut self, k: usize) {
        let s = self.set[k];
        if s == NONE || self.holder(k, k, false).is_some() {
            return;
        }
        let target = self.sets[s as usize]
            .iter()
            .copied()
            .find(|&m| m != k && (m == ACC || (self.live[m] && !self.dead(m))));
        if let Some(m) = target {
            self.emit_transfer(k, m);
            self.mat[m] = true;
        }
    }

    /// O5: no instruction from the current event on reads the value of temporary `k`: the next
    /// access to `k` in this block is a write, or `k` is released first. A jump, a switch, a
    /// terminator, a generator instruction, a label that a jump uses, or a throw into a handler
    /// that may read `k` ends the scan with "not dead"; so does the end of the scan window
    /// ([`DEAD_SCAN`] events), which keeps the compile time linear.
    fn dead(&self, k: usize) -> bool {
        if k < self.base + self.named {
            return false;
        }
        let r = (k - self.base) as u32;
        let is_r = |x: Register| x.local_index() == Some(r);
        let in_list = |l: RegisterList| {
            !l.is_empty()
                && l.first()
                    .local_index()
                    .is_some_and(|f| f <= r && r < f + l.len())
        };
        let end = (self.at + DEAD_SCAN).min(self.events.len());
        for (j, ev) in self.events.iter().enumerate().take(end).skip(self.at) {
            match *ev {
                Ev::Insn { op, at, n } => {
                    let a = &self.args[at as usize..at as usize + n as usize];
                    match (op, a) {
                        (Opcode::Ldar, [Arg::Reg(x)]) if is_r(*x) => return false,
                        (Opcode::Star, [Arg::Reg(x)]) if is_r(*x) => return true,
                        (Opcode::Mov, [Arg::Reg(x), _]) if is_r(*x) => return false,
                        (Opcode::Mov, [_, Arg::Reg(x)]) if is_r(*x) => return true,
                        (Opcode::Ldar | Opcode::Star | Opcode::Mov, _) => continue,
                        _ => {}
                    }
                    let info = op.info();
                    let f = info.flags;
                    if f.contains(OpcodeFlags::JUMP)
                        || f.contains(OpcodeFlags::SWITCH)
                        || f.contains(OpcodeFlags::TERMINATOR)
                        || f.contains(OpcodeFlags::GENERATOR)
                        || op == Opcode::Debugger
                    {
                        return false;
                    }
                    if f.contains(OpcodeFlags::THROWS)
                        && self
                            .handler_top
                            .get(j)
                            .is_some_and(|&t| t != u32::MAX && r < t)
                    {
                        return false;
                    }
                    let mut writes = false;
                    for (spec, a) in info.operands.iter().zip(a) {
                        match (spec.kind, *a) {
                            (OperandKind::Reg | OperandKind::RegInOut, Arg::Reg(x)) if is_r(x) => {
                                return false;
                            }
                            (OperandKind::RegList | OperandKind::RegPair, Arg::List(l))
                                if in_list(l) =>
                            {
                                return false;
                            }
                            (OperandKind::RegOut, Arg::Reg(x)) if is_r(x) => writes = true,
                            (
                                OperandKind::RegOutList
                                | OperandKind::RegOutPair
                                | OperandKind::RegOutTriple,
                                Arg::List(l),
                            ) if in_list(l) => writes = true,
                            _ => {}
                        }
                    }
                    if writes {
                        return true;
                    }
                }
                Ev::Bind(l) if self.used.contains(&l) => return false,
                Ev::Release(top) if r >= top => return true,
                _ => {}
            }
        }
        false
    }

    /// `k` is written next: save its value for its set, then make it a set of its own.
    fn prepare_output(&mut self, k: usize) {
        if self.mat[k] {
            self.save_value(k);
        }
        self.leave(k);
        self.mat[k] = true;
    }

    /// Makes register (or the accumulator) `k` hold its value.
    fn materialize(&mut self, k: usize) {
        if self.mat[k] {
            return;
        }
        match self.holder(k, k, false) {
            Some(m) => {
                self.emit_transfer(m, k);
                self.mat[k] = true;
            }
            None => {
                let what = if k == ACC {
                    "the accumulator".to_string()
                } else {
                    self.reg(k).to_string()
                };
                let at = self.b.instruction_count();
                self.fail(&format!(
                    "a read of {what} with no value at instruction {at} (read after release?)"
                ));
            }
        }
    }

    /// O4: the register to read for operand `k` (not the accumulator).
    fn input(&mut self, k: usize) -> Register {
        if !self.mat[k] {
            if let Some(m) = self.holder(k, k, true) {
                return self.reg(m);
            }
            self.materialize(k);
        }
        self.reg(k)
    }

    /// O4 for a call operand: a local, or a parameter or the receiver that the function
    /// never writes. A temporary `k` reads such a materialized member of its set, else it gets
    /// the value itself.
    fn call_input(&mut self, k: usize) -> Register {
        if k >= self.base && !self.mat[k] {
            let local = match self.set[k] {
                NONE => None,
                s => self.sets[s as usize].iter().copied().find(|&m| {
                    m != k && m != ACC && self.mat[m] && (m >= self.base || !self.written[m])
                }),
            };
            if let Some(m) = local {
                return self.reg(m);
            }
            self.materialize(k);
        }
        self.reg(k)
    }

    /// O6: every live register holds its value; every set ends.
    fn flush(&mut self) {
        for s in 0..self.sets.len() {
            let members = std::mem::take(&mut self.sets[s]);
            if members.is_empty() {
                continue;
            }
            match members.iter().copied().find(|&m| self.mat[m]) {
                Some(h) => {
                    for &x in &members {
                        if x != h && !self.mat[x] && (x == ACC || self.live[x]) {
                            self.emit_transfer(h, x);
                            self.mat[x] = true;
                        }
                    }
                }
                None => {
                    if members.iter().any(|&x| x == ACC || self.live[x]) {
                        self.fail("a set with live registers and no value");
                    }
                }
            }
            for &x in &members {
                self.set[x] = NONE;
            }
            // Keep the allocation for the next set.
            let mut members = members;
            members.clear();
            self.sets[s] = members;
        }
        self.free_all_sets();
    }

    /// Marks every entry of `sets` free (each is empty) and keeps the allocations. The free
    /// list gives the low indices first, as a new `sets` would.
    fn free_all_sets(&mut self) {
        self.free.clear();
        self.free.extend((0..self.sets.len() as u32).rev());
    }

    /// Materializes each live, unmaterialized member of a set for which `need` is true, and
    /// keeps the sets.
    fn save_where(&mut self, need: impl Fn(&Self, usize) -> bool) {
        let mut todo = std::mem::take(&mut self.todo);
        for s in 0..self.sets.len() {
            todo.clear();
            for &x in &self.sets[s] {
                if x != ACC && !self.mat[x] && self.live[x] && need(self, x) {
                    todo.push(x);
                }
            }
            if todo.is_empty() {
                continue;
            }
            let Some(h) = self.sets[s].iter().copied().find(|&m| self.mat[m]) else {
                self.fail("a set with live registers and no value");
                continue;
            };
            for &x in &todo {
                self.emit_transfer(h, x);
                self.mat[x] = true;
            }
        }
        self.todo = todo;
    }

    /// O7: before a throw inside a handler range.
    fn save_for_handler(&mut self, top: u32) {
        let limit = self.base + top as usize;
        self.save_where(|_, x| x < limit);
    }

    /// O8: at a statement position, temporaries that copy an observable register.
    fn save_binding_copies(&mut self) {
        if self.sets.iter().all(|v| v.is_empty()) {
            return;
        }
        let mut with_obs = std::mem::take(&mut self.with_obs);
        with_obs.clear();
        with_obs.extend(
            self.sets
                .iter()
                .map(|v| v.iter().any(|&m| self.observable(m))),
        );
        self.save_where(|r, x| with_obs[r.set[x] as usize]);
        self.with_obs = with_obs;
    }

    /// O10: after a terminator.
    fn reset(&mut self) {
        for s in 0..self.sets.len() {
            for &x in &self.sets[s] {
                self.set[x] = NONE;
            }
            self.sets[s].clear();
        }
        self.free_all_sets();
        let mut unmat = std::mem::take(&mut self.unmat);
        unmat.retain(|&k| {
            if k == ACC || self.live[k] {
                self.mat[k] = true;
            }
            !self.mat[k]
        });
        self.unmat = unmat;
    }

    /// `Ldar`, `Star` or `Mov` from BytecodeGen: `dst = src`.
    fn transfer(&mut self, src: usize, dst: usize) {
        let stmt = matches!(self.pending, Some((_, true)));
        if self.binding(src) || self.observable(dst) || stmt {
            self.real_transfer(src, dst);
            return;
        }
        // O9: the elided transfer drops its expression position.
        self.pending = None;
        if self.same(src, dst) {
            return;
        }
        if self.mat[dst] {
            self.save_value(dst);
        }
        self.leave(dst);
        self.join(dst, src);
        if self.mat[dst] {
            self.mat[dst] = false;
            self.unmat.push(dst);
        }
    }

    /// An emitted transfer (O1, O2, O9). The source is any register that holds the value.
    fn real_transfer(&mut self, src: usize, dst: usize) {
        let from = if src == ACC {
            // `Mov holder, dst` instead of `Ldar holder; Star dst` (V8 OutputRegisterTransfer).
            match (self.mat[ACC], self.holder(ACC, ACC, true)) {
                (false, Some(h)) => h,
                _ => {
                    self.materialize(ACC);
                    ACC
                }
            }
        } else {
            let r = self.input(src);
            self.key(r)
        };
        let holds = self.same(from, dst) && self.mat[dst];
        if !holds {
            self.prepare_output(dst);
        }
        self.apply_pending();
        self.emit_transfer(from, dst);
        if !holds {
            self.join(dst, from);
        }
    }

    /// Any other instruction (V8 `PrepareForBytecode`, then the operands).
    fn instruction(&mut self, at: usize, op: Opcode, recorded: &[Arg]) {
        let mut buf = [Arg::Lit(0u32.into_pending()); MAX_OPERANDS];
        buf[..recorded.len()].copy_from_slice(recorded);
        let args = &mut buf[..recorded.len()];
        let info = op.info();
        let flags = info.flags;
        if flags.contains(OpcodeFlags::JUMP)
            || flags.contains(OpcodeFlags::SWITCH)
            || matches!(
                op,
                Opcode::Debugger | Opcode::SuspendGenerator | Opcode::ResumeGenerator
            )
        {
            self.flush();
        } else if flags.contains(OpcodeFlags::THROWS) {
            let top = self.handler_top.get(at).copied().unwrap_or(u32::MAX);
            if top != u32::MAX {
                self.save_for_handler(top);
            }
        }
        if info.acc.reads() {
            self.materialize(ACC);
        }
        if info.acc.writes() || flags.contains(OpcodeFlags::CLOBBER) {
            self.prepare_output(ACC);
        }
        let call = flags.contains(OpcodeFlags::CALL);
        for (spec, a) in info.operands.iter().zip(args.iter_mut()) {
            match (spec.kind, *a) {
                (OperandKind::Reg, Arg::Reg(r)) => {
                    let k = self.key(r);
                    *a = Arg::Reg(if call {
                        self.call_input(k)
                    } else {
                        self.input(k)
                    });
                }
                (OperandKind::RegInOut, Arg::Reg(r)) => {
                    let k = self.key(r);
                    self.materialize(k);
                }
                (OperandKind::RegList | OperandKind::RegPair, Arg::List(l)) => {
                    for i in 0..l.len() {
                        let k = self.key(l.get(i));
                        self.materialize(k);
                    }
                }
                _ => {}
            }
        }
        for (spec, a) in info.operands.iter().zip(args.iter()) {
            match (spec.kind, *a) {
                (OperandKind::RegOut | OperandKind::RegInOut, Arg::Reg(r)) => {
                    let k = self.key(r);
                    self.prepare_output(k);
                }
                (
                    OperandKind::RegOutList | OperandKind::RegOutPair | OperandKind::RegOutTriple,
                    Arg::List(l),
                ) => {
                    for i in 0..l.len() {
                        let k = self.key(l.get(i));
                        self.prepare_output(k);
                    }
                }
                _ => {}
            }
        }
        self.apply_pending();
        let mut ops = [0u32.into_pending(); MAX_OPERANDS];
        for (o, a) in ops.iter_mut().zip(args.iter()) {
            *o = a.pending();
        }
        self.b.emit(op, &ops[..args.len()]);
        if flags.contains(OpcodeFlags::TERMINATOR) {
            self.reset();
        }
    }
}

/// O7: per event, the local count at the start of the innermost (largest) handler range
/// around it, or `u32::MAX`. Empty when the function has no handler.
fn handler_tops(events: &[Ev], handlers: &[Handler]) -> Vec<u32> {
    if handlers.is_empty() {
        return Vec::new();
    }
    let mut bind_at: HashMap<Label, usize> = HashMap::new();
    let mut top_at: Vec<u32> = Vec::with_capacity(events.len());
    let mut top = 0;
    for (i, ev) in events.iter().enumerate() {
        match *ev {
            Ev::Bind(l) => {
                bind_at.insert(l, i);
            }
            Ev::Alloc { first, count } => top = first + count,
            Ev::Release(t) => top = t,
            _ => {}
        }
        top_at.push(top);
    }
    // (event index, top) of each range start and end, sorted by index.
    let mut starts: Vec<(usize, u32)> = Vec::new();
    let mut ends: Vec<(usize, u32)> = Vec::new();
    for h in handlers {
        if let (Some(&s), Some(&e)) = (bind_at.get(&h.start), bind_at.get(&h.end)) {
            starts.push((s, top_at[s]));
            ends.push((e, top_at[s]));
        }
    }
    starts.sort_unstable();
    ends.sort_unstable();
    let mut open: BTreeMap<u32, usize> = BTreeMap::new();
    let (mut si, mut ei) = (0, 0);
    let mut out = vec![u32::MAX; events.len()];
    for (i, slot) in out.iter_mut().enumerate() {
        while si < starts.len() && starts[si].0 <= i {
            *open.entry(starts[si].1).or_insert(0) += 1;
            si += 1;
        }
        while ei < ends.len() && ends[ei].0 <= i {
            if let Some(c) = open.get_mut(&ends[ei].1) {
                *c -= 1;
                if *c == 0 {
                    open.remove(&ends[ei].1);
                }
            }
            ei += 1;
        }
        if let Some((&t, _)) = open.last_key_value() {
            *slot = t;
        }
    }
    out
}
