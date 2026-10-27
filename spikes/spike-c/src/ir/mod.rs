//! The Spike C IR: an SSA graph of one function in basic blocks, with a fact on each value and
//! the H30 rules (SPIKEC-1, M4-P1).
//!
//! ```text
//!  unknown                         generic path, no guard
//!     |
//!  predicted{source}               RUNTIME | PROFILED | HINTED (FactSource); needs a Guard (H32)
//!     |
//!  proven                          no guard: `&`, `|`, `^`, `~`, `<<`, `>>` give int32 (H30 PROVEN row)
//! ```
//!
//! | H30 rule | What this IR does |
//! |---|---|
//! | proven ⊑ predicted{source} ⊑ unknown | each value node carries a [`Trust`]; a phi takes the join of its inputs; only the [`Op::proven_by_spec`] ops make PROVEN |
//! | no raw heap constant; only the five ref nodes | there is no heap-constant op: [`Op::Const`] refuses a cell, and a heap value is named only through a [`RefNode`] ([`Graph::refs`]) |
//! | the guard policy is chosen per mode in the IR (H32) | every guard carries an [`ExitPolicy`] from the graph's [`Mode`]; JIT mode = [`ExitPolicy::Deopt`] |
//! | one graph builder for live and PROFILED facts | [`crate::build`] reads a FactSnapshot in both paths; a guard names its [`GuardBasis`] |
//!
//! Guards. A guard is a node whose failure leaves the compiled code. Its [`GuardBasis`] says
//! why it is there:
//! - [`GuardBasis::Fact`]: a predicted fact of that source (H32: JIT mode, DEOPT + counter).
//! - [`GuardBasis::Unbacked`]: a speculation that no fact backs. An unknown value should take
//!   the generic path, but the runtime table has no generic entry for most operations
//!   (`ep9_vm::RuntimeTable`: poll, call_any, add, inc, compare, stack_overflow, throw,
//!   unwind, and since M4-P3 get_keyed and set_keyed), so SPIKEC-1 checks the fast case
//!   inline instead. A keyed site whose record saw a key that is not an index, or a store
//!   of a value that is not an int32, takes the generic keyed entries instead and has no
//!   guard ([`Op::GetKeyedGeneric`], [`Op::SetKeyedGeneric`]; SPIKEC-4). Each one is counted in the
//!   report. An int32 BinaryOp or CompareOp record makes the int32 guards of its site fact
//!   guards (SPIKEC-2, [`crate::build`]; the exporter writes those records from FACTS2, M4-P2,
//!   on); an index-only keyed record of FACTS2 does the same for the key's int32 check of
//!   its site, while the element guards stay unbacked (SPIKEC-3: FACT_SCHEMA 1 has no
//!   receiver, bounds or hole field; [`crate::build`] "Keyed facts"). A value that an
//!   unbacked guard narrows keeps the trust [`Trust::Unknown`]: no fact knows it. The basis
//!   is a label: the emitted code of a guard is the same either way.
//!
//! A guard's [`ExitPolicy`] is set once, from the mode and the basis, when the guard is made
//! ([`Graph::set_guard`]); the builder never rewrites it, and shares a check only between
//! sites of one policy (MC-w14-20).
//!
//! Until month 5 builds the deopt entry, every exit goes through its exit stub to the shared
//! "deopt not built" trap ([`crate::emit_a64`]); each guard keeps the frame state a deopt
//! will need (the bytecode offset and the SSA value of every live register), placed so that
//! T0 can resume there ([`Graph::check`], guard placement).

use std::fmt;
use std::sync::Arc;

use ep9_bytecode::Register;
use ep9_facts::{FactSource, ShapeDesc};

/// A value or node of a [`Graph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

/// A basic block of a [`Graph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u32);

/// The trust level of a fact (H30). The order of the variants is the lattice order:
/// `Proven` ⊑ `Predicted` ⊑ `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Holds on every path: no guard.
    Proven,
    /// Seen by a fact source; holds only behind a guard (H32).
    Predicted(FactSource),
    /// Nothing known: the generic path.
    Unknown,
}

impl Trust {
    fn rank(self) -> u8 {
        match self {
            Trust::Proven => 0,
            Trust::Predicted(FactSource::Runtime) => 1,
            Trust::Predicted(FactSource::Profiled) => 2,
            Trust::Predicted(FactSource::Hinted) => 3,
            Trust::Unknown => 4,
        }
    }

