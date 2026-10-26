//! AArch64 code from an allocated [`crate::ir::Graph`] (SPIKEC-1, M4-P1), with dynasm 5.1.0 (the
//! spikes/spike-b pin).
//!
//! The output is a [`spike_b::emit::FunctionCode`]: position-independent code, entry at offset
//! 0, with the placement (b) ABI (`ep9_vm::install_native_b`: the T0 handler ABI `(cx, pc, fp,
//! acc)`, leave by a tail jump to `cx.table[*pc]`), its CFI, and `native_offsets` per bytecode
//! instruction. So the `spike_b::jit::InMemory` finalizer places it and
//! [`crate::harness::Session::install_b`] installs it.
//!
//! ```text
//!  0  stp x29, x30, [sp, #-S]!        S = 96 + 8 * spill slots, rounded up to 16 (at most 496)
//!  4  mov x29, sp
//!  8  stp x26, x28, [sp, #16] ; stp x19, x20, [sp, #32] ; stp x21, x22, [sp, #48]
//!     stp x23, x24, [sp, #64] ; stp x25, x27, [sp, #80]     spill slots from sp + 96
//! 28  mov x28, x0 ; mov x26, x2 ; native stack limit check (as spike-b)
//!     body: blocks in bytecode order; phi moves at the end of a block (a branch edge with
//!     moves gets an out-of-line edge stub)
//!  ret:   x3 = the result ; x1 = return pc, x2 = caller fp (decoded native words)
//!  leave: mov x0, x28 ; ldrb w9, [x1] ; ldr x16, [x0, x9, lsl #3]
//!         ldp (the five pairs, reversed) ; ldp x29, x30, [sp], #S ; br x16
//!  out of line: overflow, poll slow paths (x4-x15 saved around RuntimeTable::poll), edge
//!  stubs, guard traps, throw stubs, the unwind call
//! ```
//!
//! **Guards (H32, JIT mode).** A failed guard jumps to its exit stub, made from the guard's
//! `Guard.policy` (data the graph builder set: the emitter reads no mode,
//! docs/plan/13-execution-modes.md B2, P-R1). The one policy is `ExitPolicy::Deopt`. Until
//! month 5 builds the eager deopt entry in `ep9_vm`, its stub is the **"deopt not built"
//! trap**: it throws the
//! int32 [`TRAP_BASE`] + guard id through `RuntimeTable::throw` and unwinds as any throw from
//! (b) code. The run then ends with that uncaught number ([`trap_of`] reads it back), never
//! with a wrong value. It is a stand-in, not a policy: H32 never throws on a fact failure, and
//! JS code could catch the number (the protocol kernels do not catch around compiled
//! functions). SPIKEC-4 makes the per-exit position-independent metadata (pc offset from the
//! code start, bytecode offset, value locations, RW exit-counter index; H29-Q1, Q2, Q5) from
//! the guards' frame states.
//!
//! Scratch: x12-x14 stage inputs that are not in registers, x15 stages a spilled result (and
//! is a temporary in the poll and store paths), x16 and x17 are temporaries. No absolute
//! address and no relocation: branches are pc-relative inside the function; the dispatch table
//! is at `Cx` offset 0, the runtime functions are behind `Cx::runtime_table`, the IC cell of a
//! shape guard is reached from the VM frame (FunctionInfo word, then
//! [`vm_layout::FI_FEEDBACK`]).

// dynasm expands a dynamic register `X(n)` to a `u8::from(n)` conversion, which clippy flags
// at the call site when `n` is already a u8.
#![allow(clippy::useless_conversion)]

use std::collections::BTreeMap;

use dynasmrt::aarch64::Aarch64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_value::encoding::{
    DOUBLE_ENCODE_OFFSET, NOT_CELL_MASK, NUMBER_TAG, VALUE_FALSE, VALUE_TRUE,
};
use ep9_vm::layout;
use spike_b::emit::{Arch, FunctionCode, FunctionInput, Refusal, Unwind};

use crate::ir::{BlockId, Cond, ExitPolicy, Graph, Int32Op, NodeId, Op};
use crate::regalloc::{Allocation, Location};
use crate::vm_layout;

type Ops = VecAssembler<Aarch64Relocation>;

/// The first thrown number of the "deopt not built" trap: guard `g` throws `TRAP_BASE + g`.
pub const TRAP_BASE: i32 = 1_900_000_000;

/// The guard id of a "deopt not built" trap, from an uncaught exception's text.
pub fn trap_of(text: &str) -> Option<u32> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter_map(|w| w.parse::<i64>().ok())
        .find(|&n| n >= i64::from(TRAP_BASE) && n < i64::from(TRAP_BASE) + 100_000)
        .map(|n| (n - i64::from(TRAP_BASE)) as u32)
}

/// What the emitter made (the report).
#[derive(Clone, Debug, Default)]
pub struct EmitReport {
    pub code_bytes: usize,
    pub frame_bytes: u32,
    pub guards: usize,
    /// Native offset of each guard's trap stub: (guard id, offset).
    pub traps: Vec<(u32, u32)>,
}

