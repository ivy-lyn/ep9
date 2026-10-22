//! Placement (b) of D21 (P4-PLACEMENT-B; M2-D21-FAIL after gate window 1): JIT frames live
//! on the VM register stack, and the JIT code is entered and left with the 4 T0 handler
//! arguments `(cx, pc, fp, acc)`, so no JIT frame stays on the native stack across a JS call.
//! It uses the frozen [`crate::emit`] traits and does not change [`crate::jit`] (placement
//! (a)); it reuses `jit::plan` (the 11 opcodes) and the [`crate::jit::InMemory`] finalizer.
//!
//! ```text
//! T0 call (call_fast, hot) --push_frame (return pc = next bytecode pc)--> stub byte OP_ENTER_B
//!   --handler enter_b--> tail jump to the slot entry (cx, fi.code, fp, acc)    [ep9_vm, spike-native]
//! (b) code: frame record while its body runs; runtime calls through Cx::runtime_table
//!   Return --> pop: tail jump to cx.table[*return pc] with (cx, return pc, caller fp, acc)
//!   table result 0 --> RuntimeTable::unwind --> tail jump to cx.table[*out_pc] (handler or Exit)
//! T0 call (slow::call cold arm), call_any, call_from_rust --> the same stub byte
//! Dispatch::Match --> the stub byte runs the callee's bytecode (D4)
//! ```
//!
//! | Item | Placement (a) ([`crate::jit`]) | Placement (b) (this module) |
//! |---|---|---|
//! | Slot | Baseline, `ep9_vm::install_native` | Opt (the spike's (b) marker, D3), `ep9_vm::install_native_b` |
//! | Entry from T0 | `slow::call` (cold), `extern "C"` `(cx, fp)` | `call_fast` (hot) + stub byte, handler ABI `(cx, pc, fp, acc)` |
//! | Exit | `ret` to the native caller | tail jump to `cx.table[*pc]` |
//! | Exception | return 0; the caller unwinds | `RuntimeTable::unwind`, then the tail jump |
//! | Native frame across a JS call | yes (the entry is a native call) | no |
//! | JIT -> callee | `call_any` (nested) | `call_any` (nested), the same |
//!
//! The emit.rs:20 rule "the entry has the `ep9_vm::NativeEntry` ABI" is placement (a)'s; the
//! (b) entry has the T0 `Handler` ABI (a report row; emit.rs is frozen). emit.rs:19 holds: no
//! absolute address, no relocation. AArch64 is built (the gate host); x86-64 (b) is not (a
//! stretch): [`BEmitter`] refuses it with [`Refusal::Arch`], and `structural-b` prints SKIP.

pub mod a64;
pub mod structural;

use crate::emit::{Arch, Emitter, FunctionCode, FunctionInput, Refusal};
use crate::jit::plan;

/// The placement (b) emitter for one architecture (AArch64 only; x86-64 refuses).
#[derive(Clone, Copy, Debug)]
pub struct BEmitter {
    arch: Arch,
}

impl BEmitter {
    pub fn new(arch: Arch) -> BEmitter {
        BEmitter { arch }
    }

    /// The emitter for this build's architecture.
    pub fn host() -> Option<BEmitter> {
        Arch::HOST.map(BEmitter::new)
    }

    /// True if this emitter makes code for `arch` (AArch64 only).
    pub fn builds(arch: Arch) -> bool {
        arch == Arch::Aarch64
    }
}

impl Emitter for BEmitter {
    fn arch(&self) -> Arch {
        self.arch
    }

    fn emit(&mut self, input: &FunctionInput<'_>) -> Result<FunctionCode, Refusal> {
        let plan = plan(input)?;
        let (code, unwind, native_offsets) = match self.arch {
            Arch::Aarch64 => a64::emit(&plan)?,
            Arch::X86_64 => return Err(Refusal::Arch),
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

/// The (b) form of the probe of gate check (g) (see `jit::hold_probe`): native code for
/// `function hold(mk, scrub, gc) { let o = mk(); scrub(); gc(); return o; }` that keeps `o`
/// only in the callee-saved x19 between the calls (XORed with `mask` when it is not 0), with
/// the (b) entry and exit.
pub fn hold_probe(
    arch: Arch,
    input: &FunctionInput<'_>,
    mask: u64,
) -> Result<FunctionCode, Refusal> {
    use ep9_bytecode::Opcode;
    if !BEmitter::builds(arch) {
        return Err(Refusal::Arch);
    }
    let mut offs = Vec::new();
    for insn in ep9_bytecode::Instructions::new(input.bytecode) {
        let insn = insn.map_err(|(offset, e)| Refusal::Decode {
            offset,
            detail: format!("{e:?}"),
        })?;
        if insn.opcode == Opcode::CallUndefinedReceiver0 {
            let i = offs.len() as u32;
            let callee = crate::jit::slot_bytes(insn.register(0).to_operand());
            let param = crate::jit::slot_bytes(ep9_bytecode::Register::parameter(i).to_operand());
            offs.push((insn.offset, callee, param));
        }
    }
    let offs: [(u32, i32, i32); 3] = offs.try_into().map_err(|v: Vec<(u32, i32, i32)>| {
        Refusal::Limit(format!("hold has {} calls, not 3", v.len()))
    })?;
    let (code, unwind) = a64::emit_hold(offs, mask)?;
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

    #[test]
    fn b_emits_drive_and_id_for_aarch64_and_refuses_x86_64() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(LOOP_DEFS, "defs.js").result.expect("defs");
        for name in ["drive", "id"] {
            let input = s.function_input(name).expect("input");
            let b = BEmitter::new(Arch::Aarch64).emit(&input).expect("a64");
            let a = crate::jit::SpikeEmitter::new(Arch::Aarch64)
                .emit(&input)
                .expect("a");
            assert_eq!(a.ops, b.ops, "{name}: the same plan");
            // stp x29, x30, [sp, #-32]! ; mov x29, sp ; stp x26, x28, [sp, #16] ;
            // mov x28, x0 ; mov x26, x2 (placement (a): mov x26, x1)
            let w: Vec<u32> = b.code[..20]
                .chunks(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            assert_eq!(
                w,
                [
                    0xA9BE_7BFD,
                    0x9100_03FD,
                    0xA901_73FA,
                    0xAA00_03FC,
                    0xAA02_03FA
                ],
                "{name}"
            );
            assert!(b.unwind.win64.is_none());
            assert_eq!(
                BEmitter::new(Arch::X86_64).emit(&input).unwrap_err(),
                Refusal::Arch
            );
        }
    }

    /// The (b) code has no ret and leaves by `br x16` only (tail jumps), and holds no address:
    /// the same function from two isolates (other bytecode, FunctionInfo and unit addresses)
    /// gives the same bytes.
    #[test]
    fn b_code_has_no_ret_and_no_address() {
        let code = |_: u32| {
            let mut s = Session::new(&Config::default()).expect("session");
            s.run(LOOP_DEFS, "defs.js").result.expect("defs");
            let input = s.function_input("drive").expect("input");
            BEmitter::new(Arch::Aarch64).emit(&input).expect("a64").code
        };
        let (one, two) = (code(1), code(2));
        assert_eq!(one, two);
        let w: Vec<u32> = one
            .chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert!(!w.contains(&0xD65F_03C0), "a ret in (b) code");
        assert!(w.contains(&0xD61F_0200), "no br x16");
    }
}