    /// The least upper bound (a phi's trust). Two predicted sources join to the weaker one.
    pub fn join(self, other: Trust) -> Trust {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

/// The five ref nodes of H30: the only way the IR names a heap value (no raw heap constant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefNode {
    /// An isolate root by stable index (H13).
    RootRef(u32),
    /// A realm intrinsic by stable index (`IntrinsicId`).
    IntrinsicRef(u32),
    /// A shape used in a guard: an index into [`Graph::shapes`] (a shape by content, from a
    /// fact). H30 lowers it to a CellRef unless it is a snapshot root shape.
    ShapeRef(u32),
    /// A RW IC or link cell. In Spike C: feedback slot `n` of the compiled function, whose IC
    /// entry holds the run-local ShapeIds of the shapes a guard names (H30 "ShapeRef → CellRef").
    CellRef(u32),
    /// A constant of the unit's constant pool.
    UnitConst(u32),
}

/// The machine form of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repr {
    /// A Value (NaN-boxed, H6).
    Tagged,
    /// An int32 in the low 32 bits of a register.
    Int32,
    /// 0 or 1.
    Bool,
    /// A double, as its IEEE bits in a general register (SPIKEC-4; the emitter moves it to a
    /// d register for each op).
    Float64,
    /// No value (effects and control).
    None,
}

/// Where a parameter-like value comes from at entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// The receiver slot.
    Receiver,
    /// Parameter `a<i>`.
    Param(u32),
}

/// An int32 binary operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Int32Op {
    And,
    Or,
    Xor,
    /// `<<` (count & 31).
    Shl,
    /// `>>` (count & 31).
    Sar,
    /// `+`, with an overflow guard.
    Add,
    /// `-`, with an overflow guard.
    Sub,
    /// `*`, with an overflow guard and a -0 guard.
    Mul,
}

impl Int32Op {
    /// True for the H30 PROVEN list (`&`, `|`, `^`, `<<`, `>>`): the result is an int32 for any
    /// input, so the op needs no guard of its own.
    pub fn bitwise(self) -> bool {
        matches!(
            self,
            Int32Op::And | Int32Op::Or | Int32Op::Xor | Int32Op::Shl | Int32Op::Sar
        )
    }
}

/// A double binary operation (IEEE, as JS: the result is a number for any number inputs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// A signed int32 comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

/// What a node does.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A value of the frame at entry (receiver or parameter): Tagged.
    Incoming(Incoming),
    /// An immediate Value that is not a cell (undefined, a boolean, an int32 Value): Tagged.
    /// [`Graph::const_tagged`] refuses a cell: no raw heap constant (H30).
    Const(u64),
    /// An int32 constant.
    ConstInt32(i32),
    /// An SSA phi; inputs in the order of the block's predecessors.
    Phi,
    /// Guard: the Tagged input is an int32, or a double that is an int32 and not -0. Int32.
    CheckInt32,
    /// Guard: ToInt32 of the Tagged input for a bitwise op: an int32, or a double whose value is
    /// an int32 (-0 gives 0). Any other input (NaN, a non-integer, an object whose `valueOf`
    /// would run) leaves. Int32, PROVEN by the ToInt32 invariant.
    TruncateInt32,
    /// Guard: the Tagged input is a cell whose ShapeId has a case in the IC entry of `site`
    /// with the handler `handler` (an in-object field, or an Array's `length`): the live T0
    /// IC holds the run-local ids (H30: a ShapeRef lowers to a CellRef). `shapes` are the
    /// fact's shapes; a case the IC adds later with the same handler passes too, and reads
    /// the same place (the IC's own invariant). Tagged.
    CheckShape {
        /// [`RefNode::CellRef`] index: the feedback slot of the named load.
        site: u32,
        /// [`RefNode::ShapeRef`] indexes of the fact's shapes.
        shapes: Vec<u32>,
        /// The smi handler every case must have.
        handler: u64,
    },
    /// Guard: the Tagged input is an Array (kind byte). Tagged.
    CheckArray,
    /// The field at byte `offset` of the object input (behind a [`Op::CheckShape`]). Tagged.
    LoadField { offset: u32 },
    /// Guard: element `index` (Int32) of the array input: in `0..length`, inside the ELEMENTS
    /// capacity, not a hole. Tagged.
    LoadElement,
    /// Guard: stores the Int32 value into an existing element (same bounds and hole checks).
    /// An int32 is not a cell, so no barrier is needed (H11 elision E1).
    StoreElement,
    /// Guard: the Tagged input is a number (an int32 or a double Value); its value as a
    /// double. Float64 (SPIKEC-4).
    CheckNumber,
    /// The Int32 input as a double: Float64, PROVEN (every int32 is a number).
    Int32ToFloat64,
    /// A double op on two Float64 inputs: Float64, PROVEN (a number op on numbers gives a
    /// number). Not a guard: it cannot fail.
    Float64(FloatOp),
    /// Float64 to a Tagged number Value, in the form `Value::number` gives (an int32 Value
    /// for an integer in int32 range that is not -0, else a double with NaN purified).
    BoxFloat64,
    /// Slot `index` of the current context (the frame's context; `LdaCurrentContextSlot`):
    /// Tagged, unknown. Reads mutable memory: never shared or moved across an effect
    /// (SPIKEC-4, lin_solve).
    LoadContextSlot(u32),
    /// A call with an undefined receiver (`CallUndefinedReceiver0/1/2`): inputs = the callee,
    /// then the arguments, all Tagged; `regs` are the instruction's operand registers in the
    /// same order (`RuntimeTable::call_any` reads the operands from their VM frame slots).
    /// A Tagged, unknown result. An effect and a GC point; a throw unwinds (SPIKEC-4).
    Call { regs: Vec<Register> },
    /// `o[k]` by the generic keyed load (`ep9_vm::RuntimeTable::get_keyed`, entry 8,
    /// `layout::RT_GET_KEYED`; SPIKEC-4, P3-ORCH-9): the Tagged receiver and key, a Tagged
    /// result. Not a guard: any receiver and key take it. An effect and a GC point (a getter
    /// or a proxy trap may run JS); a throw unwinds from its bytecode offset. The entry
    /// records no feedback (native code records none, MC-w15-6).
    GetKeyedGeneric,
    /// `o[k] = v` by the generic keyed store (`RuntimeTable::set_keyed`, entry 9,
    /// `layout::RT_SET_KEYED`): the Tagged receiver, key and value. The entry does the
    /// store barrier, so any value may be stored. An effect and a GC point, as
    /// [`Op::GetKeyedGeneric`].
    SetKeyedGeneric,
    /// An int32 binary op; Add, Sub and Mul are guards (overflow, -0).
    Int32(Int32Op),
    /// An int32 comparison: Bool.
    Compare(Cond),
    /// The Tagged input is the `false` Value: Bool (`JumpIfFalse` on a non-Bool accumulator).
    IsFalse,
    /// The Tagged input is the `true` Value: Bool.
    IsTrue,
    /// Int32 to a Tagged int32 Value.
    BoxInt32,
    /// Bool to the Tagged `true` / `false` Value.
    BoxBool,
    /// `JumpLoop`'s budget poll (a GC point; `RuntimeTable::poll`).
    Poll,
    /// Terminator: go to the block's only successor.
    Jump,
    /// Terminator: on the Bool input go to successor 0 (true) or successor 1 (false).
    Branch,
    /// Terminator: return the Tagged input.
    Return,
}

