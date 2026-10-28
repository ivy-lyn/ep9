//! The graph builder (SPIKEC-1, M4-P1): one function's verified bytecode plus its
//! [`FunctionFacts`] -> an [`ir::Graph`] in SSA form.
//!
//! One builder serves both fact paths (H30 rule "one graph builder reads bytecode +
//! FactSnapshot"; V9-D32): the live path takes `ep9_vm::abi::export_facts` output
//! ([`crate::facts::live`]), SP-Facts takes a PROFILED file ([`crate::facts::read_profiled`]).
//! The builder never reads a FeedbackVector: a guard that needs run-local ShapeIds names the
//! IC cell where they live (a CellRef), and only the emitted code reads it.
//!
//! ```text
//!  Cfg::build (blocks, liveness; H02-Q1)
//!  ──> abstract interpretation in offset order, one environment (SSA value per register):
//!        merge: a phi where the predecessors differ; loop header: a phi per live register,
//!        back-edge inputs filled when the JumpLoop block is done
//!  ──> trivial phis removed ──> loop phis unboxed to Int32 (an entry check in the
//!      preheader) ──> redundant checks removed, boxes inserted ──> dead pure nodes removed
//! ```
//!
//! | Bytecode | Graph | Trust of the result |
//! |---|---|---|
//! | `&`, `|`, `^`, `<<`, `>>` (and the Smi forms) | TruncateInt32 on each input, Int32 op | PROVEN (H30 list) |
//! | `+`, `-`, `*`, Inc, Dec, ToNumeric | CheckInt32 on each input, Int32 op with an overflow (and -0) guard | predicted{source} with an int32 BinaryOp fact (below), else unknown with unbacked guards |
//! | `<`, `>`, `<=`, `>=` | CheckInt32 on each input (backed by an int32 CompareOp fact, else unbacked), Compare | PROVEN (a boolean) |
//! | GetNamedProperty | CheckShape (the LoadNamed fact: an in-object own field or an Array's `length`, one location for every case) + LoadField | the check is predicted{source}; the loaded value unknown |
//! | GetKeyedProperty, SetKeyedProperty | CheckArray, CheckInt32 on the key, LoadElement / StoreElement (an int32 value); or the generic keyed entry (a key that is not an index, a value that is not an int32; SPIKEC-4) | unknown; the key check is backed by an index-only keyed record, the element guards are unbacked (keyed facts, SPIKEC-3, below) |
//! | `+`, `-`, `*` whose BinaryOp record saw a double (hint inside {int32, int32-overflow, number}, with number), and `/` always | CheckNumber on each Tagged input (an Int32 input converts), a Float64 op (SPIKEC-4) | the check is predicted{source} with the record, else unknown; the op's result PROVEN (a number from numbers) |
//! | `===` | CheckInt32 on each input (backed by an int32 CompareOp fact), Compare(Eq) (SPIKEC-4) | PROVEN (a boolean) |
//! | LdaCurrentContextSlot, LdaImmutableCurrentContextSlot | LoadContextSlot (SPIKEC-4) | unknown |
//! | CallUndefinedReceiver0/1/2 | Call: the callee and the arguments Tagged (SPIKEC-4) | unknown |
//! | JumpLoop | Poll (budget), Jump to the header | - |
//!
//! A named load without a fact, a megamorphic site, an accessor, a prototype field or an
//! out-of-line field is refused: the function stays in T0.
//!
//! **Int facts (SPIKEC-2).** The BinaryOp record of an arithmetic or bitwise site and the
//! CompareOp record of a relational compare back that site's int32 guards (B2 of
//! docs/plan/13-execution-modes.md: the builder picks the guard basis, the mode picks the
//! policy). A hint inside {int32} backs them: `BinaryHint::INT32` or `CompareHint::INT32`, or
//! the empty set, which T0 leaves for "int32 or never ran" (implicit int32, MODES-D10). A hint
//! with any other bit (int32 overflow, number, string, ...) or no record leaves them
//! unbacked: the spike has no double or generic path, so the guard stays and only its basis
//! differs. The T0 profile writes these slots since M4-P1 (`ep9_vm` profile.rs); the exporter
//! writes them out from FACTS2 (M4-P2) on, so until that merge the live snapshot has no such
//! record and the guards are unbacked. The same SSA value checked twice in a block shares one
//! check; a later backed site upgrades it (the value did not change in between). A loop
//! phi's entry check (`unbox_loop_phis`) is backed if a check of the phi in the loop header
//! block was (the header runs with the entry value). The overflow guard of `+`, `-`, Inc and
//! Dec is always unbacked here; since M5-P1 (MODES-D16) T0 records their overflow as
//! `INT32_OVERFLOW` (as `*` records its overflow and -0), so a later step may back it.
//!
//! **Checks are shared by (value, policy) (SPIKEC-3, MC-w14-20).** The block-local memo keys
//! a check by its op, its input and the exit policy that the mode gives the site's basis
//! ([`crate::ir::Mode::exit_policy`]). A later site with the same three shares the check and
//! may only relabel its basis (unbacked to a fact); a guard's policy is set once, when the
//! guard is made, and never rewritten. A site whose policy differs gets its own check. JIT
//! mode has one policy, so the graphs are the same as with the (op, input) key.
//!
//! **Keyed facts (SPIKEC-3, V9-A20).** The LoadKeyed and StoreKeyed records of FACTS2
//! (`ep9_vm` facts_export.rs `keyed`) carry, in FACT_SCHEMA 1, the keys a site saw
//! ([`KeySet`]) and the key half's IC state, nothing about the receiver, the bounds or holes
//! (those value bits have no schema field until the month-6 bump, MC-w13-9). So:
//!
//! | Record of the site | Graph |
//! |---|---|
//! | keys = {index} (at a LoadKeyed site also "never ran": the implicit INDEX bit, MODES-D10) | the element path; the key's int32 check is backed by the record (an index of another form, an index string or a Number past int32, fails it and exits) |
//! | no keys (a StoreKeyed site that never ran) or no record | the element path; the key's int32 check is unbacked |
//! | a name, symbol or other key | the generic path: [`ir::Op::GetKeyedGeneric`] / [`ir::Op::SetKeyedGeneric`], no guard (SPIKEC-4) |
//!
//! The IC state (the key half: undefined, a receiver shape, or the hole once KEYED writes it)
//! is not read: the element guards test the receiver's kind and the element, not its shape.
//! CheckArray, LoadElement and StoreElement stay unbacked in every case (the receiver,
//! bounds and hole bits come with the month-6 schema bump, MODES-D18).
//!
//! **The generic keyed path (SPIKEC-4, P3-ORCH-9).** A site whose record saw a key that is not
//! an index would exit on those keys from every element guard, so it calls the generic keyed
//! entries of `ep9_vm::RuntimeTable` that M4-P3 KEYED appended (8 `get_keyed`, 9 `set_keyed`;
//! `layout::RT_GET_KEYED`, `RT_SET_KEYED`) in place of the refusal SPIKEC-3 made. A
//! SetKeyedProperty whose value is not an int32 takes the generic store too (the entry does
//! the store barrier; StoreElement stores only int32s, which need none). Both entries run
//! the T0 miss-path functions (`slow::prop`: `property::get_keyed` / `set_keyed`), which
//! record no feedback, so the compiled code still records none (native.rs:149-158;
//! MC-w15-6: a RuntimeTable entry calls only non-recording generic functions). A generic
//! access is an effect (a getter, a setter or a proxy trap may run JS) and a GC point; the
//! emitter saves the allocated caller-saved registers around the call, and a throw unwinds
//! from the access's bytecode offset.

