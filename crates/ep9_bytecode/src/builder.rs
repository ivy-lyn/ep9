//! Builders for BytecodeGen: [`FunctionBuilder`] (one function) and [`UnitBuilder`] (the unit).
//!
//! ```text
//! let mut b = FunctionBuilder::new(meta);         // meta: kind, flags, name, parent, ...
//! let r = b.new_register();                       // registers: r0, r1, ...
//! let k = b.string_constant("x");                 // constants (deduplicated)
//! let s = b.feedback_slot(FeedbackSlotKind::LoadGlobal);
//! b.set_statement_position(12);                   // latched onto the next instruction
//! b.lda_global(k, s).star(r);                     // typed emitters, one per opcode
//! let done = b.new_label();
//! b.jump_if_to_boolean_false(done);               // forward jump to a label
//! ...
//! b.bind(done);
//! b.lda_undefined().return_();
//! let code = b.finish()?;                         // relaxes jump sizes, encodes
//! unit.set_function(FunctionId(0), code);
//! let verified = unit.finish()?;                  // interns strings, verifies
//! ```
//!
//! Determinism: output order is insertion order everywhere. Hash maps only answer "seen
//! before?"; the builder never iterates one. The unit string table order is fixed at
//! [`UnitBuilder::finish`]: scopes in id order, then functions in id order. So the bytes do
//! not depend on the order in which BytecodeGen finishes functions.

use std::collections::HashMap;

use crate::feedback::{FeedbackSlotKind, RuntimeId, TypeOfLiteral};
use crate::opcodes::{MAX_OPERANDS, Opcode, OpcodeFlags};
use crate::operand::{
    ConstIndex, FeedbackSlot, FunctionId, OperandKind, OperandScale, Register, RegisterList,
    ScopeId, StringId,
};
use crate::sha256::sha256;
use crate::string::UnitString;
use crate::unit::{
    BigIntConst, CodeKind, CompiledUnit, Constant, Function, FunctionFlags, FunctionKind, Handler,
    HandlerKind, NumberBits, Oddball, PositionEntry, Scope, ScopeFlags, ScopeKind, ScopeSlot,
    TsFlags, UnitHeader, VariableMode, VerifiedUnit,
};
use crate::verify::{MAX_REGISTERS, VerifyError, verify};

/// A jump target inside one [`FunctionBuilder`]. Bind it with [`FunctionBuilder::bind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label(u32);

/// A saved register allocation state; see [`FunctionBuilder::release_registers`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterMark(u32);

/// Why a build failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// The opcode cannot be emitted (prefix, `Illegal` or reserved).
    ReservedOpcode(Opcode),
    /// Operand `operand` of `opcode` does not fit: a list length differs from its count, a pair
    /// or triple has the wrong length, or a value does not fit 32 bits.
    Operand {
        /// The opcode.
        opcode: Opcode,
        /// Operand index.
        operand: usize,
    },
    /// A label was bound twice.
    LabelBoundTwice(Label),
    /// A label is used but never bound.
    UnboundLabel(Label),
    /// A jump goes the wrong way: only `JumpLoop` may go backward, and a forward jump (or a
    /// jump-table entry) must land on a later instruction.
    JumpDirection {
        /// The jump opcode.
        opcode: Opcode,
        /// Its instruction index in the builder.
        instruction: u32,
    },
    /// A jump table is not used by exactly one switch instruction.
    JumpTableUse(ConstIndex),
    /// `release_registers` got a mark above the current allocation.
    RegisterRelease,
    /// More than [`MAX_REGISTERS`] registers, or another size limit.
    Limit(&'static str),
    /// A function id was given twice.
    DuplicateFunction(FunctionId),
    /// A function id below the highest one was never given.
    MissingFunction(FunctionId),
    /// The finished unit fails verification (a BytecodeGen bug).
    Verify(VerifyError),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BuildError {}

/// Function metadata that BytecodeGen knows before it emits code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionMeta {
    /// Kind.
    pub kind: FunctionKind,
    /// Flags.
    pub flags: FunctionFlags,
    /// Name, if any.
    pub name: Option<UnitString>,
    /// Enclosing function; `None` for function 0.
    pub parent: Option<FunctionId>,
    /// Scope in which closures of this function are created.
    pub outer_scope: Option<ScopeId>,
    /// Source range start (UTF-8 byte offset).
    pub source_start: u32,
    /// Source range end (exclusive).
    pub source_end: u32,
    /// Formal parameter count (receiver not counted).
    pub parameter_count: u32,
    /// The `length` property value.
    pub length: u32,
}