impl Op {
    /// True for ops whose result is PROVEN by the spec for any input (H30 PROVEN row:
    /// constants, the int32 bitwise ops, ToInt32, a relational comparison's boolean).
    pub fn proven_by_spec(&self) -> bool {
        match self {
            Op::Const(_) | Op::ConstInt32(_) | Op::TruncateInt32 => true,
            Op::Int32(o) => o.bitwise(),
            Op::Compare(_) | Op::IsFalse | Op::IsTrue => true,
            // A number from numbers (SPIKEC-4).
            Op::Int32ToFloat64 | Op::Float64(_) => true,
            _ => false,
        }
    }

    /// True for a guard (a node that can leave the compiled code).
    pub fn is_guard(&self) -> bool {
        match self {
            Op::CheckInt32
            | Op::TruncateInt32
            | Op::CheckShape { .. }
            | Op::CheckArray
            | Op::CheckNumber
            | Op::LoadElement
            | Op::StoreElement => true,
            Op::Int32(o) => !o.bitwise(),
            _ => false,
        }
    }

    /// True for a block terminator.
    pub fn is_terminator(&self) -> bool {
        matches!(self, Op::Jump | Op::Branch | Op::Return)
    }

    /// True for an op that changes the heap or the VM (a store, a poll, which may run a GC
    /// or an interrupt, a generic keyed access, which may run JS): a deopt must not resume
    /// T0 before it once it ran (guard placement, [`Graph::check`]). A guard's own checks
    /// run before its effect.
    pub fn is_effect(&self) -> bool {
        matches!(
            self,
            Op::StoreElement
                | Op::Poll
                | Op::GetKeyedGeneric
                | Op::SetKeyedGeneric
                | Op::Call { .. }
        )
    }

    /// True for a call into the runtime table that may run JS or a GC and throw: the
    /// allocated caller-saved registers live across it are saved around the call.
    pub fn is_call(&self) -> bool {
        matches!(
            self,
            Op::GetKeyedGeneric | Op::SetKeyedGeneric | Op::Call { .. }
        )
    }

    /// True if the node has an effect other than its result (it stays without uses).
    pub fn has_effect(&self) -> bool {
        self.is_guard() || self.is_terminator() || self.is_effect()
    }
}

/// Why a guard is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardBasis {
    /// A predicted fact of this source.
    Fact(FactSource),
    /// No fact backs it: the generic path is not built (see the module docs).
    Unbacked,
}