use ep9_bytecode::{Opcode, Register, decode_at};
use ep9_facts::{Access, BinaryHint, CompareHint, IcState, KeySet, ObjectKind, SlotFacts};
use ep9_value::encoding::{VALUE_FALSE, VALUE_NULL, VALUE_TRUE, VALUE_UNDEFINED};
use spike_b::emit::{FunctionInput, Refusal};

use crate::cfg::Cfg;
use crate::facts::FunctionFacts;
use crate::ir::{
    self, BlockId, Cond, ExitPolicy, FloatOp, FrameState, Graph, GuardBasis, Incoming, Int32Op,
    NodeId, Op, RefNode, Repr, Trust,
};
use crate::vm_layout;

/// The opcodes the builder translates.
pub fn supported(op: Opcode) -> bool {
    use Opcode::*;
    matches!(
        op,
        Ldar | Star
            | Mov
            | LdaZero
            | LdaSmi
            | LdaUndefined
            | LdaNull
            | LdaTrue
            | LdaFalse
            | GetNamedProperty
            | GetKeyedProperty
            | SetKeyedProperty
            | Add
            | Sub
            | Mul
            | BitwiseAnd
            | BitwiseOr
            | BitwiseXor
            | ShiftLeft
            | ShiftRight
            | AddSmi
            | SubSmi
            | MulSmi
            | BitwiseAndSmi
            | BitwiseOrSmi
            | BitwiseXorSmi
            | ShiftLeftSmi
            | ShiftRightSmi
            | Inc
            | Dec
            | ToNumeric
            | Div
            | TestEqualStrict
            | LdaCurrentContextSlot
            | LdaImmutableCurrentContextSlot
            | CallUndefinedReceiver0
            | CallUndefinedReceiver1
            | CallUndefinedReceiver2
            | TestLessThan
            | TestGreaterThan
            | TestLessThanOrEqual
            | TestGreaterThanOrEqual
            | JumpIfFalse
            | JumpIfTrue
            | Jump
            | JumpLoop
            | Return
    )
}

/// What the builder measured (the H02-Q1 rows of the report).
#[derive(Clone, Debug, Default)]
pub struct BuildStats {
    pub instructions: usize,
    pub blocks: usize,
    /// Liveness time, microseconds.
    pub liveness_us: u128,
    /// Liveness as one bit per register per instruction.
    pub liveness_bytes: usize,
    /// Registers (with the receiver, parameters and the accumulator).
    pub registers: usize,
    /// The keyed sites and what their records gave (SPIKEC-3).
    pub keyed: Vec<KeyedSite>,
}

/// One keyed site of the graph (the report).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyedSite {
    /// The bytecode offset of the access.
    pub offset: u32,
    /// Its feedback slot.
    pub slot: u32,
    /// True for a store (SetKeyedProperty).
    pub store: bool,
    /// The keys of its record, if it has one.
    pub keys: Option<KeySet>,
    /// The basis the record gives the key's int32 check (unbacked on the generic path, which
    /// has no key check).
    pub key_basis: GuardBasis,
    /// True if the site takes the generic keyed entry (SPIKEC-4).
    pub generic: bool,
}

/// How a keyed site is compiled (see the module docs, "Keyed facts").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyedPath {
    /// CheckArray, the key's CheckInt32 with this basis, then LoadElement or StoreElement.
    Element(GuardBasis),
    /// The generic keyed entry of the runtime table: no guard.
    Generic,
}

const PLACEHOLDER: NodeId = NodeId(u32::MAX);

struct Builder<'a> {
    g: Graph,
    cfg: Cfg,
    facts: &'a FunctionFacts<'a>,
    code: &'a [u8],
    env: Vec<Option<NodeId>>,
    out_env: Vec<Option<Vec<Option<NodeId>>>>,
    /// Per loop header: (register, phi) of its phis.
    loop_phis: Vec<Vec<(usize, NodeId)>>,
    cur: BlockId,
    /// The instruction being translated (its index in the Cfg).
    insn: usize,
    /// Checks made in the current block: (op tag, input, exit policy) -> node (a block-local
    /// CSE by value and policy, MC-w14-20).
    memo: Vec<(u8, NodeId, ExitPolicy, NodeId)>,
    keyed: Vec<KeyedSite>,
}

/// Builds the SSA graph of `input` from its bytecode and `facts`.
pub fn build(
    input: &FunctionInput<'_>,
    facts: &FunctionFacts<'_>,
) -> Result<(Graph, BuildStats), Refusal> {
    let cfg = Cfg::build(input, supported)?;
    let mut stats = BuildStats {
        instructions: cfg.insns.len(),
        blocks: cfg.blocks.len(),
        liveness_us: cfg.liveness_time.as_micros(),
        liveness_bytes: cfg.liveness_bytes(),
        registers: cfg.width(),
        keyed: Vec::new(),
    };
    let mut g = Graph::new(
        input.name,
        facts.source,
        input.parameter_count,
        input.register_count,
    );
    g.insns = cfg.insns.iter().map(|x| (x.offset, x.opcode)).collect();
    for (i, cb) in cfg.blocks.iter().enumerate() {
        g.blocks.push(ir::Block {
            offset: cb.start,
            nodes: Vec::new(),
            preds: cb.preds.iter().map(|&p| BlockId(p as u32)).collect(),
            succs: Vec::new(),
            loop_header: cb.loop_header,
        });
        let _ = i;
    }
    let n = cfg.blocks.len();
    let width = cfg.width();
    let mut b = Builder {
        g,
        cfg,
        facts,
        code: input.bytecode,
        env: vec![None; width],
        out_env: vec![None; n],
        loop_phis: vec![Vec::new(); n],
        cur: BlockId(0),
        insn: 0,
        memo: Vec::new(),
        keyed: Vec::new(),
    };
    for bi in 0..n {
        b.block(bi)?;
    }
    b.remove_trivial_phis();
    b.unbox_loop_phis();
    b.fix_reprs();
    b.dce();
    b.g.check().map_err(Refusal::Limit)?;
    stats.keyed = b.keyed;
    Ok((b.g, stats))
}