impl FunctionMeta {
    /// Metadata for an entry function of `code_kind` over `0..source_len`.
    pub fn entry(code_kind: CodeKind, strict: bool, source_len: u32) -> FunctionMeta {
        FunctionMeta {
            kind: code_kind.entry_function_kind(),
            flags: if strict {
                FunctionFlags::STRICT
            } else {
                FunctionFlags::default()
            },
            name: None,
            parent: None,
            outer_scope: None,
            source_start: 0,
            source_end: source_len,
            parameter_count: 0,
            length: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Pending {
    Value(i64),
    List { first: i64, len: u32 },
    Label(Label),
}

/// Converts a typed emitter argument into a pending operand.
#[doc(hidden)]
pub trait IntoOperand {
    #[doc(hidden)]
    fn into_pending(self) -> PendingOperand;
}

/// Opaque operand of the typed emitters.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct PendingOperand(Pending);

macro_rules! into_value {
    ($($t:ty => |$v:ident| $e:expr;)*) => {
        $( impl IntoOperand for $t {
            fn into_pending(self) -> PendingOperand {
                let $v = self;
                PendingOperand(Pending::Value($e))
            }
        } )*
    };
}

into_value! {
    Register => |r| i64::from(r.to_operand());
    u32 => |v| i64::from(v);
    i32 => |v| i64::from(v);
    u8 => |v| i64::from(v);
    u16 => |v| i64::from(v);
    ConstIndex => |k| i64::from(k.0);
    FeedbackSlot => |s| i64::from(s.0);
    RuntimeId => |id| id as i64;
    TypeOfLiteral => |t| t as i64;
}

impl IntoOperand for RegisterList {
    fn into_pending(self) -> PendingOperand {
        PendingOperand(Pending::List {
            first: i64::from(self.first().to_operand()),
            len: self.len(),
        })
    }
}

impl IntoOperand for Label {
    fn into_pending(self) -> PendingOperand {
        PendingOperand(Pending::Label(self))
    }
}

/// The Rust type of a typed-emitter argument for an operand kind.
macro_rules! operand_type {
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

#[derive(Clone, Debug)]
struct PendingInsn {
    opcode: Opcode,
    operands: [Pending; MAX_OPERANDS],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PendingConst {
    Number(NumberBits),
    String(UnitString),
    BigInt(BigIntConst),
    Function(FunctionId),
    Scope(ScopeId),
    Oddball(Oddball),
    JumpTable(Vec<Label>),
    ArrayBoilerplate(Vec<Option<ConstIndex>>),
    ObjectBoilerplate(Vec<(ConstIndex, Option<ConstIndex>)>),
    TemplateObject(Vec<(UnitString, Option<UnitString>)>),
}

#[derive(Clone, Copy, Debug)]
struct PendingHandler {
    start: Label,
    end: Label,
    target: Label,
    context: Register,
    kind: HandlerKind,
}

/// Builds the bytecode and side tables of one function.
#[derive(Clone, Debug)]
pub struct FunctionBuilder {
    meta: FunctionMeta,
    insns: Vec<PendingInsn>,
    labels: Vec<Option<u32>>,
    constants: Vec<PendingConst>,
    const_index: HashMap<PendingConst, u32>,
    feedback: Vec<FeedbackSlotKind>,
    handlers: Vec<PendingHandler>,
    positions: Vec<(u32, u32, bool)>,
    pending_position: Option<(u32, bool)>,
    throw_position: Option<u32>,
    next_register: u32,
    max_registers: u32,
    error: Option<BuildError>,
}

impl FunctionBuilder {
    /// A builder for a function with this metadata.
    pub fn new(meta: FunctionMeta) -> Self {
        Self {
            meta,
            insns: Vec::new(),
            labels: Vec::new(),
            constants: Vec::new(),
            const_index: HashMap::new(),
            feedback: Vec::new(),
            handlers: Vec::new(),
            positions: Vec::new(),
            pending_position: None,
            throw_position: None,
            next_register: 0,
            max_registers: 0,
            error: None,
        }
    }

    /// The metadata.
    pub fn meta(&self) -> &FunctionMeta {
        &self.meta
    }

    fn fail(&mut self, e: BuildError) {
        if self.error.is_none() {
            self.error = Some(e);
        }
    }

    // ---- Registers ----

    /// Parameter register `a<index>`.
    pub fn parameter(&self, index: u32) -> Register {
        Register::parameter(index)
    }

    /// The receiver register.
    pub fn receiver(&self) -> Register {
        Register::RECEIVER
    }

    /// Allocates the next local register.
    pub fn new_register(&mut self) -> Register {
        self.new_register_list(1).first()
    }

    /// Allocates `count` consecutive local registers.
    pub fn new_register_list(&mut self, count: u32) -> RegisterList {
        let first = self.next_register;
        match first.checked_add(count) {
            Some(next) if next <= MAX_REGISTERS => {
                self.next_register = next;
                self.max_registers = self.max_registers.max(next);
            }
            _ => self.fail(BuildError::Limit("too many registers")),
        }
        RegisterList::new(first, count)
    }

    /// The current allocation state.
    pub fn register_mark(&self) -> RegisterMark {
        RegisterMark(self.next_register)
    }

    /// Frees every register allocated after `mark`. The frame keeps its maximum size.
    pub fn release_registers(&mut self, mark: RegisterMark) {
        if mark.0 > self.next_register {
            self.fail(BuildError::RegisterRelease);
        } else {
            self.next_register = mark.0;
        }
    }

    /// The frame size so far (largest number of locals ever allocated).
    pub fn register_count(&self) -> u32 {
        self.max_registers
    }

    // ---- Constants ----

    fn add_const(&mut self, c: PendingConst, dedup: bool) -> ConstIndex {
        if dedup && let Some(&i) = self.const_index.get(&c) {
            return ConstIndex(i);
        }
        let i = self.constants.len() as u32;
        if dedup {
            self.const_index.insert(c.clone(), i);
        }
        self.constants.push(c);
        ConstIndex(i)
    }

    /// A Number constant. Any NaN becomes the canonical NaN.
    pub fn number_constant(&mut self, v: f64) -> ConstIndex {
        self.add_const(PendingConst::Number(NumberBits::from_f64(v)), true)
    }

    /// A String constant.
    pub fn string_constant(&mut self, s: impl Into<UnitString>) -> ConstIndex {
        self.add_const(PendingConst::String(s.into()), true)
    }

    /// A BigInt constant.
    pub fn bigint_constant(&mut self, v: BigIntConst) -> ConstIndex {
        self.add_const(PendingConst::BigInt(v), true)
    }

    /// A nested-function constant for `CreateClosure`. `id` must be a direct child.
    pub fn function_constant(&mut self, id: FunctionId) -> ConstIndex {
        self.add_const(PendingConst::Function(id), true)
    }

    /// A scope constant for context creation.
    pub fn scope_constant(&mut self, id: ScopeId) -> ConstIndex {
        self.add_const(PendingConst::Scope(id), true)
    }

    /// An oddball constant (boilerplate values only).
    pub fn oddball_constant(&mut self, v: Oddball) -> ConstIndex {
        self.add_const(PendingConst::Oddball(v), true)
    }

    /// An array literal boilerplate. Elements refer to earlier constants; `None` = computed.
    pub fn array_boilerplate(&mut self, elements: Vec<Option<ConstIndex>>) -> ConstIndex {
        self.add_const(PendingConst::ArrayBoilerplate(elements), false)
    }

    /// An object literal boilerplate: (key, value) with earlier constants; `None` = computed.
    pub fn object_boilerplate(
        &mut self,
        properties: Vec<(ConstIndex, Option<ConstIndex>)>,
    ) -> ConstIndex {
        self.add_const(PendingConst::ObjectBoilerplate(properties), false)
    }

    /// A tagged-template description: (raw, cooked) per string; `None` cooked = undefined.
    pub fn template_object(
        &mut self,
        strings: Vec<(UnitString, Option<UnitString>)>,
    ) -> ConstIndex {
        self.add_const(PendingConst::TemplateObject(strings), false)
    }

    /// A jump table with `len` entries for one `SwitchOnSmi` (or, later,
    /// `SwitchOnGeneratorState`). Bind each returned label to its case.
    pub fn jump_table(&mut self, len: u32) -> (ConstIndex, Vec<Label>) {
        let labels: Vec<Label> = (0..len).map(|_| self.new_label()).collect();
        let k = self.add_const(PendingConst::JumpTable(labels.clone()), false);
        (k, labels)
    }

    // ---- Feedback ----

    /// Adds a feedback slot of `kind`.
    pub fn feedback_slot(&mut self, kind: FeedbackSlotKind) -> FeedbackSlot {
        self.feedback.push(kind);
        FeedbackSlot(self.feedback.len() as u32 - 1)
    }

    // ---- Labels, handlers, positions ----

    /// A new, unbound label.
    pub fn new_label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() as u32 - 1)
    }

    /// Binds `label` to the next instruction emitted (or to the end of the code).
    pub fn bind(&mut self, label: Label) {
        let at = self.insns.len() as u32;
        match &mut self.labels[label.0 as usize] {
            slot @ None => *slot = Some(at),
            Some(_) => self.fail(BuildError::LabelBoundTwice(label)),
        }
    }

    /// A new label bound to the next instruction (a loop header for `JumpLoop`).
    pub fn bound_label(&mut self) -> Label {
        let l = self.new_label();
        self.bind(l);
        l
    }

    /// Adds an exception handler: a throw in `start..end` jumps to `target` with the
    /// exception in the accumulator and the context restored from `context`. The table is
    /// sorted at `finish` (start asc, end desc, stable). Add an outer handler before its inner
    /// ones (at try entry, as V8 does): for two equal ranges the later one is the inner one.
    pub fn add_handler(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        context: Register,
        kind: HandlerKind,
    ) {
        self.handlers.push(PendingHandler {
            start,
            end,
            target,
            context,
            kind,
        });
    }

    /// The next instruction gets a statement position at source byte `offset`.
    pub fn set_statement_position(&mut self, offset: u32) {
        self.pending_position = Some((offset, true));
    }

    /// The next instruction gets an expression position, unless a statement position is
    /// already pending (the statement position wins, as in V8).
    pub fn set_expression_position(&mut self, offset: u32) {
        if !matches!(self.pending_position, Some((_, true))) {
            self.pending_position = Some((offset, false));
        }
    }

    /// From now on, an instruction that may throw ([`OpcodeFlags::THROWS`]) and has no pending
    /// position gets an expression position at source byte `offset`; `None` turns this off.
    /// BytecodeGen sets it to the expression it is generating, so every throwing instruction
    /// has its own position entry (M0 exit 7: a throw reports file:line:col).
    pub fn set_throw_position(&mut self, offset: Option<u32>) {
        self.throw_position = offset;
    }

    /// Number of instructions emitted so far.
    pub fn instruction_count(&self) -> u32 {
        self.insns.len() as u32
    }

    // ---- Emit ----

    /// Emits `opcode` with operands from the typed emitters (generic form, used by tests and
    /// tools; BytecodeGen calls the typed emitters).
    pub fn emit(&mut self, opcode: Opcode, operands: &[PendingOperand]) -> &mut Self {
        let info = opcode.info();
        if info.flags.contains(OpcodeFlags::RESERVED) || info.flags.contains(OpcodeFlags::PREFIX) {
            self.fail(BuildError::ReservedOpcode(opcode));
            return self;
        }
        if operands.len() != info.operands.len() {
            self.fail(BuildError::Operand {
                opcode,
                operand: operands.len().min(info.operands.len()),
            });
            return self;
        }
        let mut ops = [Pending::Value(0); MAX_OPERANDS];
        for (k, (o, spec)) in operands.iter().zip(info.operands).enumerate() {
            let ok = match (spec.kind, o.0) {
                (OperandKind::RegList | OperandKind::RegOutList, Pending::List { len, .. }) => {
                    matches!(operands.get(k + 1), Some(PendingOperand(Pending::Value(n))) if *n == i64::from(len))
                }
                (OperandKind::RegPair | OperandKind::RegOutPair, Pending::List { len, .. }) => {
                    len == 2
                }
                (OperandKind::RegOutTriple, Pending::List { len, .. }) => len == 3,
                (OperandKind::JumpFwd | OperandKind::JumpBack, Pending::Label(_)) => true,
                (_, Pending::Value(v)) => {
                    !matches!(
                        spec.kind,
                        OperandKind::RegList
                            | OperandKind::RegOutList
                            | OperandKind::RegPair
                            | OperandKind::RegOutPair
                            | OperandKind::RegOutTriple
                            | OperandKind::JumpFwd
                            | OperandKind::JumpBack
                    ) && (i32::try_from(v).is_ok() || u32::try_from(v).is_ok())
                }
                _ => false,
            };
            if !ok {
                self.fail(BuildError::Operand { opcode, operand: k });
                return self;
            }
            ops[k] = o.0;
        }
        let position = self.pending_position.take().or_else(|| {
            info.flags
                .contains(OpcodeFlags::THROWS)
                .then_some(self.throw_position)
                .flatten()
                .map(|src| (src, false))
        });
        if let Some((src, stmt)) = position {
            self.positions.push((self.insns.len() as u32, src, stmt));
        }
        self.insns.push(PendingInsn {
            opcode,
            operands: ops,
        });
        self
    }

    // ---- Finish ----

    fn label_index(&self, l: Label) -> Result<u32, BuildError> {
        self.labels[l.0 as usize].ok_or(BuildError::UnboundLabel(l))
    }

    /// Resolves labels, picks the smallest operand scales (jump relaxation), encodes the
    /// bytecode and builds the side tables.
    pub fn finish(self) -> Result<FunctionCode, BuildError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let n = self.insns.len();
        // Jump targets as instruction indices; check directions.
        let mut targets: Vec<Option<u32>> = vec![None; n];
        for (i, insn) in self.insns.iter().enumerate() {
            let info = insn.opcode.info();
            for (k, spec) in info.operands.iter().enumerate() {
                let Pending::Label(l) = insn.operands[k] else {
                    continue;
                };
                let t = self.label_index(l)?;
                let ok = match spec.kind {
                    OperandKind::JumpFwd => (t as usize) > i && (t as usize) < n,
                    _ => (t as usize) <= i,
                };
                if !ok {
                    return Err(BuildError::JumpDirection {
                        opcode: insn.opcode,
                        instruction: i as u32,
                    });
                }
                targets[i] = Some(t);
            }
        }
        // Jump tables: exactly one switch each; entries are later instructions.
        let mut table_user: Vec<Option<u32>> = vec![None; self.constants.len()];
        for (i, insn) in self.insns.iter().enumerate() {
            if !insn.opcode.info().flags.contains(OpcodeFlags::SWITCH) {
                continue;
            }
            for (k, spec) in insn.opcode.info().operands.iter().enumerate() {
                if spec.kind != OperandKind::Const {
                    continue;
                }
                let Pending::Value(c) = insn.operands[k] else {
                    unreachable!()
                };
                let Some(user) = usize::try_from(c).ok().and_then(|c| table_user.get_mut(c)) else {
                    continue; // the verifier reports the bad index
                };
                if user.replace(i as u32).is_some() {
                    return Err(BuildError::JumpTableUse(ConstIndex(c as u32)));
                }
            }
        }
        for (c, entry) in self.constants.iter().enumerate() {
            if let PendingConst::JumpTable(labels) = entry {
                let Some(user) = table_user[c] else {
                    return Err(BuildError::JumpTableUse(ConstIndex(c as u32)));
                };
                for &l in labels {
                    let t = self.label_index(l)?;
                    if t <= user || t as usize >= n {
                        return Err(BuildError::JumpDirection {
                            opcode: self.insns[user as usize].opcode,
                            instruction: user,
                        });
                    }
                }
            }
        }
        // Scales: start from what the non-jump operands need, then grow jumps to a fixpoint.
        // Sizes only grow, so distances only grow, so the loop ends and every scale is minimal.
        let mut scales: Vec<OperandScale> = self
            .insns
            .iter()
            .map(|insn| {
                let mut s = OperandScale::Single;
                for (k, spec) in insn.opcode.info().operands.iter().enumerate() {
                    if let (true, Some(v)) = (spec.kind.is_scalable(), value_of(insn.operands[k])) {
                        let need = if spec.kind.is_signed() {
                            OperandScale::for_signed(v as i32)
                        } else {
                            OperandScale::for_unsigned(v as u32)
                        };
                        s = s.max(need);
                    }
                }
                s
            })
            .collect();
        let mut offsets = vec![0u64; n + 1];
        loop {
            for i in 0..n {
                offsets[i + 1] = offsets[i] + self.insns[i].opcode.info().size(scales[i]) as u64;
            }
            let mut changed = false;
            for i in 0..n {
                if let Some(t) = targets[i] {
                    let d = offsets[i].abs_diff(offsets[t as usize]);
                    let need = OperandScale::for_unsigned(u32::try_from(d).unwrap_or(u32::MAX));
                    if need > scales[i] {
                        scales[i] = need;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let code_len = offsets[n];
        if code_len > u64::from(u32::MAX) / 2 {
            return Err(BuildError::Limit("bytecode larger than 2 GiB"));
        }
        // Encode.
        let mut code = Vec::with_capacity(code_len as usize);
        for (i, insn) in self.insns.iter().enumerate() {
            let scale = scales[i];
            if let Some(p) = Opcode::prefix_for(scale) {
                code.push(p as u8);
            }
            code.push(insn.opcode as u8);
            for (k, spec) in insn.opcode.info().operands.iter().enumerate() {
                let v = match insn.operands[k] {
                    Pending::Value(v) => v,
                    Pending::List { first, .. } => first,
                    Pending::Label(_) => {
                        let t = targets[i].expect("resolved above") as usize;
                        offsets[i].abs_diff(offsets[t]) as i64
                    }
                };
                let size = spec.kind.size(scale);
                code.extend_from_slice(&(v as u32).to_le_bytes()[..size]);
            }
            debug_assert_eq!(code.len() as u64, offsets[i + 1]);
        }
        let label_offsets: Vec<Option<u32>> = self
            .labels
            .iter()
            .map(|b| b.map(|i| offsets[i as usize] as u32))
            .collect();
        let off = |l: Label| -> Result<u32, BuildError> {
            label_offsets[l.0 as usize].ok_or(BuildError::UnboundLabel(l))
        };
        // Constants: jump tables become offsets from their switch instruction.
        let mut constants = Vec::with_capacity(self.constants.len());
        for (c, entry) in self.constants.into_iter().enumerate() {
            constants.push(match entry {
                PendingConst::JumpTable(labels) => {
                    let base = offsets[table_user[c].expect("checked above") as usize] as u32;
                    let mut t = Vec::with_capacity(labels.len());
                    for l in labels {
                        t.push(off(l)? - base);
                    }
                    FunctionConst::JumpTable(t.into())
                }
                other => FunctionConst::Other(other),
            });
        }
        // Handlers, sorted (start asc, end desc); stable for equal ranges.
        let mut handlers = Vec::with_capacity(self.handlers.len());
        for h in &self.handlers {
            handlers.push(Handler {
                start: off(h.start)?,
                end: off(h.end)?,
                target: off(h.target)?,
                context: h.context,
                kind: h.kind,
            });
        }
        handlers.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let positions = self
            .positions
            .iter()
            .map(|&(i, src, stmt)| PositionEntry {
                bytecode_offset: offsets[i as usize] as u32,
                source_offset: src,
                is_statement: stmt,
            })
            .collect();
        Ok(FunctionCode {
            meta: self.meta,
            register_count: self.max_registers,
            bytecode: code,
            constants,
            feedback_slots: self.feedback,
            handlers,
            positions,
        })
    }
}

fn value_of(p: Pending) -> Option<i64> {
    match p {
        Pending::Value(v) => Some(v),
        Pending::List { first, .. } => Some(first),
        Pending::Label(_) => None,
    }
}

/// Only emittable opcodes get a typed emitter: tt-munch the flags for `Prefix`/`Reserved`.
macro_rules! if_emittable {
    ([] $($body:tt)*) => { $($body)* };
    ([Reserved $(, $rest:ident)*] $($body:tt)*) => {};
    ([Prefix $(, $rest:ident)*] $($body:tt)*) => {};
    ([$first:ident $(, $rest:ident)*] $($body:tt)*) => { if_emittable!([$($rest),*] $($body)*); };
}

macro_rules! define_emitters {
    ( []
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        /// Typed emitters: one method per emittable opcode, named after the opcode's snake
        /// name, with one argument per operand.
        impl FunctionBuilder {
            $(
                if_emittable! { [$($flag),*]
                    $(#[$doc])*
                    #[allow(clippy::too_many_arguments)]
                    pub fn $snake(&mut self $(, $oname: operand_type!($okind))*) -> &mut Self {
                        self.emit(Opcode::$name, &[$( IntoOperand::into_pending($oname) ),*])
                    }
                }
            )*
        }
    };
}

crate::for_each_opcode!(define_emitters []);

#[derive(Clone, Debug, PartialEq, Eq)]
enum FunctionConst {
    JumpTable(Box<[u32]>),
    Other(PendingConst),
}

/// A finished function, ready for [`UnitBuilder::set_function`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionCode {
    meta: FunctionMeta,
    register_count: u32,
    bytecode: Vec<u8>,
    constants: Vec<FunctionConst>,
    feedback_slots: Vec<FeedbackSlotKind>,
    handlers: Vec<Handler>,
    positions: Vec<PositionEntry>,
}

impl FunctionCode {
    /// The encoded bytecode.
    pub fn bytecode(&self) -> &[u8] {
        &self.bytecode
    }

    /// The frame size.
    pub fn register_count(&self) -> u32 {
        self.register_count
    }

    /// The metadata.
    pub fn meta(&self) -> &FunctionMeta {
        &self.meta
    }
}

/// Unit-wide options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitOptions {
    /// Code kind of the entry function.
    pub code_kind: CodeKind,
    /// Top-level strictness (true for modules).
    pub strict: bool,
    /// TS flags for TypeScript source (H3); `None` for JavaScript.
    pub typescript: Option<TsFlags>,
    /// Engine build id (not covered by the content hash).
    pub engine_build_id: [u8; 16],
    /// Embed the source text (always for AOT units, H5).
    pub embed_source: bool,
}

#[derive(Clone, Debug)]
struct PendingScope {
    kind: ScopeKind,
    flags: ScopeFlags,
    outer: Option<ScopeId>,
    slots: Vec<(UnitString, VariableMode)>,
}

/// Builds a [`CompiledUnit`] from finished functions.
#[derive(Clone, Debug)]
pub struct UnitBuilder {
    opts: UnitOptions,
    source_hash: [u8; 32],
    source_len: u32,
    source: Option<String>,
    scopes: Vec<PendingScope>,
    functions: Vec<Option<FunctionCode>>,
    hints: Option<Vec<u8>>,
    error: Option<BuildError>,
}

impl UnitBuilder {
    /// A builder for the unit compiled from the decoded `source`.
    pub fn new(source: &str, opts: UnitOptions) -> Self {
        let source_len = u32::try_from(source.len());
        let mut b = Self {
            opts,
            source_hash: sha256(source.as_bytes()),
            source_len: source_len.unwrap_or(0),
            source: opts.embed_source.then(|| source.to_owned()),
            scopes: Vec::new(),
            functions: Vec::new(),
            hints: None,
            error: None,
        };
        if source_len.is_err() {
            b.error = Some(BuildError::Limit("source larger than 4 GiB"));
        }
        b
    }

    /// The source length in bytes.
    pub fn source_len(&self) -> u32 {
        self.source_len
    }

    /// Adds a scope descriptor. `outer` must be an earlier scope.
    pub fn add_scope(
        &mut self,
        kind: ScopeKind,
        flags: ScopeFlags,
        outer: Option<ScopeId>,
        slots: Vec<(UnitString, VariableMode)>,
    ) -> ScopeId {
        self.scopes.push(PendingScope {
            kind,
            flags,
            outer,
            slots,
        });
        ScopeId(self.scopes.len() as u32 - 1)
    }

    /// Sets the code of function `id` (its preorder index).
    pub fn set_function(&mut self, id: FunctionId, code: FunctionCode) {
        let i = id.index();
        if i >= self.functions.len() {
            self.functions.resize(i + 1, None);
        }
        if self.functions[i].is_some() {
            if self.error.is_none() {
                self.error = Some(BuildError::DuplicateFunction(id));
            }
            return;
        }
        self.functions[i] = Some(code);
    }

    /// Sets the hint section bytes (format owned by `ep9_facts`).
    pub fn set_hints(&mut self, bytes: Vec<u8>) {
        self.hints = Some(bytes);
    }

    /// Interns strings, assembles the unit and verifies it.
    pub fn finish(self) -> Result<VerifiedUnit, BuildError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let mut strings = StringTable::default();
        let scopes: Vec<Scope> = self
            .scopes
            .into_iter()
            .map(|s| Scope {
                kind: s.kind,
                flags: s.flags,
                outer: s.outer,
                slots: s
                    .slots
                    .into_iter()
                    .map(|(name, mode)| ScopeSlot {
                        name: strings.intern(name),
                        mode,
                    })
                    .collect(),
            })
            .collect();
        let mut functions = Vec::with_capacity(self.functions.len());
        for (i, f) in self.functions.into_iter().enumerate() {
            let Some(f) = f else {
                return Err(BuildError::MissingFunction(FunctionId(i as u32)));
            };
            let name = f.meta.name.map(|n| strings.intern(n));
            let constants = f
                .constants
                .into_iter()
                .map(|c| match c {
                    FunctionConst::JumpTable(t) => Constant::JumpTable(t),
                    FunctionConst::Other(p) => match p {
                        PendingConst::Number(n) => Constant::Number(n),
                        PendingConst::String(s) => Constant::String(strings.intern(s)),
                        PendingConst::BigInt(b) => Constant::BigInt(b),
                        PendingConst::Function(id) => Constant::Function(id),
                        PendingConst::Scope(id) => Constant::Scope(id),
                        PendingConst::Oddball(o) => Constant::Oddball(o),
                        PendingConst::ArrayBoilerplate(e) => Constant::ArrayBoilerplate(e.into()),
                        PendingConst::ObjectBoilerplate(p) => Constant::ObjectBoilerplate(p.into()),
                        PendingConst::TemplateObject(t) => Constant::TemplateObject(
                            t.into_iter()
                                .map(|(raw, cooked)| {
                                    (strings.intern(raw), cooked.map(|c| strings.intern(c)))
                                })
                                .collect(),
                        ),
                        PendingConst::JumpTable(_) => unreachable!("resolved in finish"),
                    },
                })
                .collect();
            functions.push(Function {
                kind: f.meta.kind,
                flags: f.meta.flags,
                name,
                parent: f.meta.parent,
                outer_scope: f.meta.outer_scope,
                source_start: f.meta.source_start,
                source_end: f.meta.source_end,
                parameter_count: f.meta.parameter_count,
                length: f.meta.length,
                register_count: f.register_count,
                bytecode: f.bytecode,
                constants,
                feedback_slots: f.feedback_slots,
                handlers: f.handlers,
                positions: f.positions,
            });
        }
        let unit = CompiledUnit {
            header: UnitHeader {
                code_kind: self.opts.code_kind,
                strict: self.opts.strict,
                typescript: self.opts.typescript,
                engine_build_id: self.opts.engine_build_id,
                source_hash: self.source_hash,
                source_len: self.source_len,
            },
            strings: strings.list,
            scopes,
            functions,
            source: self.source,
            hints: self.hints,
        };
        verify(unit).map_err(BuildError::Verify)
    }
}

#[derive(Default)]
struct StringTable {
    list: Vec<UnitString>,
    index: HashMap<UnitString, StringId>,
}

impl StringTable {
    fn intern(&mut self, s: UnitString) -> StringId {
        if let Some(&id) = self.index.get(&s) {
            return id;
        }
        let id = StringId(self.list.len() as u32);
        self.index.insert(s.clone(), id);
        self.list.push(s);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::Instructions;

    fn script_unit(src: &str) -> UnitBuilder {
        UnitBuilder::new(
            src,
            UnitOptions {
                code_kind: CodeKind::Script,
                strict: false,
                typescript: None,
                engine_build_id: [0; 16],
                embed_source: true,
            },
        )
    }

    fn entry(src_len: u32) -> FunctionBuilder {
        FunctionBuilder::new(FunctionMeta::entry(CodeKind::Script, false, src_len))
    }

    #[test]
    fn minimal_unit() {
        let src = "1";
        let mut b = entry(1);
        b.set_statement_position(0);
        b.lda_smi(1).return_();
        let code = b.finish().unwrap();
        assert_eq!(
            code.bytecode(),
            &[Opcode::LdaSmi as u8, 1, Opcode::Return as u8]
        );
        let mut u = script_unit(src);
        u.set_function(FunctionId(0), code);
        let v = u.finish().unwrap();
        assert_eq!(v.functions.len(), 1);
        assert_eq!(v.functions[0].positions.len(), 1);
    }

    #[test]
    fn throw_position_marks_only_throwing_instructions() {
        let src = "a.b";
        let mut b = entry(3);
        b.set_statement_position(0);
        b.set_throw_position(Some(2));
        let r = b.new_register();
        let k = b.string_constant("b");
        let s = b.feedback_slot(FeedbackSlotKind::LoadNamed);
        // LdaUndefined: statement position; Star: none (cannot throw);
        // GetNamedProperty: the default throw position.
        b.lda_undefined().star(r).get_named_property(r, k, s);
        // An explicit expression position wins over the default.
        b.set_expression_position(1);
        b.get_named_property(r, k, s);
        b.set_throw_position(None);
        b.get_named_property(r, k, s).return_();
        let code = b.finish().unwrap();
        let mut u = script_unit(src);
        u.set_function(FunctionId(0), code);
        let v = u.finish().unwrap();
        let p: Vec<_> = v.functions[0]
            .positions
            .iter()
            .map(|p| (p.bytecode_offset, p.source_offset, p.is_statement))
            .collect();
        assert_eq!(p, vec![(0, 0, true), (3, 2, false), (7, 1, false)]);
    }

    #[test]
    fn operand_scales_are_minimal() {
        let mut b = entry(0);
        b.lda_smi(127)
            .lda_smi(128)
            .lda_smi(-129)
            .lda_smi(40000)
            .lda_smi(-1);
        b.return_();
        let code = b.finish().unwrap();
        let scales: Vec<_> = Instructions::new(code.bytecode())
            .map(|i| i.unwrap().scale)
            .collect();
        use OperandScale::*;
        assert_eq!(scales, vec![Single, Wide, Wide, ExtraWide, Single, Single]);
    }

    #[test]
    fn jump_relaxation() {
        // A forward jump over 300 bytes needs Wide; a short one stays Single.
        let mut b = entry(0);
        let far = b.new_label();
        let near = b.new_label();
        b.jump_if_true(near);
        b.bind(near);
        b.jump_if_false(far);
        for _ in 0..150 {
            b.lda_smi(1);
        }
        b.bind(far);
        let header = b.bound_label();
        let slot = b.feedback_slot(FeedbackSlotKind::JumpLoop);
        b.lda_zero();
        b.jump_loop(header, 0, slot);
        let code = b.finish().unwrap();
        let insns: Vec<_> = Instructions::new(code.bytecode())
            .map(|i| i.unwrap())
            .collect();
        assert_eq!(insns[0].scale, OperandScale::Single);
        assert_eq!(insns[0].jump_target(), Some(insns[1].offset));
        assert_eq!(insns[1].scale, OperandScale::Wide);
        assert_eq!(insns[1].jump_target(), Some(insns[152].offset));
        let jl = insns.last().unwrap();
        assert_eq!(jl.opcode, Opcode::JumpLoop);
        assert_eq!(jl.jump_target(), Some(insns[152].offset));
    }

    #[test]
    fn build_errors() {
        // Unbound label.
        let mut b = entry(0);
        let l = b.new_label();
        b.jump(l);
        assert_eq!(b.finish().unwrap_err(), BuildError::UnboundLabel(l));
        // Backward Jump is not allowed.
        let mut b = entry(0);
        let l = b.bound_label();
        b.jump(l);
        assert!(matches!(b.finish(), Err(BuildError::JumpDirection { .. })));
        // List length differs from the count.
        let mut b = entry(0);
        let f = b.new_register();
        let args = b.new_register_list(2);
        let s = b.feedback_slot(FeedbackSlotKind::Call);
        b.call_undefined_receiver(f, args, 3, s);
        assert!(matches!(
            b.finish(),
            Err(BuildError::Operand { operand: 1, .. })
        ));
        // Reserved opcodes have no typed emitter; the generic path rejects them.
        let mut b = entry(0);
        b.emit(Opcode::TailCall, &[]);
        assert_eq!(
            b.finish().unwrap_err(),
            BuildError::ReservedOpcode(Opcode::TailCall)
        );
        // Releasing above the allocation.
        let mut b = entry(0);
        let m = b.register_mark();
        b.new_register();
        let m2 = b.register_mark();
        b.release_registers(m);
        b.release_registers(m2);
        assert_eq!(b.finish().unwrap_err(), BuildError::RegisterRelease);
    }

    #[test]
    fn constants_are_deduplicated_in_order() {
        let mut b = entry(0);
        let a = b.string_constant("a");
        let n = b.number_constant(1.5);
        let a2 = b.string_constant(UnitString::new("a"));
        let nan1 = b.number_constant(f64::NAN);
        let nan2 = b.number_constant(-f64::NAN);
        let z = b.number_constant(0.0);
        let mz = b.number_constant(-0.0);
        assert_eq!((a, n, a2), (ConstIndex(0), ConstIndex(1), ConstIndex(0)));
        assert_eq!(nan1, nan2);
        assert_ne!(z, mz);
    }

    #[test]
    fn registers_and_marks() {
        let mut b = entry(0);
        let r0 = b.new_register();
        let m = b.register_mark();
        let list = b.new_register_list(3);
        assert_eq!(r0, Register::local(0));
        assert_eq!(list.first(), Register::local(1));
        b.release_registers(m);
        let r1 = b.new_register();
        assert_eq!(r1, Register::local(1));
        assert_eq!(b.register_count(), 4);
    }

    #[test]
    fn handlers_sorted_and_strings_interned_in_id_order() {
        let src = "try { x } catch (e) {}";
        let mut b = entry(src.len() as u32);
        let ctx = b.new_register();
        let (outer_s, inner_s, inner_e, outer_e, h_in, h_out) = (
            b.new_label(),
            b.new_label(),
            b.new_label(),
            b.new_label(),
            b.new_label(),
            b.new_label(),
        );
        // Inner handler added first; the table must still be outer first.
        b.add_handler(inner_s, inner_e, h_in, ctx, HandlerKind::Caught);
        b.add_handler(outer_s, outer_e, h_out, ctx, HandlerKind::Uncaught);
        b.bind(outer_s);
        b.lda_zero();
        b.bind(inner_s);
        let name = b.string_constant("x");
        let slot = b.feedback_slot(FeedbackSlotKind::LoadGlobal);
        b.lda_global(name, slot);
        b.bind(inner_e);
        b.lda_zero();
        b.bind(outer_e);
        b.return_();
        b.bind(h_in);
        b.return_();
        b.bind(h_out);
        b.re_throw();
        let code = b.finish().unwrap();
        let mut u = script_unit(src);
        // Function names are interned after scopes, in function-id order.
        let scope = u.add_scope(
            ScopeKind::Script,
            ScopeFlags::default(),
            None,
            vec![(UnitString::new("x"), VariableMode::Var)],
        );
        assert_eq!(scope, ScopeId(0));
        u.set_function(FunctionId(0), code);
        let v = u.finish().unwrap();
        let hs = &v.functions[0].handlers;
        assert_eq!(hs.len(), 2);
        assert!(hs[0].start <= hs[1].start && hs[0].end >= hs[1].end);
        assert_eq!(hs[0].kind, HandlerKind::Uncaught);
        assert_eq!(v.strings, vec![UnitString::new("x")]);
    }

    #[test]
    fn missing_and_duplicate_functions() {
        let mut u = script_unit("");
        let mut b = entry(0);
        b.lda_undefined().return_();
        let code = b.finish().unwrap();
        u.set_function(FunctionId(1), code.clone());
        assert_eq!(
            u.clone().finish().unwrap_err(),
            BuildError::MissingFunction(FunctionId(0))
        );
        u.set_function(FunctionId(1), code);
        assert_eq!(
            u.finish().unwrap_err(),
            BuildError::DuplicateFunction(FunctionId(1))
        );
    }
}