/// What the compiled code does when a guard fails (H32, chosen per mode in the IR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitPolicy {
    /// Eager deopt to T0 at the guard's bytecode offset, plus an exit counter (JIT mode).
    Deopt,
}

/// The compile mode (H32 table columns). Spike C compiles in JIT mode only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Jit,
}

impl Mode {
    /// The H32 policy of a failed guard in this mode.
    pub fn exit_policy(self, basis: GuardBasis) -> ExitPolicy {
        match (self, basis) {
            (Mode::Jit, _) => ExitPolicy::Deopt,
        }
    }
}

/// The state a deopt at a guard rebuilds: the bytecode offset where T0 goes on and the SSA
/// value of each register that is live there (the graph builder's liveness, H02-Q1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameState {
    /// The bytecode offset T0 runs next.
    pub offset: u32,
    /// Live registers and their values.
    pub registers: Vec<(Register, NodeId)>,
    /// The accumulator, if live.
    pub acc: Option<NodeId>,
}

/// A guard's exit.
#[derive(Clone, Debug, PartialEq)]
pub struct Guard {
    /// Dense guard index (the trap names it).
    pub id: u32,
    pub basis: GuardBasis,
    pub policy: ExitPolicy,
    pub state: FrameState,
}

/// One node.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub op: Op,
    pub inputs: Vec<NodeId>,
    pub repr: Repr,
    /// The fact on the value (`Trust::Unknown` for effects and control).
    pub trust: Trust,
    pub block: BlockId,
    /// The bytecode offset of the instruction that made the node.
    pub offset: u32,
    pub guard: Option<Guard>,
    /// False once a pass removed it.
    pub live: bool,
}

/// One basic block: phis first, the terminator last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    /// The bytecode offset of its first instruction.
    pub offset: u32,
    pub nodes: Vec<NodeId>,
    pub preds: Vec<BlockId>,
    pub succs: Vec<BlockId>,
    /// The target of a `JumpLoop`.
    pub loop_header: bool,
}

/// The SSA graph of one function.
#[derive(Clone, Debug)]
pub struct Graph {
    pub name: String,
    pub mode: Mode,
    /// The fact source of the snapshot the graph was built from.
    pub source: FactSource,
    pub nodes: Vec<Node>,
    /// Blocks in bytecode order; block 0 is the entry.
    pub blocks: Vec<Block>,
    /// The shapes [`RefNode::ShapeRef`] names (from facts, by content).
    pub shapes: Vec<Arc<ShapeDesc>>,
    /// The ref nodes the graph uses.
    pub refs: Vec<RefNode>,
    /// Guards made so far (the next guard id).
    pub guards: u32,
    pub parameter_count: u32,
    pub register_count: u32,
    /// The reachable bytecode instructions (offset, opcode), in offset order.
    pub insns: Vec<(u32, ep9_bytecode::Opcode)>,
}

impl Graph {
    pub fn new(name: &str, source: FactSource, parameter_count: u32, register_count: u32) -> Graph {
        Graph {
            name: name.to_string(),
            mode: Mode::Jit,
            source,
            nodes: Vec::new(),
            blocks: Vec::new(),
            shapes: Vec::new(),
            refs: Vec::new(),
            guards: 0,
            parameter_count,
            register_count,
            insns: Vec::new(),
        }
    }

    pub fn node(&self, n: NodeId) -> &Node {
        &self.nodes[n.0 as usize]
    }

    pub fn node_mut(&mut self, n: NodeId) -> &mut Node {
        &mut self.nodes[n.0 as usize]
    }

    pub fn block(&self, b: BlockId) -> &Block {
        &self.blocks[b.0 as usize]
    }

