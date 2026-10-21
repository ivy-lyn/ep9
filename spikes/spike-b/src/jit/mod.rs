//! Placement (a) of D21: JIT frames are canonical VM frames on the VM register stack, and the
//! JIT code runs on the native control stack (H14; the Spike B protocol). This module compiles
//! the 11 Spike B opcodes ([`OPCODES`]) of one function with dynasm 5.1.0 for AArch64
//! ([`a64`]) and x86-64 System V ([`x64`]), and places the code in the process code range
//! ([`JitModule`]).
//!
//! ```text
//! bytecode ──plan (reachable instructions, 11 opcodes, no prefix, no handler)──> Plan
//!   Plan ──a64::emit / x64::emit──> FunctionCode (prologue, body, epilogue, slow paths)
//!   [FunctionCode] ──InMemory::finalize──> JitModule: one CodeRange chunk, one unwind
//!                                          registration, then install_native per function
//! ```
//!
//! | Machine state | AArch64 | x86-64 | Rule |
//! |---|---|---|---|
//! | isolate register (`Cx`) | x28 | r15 | M2-JIT-ABI; callee-saved |
//! | VM frame pointer (`fp`, the frame of this function) | x26 | r13 | callee-saved |
//! | unit-data register | x27 (not used) | r14 (not used) | H35-Q9, SP-Unit |
//! | accumulator | x0 | rax | the result register of every table call |
//! | frame record | x29/x30 | rbp | frame pointers kept; CFI / UNWIND_INFO describe it |
//!
//! | Rule | Where |
//! |---|---|
//! | the prologue compares sp with `Cx::native_limit` once; at or below it, `RuntimeTable::stack_overflow` (a catchable RangeError) | `layout::CX_NATIVE_LIMIT` |
//! | `JumpLoop` takes one unit of `Cx::budget` and calls `RuntimeTable::poll` when the budget is ≤ 0 | `layout::CX_BUDGET`, `RT_POLL` |
//! | int32 fast paths inline (`Add`, `AddSmi`, `Inc`, `TestLessThan`); every other case calls the table function (`add` with the immediate as an int32 Value for `AddSmi`, `inc`, `compare`) | the result is T0's (`slow::binary` and friends are the T0 cold paths) |
//! | every table call passes (cx, its own fp, the bytecode offset of the instruction); the table function publishes fp and the offset (`Runtime::enter`); native code writes no VmStack field | M2-P1-NE-OFFSET |
//! | a call instruction runs through `RuntimeTable::call_any` (native callee: nested native entry; T0, host or bound callee: `call::call_with`) | JIT→JIT and JIT→T0 |
//! | a table result of 0 means an exception is pending: the function returns 0 at once (no handlers in JIT code) | the caller unwinds |
//! | no feedback recording | M2-P1-NE-NOFEEDBACK |

pub mod a64;
mod mem;
pub mod x64;

use std::collections::BTreeSet;

use ep9_bytecode::{Opcode, decode_at};

use crate::emit::{Arch, Emitter, FunctionCode, FunctionInput, Refusal};

pub use mem::{InMemory, JitModule, Placed};

/// The opcodes the spike JIT compiles (plan-v4 §5 P2-SPIKE-B task 1). A function with any
/// other reachable opcode stays in T0.
pub const OPCODES: [Opcode; 11] = [
    Opcode::LdaZero,
    Opcode::Ldar,
    Opcode::Star,
    Opcode::Add,
    Opcode::AddSmi,
    Opcode::Inc,
    Opcode::TestLessThan,
    Opcode::JumpIfFalse,
    Opcode::JumpLoop,
    Opcode::CallUndefinedReceiver1,
    Opcode::Return,
];

/// One instruction's work, with its operands decoded. Register operands are byte offsets from
/// the VM frame pointer (H14: `o ≥ 0` → `fp + 8o`, `o < 0` → `fp + 8(o - 8)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    LdaZero,
    Ldar(i32),
    Star(i32),
    Add(i32),
    AddSmi(i32),
    Inc,
    TestLessThan(i32),
    JumpIfFalse(u32),
    JumpLoop(u32),
    CallUndefinedReceiver1,
    Return,
}

/// A reachable instruction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Step {
    pub offset: u32,
    pub opcode: Opcode,
    pub op: Op,
    /// The offset after this instruction.
    pub next: u32,
}

impl Step {
    /// True if control can continue at `next`.
    pub fn falls_through(&self) -> bool {
        !matches!(self.op, Op::JumpLoop(_) | Op::Return)
    }
}

/// The reachable instructions of one function, in offset order.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
}

/// Byte offset from the VM frame pointer of register operand `o`.
pub(crate) fn slot_bytes(o: i32) -> i32 {
    if o >= 0 { o * 8 } else { (o - 8) * 8 }
}