/// DWARF register numbers.
const FP: u16 = 29;
const LR: u16 = 30;
const SP: u16 = 31;
/// The callee-saved pairs after the frame record: (register, its offset from sp).
const SAVED: [(u8, u32); 10] = [
    (26, 16),
    (28, 24),
    (19, 32),
    (20, 40),
    (21, 48),
    (22, 56),
    (23, 64),
    (24, 72),
    (25, 80),
    (27, 88),
];
/// The first spill slot (sp + 96).
const SPILL_BASE: u32 = 96;
/// The largest frame: the epilogue's `ldp x29, x30, [sp], #S` takes a post-index immediate of
/// at most +504 (the prologue's `stp` reaches -512), so 496, the largest multiple of 16 below
/// it (50 spill slots).
const MAX_FRAME: u32 = 496;

/// The frame size for `spill_slots` spill slots, or a refusal past [`MAX_FRAME`].
fn frame_bytes(spill_slots: u32) -> Result<u32, Refusal> {
    let frame = (SPILL_BASE + 8 * spill_slots).next_multiple_of(16);
    if frame > MAX_FRAME {
        return Err(Refusal::Limit(format!(
            "{spill_slots} spill slots: a frame of {frame} bytes"
        )));
    }
    Ok(frame)
}

/// `ldr`/`str` with an unsigned scaled 12-bit offset: 0..=32760, a multiple of 8.
fn scaled(off: usize) -> Result<u32, Refusal> {
    if off.is_multiple_of(8) && off <= 32760 {
        Ok(off as u32)
    } else {
        Err(Refusal::Limit(format!("offset {off}")))
    }
}

/// x`rd` = `v` (movz, then movk for each other non-zero halfword). `v` is a constant (a Value
/// tag, an immediate Value or an offset), never an address.
fn mov64(ops: &mut Ops, rd: u8, v: u64) {
    let h = |i: u32| ((v >> (16 * i)) & 0xFFFF) as u32;
    let first = (0..4).find(|&i| h(i) != 0).unwrap_or(0);
    let (f, fs) = (h(first), first * 16);
    dynasm!(ops ; .arch aarch64 ; movz X(rd), f, lsl fs);
    for i in first + 1..4 {
        if h(i) != 0 {
            let (p, s) = (h(i), i * 16);
            dynasm!(ops ; .arch aarch64 ; movk X(rd), p, lsl s);
        }
    }
}

/// `ep9_vm::layout::NATIVE_WORD_OFFSET`, the offset of a frame's native words, for an `eor`
/// that decodes one (MC-w13-14: native code uses the named constant). It must be one bit,
/// so it is a logical immediate and an `eor` undoes the `+` (frame.rs `native`); otherwise
/// the compile is refused.
fn native_word_offset() -> Result<u64, Refusal> {
    let w = layout::NATIVE_WORD_OFFSET;
    if w.is_power_of_two() {
        Ok(w)
    } else {
        Err(Refusal::Limit(format!(
            "NATIVE_WORD_OFFSET {w:#x} is not one bit: an eor cannot decode a native word"
        )))
    }
}

/// x`rt` = the VM frame slot at byte offset `off` from x26.
fn load_fp(ops: &mut Ops, rt: u8, off: i32) {
    if off >= 0 && off % 8 == 0 && off <= 32760 {
        let o = off as u32;
        dynasm!(ops ; .arch aarch64 ; ldr X(rt), [x26, o]);
    } else if (-256..256).contains(&off) {
        dynasm!(ops ; .arch aarch64 ; ldur X(rt), [x26, off]);
    } else {
        mov64(ops, 16, off as i64 as u64);
        dynasm!(ops ; .arch aarch64 ; ldr X(rt), [x26, x16]);
    }
}

/// Calls the runtime-table function at `entry` with x0 = cx, x1 = fp, w2 = `off` (value
/// operands already in x3..). Result in x0.
fn call_table(ops: &mut Ops, entry: usize, off: u32) -> Result<(), Refusal> {
    let table = scaled(layout::CX_RUNTIME_TABLE)?;
    let entry = scaled(entry)?;
    dynasm!(ops ; .arch aarch64 ; mov x0, x28 ; mov x1, x26);
    mov64(ops, 2, u64::from(off));
    dynasm!(ops
        ; .arch aarch64
        ; ldr x16, [x28, table]
        ; ldr x16, [x16, entry]
        ; blr x16
    );
    Ok(())
}

/// A source of a phi move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Node(NodeId),
    /// x17, holding a value saved to break a cycle.
    Temp,
}

struct Emitter<'a> {
    ops: Ops,
    g: &'a Graph,
    a: &'a Allocation,
    labels: Vec<DynamicLabel>,
    frame: u32,
    /// Guard exits: (label, guard id, bytecode offset, the guard's policy).
    traps: Vec<(DynamicLabel, u32, u32, ExitPolicy)>,
    throws: BTreeMap<u32, DynamicLabel>,
    unwind: DynamicLabel,
    leave: DynamicLabel,
    ret: DynamicLabel,
    /// Poll slow paths: (slow, continue, bytecode offset).
    polls: Vec<(DynamicLabel, DynamicLabel, u32)>,
    /// Branch edge stubs: (label, from block, to block).
    edges: Vec<(DynamicLabel, usize, usize)>,
    /// First native offset of each bytecode offset that made code.
    native_at: BTreeMap<u32, u32>,
    /// (guard id, native offset of its trap stub).
    trap_at: Vec<(u32, u32)>,
}