    /// Adds a node (not yet in a block's list).
    pub fn add(
        &mut self,
        op: Op,
        inputs: Vec<NodeId>,
        repr: Repr,
        trust: Trust,
        block: BlockId,
        offset: u32,
    ) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            op,
            inputs,
            repr,
            trust,
            block,
            offset,
            guard: None,
            live: true,
        });
        id
    }

    /// A Tagged immediate. `None` for a cell: the IR has no raw heap constant (H30).
    pub fn const_tagged(bits: u64) -> Option<Op> {
        (!crate::vm_layout::is_cell(bits)).then_some(Op::Const(bits))
    }

    /// Makes `n` a guard with `basis` and the frame state `state`.
    pub fn set_guard(&mut self, n: NodeId, basis: GuardBasis, state: FrameState) {
        let id = self.guards;
        self.guards += 1;
        let policy = self.mode.exit_policy(basis);
        self.node_mut(n).guard = Some(Guard {
            id,
            basis,
            policy,
            state,
        });
    }

    /// The index of `r` in [`Graph::refs`] (added if new).
    pub fn add_ref(&mut self, r: RefNode) -> u32 {
        if let Some(i) = self.refs.iter().position(|x| *x == r) {
            return i as u32;
        }
        self.refs.push(r);
        (self.refs.len() - 1) as u32
    }

    /// Live nodes in block order (phis first in each block).
    pub fn order(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.blocks
            .iter()
            .flat_map(|b| b.nodes.iter().copied())
            .filter(|&n| self.node(n).live)
    }

    /// The live guards.
    pub fn guard_nodes(&self) -> impl Iterator<Item = (NodeId, &Guard)> + '_ {
        self.order()
            .filter_map(|n| self.node(n).guard.as_ref().map(|g| (n, g)))
    }

    /// Counts of the live guards: (by a fact, unbacked).
    pub fn guard_counts(&self) -> (usize, usize) {
        let (mut fact, mut unbacked) = (0, 0);
        for (_, g) in self.guard_nodes() {
            match g.basis {
                GuardBasis::Fact(_) => fact += 1,
                GuardBasis::Unbacked => unbacked += 1,
            }
        }
        (fact, unbacked)
    }

    /// The structural checks of protocol §6.1 (C5a) on this graph: every value has one trust
    /// level; PROVEN only from the H30 PROVEN list (or a phi or box of PROVEN values); every
    /// predicted value comes from a guard with a fact of that source (or a phi or box of
    /// such values); every guard has the mode's exit policy and a frame state at its own
    /// offset or later; no raw heap constant; SSA (an input is defined before its use in
    /// block order, or is a phi input).
    ///
    /// Guard placement (SPIKEC-3; the month-5 eager deopt resumes T0 at the frame state's
    /// offset and runs the code from there again, protocol §4):
    /// - every frame-state value is defined before the guard (in block order), so the exit
    ///   can read it there;
    /// - no effect (a StoreElement, a Poll) of an instruction at or after the frame state's
    ///   offset runs before the guard in its block: T0 would run that effect a second time;
    /// - a frame state before the guard's block (a cross-block resume) needs a block path from
    ///   the state's block to the guard's, and no effect on any such path: none at or after
    ///   the state's offset in the state's block, none in a block between, none before the
    ///   guard in its own block, and none at all in either end block when it lies on a cycle
    ///   of those paths (conservative: `Graph::effect_on_the_way`).
    pub fn check(&self) -> Result<(), String> {
        let mut seen = vec![false; self.nodes.len()];
        for b in &self.blocks {
            for &n in &b.nodes {
                if self.node(n).live {
                    seen[n.0 as usize] = true;
                }
            }
        }
        let mut defined = vec![false; self.nodes.len()];
        for (bi, b) in self.blocks.iter().enumerate() {
            // Bytecode offsets of the effects made so far in this block.
            let mut effects: Vec<u32> = Vec::new();
            for (pos, &n) in b.nodes.iter().enumerate() {
                let node = self.node(n);
                if !node.live {
                    continue;
                }
                let at = format!("{}: n{} {:?}", self.name, n.0, node.op);
                if let Some(g) = &node.guard {
                    let state = g.state.registers.iter().map(|&(_, v)| v).chain(g.state.acc);
                    for v in state {
                        if !defined[v.0 as usize] {
                            return Err(format!(
                                "{at}: frame-state value n{} is not defined before the guard",
                                v.0
                            ));
                        }
                    }
                    if let Some(&e) = effects.iter().find(|&&e| e >= g.state.offset) {
                        return Err(format!(
                            "{at}: guard g{} resumes T0 at @{}, after which an effect at @{e} \
                             already ran (not replayable)",
                            g.id, g.state.offset
                        ));
                    }
                    if g.state.offset < b.offset {
                        match self.effect_on_the_way(bi, pos, g.state.offset) {
                            Err(e) => return Err(format!("{at}: guard g{}: {e}", g.id)),
                            Ok(Some(e)) => {
                                return Err(format!(
                                    "{at}: guard g{} resumes T0 at @{} before its block (b{bi} \
                                     @{}), and an effect at @{e} on a path from there may \
                                     already have run (not replayable)",
                                    g.id, g.state.offset, b.offset
                                ));
                            }
                            Ok(None) => {}
                        }
                    }
                }
                if node.op.is_effect() {
                    effects.push(node.offset);
                }
                for &i in &node.inputs {
                    if !self.node(i).live || !seen[i.0 as usize] {
                        return Err(format!("{at}: input n{} is not in the graph", i.0));
                    }
                    if node.op != Op::Phi && !defined[i.0 as usize] {
                        return Err(format!("{at}: input n{} is used before it is defined", i.0));
                    }
                }
                defined[n.0 as usize] = true;
                if let Op::Const(bits) = node.op
                    && crate::vm_layout::is_cell(bits)
                {
                    return Err(format!("{at}: a raw heap constant"));
                }
                if let Op::CheckShape { site, shapes, .. } = &node.op {
                    if !self.refs.contains(&RefNode::CellRef(*site)) {
                        return Err(format!("{at}: its CellRef is not in the ref list"));
                    }
                    if shapes.iter().any(|s| {
                        *s as usize >= self.shapes.len()
                            || !self.refs.contains(&RefNode::ShapeRef(*s))
                    }) {
                        return Err(format!("{at}: a ShapeRef is not in the ref list"));
                    }
                }
                let carrier = matches!(
                    node.op,
                    Op::Phi | Op::BoxInt32 | Op::BoxBool | Op::BoxFloat64
                ) && !node.inputs.is_empty();
                match node.trust {
                    Trust::Proven => {
                        let ok = node.op.proven_by_spec()
                            || (carrier
                                && node
                                    .inputs
                                    .iter()
                                    .all(|&i| self.node(i).trust == Trust::Proven));
                        if !ok {
                            return Err(format!("{at}: PROVEN, but not from the H30 PROVEN list"));
                        }
                    }
                    Trust::Predicted(src) => {
                        let guarded = node
                            .guard
                            .as_ref()
                            .is_some_and(|g| g.basis == GuardBasis::Fact(src));
                        let carried = carrier
                            && node.inputs.iter().all(|&i| {
                                let t = self.node(i).trust;
                                t != Trust::Unknown && t.rank() <= Trust::Predicted(src).rank()
                            });
                        if !(guarded || carried) {
                            return Err(format!(
                                "{at}: predicted {src:?} without a guard of that fact"
                            ));
                        }
                    }
                    Trust::Unknown => {}
                }
                if node.op.is_guard() != node.guard.is_some() {
                    return Err(format!("{at}: a guard op without a guard, or the reverse"));
                }
                if let Some(g) = &node.guard {
                    if g.policy != self.mode.exit_policy(g.basis) {
                        return Err(format!(
                            "{at}: exit policy {:?} is not the mode's",
                            g.policy
                        ));
                    }
                    if g.state.offset < node.offset
                        && !matches!(node.op, Op::CheckInt32 | Op::CheckNumber)
                    {
                        return Err(format!("{at}: frame state before the guard"));
                    }
                    for &(_, v) in &g.state.registers {
                        if !self.node(v).live {
                            return Err(format!("{at}: a frame state value was removed"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Graph {
    /// The blocks reachable from block `from` (itself included) along `succs`, or along
    /// `preds` when `back`.
    fn reach(&self, from: usize, back: bool) -> Vec<bool> {
        let mut seen = vec![false; self.blocks.len()];
        let mut work = vec![from];
        while let Some(x) = work.pop() {
            if std::mem::replace(&mut seen[x], true) {
                continue;
            }
            let b = &self.blocks[x];
            let next = if back { &b.preds } else { &b.succs };
            work.extend(next.iter().map(|t| t.0 as usize));
        }
        seen
    }

    /// For the guard at position `pos` of block `b` whose frame state offset `s` lies before
    /// that block: an effect that may run after T0's resume point `s` and before the guard on
    /// some block path, as its bytecode offset. The state's block is the block with the
    /// greatest start offset at or below `s`. On the paths from it to `b` every effect
    /// counts, except in the state's block those before `s` and in `b` those after the
    /// guard, unless that end block lies on a cycle of the paths (then all of its effects
    /// count). `Err` when no block path leads from `s` to `b`.
    fn effect_on_the_way(&self, b: usize, pos: usize, s: u32) -> Result<Option<u32>, String> {
        let a = (0..self.blocks.len())
            .filter(|&i| self.blocks[i].offset <= s)
            .max_by_key(|&i| self.blocks[i].offset)
            .ok_or_else(|| format!("its frame state @{s} is before every block"))?;
        let fwd = self.reach(a, false);
        let back = self.reach(b, true);
        if !back[a] {
            return Err(format!(
                "its frame state @{s} (in b{a}) has no block path to the guard's block b{b}"
            ));
        }
        let on = |i: usize| fwd[i] && back[i];
        // On a cycle of the paths: a successor on the paths leads back to it.
        let cyclic = |x: usize| {
            self.blocks[x]
                .succs
                .iter()
                .map(|t| t.0 as usize)
                .any(|t| on(t) && self.reach(t, false)[x])
        };
        let effect = |n: NodeId| {
            let node = self.node(n);
            (node.live && node.op.is_effect()).then_some(node.offset)
        };
        for x in (0..self.blocks.len()).filter(|&x| on(x)) {
            let nodes = &self.blocks[x].nodes;
            let all = (x != a && x != b) || cyclic(x);
            let found = if all {
                nodes.iter().find_map(|&n| effect(n))
            } else if x == a {
                nodes.iter().filter_map(|&n| effect(n)).find(|&e| e >= s)
            } else {
                nodes[..pos].iter().find_map(|&n| effect(n))
            };
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }
}

impl fmt::Display for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "graph {} ({:?}, facts {:?})",
            self.name, self.mode, self.source
        )?;
        for (i, b) in self.blocks.iter().enumerate() {
            let preds: Vec<String> = b.preds.iter().map(|p| format!("b{}", p.0)).collect();
            let succs: Vec<String> = b.succs.iter().map(|p| format!("b{}", p.0)).collect();
            writeln!(
                f,
                "b{i} @{}{} preds [{}] succs [{}]",
                b.offset,
                if b.loop_header { " loop" } else { "" },
                preds.join(" "),
                succs.join(" ")
            )?;
            for &n in &b.nodes {
                let node = self.node(n);
                if !node.live {
                    continue;
                }
                let ins: Vec<String> = node.inputs.iter().map(|i| format!("n{}", i.0)).collect();
                let g = node.guard.as_ref().map_or(String::new(), |g| {
                    format!("  guard g{} {:?} -> @{}", g.id, g.basis, g.state.offset)
                });
                writeln!(
                    f,
                    "  n{} = {:?}({}) : {:?} {:?} @{}{g}",
                    n.0,
                    node.op,
                    ins.join(", "),
                    node.repr,
                    node.trust,
                    node.offset
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lattice_join() {
        let r = Trust::Predicted(FactSource::Runtime);
        let h = Trust::Predicted(FactSource::Hinted);
        assert_eq!(Trust::Proven.join(r), r);
        assert_eq!(r.join(Trust::Proven), r);
        assert_eq!(r.join(h), h);
        assert_eq!(h.join(Trust::Unknown), Trust::Unknown);
        assert_eq!(Trust::Proven.join(Trust::Proven), Trust::Proven);
    }

    #[test]
    fn no_raw_heap_constant() {
        assert!(Graph::const_tagged(0x1000).is_none(), "a cell address");
        assert!(Graph::const_tagged(ep9_value::encoding::VALUE_UNDEFINED).is_some());
        assert!(Graph::const_tagged(crate::vm_layout::int32_bits(3)).is_some());
    }

    /// The checker refuses a PROVEN value that is not on the H30 list, a predicted value with
    /// no guard of its fact, and a guard op without a guard.
    #[test]
    fn check_refuses_bad_trust() {
        let mut g = Graph::new("t", FactSource::Runtime, 1, 0);
        g.blocks.push(Block::default());
        let b = BlockId(0);
        let p = g.add(
            Op::Incoming(Incoming::Param(0)),
            vec![],
            Repr::Tagged,
            Trust::Unknown,
            b,
            0,
        );
        g.blocks[0].nodes.push(p);
        let bad = g.add(
            Op::LoadField { offset: 24 },
            vec![p],
            Repr::Tagged,
            Trust::Proven,
            b,
            0,
        );
        g.blocks[0].nodes.push(bad);
        assert!(g.check().unwrap_err().contains("PROVEN"));
        g.node_mut(bad).trust = Trust::Predicted(FactSource::Runtime);
        assert!(g.check().unwrap_err().contains("predicted"));
        g.node_mut(bad).trust = Trust::Unknown;
        assert!(g.check().is_ok());
        let c = g.add(Op::CheckInt32, vec![p], Repr::Int32, Trust::Unknown, b, 0);
        g.blocks[0].nodes.push(c);
        assert!(g.check().unwrap_err().contains("guard"));
        g.set_guard(c, GuardBasis::Unbacked, FrameState::default());
        assert!(g.check().is_ok());
        assert_eq!(g.guard_counts(), (0, 1));
    }

    /// Guard placement: a guard after an effect of its own resume range is refused (T0
    /// would run the effect again), and so is a frame state that names a value made after
    /// the guard; the same guard before the effect, or resuming after it, passes.
    #[test]
    fn check_refuses_a_guard_that_cannot_resume() {
        let block = |g: &mut Graph, op: Op, inputs: Vec<NodeId>, repr: Repr, off: u32| {
            let n = g.add(op, inputs, repr, Trust::Unknown, BlockId(0), off);
            g.blocks[0].nodes.push(n);
            n
        };
        let build = |guard_after: u32, state_off: u32| {
            let mut g = Graph::new("t", FactSource::Runtime, 1, 0);
            g.blocks.push(Block::default());
            let p = block(
                &mut g,
                Op::Incoming(Incoming::Param(0)),
                vec![],
                Repr::Tagged,
                0,
            );
            let poll = block(&mut g, Op::Poll, vec![], Repr::None, 10);
            let c = block(&mut g, Op::CheckInt32, vec![p], Repr::Int32, guard_after);
            g.set_guard(
                c,
                GuardBasis::Unbacked,
                FrameState {
                    offset: state_off,
                    registers: vec![(Register::parameter(0), p)],
                    acc: None,
                },
            );
            let _ = poll;
            g
        };
        // The poll at @10 ran; resuming at @10 runs it again.
        let e = build(12, 10).check().unwrap_err();
        assert!(e.contains("not replayable"), "{e}");
        let e = build(12, 4).check().unwrap_err();
        assert!(e.contains("not replayable"), "{e}");
        // Resuming after the poll is fine.
        assert!(build(12, 12).check().is_ok());
        // A frame-state value made after the guard.
        let mut g = build(12, 12);
        let late = block(&mut g, Op::ConstInt32(1), vec![], Repr::Int32, 14);
        g.node_mut(late).trust = Trust::Proven;
        let c = g.blocks[0].nodes[2];
        g.node_mut(c).guard.as_mut().expect("guard").state.acc = Some(late);
        let e = g.check().unwrap_err();
        assert!(e.contains("not defined before the guard"), "{e}");
    }

    /// The graph of `name` in `defs` from the live facts after `warm` (T0).
    fn graph_of(defs: &str, warm: &str, name: &str) -> Graph {
        use crate::harness::Session;
        let mut s = Session::new(&spike_b::harness::Config::default().threaded()).expect("session");
        s.run(defs, "defs.js").result.expect("defs");
        s.run(warm, "warm.js").result.expect("warm");
        let script = s.script("defs.js").expect("script");
        let id = *script.functions_named(name).first().expect("function");
        let live = crate::facts::live(&s, script).expect("facts");
        let input = s.function_input(name).expect("input");
        let (g, _) =
            crate::build::build(&input, &crate::facts::for_function(&live, id)).expect("build");
        g
    }

    /// Codex round 1 (spikec3): a guard whose frame state lies before its block must not
    /// resume T0 there when an effect on the way (here the StoreElement of `a[0] = ...` in
    /// the entry block) already ran: T0 would increment the element twice. The graph of
    /// `bump` with the last CheckInt32's state moved back to the first one's (@8) is refused.
    #[test]
    fn check_refuses_a_cross_block_resume_past_an_effect() {
        let g = graph_of(
            "function bump(a, k, n) { a[0] = (a[0] + 1) | 0; if (n < 0) return 0; return k + 1; }",
            "var w = [1]; for (var q = 0; q < 20; q++) bump(w, q, 1);",
            "bump",
        );
        g.check().expect("the built graph passes");
        let checks: Vec<NodeId> = g
            .guard_nodes()
            .filter(|(n, _)| g.node(*n).op == Op::CheckInt32)
            .map(|(n, _)| n)
            .collect();
        let last = *checks.last().expect("a check");
        let (first, _) = g.guard_nodes().next().expect("a guard");
        assert_ne!(g.node(first).block, g.node(last).block, "{g}");
        let early = g.node(first).guard.as_ref().expect("guard").state.clone();
        assert_eq!(early.offset, 8, "{g}");
        let mut bad = g.clone();
        bad.node_mut(last).guard.as_mut().expect("guard").state = early.clone();
        let e = bad
            .check()
            .expect_err(&format!("resume at @{} passed\n{bad}", early.offset));
        assert!(e.contains("not replayable"), "{e}");
        assert!(e.contains("effect at @17"), "{e}");
        // The same move with no effect on the way (no store) passes: T0 replays only checks.
        let g = graph_of(
            "function inc(k, n) { var z = n | 0; if (z < 0) return 0; return k + 1; }",
            "for (var q = 0; q < 20; q++) inc(q, 1);",
            "inc",
        );
        let (first, g0) = g.guard_nodes().next().expect("a guard");
        let state = g0.state.clone();
        let (last, _) = g
            .guard_nodes()
            .filter(|(n, _)| g.node(*n).op == Op::CheckInt32)
            .last()
            .expect("a check");
        assert_ne!(g.node(first).block, g.node(last).block, "{g}");
        let mut ok = g.clone();
        ok.node_mut(last).guard.as_mut().expect("guard").state = state.clone();
        ok.check()
            .unwrap_or_else(|e| panic!("{e}: resume at @{} with no effect\n{ok}", state.offset));
        // A state in a block with no path to the guard's block is refused.
        let mut cut = ok.clone();
        let lb = cut.node(last).block.0 as usize;
        cut.blocks[lb].preds.clear();
        for blk in &mut cut.blocks {
            blk.succs.retain(|t| t.0 as usize != lb);
        }
        let e = cut.check().expect_err("a state with no path passed");
        assert!(e.contains("has no block path"), "{e}");
    }
}
