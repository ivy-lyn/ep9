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
//! absolute address, no relocation. Both architectures are built: AArch64 ([`a64`], the gate
//! host; the Rust ABI, x0-x3) and x86-64 ([`x64`], M3-P0 P0-B-X64, plan-v7 D25; `extern
//! "sysv64"` on every OS, rdi, rsi, rdx, rcx). Every exit is a jump: `br x16` on AArch64,
//! `rex.w jmp qword ptr [rdi + rax*8]` on x86-64 (the form the Win64 unwinder takes as the end
//! of an epilogue).

pub mod a64;
pub mod structural;
pub mod x64;

use crate::emit::{Arch, Emitter, FunctionCode, FunctionInput, Refusal};
use crate::jit::plan;

/// The placement (b) emitter for one architecture (AArch64 or x86-64).
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

    /// True if this emitter makes code for `arch` (every [`Arch`]: AArch64 and x86-64).
    pub fn builds(arch: Arch) -> bool {
        matches!(arch, Arch::Aarch64 | Arch::X86_64)
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
            Arch::X86_64 => {
                let e = x64::emit(&plan)?;
                (e.code, e.unwind, e.native_offsets)
            }
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
/// only in a callee-saved register between the calls (x19 on AArch64, rbx on x86-64; XORed
/// with `mask` when it is not 0), with the (b) entry and exit.
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
    let (code, unwind) = match arch {
        Arch::Aarch64 => a64::emit_hold(offs, mask)?,
        Arch::X86_64 => {
            let e = x64::emit_hold(offs, mask)?;
            (e.code, e.unwind)
        }
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
    use crate::harness::{Config, InstallKind, Session};
    use crate::structural::LOOP_DEFS;

    /// The function of gate check (g) (jit_b/structural.rs `HOLD_DEFS`).
    const HOLD_DEFS: &str =
        "function hold(mk, scrub, gc) { let o = mk(); scrub(); gc(); return o; }\n";

    /// The documented x86-64 prologue: `push rbp ; push r15 ; push r13 ; lea rbp, [rsp + 16] ;
    /// mov r15, rdi ; mov r13, rdx` (placement (a): `mov r13, rsi`), as `llvm-mc
    /// -triple=x86_64` encodes it.
    const X64_PROLOGUE: [u8; 16] = [
        0x55, 0x41, 0x57, 0x41, 0x55, 0x48, 0x8D, 0x6C, 0x24, 0x10, 0x49, 0x89, 0xFF, 0x49, 0x89,
        0xD5,
    ];

    /// `drive` and `id` compile for both architectures, with placement (a)'s plan and the
    /// documented prologue. The x86-64 code has the Win64 UNWIND_INFO of (a)'s frame.
    #[test]
    fn b_emits_drive_and_id_for_aarch64_and_x86_64() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(LOOP_DEFS, "defs.js").result.expect("defs");
        for name in ["drive", "id"] {
            let input = s.function_input(name).expect("input");
            for arch in [Arch::Aarch64, Arch::X86_64] {
                assert!(BEmitter::builds(arch));
                let b = BEmitter::new(arch).emit(&input).expect("b");
                let a = crate::jit::SpikeEmitter::new(arch).emit(&input).expect("a");
                assert_eq!(a.ops, b.ops, "{name} {arch:?}: the same plan");
                assert_eq!(b.native_offsets.len(), b.ops.len(), "{name} {arch:?}");
                match arch {
                    Arch::Aarch64 => {
                        // stp x29, x30, [sp, #-32]! ; mov x29, sp ; stp x26, x28, [sp, #16] ;
                        // mov x28, x0 ; mov x26, x2 (placement (a): mov x26, x1) ; then
                        // mov x0, x3 (the incoming accumulator into the body's x0)
                        let w: Vec<u32> = b.code[..24]
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
                                0xAA02_03FA,
                                0xAA03_03E0
                            ],
                            "{name}"
                        );
                        assert!(b.unwind.win64.is_none());
                    }
                    Arch::X86_64 => {
                        assert_eq!(b.code[..16], X64_PROLOGUE, "{name}");
                        // mov rax, rcx: the incoming accumulator into the body's rax, after
                        // the prologue (the UNWIND_INFO prologue still ends at 10).
                        assert_eq!(b.code[16..19], [0x48, 0x89, 0xC8], "{name}: mov rax, rcx");
                        assert!(b.unwind.win64.is_some(), "{name}: no UNWIND_INFO");
                        assert_eq!(b.unwind.win64, a.unwind.win64, "{name}: (a)'s frame codes");
                    }
                }
            }
        }
    }

    /// x86-64: every exit ends with the exit jump `48 FF 24 C7` (`rex.w jmp qword ptr [rdi +
    /// rax*8]`), and no instruction the emitter reports (each step's first instruction, the
    /// prologue, every exit, throw, unwind, overflow and slow-path instruction it marks) is a
    /// `ret` (0xC3). The epilogue is `lea rsp, [rbp - 16] ; pop r13 ; pop r15 ; pop rbp`
    /// right before the jump, and the jump is the only exit. The same holds for the (g) probe.
    #[test]
    fn x86_64_b_code_leaves_by_the_exit_jump_only() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(LOOP_DEFS, "defs.js").result.expect("defs");
        s.run(HOLD_DEFS, "hold.js").result.expect("hold");
        let mut all = Vec::new();
        for name in ["drive", "id"] {
            let input = s.function_input(name).expect("input");
            let e = x64::emit(&plan(&input).expect("plan")).expect("x64");
            let b = BEmitter::new(Arch::X86_64).emit(&input).expect("b");
            assert_eq!((&e.code, &e.native_offsets), (&b.code, &b.native_offsets));
            all.push((name, e, 3));
        }
        let input = s.function_input("hold").expect("hold");
        let probe = hold_probe(Arch::X86_64, &input, 0).expect("probe");
        // The probe's call offsets, as `hold_probe` finds them.
        let calls: Vec<(u32, i32, i32)> = ep9_bytecode::Instructions::new(input.bytecode)
            .map(|i| i.expect("decode"))
            .filter(|i| i.opcode == ep9_bytecode::Opcode::CallUndefinedReceiver0)
            .enumerate()
            .map(|(k, i)| {
                (
                    i.offset,
                    crate::jit::slot_bytes(i.register(0).to_operand()),
                    crate::jit::slot_bytes(
                        ep9_bytecode::Register::parameter(k as u32).to_operand(),
                    ),
                )
            })
            .collect();
        let e = x64::emit_hold(calls.try_into().expect("3 calls"), 0).expect("hold");
        assert_eq!(e.code, probe.code);
        all.push(("hold", e, 5));
        for (name, e, pushes) in all {
            let f = &e.frame;
            assert_eq!(f.prologue.len(), pushes + 1, "{name}");
            assert_eq!(f.epilogue.len(), pushes + 2, "{name}");
            let jmp = *f.epilogue.last().unwrap() as usize;
            assert_eq!(e.code[jmp..jmp + 4], x64::EXIT_JMP, "{name}");
            let lea: &[u8] = if pushes == 3 {
                &[0x48, 0x8D, 0x65, 0xF0]
            } else {
                &[0x48, 0x8D, 0x65, 0xE0]
            };
            let ep = f.epilogue[0] as usize;
            assert_eq!(&e.code[ep..ep + 4], lea, "{name}: lea rsp, [rbp - d]");
            // pop r12 ; pop rbx (hold) ; pop r13 ; pop r15 ; pop rbp
            let pops: &[u8] = if pushes == 3 {
                &[0x41, 0x5D, 0x41, 0x5F, 0x5D]
            } else {
                &[0x41, 0x5C, 0x5B, 0x41, 0x5D, 0x41, 0x5F, 0x5D]
            };
            assert_eq!(&e.code[ep + 4..jmp], pops, "{name}: the pops");
            let starts = [0]
                .into_iter()
                .chain(f.prologue.iter().copied())
                .chain(e.native_offsets.iter().copied())
                .chain(f.exits.iter().copied());
            let mut n = 0;
            for at in starts {
                assert_ne!(e.code[at as usize], 0xC3, "{name}: ret at {at}");
                n += 1;
            }
            assert!(n > 20, "{name}: {n} instruction starts");
            // One exit jump: no other `FF 24 C7` among the reported starts.
            let jumps = f
                .exits
                .iter()
                .filter(|&&at| e.code[at as usize..].starts_with(&x64::EXIT_JMP))
                .count();
            assert_eq!(jumps, 1, "{name}");
        }
    }

    /// The (b) code has no ret and leaves by tail jumps only (`br x16`; x86-64: the exit jump),
    /// and holds no address: the same function from two isolates (other bytecode, FunctionInfo
    /// and unit addresses) gives the same bytes, on both architectures.
    #[test]
    fn b_code_has_no_ret_and_no_address() {
        for arch in [Arch::Aarch64, Arch::X86_64] {
            let code = |_: u32| {
                let mut s = Session::new(&Config::default()).expect("session");
                s.run(LOOP_DEFS, "defs.js").result.expect("defs");
                let input = s.function_input("drive").expect("input");
                BEmitter::new(arch).emit(&input).expect("b").code
            };
            let (one, two) = (code(1), code(2));
            assert_eq!(one, two, "{arch:?}");
            if arch == Arch::Aarch64 {
                let w: Vec<u32> = one
                    .chunks(4)
                    .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                assert!(!w.contains(&0xD65F_03C0), "a ret in (b) code");
                assert!(w.contains(&0xD61F_0200), "no br x16");
            } else {
                assert!(
                    one.windows(4).any(|w| w == x64::EXIT_JMP),
                    "no exit jump in x86-64 (b) code"
                );
            }
        }
    }

    /// The (b) entry keeps the incoming accumulator (`acc`: x3 on AArch64, rcx on x86-64) as
    /// the body's accumulator (x0, rax), as a T0 handler does. `id` cut down to its `Return`
    /// reads the accumulator before any instruction writes it; entered from Rust (`run_b`:
    /// acc = undefined) under the threaded dispatch, its (b) code must return undefined, not
    /// whatever the entry left in x0 (the cx pointer) or rax.
    #[test]
    fn b_return_only_returns_the_incoming_acc() {
        let mut s = Session::new(&Config::default().threaded()).expect("session");
        s.run(LOOP_DEFS, "defs.js").result.expect("defs");
        let id = ep9_vm::NativeCx::global_value(&s.iso, "id").expect("id");
        let seven = ep9_vm::Value::int32(7);
        let t0 = ep9_vm::call_from_rust(&mut s.iso, id, ep9_vm::Value::UNDEFINED, &[seven])
            .expect("T0 id");
        assert_eq!(t0.to_bits(), seven.to_bits(), "T0 id(7)");
        let input = s.function_input("id").expect("input");
        let ret = ep9_bytecode::Instructions::new(input.bytecode)
            .map(|i| i.expect("decode"))
            .find(|i| i.opcode == ep9_bytecode::Opcode::Return)
            .expect("a Return");
        let cut = FunctionInput {
            bytecode: &input.bytecode[ret.offset as usize..],
            ..input
        };
        let p = plan(&cut).expect("plan");
        assert_eq!(p.steps.len(), 1, "only the Return");
        let arch = Arch::HOST.expect("a host arch");
        let code = BEmitter::new(arch).emit(&cut).expect("b");
        s.install_as(InstallKind::B, &["id"], vec![code])
            .expect("install");
        let r = ep9_vm::call_from_rust(&mut s.iso, id, ep9_vm::Value::UNDEFINED, &[seven])
            .expect("(b) id");
        assert_eq!(
            r.to_bits(),
            ep9_vm::Value::UNDEFINED.to_bits(),
            "{arch:?}: the (b) Return gave {:#x}, not the incoming undefined",
            r.to_bits()
        );
    }
}