/// Decodes the reachable instructions of `input` (from offset 0, along fall-through and jump
/// edges; handlers are refused). Refuses a reachable opcode outside [`OPCODES`] or a prefixed
/// instruction. Unreachable bytecode (for example the `LdaUndefined; Return` tail) is skipped.
pub(crate) fn plan(input: &FunctionInput<'_>) -> Result<Plan, Refusal> {
    if input.handler_count != 0 {
        return Err(Refusal::Handlers(input.handler_count));
    }
    let code = input.bytecode;
    let len = code.len() as u32;
    let mut seen = BTreeSet::new();
    let mut work = vec![0u32];
    let mut steps = Vec::new();
    while let Some(off) = work.pop() {
        if !seen.insert(off) {
            continue;
        }
        let insn = decode_at(code, off).map_err(|e| Refusal::Decode {
            offset: off,
            detail: format!("{e:?}"),
        })?;
        if insn.scale != ep9_bytecode::OperandScale::Single {
            return Err(Refusal::Prefix {
                offset: off,
                opcode: insn.opcode,
            });
        }
        let reg = |i: usize| slot_bytes(insn.register(i).to_operand());
        let target = || {
            insn.jump_target()
                .filter(|&t| t < len)
                .ok_or_else(|| Refusal::Decode {
                    offset: off,
                    detail: "jump target outside the bytecode".into(),
                })
        };
        let op = match insn.opcode {
            Opcode::LdaZero => Op::LdaZero,
            Opcode::Ldar => Op::Ldar(reg(0)),
            Opcode::Star => Op::Star(reg(0)),
            Opcode::Add => Op::Add(reg(0)),
            Opcode::AddSmi => Op::AddSmi(insn.signed(0)),
            Opcode::Inc => Op::Inc,
            Opcode::TestLessThan => Op::TestLessThan(reg(0)),
            Opcode::JumpIfFalse => Op::JumpIfFalse(target()?),
            Opcode::JumpLoop => Op::JumpLoop(target()?),
            Opcode::CallUndefinedReceiver1 => Op::CallUndefinedReceiver1,
            Opcode::Return => Op::Return,
            opcode => {
                return Err(Refusal::Opcode {
                    offset: off,
                    opcode,
                });
            }
        };
        let step = Step {
            offset: off,
            opcode: insn.opcode,
            op,
            next: insn.next_offset(),
        };
        match op {
            Op::JumpIfFalse(t) | Op::JumpLoop(t) => work.push(t),
            _ => {}
        }
        if step.falls_through() {
            if step.next >= len {
                return Err(Refusal::Decode {
                    offset: off,
                    detail: "control falls off the end".into(),
                });
            }
            work.push(step.next);
        }
        steps.push(step);
    }
    steps.sort_by_key(|s| s.offset);
    Ok(Plan { steps })
}

/// The Spike B emitter (placement (a)) for one architecture.
#[derive(Clone, Copy, Debug)]
pub struct SpikeEmitter {
    arch: Arch,
}

impl SpikeEmitter {
    /// An emitter for `arch`. Both generators build on every host, so code for the other
    /// architecture can be made (and its encodings tested); only host code can run.
    pub fn new(arch: Arch) -> SpikeEmitter {
        SpikeEmitter { arch }
    }

    /// The emitter for this build's architecture.
    pub fn host() -> Option<SpikeEmitter> {
        Arch::HOST.map(SpikeEmitter::new)
    }
}

impl Emitter for SpikeEmitter {
    fn arch(&self) -> Arch {
        self.arch
    }

    fn emit(&mut self, input: &FunctionInput<'_>) -> Result<FunctionCode, Refusal> {
        let plan = plan(input)?;
        let (code, unwind, native_offsets) = match self.arch {
            Arch::Aarch64 => a64::emit(&plan)?,
            Arch::X86_64 => x64::emit(&plan)?,
        };
        Ok(FunctionCode {
            name: input.name.to_string(),
            arch: self.arch,
            key: input.key,
            code,
            unwind,
            ops: plan.steps.iter().map(|s| (s.offset, s.opcode)).collect(),
            native_offsets,
        })
    }
}

/// The out-of-line part of an instruction: its slow path, entered from the fast path, which
/// continues at the next instruction's label.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Slow {
    /// `add(l, r)`: `l` from a frame slot (`None`: the accumulator is `l` and `r` the int32
    /// immediate of `AddSmi`).
    Add {
        lhs: Option<i32>,
        imm: i32,
    },
    Inc,
    /// `compare(LT, l, acc)`.
    Less {
        lhs: i32,
    },
    /// The JumpLoop poll; continues at the loop target.
    Poll,
}