/// Emits the code of `graph` (built from `input`) with the registers of `alloc`.
pub fn emit(
    input: &FunctionInput<'_>,
    graph: &Graph,
    alloc: &Allocation,
) -> Result<(FunctionCode, EmitReport), Refusal> {
    let frame = frame_bytes(alloc.spill_slots)?;
    let mut ops = Ops::new(0);
    let labels = graph
        .blocks
        .iter()
        .map(|_| ops.new_dynamic_label())
        .collect();
    let (unwind, leave, ret) = (
        ops.new_dynamic_label(),
        ops.new_dynamic_label(),
        ops.new_dynamic_label(),
    );
    let mut e = Emitter {
        ops,
        g: graph,
        a: alloc,
        labels,
        frame,
        traps: Vec::new(),
        throws: BTreeMap::new(),
        unwind,
        leave,
        ret,
        polls: Vec::new(),
        edges: Vec::new(),
        native_at: BTreeMap::new(),
        trap_at: Vec::new(),
    };
    let overflow = e.ops.new_dynamic_label();
    e.prologue(overflow)?;
    for bi in 0..graph.blocks.len() {
        e.block(bi)?;
    }
    let epilogue = e.ret_and_leave()?;
    e.out_of_line(overflow)?;
    let report_traps = std::mem::take(&mut e.trap_at);
    let guards = e.traps.len();
    let native_at = std::mem::take(&mut e.native_at);
    let code = e
        .ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let report = EmitReport {
        code_bytes: code.len(),
        frame_bytes: frame,
        guards,
        traps: report_traps,
    };
    // ops / native_offsets: every reachable instruction, with the first native offset of its
    // code (an instruction that made none gets the next one's).
    let reachable = graph.insns.clone();
    let mut native_offsets = vec![0u32; reachable.len()];
    let mut next = code.len() as u32;
    for (i, (o, _)) in reachable.iter().enumerate().rev() {
        if let Some(&n) = native_at.get(o) {
            next = n;
        }
        native_offsets[i] = next;
    }
    Ok((
        FunctionCode {
            name: input.name.to_string(),
            arch: Arch::Aarch64,
            key: input.key,
            code,
            unwind: Unwind {
                cfi: cfi(frame, epilogue),
                win64: None,
            },
            ops: reachable,
            native_offsets,
        },
        report,
    ))
}