impl Builder<'_> {
    fn offset(&self) -> u32 {
        self.cfg.insns[self.insn].offset
    }

    fn node(&self, n: NodeId) -> &ir::Node {
        self.g.node(n)
    }

    /// Adds a node at the end of the current block.
    fn emit(&mut self, op: Op, inputs: Vec<NodeId>, repr: Repr, trust: Trust) -> NodeId {
        let off = self.offset();
        let n = self.g.add(op, inputs, repr, trust, self.cur, off);
        self.g.blocks[self.cur.0 as usize].nodes.push(n);
        n
    }

    /// The frame state before the current instruction: T0 runs it again after a deopt.
    fn state(&self) -> FrameState {
        let i = self.insn;
        let live = &self.cfg.live_before[i];
        let acc = self.cfg.acc();
        FrameState {
            offset: self.cfg.insns[i].offset,
            registers: (0..acc)
                .filter(|&r| live[r])
                .filter_map(|r| self.env[r].map(|v| (self.cfg.register(r), v)))
                .collect(),
            acc: if live[acc] { self.env[acc] } else { None },
        }
    }

    fn guard(&mut self, n: NodeId, basis: GuardBasis, state: FrameState) {
        self.g.set_guard(n, basis, state);
    }

    fn reg(&self, r: Register) -> Result<NodeId, Refusal> {
        let i = self.cfg.index(r);
        self.env[i].ok_or_else(|| Refusal::Decode {
            offset: self.offset(),
            detail: format!("{r} is read but has no value (liveness)"),
        })
    }

    fn acc(&self) -> Result<NodeId, Refusal> {
        self.env[self.cfg.acc()].ok_or_else(|| Refusal::Decode {
            offset: self.offset(),
            detail: "the accumulator is read but has no value".into(),
        })
    }

    fn set_acc(&mut self, v: NodeId) {
        let a = self.cfg.acc();
        self.env[a] = Some(v);
    }

    fn konst(&mut self, bits: u64) -> NodeId {
        let op = Graph::const_tagged(bits).expect("an immediate");
        self.emit(op, vec![], Repr::Tagged, Trust::Proven)
    }

    fn int(&mut self, v: i32) -> NodeId {
        self.emit(Op::ConstInt32(v), vec![], Repr::Int32, Trust::Proven)
    }

    /// The check `tag` of `input` with the exit policy `policy` in the current block: the one
    /// made before, or a new one from `make` (MC-w14-20: shared by value and policy, never by
    /// value alone).
    fn memo(
        &mut self,
        tag: u8,
        input: NodeId,
        policy: ExitPolicy,
        make: impl FnOnce(&mut Self) -> NodeId,
    ) -> NodeId {
        if let Some(&(.., n)) = self
            .memo
            .iter()
            .find(|m| m.0 == tag && m.1 == input && m.2 == policy)
        {
            return n;
        }
        let n = make(self);
        self.memo.push((tag, input, policy, n));
        n
    }

    /// The exit policy the graph's mode gives a guard with `basis` (read once, when a guard
    /// or a memo key is made).
    fn policy(&self, basis: GuardBasis) -> ExitPolicy {
        self.g.mode.exit_policy(basis)
    }

    /// `v` as an Int32 for an arithmetic use: a CheckInt32 guard with `basis` unless it is one.
    fn int32(&mut self, v: NodeId, basis: GuardBasis) -> NodeId {
        match (self.node(v).repr, &self.node(v).op) {
            (Repr::Int32, _) => v,
            (_, Op::Const(bits)) if vm_layout::int32_of(*bits).is_some() => {
                let x = vm_layout::int32_of(*bits).expect("int32");
                self.int(x)
            }
            (Repr::Bool, _) => {
                // A boolean in arithmetic is a number (0 or 1); the spike keeps it simple.
                let b = self.emit(Op::BoxBool, vec![v], Repr::Tagged, self.node(v).trust);
                self.int32(b, basis)
            }
            _ => {
                let st = self.state();
                let policy = self.policy(basis);
                let n = self.memo(0, v, policy, |s| {
                    let n = s.emit(Op::CheckInt32, vec![v], Repr::Int32, trust_of(basis));
                    s.guard(n, basis, st);
                    n
                });
                self.back(n, basis);
                n
            }
        }
    }

    /// ToInt32 of `v` for a bitwise use: a TruncateInt32 guard with `basis` unless it is an
    /// Int32. Its result is PROVEN (the ToInt32 invariant) whatever the basis.
    fn trunc(&mut self, v: NodeId, basis: GuardBasis) -> NodeId {
        match (self.node(v).repr, &self.node(v).op) {
            (Repr::Int32, _) => v,
            (_, Op::Const(bits)) if vm_layout::int32_of(*bits).is_some() => {
                let x = vm_layout::int32_of(*bits).expect("int32");
                self.int(x)
            }
            _ => {
                let st = self.state();
                let v = self.tagged(v);
                let policy = self.policy(basis);
                let n = self.memo(1, v, policy, |s| {
                    let n = s.emit(Op::TruncateInt32, vec![v], Repr::Int32, Trust::Proven);
                    s.guard(n, basis, st);
                    n
                });
                self.back(n, basis);
                n
            }
        }
    }

    /// `v` as a Tagged Value.
    fn tagged(&mut self, v: NodeId) -> NodeId {
        let t = self.node(v).trust;
        // A box is no guard: its key takes the one policy of an unbacked guard as a filler.
        let p = self.policy(GuardBasis::Unbacked);
        match self.node(v).repr {
            Repr::Int32 => self.memo(2, v, p, |s| s.emit(Op::BoxInt32, vec![v], Repr::Tagged, t)),
            Repr::Bool => self.memo(3, v, p, |s| s.emit(Op::BoxBool, vec![v], Repr::Tagged, t)),
            _ => v,
        }
    }

    /// The basis of the int32 guards of the site with feedback slot `slot`: its BinaryOp or
    /// CompareOp record, if the hint is inside {int32} (see the module docs).
    fn int_basis(&self, slot: u32) -> GuardBasis {
        int_basis_of(self.facts, slot)
    }

    /// A shared check `n` now also guards a site with `basis`: a fact basis replaces an
    /// unbacked one (the same SSA value, checked once). The memo shared `n` only with a site
    /// of the same exit policy, so the relabel keeps the guard's policy as it was made: the
    /// policy is never rewritten (MC-w14-20).
    fn back(&mut self, n: NodeId, basis: GuardBasis) {
        let GuardBasis::Fact(src) = basis else { return };
        let policy = self.policy(basis);
        let node = self.g.node_mut(n);
        if let Some(g) = &mut node.guard
            && g.basis == GuardBasis::Unbacked
        {
            assert_eq!(
                g.policy, policy,
                "a check shared across exit policies (the memo keys the policy)"
            );
            g.basis = basis;
            if node.op == Op::CheckInt32 {
                node.trust = Trust::Predicted(src);
            }
        }
    }

    /// The keyed record of `slot` at the access at the current offset (`store` for
    /// SetKeyedProperty): the path of the site, or a refusal for a record of another kind
    /// (see the module docs, "Keyed facts"). `generic` takes the generic path whatever the
    /// record says (a store of a value that is not an int32). Records the site for the
    /// report.
    fn keyed_site(&mut self, slot: u32, store: bool, generic: bool) -> Result<KeyedPath, Refusal> {
        let off = self.offset();
        let refuse = |why: String| Refusal::Limit(format!("keyed access at {off}: {why}"));
        let rec = self.facts.records.iter().find(|(s, _)| *s == slot);
        let keyed = match rec {
            None => None,
            Some((_, SlotFacts::LoadKeyed(k))) if !store => Some(k),
            Some((_, SlotFacts::StoreKeyed(k))) if store => Some(k),
            Some((_, f)) => {
                return Err(refuse(format!(
                    "fact of kind {}",
                    crate::facts::kind_name(f)
                )));
            }
        };
        let path = match keyed {
            _ if generic => KeyedPath::Generic,
            None => KeyedPath::Element(GuardBasis::Unbacked),
            Some(k) => keyed_path_of(k, self.facts.source),
        };
        self.keyed.push(KeyedSite {
            offset: off,
            slot,
            store,
            keys: keyed.map(|k| k.keys),
            key_basis: match path {
                KeyedPath::Element(b) => b,
                KeyedPath::Generic => GuardBasis::Unbacked,
            },
            generic: path == KeyedPath::Generic,
        });
        Ok(path)
    }

    fn binop(&mut self, op: Int32Op, l: NodeId, r: NodeId, slot: u32) -> NodeId {
        let st = self.state();
        let basis = self.int_basis(slot);
        let (l, r) = if op.bitwise() {
            (self.trunc(l, basis), self.trunc(r, basis))
        } else {
            (self.int32(l, basis), self.int32(r, basis))
        };
        // The op's own guard (overflow, and -0 for `*`): T0 records an overflow of `*` in the
        // BinaryOp hint (INT32_OVERFLOW), and since M5-P1 (MODES-D16) also an int32 `+`, `-`,
        // Inc or Dec that overflows; the `+`/`-` guard stays unbacked here (a later SPIKEC
        // step may back it from a hint inside {int32}). The input checks keep the site's basis.
        let op_basis = match op {
            Int32Op::Add | Int32Op::Sub => GuardBasis::Unbacked,
            _ => basis,
        };
        let trust = if op.bitwise() {
            Trust::Proven
        } else {
            trust_of(op_basis)
        };
        let n = self.emit(Op::Int32(op), vec![l, r], Repr::Int32, trust);
        if !op.bitwise() {
            self.guard(n, op_basis, st);
        }
        n
    }

    /// `v` as a Float64 for a number use: a CheckNumber guard with `basis` if it is Tagged
    /// (an Int32 converts, PROVEN).
    fn number(&mut self, v: NodeId, basis: GuardBasis) -> NodeId {
        match (self.node(v).repr, &self.node(v).op) {
            (Repr::Float64, _) => v,
            (_, Op::Const(bits)) if vm_layout::int32_of(*bits).is_some() => {
                let x = vm_layout::int32_of(*bits).expect("int32");
                let i = self.int(x);
                self.number(i, basis)
            }
            (Repr::Int32, _) => {
                let p = self.policy(GuardBasis::Unbacked);
                self.memo(6, v, p, |s| {
                    s.emit(Op::Int32ToFloat64, vec![v], Repr::Float64, Trust::Proven)
                })
            }
            _ => {
                let st = self.state();
                let v = self.tagged(v);
                let policy = self.policy(basis);
                let n = self.memo(5, v, policy, |s| {
                    let n = s.emit(Op::CheckNumber, vec![v], Repr::Float64, trust_of(basis));
                    s.guard(n, basis, st);
                    n
                });
                self.back(n, basis);
                n
            }
        }
    }

    /// A double op of the site with feedback slot `slot` (see the module docs).
    fn float_op(&mut self, op: FloatOp, l: NodeId, r: NodeId, slot: u32) -> NodeId {
        let basis = number_basis_of(self.facts, slot);
        let (l, r) = (self.number(l, basis), self.number(r, basis));
        self.emit(Op::Float64(op), vec![l, r], Repr::Float64, Trust::Proven)
    }

    fn compare(&mut self, c: Cond, l: NodeId, r: NodeId, slot: u32) -> NodeId {
        let basis = self.int_basis(slot);
        let (l, r) = (self.int32(l, basis), self.int32(r, basis));
        self.emit(Op::Compare(c), vec![l, r], Repr::Bool, Trust::Proven)
    }

    /// The entry environment of block `bi`.
    fn enter(&mut self, bi: usize) -> Result<(), Refusal> {
        self.cur = BlockId(bi as u32);
        self.memo.clear();
        self.insn = self.cfg.blocks[bi].insns.start;
        let width = self.cfg.width();
        if bi == 0 {
            let mut env = vec![None; width];
            env[0] = Some(self.emit(
                Op::Incoming(Incoming::Receiver),
                vec![],
                Repr::Tagged,
                Trust::Unknown,
            ));
            for k in 0..self.cfg.parameter_count {
                env[1 + k as usize] = Some(self.emit(
                    Op::Incoming(Incoming::Param(k)),
                    vec![],
                    Repr::Tagged,
                    Trust::Unknown,
                ));
            }
            let undef = self.konst(VALUE_UNDEFINED);
            for slot in env.iter_mut().skip(1 + self.cfg.parameter_count as usize) {
                *slot = Some(undef);
            }
            self.env = env;
            if !self.cfg.blocks[0].preds.is_empty() {
                return Err(Refusal::Limit("the entry block is a jump target".into()));
            }
            return Ok(());
        }
        let preds = self.cfg.blocks[bi].preds.clone();
        if preds.is_empty() {
            return Err(Refusal::Decode {
                offset: self.cfg.blocks[bi].start,
                detail: "a reachable block with no predecessor".into(),
            });
        }
        let live = self.cfg.live_before[self.cfg.blocks[bi].insns.start].clone();
        let mut env = vec![None; width];
        if self.cfg.blocks[bi].loop_header {
            for r in 0..width {
                if !live[r] {
                    continue;
                }
                let inputs: Vec<NodeId> = preds
                    .iter()
                    .map(|&p| match &self.out_env[p] {
                        Some(e) => e[r].unwrap_or(PLACEHOLDER),
                        None => PLACEHOLDER,
                    })
                    .collect();
                if preds
                    .iter()
                    .any(|&p| p < bi && self.out_env[p].as_ref().is_some_and(|e| e[r].is_none()))
                {
                    return Err(Refusal::Decode {
                        offset: self.cfg.blocks[bi].start,
                        detail: format!(
                            "{} is live at a loop header without a value",
                            self.cfg.register(r)
                        ),
                    });
                }
                let phi = self.emit(Op::Phi, inputs, Repr::Tagged, Trust::Unknown);
                self.loop_phis[bi].push((r, phi));
                env[r] = Some(phi);
            }
        } else {
            for r in 0..width {
                if !live[r] {
                    continue;
                }
                let mut vals = Vec::with_capacity(preds.len());
                for &p in &preds {
                    let e = self.out_env[p]
                        .as_ref()
                        .ok_or(Refusal::Limit("a forward edge from a later block".into()))?;
                    vals.push(e[r].ok_or_else(|| Refusal::Decode {
                        offset: self.cfg.blocks[bi].start,
                        detail: format!(
                            "{} is live at a merge without a value",
                            self.cfg.register(r)
                        ),
                    })?);
                }
                env[r] = Some(if vals.iter().all(|&v| v == vals[0]) {
                    vals[0]
                } else {
                    let repr = if vals.iter().all(|&v| self.node(v).repr == Repr::Int32) {
                        Repr::Int32
                    } else {
                        Repr::Tagged
                    };
                    let trust = vals
                        .iter()
                        .fold(Trust::Proven, |t, &v| t.join(self.node(v).trust));
                    let vals = if repr == Repr::Int32 {
                        vals
                    } else {
                        // Boxes go at the end of each predecessor (fix_reprs does it).
                        vals
                    };
                    self.emit(Op::Phi, vals, repr, trust)
                });
            }
        }
        self.env = env;
        Ok(())
    }

    fn block(&mut self, bi: usize) -> Result<(), Refusal> {
        self.enter(bi)?;
        let range = self.cfg.blocks[bi].insns.clone();
        let mut terminated = false;
        for i in range.clone() {
            self.insn = i;
            terminated = self.translate()?;
        }
        if !terminated {
            self.insn = range.end - 1;
            self.emit(Op::Jump, vec![], Repr::None, Trust::Unknown);
            let s = self.cfg.blocks[bi].succs.clone();
            self.g.blocks[bi].succs = s.iter().map(|&x| BlockId(x as u32)).collect();
        }
        self.out_env[bi] = Some(self.env.clone());
        // Back edges: fill the loop header's phi inputs from this block.
        for &s in &self.cfg.blocks[bi].succs.clone() {
            if s > bi {
                continue;
            }
            let k = self.cfg.blocks[s]
                .preds
                .iter()
                .position(|&p| p == bi)
                .expect("a pred");
            for (r, phi) in self.loop_phis[s].clone() {
                let v = self.env[r].ok_or_else(|| Refusal::Decode {
                    offset: self.cfg.blocks[s].start,
                    detail: format!("{} has no value on a back edge", self.cfg.register(r)),
                })?;
                self.g.node_mut(phi).inputs[k] = v;
            }
        }
        Ok(())
    }

    /// Translates the current instruction; true if it ended the block (a terminator node).
    fn translate(&mut self) -> Result<bool, Refusal> {
        use Opcode::*;
        let x = self.cfg.insns[self.insn].clone();
        let insn = decode_at(self.code, x.offset).map_err(|e| Refusal::Decode {
            offset: x.offset,
            detail: format!("{e:?}"),
        })?;
        let bi = self.cur.0 as usize;
        let succ_block = |cfg: &Cfg, off: u32| cfg.block_at(off).map(|b| BlockId(b as u32));
        match x.opcode {
            Ldar => {
                let v = self.reg(insn.register(0))?;
                self.set_acc(v);
            }
            Star => {
                let v = self.acc()?;
                let i = self.cfg.index(insn.register(0));
                self.env[i] = Some(v);
            }
            Mov => {
                let v = self.reg(insn.register(0))?;
                let i = self.cfg.index(insn.register(1));
                self.env[i] = Some(v);
            }
            LdaZero => {
                let n = self.int(0);
                self.set_acc(n);
            }
            LdaSmi => {
                let n = self.int(insn.signed(0));
                self.set_acc(n);
            }
            LdaUndefined | LdaNull | LdaTrue | LdaFalse => {
                let bits = match x.opcode {
                    LdaUndefined => VALUE_UNDEFINED,
                    LdaNull => VALUE_NULL,
                    LdaTrue => VALUE_TRUE,
                    _ => VALUE_FALSE,
                };
                let n = self.konst(bits);
                self.set_acc(n);
            }
            GetNamedProperty => {
                let o = self.reg(insn.register(0))?;
                let slot = insn.unsigned(2);
                let v = self.named_load(o, slot)?;
                self.set_acc(v);
            }
            GetKeyedProperty => {
                let st = self.state();
                let o = self.reg(insn.register(0))?;
                let k = self.acc()?;
                let n = match self.keyed_site(insn.unsigned(1), false, false)? {
                    KeyedPath::Element(key_basis) => {
                        let a = self.check_array(o);
                        let i = self.int32(k, key_basis);
                        let n =
                            self.emit(Op::LoadElement, vec![a, i], Repr::Tagged, Trust::Unknown);
                        self.guard(n, GuardBasis::Unbacked, st);
                        n
                    }
                    KeyedPath::Generic => {
                        let (o, k) = (self.tagged(o), self.tagged(k));
                        self.emit(
                            Op::GetKeyedGeneric,
                            vec![o, k],
                            Repr::Tagged,
                            Trust::Unknown,
                        )
                    }
                };
                self.set_acc(n);
            }
            SetKeyedProperty => {
                let st = self.state();
                let o = self.reg(insn.register(0))?;
                let k = self.reg(insn.register(1))?;
                let v = self.acc()?;
                // StoreElement stores an int32 (no barrier); any other value goes through the
                // generic store, whose entry does the barrier.
                let not_int32 = self.node(v).repr != Repr::Int32;
                match self.keyed_site(insn.unsigned(2), true, not_int32)? {
                    KeyedPath::Element(key_basis) => {
                        let a = self.check_array(o);
                        let i = self.int32(k, key_basis);
                        let n =
                            self.emit(Op::StoreElement, vec![a, i, v], Repr::None, Trust::Unknown);
                        self.guard(n, GuardBasis::Unbacked, st);
                    }
                    KeyedPath::Generic => {
                        let (o, k, v) = (self.tagged(o), self.tagged(k), self.tagged(v));
                        self.emit(
                            Op::SetKeyedGeneric,
                            vec![o, k, v],
                            Repr::None,
                            Trust::Unknown,
                        );
                    }
                }
                let u = self.konst(VALUE_UNDEFINED);
                self.set_acc(u);
            }
            Add | Sub | Mul | BitwiseAnd | BitwiseOr | BitwiseXor | ShiftLeft | ShiftRight => {
                let l = self.reg(insn.register(0))?;
                let r = self.acc()?;
                let slot = insn.unsigned(1);
                let n = match float_op_of(x.opcode) {
                    Some(op) if saw_double(self.facts, slot) => self.float_op(op, l, r, slot),
                    _ => self.binop(int_op(x.opcode), l, r, slot),
                };
                self.set_acc(n);
            }
            Div => {
                let l = self.reg(insn.register(0))?;
                let r = self.acc()?;
                let n = self.float_op(FloatOp::Div, l, r, insn.unsigned(1));
                self.set_acc(n);
            }
            TestEqualStrict => {
                let l = self.reg(insn.register(0))?;
                let r = self.acc()?;
                let n = self.compare(Cond::Eq, l, r, insn.unsigned(1));
                self.set_acc(n);
            }
            LdaCurrentContextSlot | LdaImmutableCurrentContextSlot => {
                let n = self.emit(
                    Op::LoadContextSlot(insn.unsigned(0)),
                    vec![],
                    Repr::Tagged,
                    Trust::Unknown,
                );
                self.set_acc(n);
            }
            CallUndefinedReceiver0 | CallUndefinedReceiver1 | CallUndefinedReceiver2 => {
                let argc = match x.opcode {
                    CallUndefinedReceiver0 => 0,
                    CallUndefinedReceiver1 => 1,
                    _ => 2,
                };
                let regs: Vec<Register> = (0..=argc).map(|i| insn.register(i)).collect();
                let mut inputs = Vec::new();
                for &r in &regs {
                    let v = self.reg(r)?;
                    inputs.push(self.tagged(v));
                }
                let n = self.emit(Op::Call { regs }, inputs, Repr::Tagged, Trust::Unknown);
                self.set_acc(n);
            }
            AddSmi | SubSmi | MulSmi | BitwiseAndSmi | BitwiseOrSmi | BitwiseXorSmi
            | ShiftLeftSmi | ShiftRightSmi => {
                let l = self.acc()?;
                let r = self.int(insn.signed(0));
                let n = self.binop(int_op(x.opcode), l, r, insn.unsigned(1));
                self.set_acc(n);
            }
            Inc | Dec => {
                let l = self.acc()?;
                let one = self.int(1);
                let op = if x.opcode == Inc {
                    Int32Op::Add
                } else {
                    Int32Op::Sub
                };
                let n = self.binop(op, l, one, insn.unsigned(0));
                self.set_acc(n);
            }
            ToNumeric => {
                let v = self.acc()?;
                let basis = self.int_basis(insn.unsigned(0));
                let n = self.int32(v, basis);
                self.set_acc(n);
            }
            TestLessThan | TestGreaterThan | TestLessThanOrEqual | TestGreaterThanOrEqual => {
                let l = self.reg(insn.register(0))?;
                let r = self.acc()?;
                let c = match x.opcode {
                    TestLessThan => Cond::Lt,
                    TestGreaterThan => Cond::Gt,
                    TestLessThanOrEqual => Cond::Le,
                    _ => Cond::Ge,
                };
                let n = self.compare(c, l, r, insn.unsigned(1));
                self.set_acc(n);
            }
            JumpIfFalse | JumpIfTrue => {
                let a = self.acc()?;
                let fall = succ_block(&self.cfg, x.next)
                    .ok_or(Refusal::Limit("no fall-through block".into()))?;
                let target = succ_block(&self.cfg, x.target.expect("a jump"))
                    .ok_or(Refusal::Limit("no target block".into()))?;
                // Branch: successor 0 when the condition is true. T0 jumps only on the exact
                // `true` / `false` Value (no ToBoolean), so a Tagged accumulator is tested with
                // IsTrue / IsFalse; a Bool is 0 or 1.
                let (c, succs) = if self.node(a).repr == Repr::Bool {
                    let s = if x.opcode == JumpIfTrue {
                        vec![target, fall]
                    } else {
                        vec![fall, target]
                    };
                    (a, s)
                } else {
                    let t = self.tagged(a);
                    let op = if x.opcode == JumpIfTrue {
                        Op::IsTrue
                    } else {
                        Op::IsFalse
                    };
                    (
                        self.emit(op, vec![t], Repr::Bool, Trust::Proven),
                        vec![target, fall],
                    )
                };
                self.emit(Op::Branch, vec![c], Repr::None, Trust::Unknown);
                self.g.blocks[bi].succs = succs;
                return Ok(true);
            }
            Jump => {
                let t = succ_block(&self.cfg, x.target.expect("a jump"))
                    .ok_or(Refusal::Limit("no target block".into()))?;
                self.emit(Op::Jump, vec![], Repr::None, Trust::Unknown);
                self.g.blocks[bi].succs = vec![t];
                return Ok(true);
            }
            JumpLoop => {
                let t = succ_block(&self.cfg, x.target.expect("a jump"))
                    .ok_or(Refusal::Limit("no target block".into()))?;
                self.emit(Op::Poll, vec![], Repr::None, Trust::Unknown);
                let u = self.konst(VALUE_UNDEFINED);
                self.set_acc(u);
                self.emit(Op::Jump, vec![], Repr::None, Trust::Unknown);
                self.g.blocks[bi].succs = vec![t];
                return Ok(true);
            }
            Return => {
                let a = self.acc()?;
                let t = self.tagged(a);
                self.emit(Op::Return, vec![t], Repr::None, Trust::Unknown);
                return Ok(true);
            }
            op => {
                return Err(Refusal::Opcode {
                    offset: x.offset,
                    opcode: op,
                });
            }
        }
        Ok(false)
    }

    fn check_array(&mut self, o: NodeId) -> NodeId {
        let st = self.state();
        let o = self.tagged(o);
        let policy = self.policy(GuardBasis::Unbacked);
        self.memo(4, o, policy, |s| {
            let n = s.emit(Op::CheckArray, vec![o], Repr::Tagged, Trust::Unknown);
            s.guard(n, GuardBasis::Unbacked, st);
            n
        })
    }

    /// `o.name` from the LoadNamed fact of `slot`: a shape guard through the IC cell and an
    /// in-object field load.
    fn named_load(&mut self, o: NodeId, slot: u32) -> Result<NodeId, Refusal> {
        let off = self.offset();
        let refuse = |why: String| Refusal::Limit(format!("named load at {off}: {why}"));
        let Some(&(_, facts)) = self.facts.records.iter().find(|(s, _)| *s == slot) else {
            return Err(refuse("no fact (the generic path is not built)".into()));
        };
        let SlotFacts::LoadNamed(pf) = facts else {
            return Err(refuse(format!(
                "fact of kind {}",
                crate::facts::kind_name(facts)
            )));
        };
        if !matches!(pf.state, IcState::Monomorphic | IcState::Polymorphic) || pf.cases.is_empty() {
            return Err(refuse(format!("IC state {:?}", pf.state)));
        }
        // Every case must read the same place: an in-object field of an ordinary object at
        // one offset, or an Array's length (the int32 at +24).
        let mut location: Option<(u32, u64)> = None;
        let mut shapes = Vec::new();
        for c in &pf.cases {
            let here = match c.access {
                Access::OwnField { field }
                    if c.receiver.kind == ObjectKind::Ordinary && field < c.receiver.capacity =>
                {
                    let at = vm_layout::OBJECT_FIELDS + 8 * u32::from(field);
                    (at, vm_layout::field_in_object_handler(at))
                }
                Access::ArrayLength if c.receiver.kind == ObjectKind::Array => {
                    (vm_layout::ARRAY_LENGTH, vm_layout::ARRAY_LENGTH_HANDLER)
                }
                ref a => {
                    return Err(refuse(format!(
                        "access {a:?} on a {:?} shape",
                        c.receiver.kind
                    )));
                }
            };
            if location.is_some_and(|l| l != here) {
                return Err(refuse("the cases read different places".into()));
            }
            location = Some(here);
            let i = self.g.shapes.len() as u32;
            self.g.shapes.push(c.receiver.clone());
            self.g.add_ref(RefNode::ShapeRef(i));
            shapes.push(i);
        }
        let (at, handler) = location.expect("a case");
        self.g.add_ref(RefNode::CellRef(slot));
        let st = self.state();
        let o = self.tagged(o);
        let source = self.facts.source;
        let chk = self.emit(
            Op::CheckShape {
                site: slot,
                shapes,
                handler,
            },
            vec![o],
            Repr::Tagged,
            Trust::Predicted(source),
        );
        self.guard(chk, GuardBasis::Fact(source), st);
        Ok(self.emit(
            Op::LoadField { offset: at },
            vec![chk],
            Repr::Tagged,
            Trust::Unknown,
        ))
    }

    /// Replaces every use of `old` (inputs and frame states) with `new`.
    fn replace(&mut self, old: NodeId, new: NodeId) {
        for n in &mut self.g.nodes {
            for i in &mut n.inputs {
                if *i == old {
                    *i = new;
                }
            }
            if let Some(g) = &mut n.guard {
                for (_, v) in &mut g.state.registers {
                    if *v == old {
                        *v = new;
                    }
                }
                if g.state.acc == Some(old) {
                    g.state.acc = Some(new);
                }
            }
        }
        for phis in &mut self.loop_phis {
            for (_, p) in phis.iter_mut() {
                if *p == old {
                    *p = new;
                }
            }
        }
    }

    fn kill(&mut self, n: NodeId) {
        self.g.node_mut(n).live = false;
        let b = self.g.node(n).block.0 as usize;
        self.g.blocks[b].nodes.retain(|&x| x != n);
    }

    /// Removes phis whose inputs are all one value (or the phi itself).
    fn remove_trivial_phis(&mut self) {
        loop {
            let mut changed = false;
            for i in 0..self.g.nodes.len() {
                let n = NodeId(i as u32);
                let node = self.g.node(n);
                if !node.live || node.op != Op::Phi {
                    continue;
                }
                let others: Vec<NodeId> = node.inputs.iter().copied().filter(|&x| x != n).collect();
                if others.is_empty() || others.iter().any(|&x| x != others[0]) {
                    continue;
                }
                let v = others[0];
                self.replace(n, v);
                self.kill(n);
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }

    /// A loop phi whose back-edge inputs are all Int32 becomes Int32: its entry input gets an
    /// (unbacked) CheckInt32 at the end of the preheader, with the frame state of the loop
    /// header (T0 would enter the loop there). Repeats to a fixed point (nested phis).
    ///
    /// R-SPIKE-P4-1 (M5-P1): the check goes only into a preheader whose one successor is the
    /// header. A preheader that may branch past the loop (`if (c) { while (..) .. }`) would
    /// run the check on a path that never enters the loop, and a deopt there would resume T0
    /// inside the loop. Such a phi stays Tagged (no edge split: the spike keeps blocks in
    /// bytecode order). [`crate::ir::Graph::check`] refuses the other form.
    fn unbox_loop_phis(&mut self) {
        loop {
            let mut changed = false;
            for h in 0..self.g.blocks.len() {
                if !self.g.blocks[h].loop_header {
                    continue;
                }
                let preds = self.g.blocks[h].preds.clone();
                let fwd: Vec<usize> = (0..preds.len())
                    .filter(|&k| (preds[k].0 as usize) < h)
                    .collect();
                let [entry] = fwd[..] else { continue };
                let pre = preds[entry];
                let pre_enters_only = self.g.blocks[pre.0 as usize].succs == [BlockId(h as u32)];
                for (r, phi) in self.loop_phis[h].clone() {
                    let node = self.g.node(phi);
                    if !node.live || node.op != Op::Phi || node.repr != Repr::Tagged {
                        continue;
                    }
                    let back_int = node
                        .inputs
                        .iter()
                        .enumerate()
                        .filter(|&(k, _)| k != entry)
                        .all(|(_, &v)| self.is_int32_or_check_of(v, phi));
                    if !back_int {
                        continue;
                    }
                    let e = node.inputs[entry];
                    if self.g.node(e).repr != Repr::Int32 && !pre_enters_only {
                        continue;
                    }
                    // Backed only if a check of the phi in the header block is: the header
                    // runs once at each entry with the entry value, so that site's record
                    // saw the entry value of each run. A check elsewhere in the loop (in a
                    // branch, or one that runs only on later iterations) or after it may
                    // never have seen an entry value.
                    let basis = self.g.blocks[h]
                        .nodes
                        .iter()
                        .copied()
                        .filter(|&c| {
                            let cn = self.g.node(c);
                            cn.live
                                && matches!(cn.op, Op::CheckInt32 | Op::TruncateInt32)
                                && cn.inputs[0] == phi
                        })
                        .filter_map(|c| self.g.node(c).guard.as_ref().map(|g| g.basis))
                        .find(|b| matches!(b, GuardBasis::Fact(_)))
                        .unwrap_or(GuardBasis::Unbacked);
                    let checked = if self.g.node(e).repr == Repr::Int32 {
                        e
                    } else {
                        // The header's frame state: every live register's value at the end
                        // of the preheader.
                        let start = self.cfg.blocks[h].insns.start;
                        let live = self.cfg.live_before[start].clone();
                        let pre_env = self.out_env[pre.0 as usize].clone().expect("done");
                        let acc = self.cfg.acc();
                        let state = FrameState {
                            offset: self.cfg.blocks[h].start,
                            registers: (0..acc)
                                .filter(|&x| live[x])
                                .filter_map(|x| pre_env[x].map(|v| (self.cfg.register(x), v)))
                                .collect(),
                            acc: if live[acc] { pre_env[acc] } else { None },
                        };
                        let c = self.insert_before_terminator(
                            pre,
                            Op::CheckInt32,
                            vec![e],
                            Repr::Int32,
                            trust_of(basis),
                        );
                        self.g.set_guard(c, basis, state);
                        c
                    };
                    let _ = r;
                    let p = self.g.node_mut(phi);
                    p.inputs[entry] = checked;
                    p.repr = Repr::Int32;
                    changed = true;
                }
            }
            // Checks of a value that is now Int32 are no-ops; so is a CheckNumber of a value
            // that is now Float64 (a loop-invariant double whose trivial phi went away).
            for i in 0..self.g.nodes.len() {
                let n = NodeId(i as u32);
                let node = self.g.node(n);
                let input = node.inputs.first().map(|&v| self.g.node(v).repr);
                let noop = match node.op {
                    Op::CheckInt32 | Op::TruncateInt32 => input == Some(Repr::Int32),
                    Op::CheckNumber => input == Some(Repr::Float64),
                    _ => false,
                };
                if node.live && noop {
                    let v = node.inputs[0];
                    self.replace(n, v);
                    self.kill(n);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Phi trust: the join of the inputs, to a fixed point (phis of phis). Phis start at
        // Proven here so a loop of phis settles at the least fixed point.
        let phis: Vec<NodeId> = (0..self.g.nodes.len() as u32)
            .map(NodeId)
            .filter(|&n| self.g.node(n).live && self.g.node(n).op == Op::Phi)
            .collect();
        for &n in &phis {
            self.g.node_mut(n).trust = Trust::Proven;
        }
        loop {
            let mut changed = false;
            for &n in &phis {
                let t = self
                    .g
                    .node(n)
                    .inputs
                    .iter()
                    .fold(Trust::Proven, |t, &v| t.join(self.g.node(v).trust));
                if t != self.g.node(n).trust {
                    self.g.node_mut(n).trust = t;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// True if `v` is an Int32, or a CheckInt32 / TruncateInt32 of `phi` (the phi's own use
    /// inside the loop, which unboxing makes a no-op).
    fn is_int32_or_check_of(&self, v: NodeId, phi: NodeId) -> bool {
        let n = self.g.node(v);
        n.repr == Repr::Int32 || (n.op == Op::Phi && v == phi)
    }

    fn insert_before_terminator(
        &mut self,
        b: BlockId,
        op: Op,
        inputs: Vec<NodeId>,
        repr: Repr,
        trust: Trust,
    ) -> NodeId {
        let blk = &self.g.blocks[b.0 as usize];
        let last = *blk.nodes.last().expect("a terminator");
        let off = self.g.node(last).offset;
        let n = self.g.add(op, inputs, repr, trust, b, off);
        let blk = &mut self.g.blocks[b.0 as usize];
        let at = blk.nodes.len() - 1;
        blk.nodes.insert(at, n);
        n
    }

    /// Inserts the boxes the reprs need: a Tagged use of an Int32 or Bool value (a Tagged phi's
    /// inputs get theirs at the end of the predecessor).
    fn fix_reprs(&mut self) {
        let order: Vec<NodeId> = self.g.order().collect();
        for n in order {
            let node = self.g.node(n).clone();
            let want: Vec<Repr> = match &node.op {
                Op::Phi => vec![node.repr; node.inputs.len()],
                Op::CheckInt32
                | Op::TruncateInt32
                | Op::CheckShape { .. }
                | Op::CheckArray
                | Op::IsFalse
                | Op::IsTrue
                | Op::Return
                | Op::LoadField { .. } => vec![Repr::Tagged],
                Op::LoadElement => vec![Repr::Tagged, Repr::Int32],
                Op::StoreElement => vec![Repr::Tagged, Repr::Int32, Repr::Int32],
                Op::CheckNumber => vec![Repr::Tagged],
                Op::Int32ToFloat64 => vec![Repr::Int32],
                Op::Float64(_) => vec![Repr::Float64, Repr::Float64],
                Op::BoxFloat64 => vec![Repr::Float64],
                Op::Call { regs } => vec![Repr::Tagged; regs.len()],
                Op::GetKeyedGeneric => vec![Repr::Tagged, Repr::Tagged],
                Op::SetKeyedGeneric => vec![Repr::Tagged, Repr::Tagged, Repr::Tagged],
                Op::Int32(_) | Op::Compare(_) => vec![Repr::Int32, Repr::Int32],
                Op::BoxInt32 => vec![Repr::Int32],
                Op::BoxBool | Op::Branch => vec![Repr::Bool],
                _ => vec![],
            };
            for (k, &w) in want.iter().enumerate() {
                let v = node.inputs[k];
                let have = self.g.node(v).repr;
                if have == w {
                    continue;
                }
                let t = self.g.node(v).trust;
                let fix = match (have, w) {
                    (Repr::Int32, Repr::Tagged) => Op::BoxInt32,
                    (Repr::Bool, Repr::Tagged) => Op::BoxBool,
                    (Repr::Float64, Repr::Tagged) => Op::BoxFloat64,
                    (Repr::Int32, Repr::Float64) => Op::Int32ToFloat64,
                    _ => panic!(
                        "{}: n{} wants {w:?}, input n{} is {have:?}",
                        self.g.name, n.0, v.0
                    ),
                };
                let (t, fix_repr) = if fix == Op::Int32ToFloat64 {
                    (Trust::Proven, Repr::Float64)
                } else {
                    (t, Repr::Tagged)
                };
                let b = if node.op == Op::Phi {
                    let pred = self.g.blocks[node.block.0 as usize].preds[k];
                    self.insert_before_terminator(pred, fix, vec![v], fix_repr, t)
                } else {
                    let blk = node.block.0 as usize;
                    let at = self.g.blocks[blk]
                        .nodes
                        .iter()
                        .position(|&x| x == n)
                        .expect("in block");
                    let b = self
                        .g
                        .add(fix, vec![v], fix_repr, t, node.block, node.offset);
                    self.g.blocks[blk].nodes.insert(at, b);
                    b
                };
                self.g.node_mut(n).inputs[k] = b;
            }
        }
    }

    /// Removes pure nodes that nothing uses (guards, effects, terminators and frame-state
    /// values stay).
    fn dce(&mut self) {
        let mut used = vec![false; self.g.nodes.len()];
        let mut work: Vec<NodeId> = self
            .g
            .order()
            .filter(|&n| self.g.node(n).op.has_effect())
            .collect();
        while let Some(n) = work.pop() {
            if std::mem::replace(&mut used[n.0 as usize], true) {
                continue;
            }
            let node = self.g.node(n);
            work.extend(node.inputs.iter().copied());
            if let Some(g) = &node.guard {
                work.extend(g.state.registers.iter().map(|&(_, v)| v));
                work.extend(g.state.acc);
            }
        }
        let dead: Vec<NodeId> = used
            .iter()
            .enumerate()
            .filter(|&(i, &u)| self.g.nodes[i].live && !u)
            .map(|(i, _)| NodeId(i as u32))
            .collect();
        for n in dead {
            self.kill(n);
        }
    }
}

/// The trust of a value that a guard with `basis` narrows: predicted{source} behind a fact,
/// else unknown (no fact knows it).
fn trust_of(basis: GuardBasis) -> Trust {
    match basis {
        GuardBasis::Fact(src) => Trust::Predicted(src),
        GuardBasis::Unbacked => Trust::Unknown,
    }
}

/// The guard basis that the record of `slot` in `facts` gives an int32 guard: a BinaryOp or
/// CompareOp hint inside {int32} (the empty set included: implicit int32, MODES-D10) is a fact
/// of the snapshot's source; anything else, or no record, is unbacked.
pub fn int_basis_of(facts: &FunctionFacts<'_>, slot: u32) -> GuardBasis {
    let int32 = match facts.records.iter().find(|(s, _)| *s == slot) {
        Some((_, SlotFacts::BinaryOp(h))) => BinaryHint::INT32.contains(*h),
        Some((_, SlotFacts::CompareOp(h))) => CompareHint::INT32.contains(*h),
        _ => false,
    };
    if int32 {
        GuardBasis::Fact(facts.source)
    } else {
        GuardBasis::Unbacked
    }
}

/// The path that the keyed record `k` (of a snapshot of `source`) gives its site: the
/// generic entry for a key that is not an index, else the element path with the basis of
/// the key's int32 check (see the module docs, "Keyed facts"). Only the keys are read.
pub fn keyed_path_of(k: &ep9_facts::KeyedFacts, source: ep9_facts::FactSource) -> KeyedPath {
    let other = KeySet::NAME.union(KeySet::SYMBOL).union(KeySet::OTHER);
    if k.keys.bits() & other.bits() != 0 {
        KeyedPath::Generic
    } else if k.keys == KeySet::INDEX {
        KeyedPath::Element(GuardBasis::Fact(source))
    } else {
        KeyedPath::Element(GuardBasis::Unbacked)
    }
}

/// The doubles a BinaryOp site may see and still back its CheckNumber guards: int32
/// operands (with any result) and number operands.
const NUMBER_HINTS: BinaryHint = BinaryHint::INT32
    .union(BinaryHint::INT32_OVERFLOW)
    .union(BinaryHint::NUMBER);

/// True if the BinaryOp record of `slot` saw a number operand that is not an int32 and only
/// numbers: the site takes the double path (SPIKEC-4). Any other hint keeps the int32 path.
pub fn saw_double(facts: &FunctionFacts<'_>, slot: u32) -> bool {
    matches!(
        facts.records.iter().find(|(s, _)| *s == slot),
        Some((_, SlotFacts::BinaryOp(h))) if h.contains(BinaryHint::NUMBER) && NUMBER_HINTS.contains(*h)
    )
}

/// The basis of the CheckNumber guards of the double site with feedback slot `slot`: a
/// BinaryOp record whose hint has only number bits (the empty set included: implicit int32,
/// MODES-D10) is a fact of the snapshot's source; anything else, or no record, is unbacked.
pub fn number_basis_of(facts: &FunctionFacts<'_>, slot: u32) -> GuardBasis {
    match facts.records.iter().find(|(s, _)| *s == slot) {
        Some((_, SlotFacts::BinaryOp(h))) if NUMBER_HINTS.contains(*h) => {
            GuardBasis::Fact(facts.source)
        }
        _ => GuardBasis::Unbacked,
    }
}

fn float_op_of(op: Opcode) -> Option<FloatOp> {
    match op {
        Opcode::Add => Some(FloatOp::Add),
        Opcode::Sub => Some(FloatOp::Sub),
        Opcode::Mul => Some(FloatOp::Mul),
        _ => None,
    }
}

fn int_op(op: Opcode) -> Int32Op {
    use Opcode::*;
    match op {
        Add | AddSmi | Inc => Int32Op::Add,
        Sub | SubSmi | Dec => Int32Op::Sub,
        Mul | MulSmi => Int32Op::Mul,
        BitwiseAnd | BitwiseAndSmi => Int32Op::And,
        BitwiseOr | BitwiseOrSmi => Int32Op::Or,
        BitwiseXor | BitwiseXorSmi => Int32Op::Xor,
        ShiftLeft | ShiftLeftSmi => Int32Op::Shl,
        _ => Int32Op::Sar,
    }
}