/// The hand-written probe of gate check (g): native code for `function hold(mk, scrub, gc)
/// { let o = mk(); scrub(); gc(); return o; }` that keeps `o` only in a callee-saved register
/// (x19 / rbx) between the calls, XORed with `mask` when `mask` is not 0 (the negative
/// control: then it returns the masked bits, a double Value). It is not made from the
/// bytecode by the [`Emitter`]. It does what the bytecode's `Ldar a<i>; Star r<k>` moves do
/// before call `i` (the callee into the call's callee register; the object is never stored
/// in the frame), and `call_any` decodes the three call instructions.
pub fn hold_probe(
    arch: Arch,
    input: &FunctionInput<'_>,
    mask: u64,
) -> Result<FunctionCode, Refusal> {
    let mut offs = Vec::new();
    for insn in ep9_bytecode::Instructions::new(input.bytecode) {
        let insn = insn.map_err(|(offset, e)| Refusal::Decode {
            offset,
            detail: format!("{e:?}"),
        })?;
        if insn.opcode == Opcode::CallUndefinedReceiver0 {
            let i = offs.len() as u32;
            let callee = slot_bytes(insn.register(0).to_operand());
            let param = slot_bytes(ep9_bytecode::Register::parameter(i).to_operand());
            offs.push((insn.offset, callee, param));
        }
    }
    let offs: [(u32, i32, i32); 3] = offs.try_into().map_err(|v: Vec<(u32, i32, i32)>| {
        Refusal::Limit(format!("hold has {} calls, not 3", v.len()))
    })?;
    let (code, unwind) = match arch {
        Arch::Aarch64 => a64::emit_hold(offs, mask)?,
        Arch::X86_64 => x64::emit_hold(offs, mask)?,
    };
    Ok(FunctionCode {
        name: input.name.to_string(),
        arch,
        key: input.key,
        code,
        unwind,
        ops: offs
            .iter()
            .map(|&(o, ..)| (o, Opcode::CallUndefinedReceiver0))
            .collect(),
        native_offsets: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{Config, Session};
    use crate::structural::LOOP_DEFS;

    /// The emitter input of `name` after running `defs`, kept with its session.
    fn with_input<R>(defs: &str, name: &str, f: impl FnOnce(&FunctionInput<'_>) -> R) -> R {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(defs, "defs.js").result.expect("defs run");
        let input = s.function_input(name).expect("input");
        f(&input)
    }

    #[test]
    fn drive_plans_its_reachable_instructions_only() {
        with_input(LOOP_DEFS, "drive", |input| {
            let p = plan(input).expect("drive JITs");
            let ops: Vec<Opcode> = p.steps.iter().map(|s| s.opcode).collect();
            assert!(ops.contains(&Opcode::AddSmi), "{ops:?}");
            assert!(ops.contains(&Opcode::JumpLoop), "{ops:?}");
            // The `LdaUndefined; Return` tail after the loop's return is unreachable.
            assert!(!ops.contains(&Opcode::LdaUndefined), "{ops:?}");
            assert!(ops.iter().all(|o| OPCODES.contains(o)), "{ops:?}");
            assert!(p.steps.windows(2).all(|w| w[0].offset < w[1].offset));
        });
    }

    #[test]
    fn both_generators_emit_drive_and_id() {
        for name in ["drive", "id"] {
            with_input(LOOP_DEFS, name, |input| {
                let a = SpikeEmitter::new(Arch::Aarch64).emit(input).expect("a64");
                let x = SpikeEmitter::new(Arch::X86_64).emit(input).expect("x64");
                assert_eq!(a.ops, x.ops);
                assert_eq!(a.code.len() % 4, 0);
                // a64: stp x29, x30, [sp, #-32]!   x64: push rbp
                assert_eq!(a.code[..4], 0xa9be_7bfd_u32.to_le_bytes(), "{name}");
                assert_eq!(x.code[0], 0x55, "{name}");
                assert!(x.unwind.win64.is_some(), "{name}: no UNWIND_INFO");
            });
        }
    }

    #[test]
    fn other_opcodes_handlers_and_prefixes_stay_in_t0() {
        let defs = "function mul(x) { return x * 2; }\n\
                    function guard(f) { try { return f(); } catch (e) { return 0; } }\n\
                    function wide(x) { return x + 1000; }\n";
        with_input(defs, "mul", |input| {
            assert!(
                matches!(plan(input), Err(Refusal::Opcode { .. })),
                "{:?}",
                plan(input)
            );
        });
        with_input(defs, "guard", |input| {
            assert!(
                matches!(plan(input), Err(Refusal::Handlers(_))),
                "{:?}",
                plan(input)
            );
        });
        with_input(defs, "wide", |input| {
            assert!(
                matches!(plan(input), Err(Refusal::Prefix { .. })),
                "{:?}",
                plan(input)
            );
        });
    }

    #[test]
    fn register_operands_map_to_frame_slots() {
        // H14: r0 at fp, the receiver (operand -1) at fp - 72, a0 (operand -2) at fp - 80.
        assert_eq!(slot_bytes(0), 0);
        assert_eq!(slot_bytes(3), 24);
        assert_eq!(slot_bytes(-1), -72);
        assert_eq!(slot_bytes(-2), -80);
    }
}