impl Emitter<'_> {
    fn prologue(&mut self, overflow: DynamicLabel) -> Result<(), Refusal> {
        let native_limit = scaled(layout::CX_NATIVE_LIMIT)?;
        let neg = -(self.frame as i32);
        dynasm!(self.ops
            ; .arch aarch64
            ; stp x29, x30, [sp, neg]!
            ; mov x29, sp
            ; stp x26, x28, [sp, #16]
            ; stp x19, x20, [sp, #32]
            ; stp x21, x22, [sp, #48]
            ; stp x23, x24, [sp, #64]
            ; stp x25, x27, [sp, #80]
            ; mov x28, x0
            ; mov x26, x2
            ; ldr x16, [x28, native_limit]
            ; mov x17, sp
            ; cmp x17, x16
            ; b.ls =>overflow
        );
        Ok(())
    }

    fn trap(&mut self, n: NodeId) -> DynamicLabel {
        let node = self.g.node(n);
        let g = node.guard.as_ref().expect("a guard");
        let l = self.ops.new_dynamic_label();
        self.traps.push((l, g.id, g.state.offset, g.policy));
        l
    }

    fn throw_at(&mut self, off: u32) -> DynamicLabel {
        let ops = &mut self.ops;
        *self
            .throws
            .entry(off)
            .or_insert_with(|| ops.new_dynamic_label())
    }

    fn slot_off(k: u32) -> u32 {
        SPILL_BASE + 8 * k
    }

    /// The register that holds `v` (loaded into `stage` if it is not in one).
    fn src(&mut self, v: NodeId, stage: u8) -> Result<u8, Refusal> {
        match self.a.of(v) {
            Location::Reg(r) => Ok(r),
            Location::Stack(k) => {
                let o = Self::slot_off(k);
                dynasm!(self.ops ; .arch aarch64 ; ldr X(stage), [sp, o]);
                Ok(stage)
            }
            Location::Frame(off) => {
                load_fp(&mut self.ops, stage, off);
                Ok(stage)
            }
            Location::Const => {
                let bits = match self.g.node(v).op {
                    Op::Const(b) => b,
                    Op::ConstInt32(i) => u64::from(i as u32),
                    ref op => return Err(Refusal::Limit(format!("constant {op:?}"))),
                };
                mov64(&mut self.ops, stage, bits);
                Ok(stage)
            }
            l => Err(Refusal::Limit(format!(
                "n{} has no value location ({l:?})",
                v.0
            ))),
        }
    }

    /// The register the result of `n` is written to (x15 for a spilled result).
    fn dst(&self, n: NodeId) -> Result<u8, Refusal> {
        match self.a.of(n) {
            Location::Reg(r) => Ok(r),
            Location::Stack(_) => Ok(15),
            l => Err(Refusal::Limit(format!(
                "n{} has no result location ({l:?})",
                n.0
            ))),
        }
    }

    /// Stores a spilled result from x15.
    fn finish(&mut self, n: NodeId) {
        if let Location::Stack(k) = self.a.of(n) {
            let o = Self::slot_off(k);
            dynasm!(self.ops ; .arch aarch64 ; str x15, [sp, o]);
        }
    }

    fn block(&mut self, bi: usize) -> Result<(), Refusal> {
        let l = self.labels[bi];
        dynasm!(self.ops ; .arch aarch64 ; =>l);
        let nodes: Vec<NodeId> = self.g.blocks[bi]
            .nodes
            .iter()
            .copied()
            .filter(|&n| self.g.node(n).live)
            .collect();
        for n in nodes {
            let off = self.g.node(n).offset;
            let here = self.ops.offset().0 as u32;
            self.native_at.entry(off).or_insert(here);
            self.node(bi, n)?;
        }
        Ok(())
    }

    fn node(&mut self, bi: usize, n: NodeId) -> Result<(), Refusal> {
        let node = self.g.node(n).clone();
        let ins = node.inputs.clone();
        match &node.op {
            Op::Incoming(_) | Op::Const(_) | Op::ConstInt32(_) | Op::Phi => {}
            Op::CheckInt32 | Op::TruncateInt32 => {
                let trap = self.trap(n);
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                let (slow, done, ok) = (
                    self.ops.new_dynamic_label(),
                    self.ops.new_dynamic_label(),
                    self.ops.new_dynamic_label(),
                );
                mov64(&mut self.ops, 17, NUMBER_TAG);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; cmp X(x), x17
                    ; b.lo =>slow
                    ; mov W(d), W(x)
                    ; b =>done
                    ; =>slow
                    ; tst X(x), x17
                    ; b.eq =>trap
                );
                mov64(&mut self.ops, 16, DOUBLE_ENCODE_OFFSET);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; sub x16, X(x), x16
                    ; fmov d16, x16
                    ; fcvtzs w16, d16
                    ; scvtf d17, w16
                    ; fcmp d16, d17
                    ; b.ne =>trap
                );
                if node.op == Op::CheckInt32 {
                    // -0 is not an int32.
                    dynasm!(self.ops
                        ; .arch aarch64
                        ; cbnz w16, =>ok
                        ; fmov x17, d16
                        ; tbnz x17, #63, =>trap
                    );
                }
                dynasm!(self.ops
                    ; .arch aarch64
                    ; =>ok
                    ; mov W(d), w16
                    ; =>done
                );
                self.finish(n);
            }
            Op::CheckArray => {
                let trap = self.trap(n);
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                let kind = vm_layout::KIND_BYTE;
                let arr = u32::from(vm_layout::KIND_ARRAY);
                mov64(&mut self.ops, 17, NOT_CELL_MASK);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; tst X(x), x17
                    ; b.ne =>trap
                    ; cbz X(x), =>trap
                    ; ldrb w16, [X(x), kind]
                    ; cmp w16, arr
                    ; b.ne =>trap
                );
                if d != x {
                    dynasm!(self.ops ; .arch aarch64 ; mov X(d), X(x));
                }
                self.finish(n);
            }
            Op::CheckShape { site, handler, .. } => {
                let trap = self.trap(n);
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                let fi_feedback = scaled(vm_layout::FI_FEEDBACK)?;
                let fi_word = layout::FP_FUNCTION_INFO as i32;
                let entry = vm_layout::FEEDBACK_SLOTS + vm_layout::FEEDBACK_SLOT_SIZE * site;
                let (poly, hit) = (self.ops.new_dynamic_label(), self.ops.new_dynamic_label());
                let hole = vm_layout::HOLE as u32;
                let native_word = native_word_offset()?;
                mov64(&mut self.ops, 17, NOT_CELL_MASK);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; tst X(x), x17
                    ; b.ne =>trap
                    ; cbz X(x), =>trap
                    ; ldr w15, [X(x)]
                );
                // The IC entry: fp's FunctionInfo -> its feedback slot -> the FEEDBACK cell.
                load_fp(&mut self.ops, 16, fi_word);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; eor x16, x16, native_word
                    ; ldr x16, [x16, fi_feedback]
                    ; cbz x16, =>trap
                    ; ldr x16, [x16]
                );
                mov64(&mut self.ops, 17, u64::from(entry));
                dynasm!(self.ops
                    ; .arch aarch64
                    ; add x16, x16, x17
                    ; ldr x17, [x16]
                    ; cmp w17, w15
                    ; b.ne =>poly
                    ; ldr x17, [x16, #8]
                    ; b =>hit
                    ; =>poly
                    ; cmp x17, hole
                    ; b.ne =>trap
                    ; ldr x16, [x16, #8]
                );
                mov64(&mut self.ops, 17, NOT_CELL_MASK);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; tst x16, x17
                    ; b.ne =>trap
                );
                for i in 0..vm_layout::POLY_COUNT {
                    let k = vm_layout::POLY_CASES + vm_layout::POLY_CASE_SIZE * i;
                    let h = k + 8;
                    let next = self.ops.new_dynamic_label();
                    dynasm!(self.ops
                        ; .arch aarch64
                        ; ldr w17, [x16, k]
                        ; cmp w17, w15
                        ; b.ne =>next
                        ; ldr x17, [x16, h]
                        ; b =>hit
                        ; =>next
                    );
                }
                dynasm!(self.ops ; .arch aarch64 ; b =>trap ; =>hit);
                mov64(&mut self.ops, 16, *handler);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; cmp x17, x16
                    ; b.ne =>trap
                );
                if d != x {
                    dynasm!(self.ops ; .arch aarch64 ; mov X(d), X(x));
                }
                self.finish(n);
            }
            Op::LoadField { offset } => {
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                let o = scaled(*offset as usize)?;
                dynasm!(self.ops ; .arch aarch64 ; ldr X(d), [X(x), o]);
                self.finish(n);
            }
            Op::LoadElement | Op::StoreElement => {
                let trap = self.trap(n);
                // Stage every input before x16 holds the elements (a far frame slot would
                // load its offset through x16).
                let arr = self.src(ins[0], 12)?;
                let i = self.src(ins[1], 13)?;
                let v = if node.op == Op::StoreElement {
                    Some(self.src(ins[2], 14)?)
                } else {
                    None
                };
                let (len, elems, cap, data) = (
                    vm_layout::ARRAY_LENGTH,
                    vm_layout::ARRAY_ELEMENTS,
                    vm_layout::ELEMENTS_CAPACITY,
                    vm_layout::ELEMENTS_DATA,
                );
                let hole = vm_layout::HOLE as u32;
                dynasm!(self.ops
                    ; .arch aarch64
                    ; ldr w16, [X(arr), len]
                    ; cmp W(i), w16
                    ; b.hs =>trap
                    ; ldr x16, [X(arr), elems]
                );
                mov64(&mut self.ops, 17, NOT_CELL_MASK);
                dynasm!(self.ops
                    ; .arch aarch64
                    ; tst x16, x17
                    ; b.ne =>trap
                    ; ldr w17, [x16, cap]
                    ; cmp W(i), w17
                    ; b.hs =>trap
                    ; add x16, x16, data
                    ; ldr x17, [x16, W(i), uxtw #3]
                    ; cmp x17, hole
                    ; b.eq =>trap
                );
                if let Some(v) = v {
                    dynasm!(self.ops ; .arch aarch64 ; mov w17, W(v));
                    mov64(&mut self.ops, 15, NUMBER_TAG);
                    dynasm!(self.ops
                        ; .arch aarch64
                        ; orr x17, x17, x15
                        ; str x17, [x16, W(i), uxtw #3]
                    );
                } else {
                    let d = self.dst(n)?;
                    dynasm!(self.ops ; .arch aarch64 ; mov X(d), x17);
                    self.finish(n);
                }
            }
            Op::Int32(op) => {
                let l = self.src(ins[0], 12)?;
                let r = self.src(ins[1], 13)?;
                let d = self.dst(n)?;
                match op {
                    Int32Op::And => dynasm!(self.ops ; .arch aarch64 ; and W(d), W(l), W(r)),
                    Int32Op::Or => dynasm!(self.ops ; .arch aarch64 ; orr W(d), W(l), W(r)),
                    Int32Op::Xor => dynasm!(self.ops ; .arch aarch64 ; eor W(d), W(l), W(r)),
                    Int32Op::Shl => dynasm!(self.ops ; .arch aarch64 ; lsl W(d), W(l), W(r)),
                    Int32Op::Sar => dynasm!(self.ops ; .arch aarch64 ; asr W(d), W(l), W(r)),
                    Int32Op::Add => {
                        let trap = self.trap(n);
                        dynasm!(self.ops ; .arch aarch64 ; adds W(d), W(l), W(r) ; b.vs =>trap);
                    }
                    Int32Op::Sub => {
                        let trap = self.trap(n);
                        dynasm!(self.ops ; .arch aarch64 ; subs W(d), W(l), W(r) ; b.vs =>trap);
                    }
                    Int32Op::Mul => {
                        let trap = self.trap(n);
                        let ok = self.ops.new_dynamic_label();
                        dynasm!(self.ops
                            ; .arch aarch64
                            ; smull x16, W(l), W(r)
                            ; cmp x16, w16, sxtw
                            ; b.ne =>trap
                            ; cbnz w16, =>ok
                            // 0 from a negative operand is -0: not an int32.
                            ; orr w17, W(l), W(r)
                            ; tbnz w17, #31, =>trap
                            ; =>ok
                            ; mov W(d), w16
                        );
                    }
                }
                self.finish(n);
            }
            Op::Compare(c) => {
                if crate::regalloc::fused(self.g, n) {
                    return Ok(());
                }
                let l = self.src(ins[0], 12)?;
                let r = self.src(ins[1], 13)?;
                let d = self.dst(n)?;
                dynasm!(self.ops ; .arch aarch64 ; cmp W(l), W(r));
                match c {
                    Cond::Lt => dynasm!(self.ops ; .arch aarch64 ; cset W(d), lt),
                    Cond::Gt => dynasm!(self.ops ; .arch aarch64 ; cset W(d), gt),
                    Cond::Le => dynasm!(self.ops ; .arch aarch64 ; cset W(d), le),
                    Cond::Ge => dynasm!(self.ops ; .arch aarch64 ; cset W(d), ge),
                    Cond::Eq => dynasm!(self.ops ; .arch aarch64 ; cset W(d), eq),
                    Cond::Ne => dynasm!(self.ops ; .arch aarch64 ; cset W(d), ne),
                }
                self.finish(n);
            }
            Op::IsFalse | Op::IsTrue => {
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                let want = if node.op == Op::IsTrue {
                    VALUE_TRUE
                } else {
                    VALUE_FALSE
                } as u32;
                dynasm!(self.ops ; .arch aarch64 ; cmp XSP(x), want ; cset W(d), eq);
                self.finish(n);
            }
            Op::BoxInt32 => {
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                dynasm!(self.ops ; .arch aarch64 ; mov w16, W(x));
                mov64(&mut self.ops, 17, NUMBER_TAG);
                dynasm!(self.ops ; .arch aarch64 ; orr X(d), x16, x17);
                self.finish(n);
            }
            Op::BoxBool => {
                let x = self.src(ins[0], 12)?;
                let d = self.dst(n)?;
                let f = VALUE_FALSE as u32;
                dynasm!(self.ops ; .arch aarch64 ; add WSP(d), WSP(x), f);
                self.finish(n);
            }
            Op::Poll => {
                let budget = scaled(layout::CX_BUDGET)?;
                let (slow, cont) = (self.ops.new_dynamic_label(), self.ops.new_dynamic_label());
                dynasm!(self.ops
                    ; .arch aarch64
                    ; ldr x16, [x28, budget]
                    ; subs x16, x16, #1
                    ; str x16, [x28, budget]
                    ; b.le =>slow
                    ; =>cont
                );
                self.polls.push((slow, cont, node.offset));
            }
            Op::Jump => {
                let s = self.g.blocks[bi].succs[0].0 as usize;
                self.phi_moves(bi, s)?;
                if s != bi + 1 {
                    let l = self.labels[s];
                    dynasm!(self.ops ; .arch aarch64 ; b =>l);
                }
            }
            Op::Branch => {
                let succs = self.g.blocks[bi].succs.clone();
                let (t, f) = (succs[0].0 as usize, succs[1].0 as usize);
                let lt = self.edge(bi, t);
                let lf = self.edge(bi, f);
                let c = ins[0];
                if crate::regalloc::fused(self.g, c) {
                    let Op::Compare(cond) = self.g.node(c).op else {
                        unreachable!()
                    };
                    let ci = self.g.node(c).inputs.clone();
                    let l = self.src(ci[0], 12)?;
                    let r = self.src(ci[1], 13)?;
                    dynasm!(self.ops ; .arch aarch64 ; cmp W(l), W(r));
                    match cond {
                        Cond::Lt => dynasm!(self.ops ; .arch aarch64 ; b.lt =>lt),
                        Cond::Gt => dynasm!(self.ops ; .arch aarch64 ; b.gt =>lt),
                        Cond::Le => dynasm!(self.ops ; .arch aarch64 ; b.le =>lt),
                        Cond::Ge => dynasm!(self.ops ; .arch aarch64 ; b.ge =>lt),
                        Cond::Eq => dynasm!(self.ops ; .arch aarch64 ; b.eq =>lt),
                        Cond::Ne => dynasm!(self.ops ; .arch aarch64 ; b.ne =>lt),
                    }
                } else {
                    let x = self.src(c, 12)?;
                    dynasm!(self.ops ; .arch aarch64 ; cbnz W(x), =>lt);
                }
                if !(f == bi + 1 && lf == self.labels[f]) {
                    dynasm!(self.ops ; .arch aarch64 ; b =>lf);
                }
            }
            Op::Return => {
                let x = self.src(ins[0], 12)?;
                let ret = self.ret;
                dynasm!(self.ops ; .arch aarch64 ; mov x3, X(x) ; b =>ret);
            }
        }
        Ok(())
    }

    /// The label to branch to for the edge `from -> to`: the block itself, or an edge stub
    /// that does the phi moves first.
    fn edge(&mut self, from: usize, to: usize) -> DynamicLabel {
        if self.moves_of(from, to).is_empty() {
            return self.labels[to];
        }
        let l = self.ops.new_dynamic_label();
        self.edges.push((l, from, to));
        l
    }

    /// The phi moves of the edge `from -> to`: (destination, source).
    fn moves_of(&self, from: usize, to: usize) -> Vec<(Location, Src)> {
        let tb = &self.g.blocks[to];
        let Some(k) = tb.preds.iter().position(|&p| p == BlockId(from as u32)) else {
            return Vec::new();
        };
        tb.nodes
            .iter()
            .filter(|&&p| self.g.node(p).live && self.g.node(p).op == Op::Phi)
            .map(|&p| (self.a.of(p), Src::Node(self.g.node(p).inputs[k])))
            .filter(|&(d, s)| {
                matches!(d, Location::Reg(_) | Location::Stack(_)) && self.src_loc(s) != Some(d)
            })
            .collect()
    }

    fn phi_moves(&mut self, from: usize, to: usize) -> Result<(), Refusal> {
        let moves = self.moves_of(from, to);
        self.parallel_moves(moves)
    }

    /// The location a move source is read from (`None` for a constant or a frame slot, which
    /// no move writes).
    fn src_loc(&self, s: Src) -> Option<Location> {
        match s {
            Src::Node(v) => match self.a.of(v) {
                l @ (Location::Reg(_) | Location::Stack(_)) => Some(l),
                _ => None,
            },
            Src::Temp => None,
        }
    }

    /// Emits the parallel copy `moves` (every destination gets the old value of its source),
    /// breaking cycles through x17.
    fn parallel_moves(&mut self, mut pending: Vec<(Location, Src)>) -> Result<(), Refusal> {
        while !pending.is_empty() {
            let free = (0..pending.len()).find(|&i| {
                let d = pending[i].0;
                !pending
                    .iter()
                    .enumerate()
                    .any(|(j, &(_, s))| j != i && self.src_loc(s) == Some(d))
            });
            match free {
                Some(i) => {
                    let (d, s) = pending.remove(i);
                    let r = match s {
                        Src::Temp => 17,
                        Src::Node(v) => self.src(v, 16)?,
                    };
                    match d {
                        Location::Reg(x) => {
                            if x != r {
                                dynasm!(self.ops ; .arch aarch64 ; mov X(x), X(r));
                            }
                        }
                        Location::Stack(k) => {
                            let o = Self::slot_off(k);
                            dynasm!(self.ops ; .arch aarch64 ; str X(r), [sp, o]);
                        }
                        _ => unreachable!("moves_of keeps register and stack destinations"),
                    }
                }
                None => {
                    // A cycle: save the first move's source in x17.
                    let Src::Node(v) = pending[0].1 else {
                        return Err(Refusal::Limit("two move cycles at once".into()));
                    };
                    let from = self.src_loc(pending[0].1);
                    let r = self.src(v, 16)?;
                    dynasm!(self.ops ; .arch aarch64 ; mov x17, X(r));
                    for m in &mut pending {
                        if self.src_loc(m.1) == from {
                            m.1 = Src::Temp;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `ret` and the shared `leave` tail jump. Returns the offset of the first restoring
    /// instruction (where the CFI epilogue rows start).
    fn ret_and_leave(&mut self) -> Result<u32, Refusal> {
        let ret_pc = layout::FP_RETURN_PC as i32;
        let caller = layout::FP_CALLER_FP as i32;
        let native_word = native_word_offset()?;
        if layout::CX_TABLE != 0 {
            return Err(Refusal::Limit("the (b) exit layout changed".into()));
        }
        let (ret, leave) = (self.ret, self.leave);
        dynasm!(self.ops ; .arch aarch64 ; =>ret);
        load_fp(&mut self.ops, 1, ret_pc);
        load_fp(&mut self.ops, 2, caller);
        dynasm!(self.ops
            ; .arch aarch64
            ; eor x1, x1, native_word
            ; eor x2, x2, native_word
            ; =>leave
            ; mov x0, x28
            ; ldrb w9, [x1]
            ; ldr x16, [x0, x9, lsl #3]
        );
        let epilogue = self.ops.offset().0 as u32;
        let frame = self.frame as i32;
        dynasm!(self.ops
            ; .arch aarch64
            ; ldp x25, x27, [sp, #80]
            ; ldp x23, x24, [sp, #64]
            ; ldp x21, x22, [sp, #48]
            ; ldp x19, x20, [sp, #32]
            ; ldp x26, x28, [sp, #16]
            ; ldp x29, x30, [sp], frame
            ; br x16
        );
        Ok(epilogue)
    }

    fn out_of_line(&mut self, overflow: DynamicLabel) -> Result<(), Refusal> {
        let throw0 = self.throw_at(0);
        dynasm!(self.ops ; .arch aarch64 ; =>overflow);
        call_table(&mut self.ops, layout::RT_STACK_OVERFLOW, 0)?;
        dynasm!(self.ops ; .arch aarch64 ; b =>throw0);
        for (slow, cont, off) in std::mem::take(&mut self.polls) {
            let throw = self.throw_at(off);
            dynasm!(self.ops
                ; .arch aarch64
                ; =>slow
                ; stp x4, x5, [sp, #-96]!
                ; stp x6, x7, [sp, #16]
                ; stp x8, x9, [sp, #32]
                ; stp x10, x11, [sp, #48]
                ; stp x12, x13, [sp, #64]
                ; stp x14, x15, [sp, #80]
            );
            call_table(&mut self.ops, layout::RT_POLL, off)?;
            dynasm!(self.ops
                ; .arch aarch64
                ; ldp x14, x15, [sp, #80]
                ; ldp x12, x13, [sp, #64]
                ; ldp x10, x11, [sp, #48]
                ; ldp x8, x9, [sp, #32]
                ; ldp x6, x7, [sp, #16]
                ; ldp x4, x5, [sp], #96
                ; cbz x0, =>throw
                ; b =>cont
            );
        }
        for (l, from, to) in std::mem::take(&mut self.edges) {
            dynasm!(self.ops ; .arch aarch64 ; =>l);
            self.phi_moves(from, to)?;
            let t = self.labels[to];
            dynasm!(self.ops ; .arch aarch64 ; b =>t);
        }
        for (l, id, off, policy) in self.traps.clone() {
            let throw = self.throw_at(off);
            dynasm!(self.ops ; .arch aarch64 ; =>l);
            let here = self.ops.offset().0 as u32;
            self.trap_at.push((id, here));
            match policy {
                // Eager deopt to T0 at `off` plus an exit counter (H32, JIT mode). The deopt
                // entry is month 5; until then the stand-in trap.
                ExitPolicy::Deopt => {
                    mov64(
                        &mut self.ops,
                        3,
                        vm_layout::int32_bits(TRAP_BASE + id as i32),
                    );
                    call_table(&mut self.ops, layout::RT_THROW, off)?;
                    dynasm!(self.ops ; .arch aarch64 ; b =>throw);
                }
            }
        }
        let unwind = self.unwind;
        for (&off, &l) in &self.throws.clone() {
            dynasm!(self.ops ; .arch aarch64 ; =>l);
            mov64(&mut self.ops, 2, u64::from(off));
            dynasm!(self.ops ; .arch aarch64 ; b =>unwind);
        }
        let table = scaled(layout::CX_RUNTIME_TABLE)?;
        let entry = scaled(layout::RT_UNWIND)?;
        let out_pc = scaled(layout::CX_OUT_PC)?;
        let out_fp = scaled(layout::CX_OUT_FP)?;
        let leave = self.leave;
        dynasm!(self.ops
            ; .arch aarch64
            ; =>unwind
            ; mov x0, x28
            ; mov x1, x26
            ; ldr x16, [x28, table]
            ; ldr x16, [x16, entry]
            ; blr x16
            ; mov x3, x0
            ; ldr x1, [x28, out_pc]
            ; ldr x2, [x28, out_fp]
            ; b =>leave
        );
        Ok(())
    }
}

/// The CFI of the frame (code alignment 4, data alignment -8): `stp x29, x30, [sp, #-S]!` at
/// 0, `mov x29, sp` at 4, then one `stp` per pair of [`SAVED`]. The epilogue at `epilogue`
/// restores the pairs (one `ldp` each, reversed), then `ldp x29, x30`, then `br`; its rows sit
/// between remember/restore state, so the out-of-line code after the `br` has the body's rule.
fn cfi(frame: u32, epilogue: u32) -> CfiProgram {
    let mut p = CfiProgram::new();
    let words = u64::from(frame / 8);
    p.advance_loc(1)
        .def_cfa_offset(u64::from(frame))
        .offset(FP, words)
        .offset(LR, words - 1)
        .advance_loc(1)
        .def_cfa_register(FP);
    let pairs: Vec<&[(u8, u32)]> = SAVED.chunks(2).collect();
    for pair in &pairs {
        p.advance_loc(1);
        for &(r, o) in *pair {
            p.offset(u16::from(r), u64::from((frame - o) / 8));
        }
    }
    let prologue_insns = 2 + pairs.len() as u32;
    p.advance_loc(epilogue / 4 - prologue_insns)
        .remember_state();
    for pair in pairs.iter().rev() {
        p.advance_loc(1);
        for &(r, _) in *pair {
            p.restore(u16::from(r));
        }
    }
    p.advance_loc(1)
        .def_cfa(SP, 0)
        .restore(FP)
        .restore(LR)
        .advance_loc(1)
        .restore_state();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trap_text() {
        assert_eq!(trap_of("Uncaught 1900000003"), Some(3));
        assert_eq!(trap_of("uncaught: 1900000000\n    at am3"), Some(0));
        assert_eq!(trap_of("TypeError: x is 42"), None);
    }

    #[test]
    fn mov64_and_scaled() {
        let mut ops = Ops::new(0);
        mov64(&mut ops, 17, NUMBER_TAG);
        let code = ops.finalize().expect("code");
        // movz x17, #0xfffe, lsl #48
        assert_eq!(
            u32::from_le_bytes(code[..4].try_into().unwrap()),
            0xD2FF_FFD1
        );
        assert!(scaled(32768).is_err() && scaled(12).is_err() && scaled(8).is_ok());
    }

    /// The `eor` with the named NATIVE_WORD_OFFSET is the instruction the literal 2^49 gave
    /// before (MC-w13-14), so naming the constant changed no byte.
    #[test]
    fn native_word_eor_by_name_is_the_literal() {
        let w = native_word_offset().expect("one bit");
        let mut a = Ops::new(0);
        dynasm!(a ; .arch aarch64 ; eor x16, x16, w ; eor x1, x1, w);
        let mut b = Ops::new(0);
        dynasm!(b
            ; .arch aarch64
            ; eor x16, x16, #0x0002_0000_0000_0000_u64
            ; eor x1, x1, #0x0002_0000_0000_0000_u64
        );
        assert_eq!(w, 1 << 49);
        assert_eq!(a.finalize().expect("a"), b.finalize().expect("b"));
    }

    /// The largest frame that `emit` accepts encodes in both the prologue `stp` (pre-index,
    /// down to -512) and the epilogue `ldp` (post-index, up to +504); one more spill slot is
    /// a refusal, not a dynasm panic (M4-P1 review F3).
    #[test]
    fn the_largest_frame_encodes_and_one_more_slot_is_refused() {
        let max_slots = (MAX_FRAME - SPILL_BASE) / 8;
        assert_eq!(frame_bytes(max_slots).ok(), Some(MAX_FRAME));
        assert!(matches!(frame_bytes(max_slots + 1), Err(Refusal::Limit(_))));
        let (neg, pos) = (-(MAX_FRAME as i32), MAX_FRAME as i32);
        let mut ops = Ops::new(0);
        dynasm!(ops
            ; .arch aarch64
            ; stp x29, x30, [sp, neg]!
            ; ldp x29, x30, [sp], pos
        );
        assert_eq!(ops.finalize().expect("code").len(), 8);
    }
}
