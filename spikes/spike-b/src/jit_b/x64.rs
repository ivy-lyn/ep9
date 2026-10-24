//! The x86-64 code generator of placement (b) (M3-P0 P0-B-X64, plan-v7 D25): the 11 Spike B
//! opcodes, entered and left with the T0 handler ABI. On x86-64 that ABI is `extern "sysv64"`
//! on every OS, Windows too (`interp::Handler`, `dsl::handler!`): `(cx, pc, fp, acc)` in rdi,
//! rsi, rdx, rcx. One generator serves Linux, macOS and Windows; only the unwind form differs
//! (CFI for unix, UNWIND_INFO for Windows, both made here). The frame is placement (a)'s
//! (`jit::x64`), with fp from rdx, not rsi.
//!
//! ```text
//!  0  push rbp                  CFA = rsp + 16; rbp at CFA-16        UWOP_PUSH_NONVOL rbp
//!  1  push r15                  CFA = rsp + 24; r15 at CFA-24        UWOP_PUSH_NONVOL r15
//!  3  push r13                  CFA = rsp + 32; r13 at CFA-32        UWOP_PUSH_NONVOL r13
//!  5  lea rbp, [rsp + 16]       CFA = rbp + 16 (rbp -> saved rbp)    UWOP_SET_FPREG rbp, 16
//! 10  mov r15, rdi ; mov r13, rdx          isolate register (cx), VM frame pointer (fp)
//! 16  mov rax, rcx ; cmp rsp, [r15 + native_limit] ; jbe overflow     rax = acc (rcx)
//!     body: one label per reachable instruction, accumulator in rax (as placement (a))
//! ret:   mov rcx, rax ; mov rsi, [r13 - 56] ; mov rdx, [r13 - 64]   return pc, caller fp
//!        btc rsi, 49 ; btc rdx, 49                                   decode the native words
//! leave: mov rdi, r15 ; movzx eax, byte [rsi]                        table[*pc] (Cx offset 0)
//!        lea rsp, [rbp - 16] ; pop r13 ; pop r15 ; pop rbp
//!        rex.w jmp qword ptr [rdi + rax*8]                           (cx, pc, fp, acc)
//!     overflow: stack_overflow(cx, fp, 0) ; then as a throw at offset 0
//!     slow paths: table call ; test rax, rax ; jz throw<off> ; jmp <next instruction>
//!     throw<off>: mov edx, off ; jmp unwind
//!     unwind: RuntimeTable::unwind(cx, fp, off) ; rcx = rax ; rsi = cx.out_pc
//!             ; rdx = cx.out_fp ; jmp leave
//! ```
//!
//! The entry rsp is 8 mod 16 (a handler is entered by a call or a tail jump), so after the 3
//! pushes every table call is 16-byte aligned. The native frame record lives only while the
//! body runs: every exit restores r13, r15, rbp and rsp to their entry values and leaves by a
//! jump, so no (b) frame stays on the native stack across a JS call that T0 makes. There is no
//! `ret` and no `push target; ret`: the return address on the stack stays the one a real
//! `call` pushed, so a shadow stack (CET) stays valid. A call instruction runs through
//! `RuntimeTable::call_any` (one nested native level, as in placement (a)).
//!
//! The exit jump is `48 FF 24 C7`: `jmp qword ptr [rdi + rax*8]` (ModRM mod 00, SIB scale 8)
//! with a REX.W prefix that changes nothing on x86-64. The Win64 unwinder takes an indirect
//! `jmp` as the end of an epilogue only with REX.W (or the `FF 25` form); LLVM emits
//! `rex64 jmp` for every Win64 indirect tail call for this reason, and no REX on Linux. One
//! generator for both OSes, so the prefix is always there.
//!
//! Scratch registers: rax, rcx, rdx, rsi, rdi, r8-r11 (r11: the int32 tag `0xFFFE << 48`). r14
//! (the unit-data register, H35-Q9) is never touched. No absolute address and no relocation:
//! branches are rip-relative inside the function; the dispatch table is at `Cx` offset 0, the
//! runtime functions are behind `Cx::runtime_table`.

use std::collections::BTreeMap;

use dynasmrt::x64::X64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_platform::unwind::win64::{UnwindInfoBuilder, reg as wreg};
use ep9_value::encoding::{NUMBER_TAG, VALUE_FALSE};
use ep9_vm::{cmp, layout};

use crate::emit::{Refusal, Unwind};
use crate::jit::{Op, Plan, Slow};

type Ops = VecAssembler<X64Relocation>;

/// The exit jump: `rex.w jmp qword ptr [rdi + rax*8]`.
pub const EXIT_JMP: [u8; 4] = [0x48, 0xFF, 0x24, 0xC7];

/// DWARF register numbers (System V psABI figure 3.36).
const RBX: u16 = 3;
const RBP: u16 = 6;
const RSP: u16 = 7;
const R12: u16 = 12;
const R13: u16 = 13;
const R15: u16 = 15;

/// Where the emitter put the frame code (byte offsets in the function), for the unwind data
/// and the tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameOffsets {
    /// The offset after each push of the prologue, then after its `lea rbp`.
    pub prologue: Vec<u32>,
    /// The offsets of the epilogue: its `lea rsp`, each `pop`, then the exit jump.
    pub epilogue: Vec<u32>,
    /// The start of every instruction of the exit code: `ret` (the `Return` path), `leave`,
    /// the epilogue, the throw stubs, the unwind call, the overflow call and the slow paths.
    pub exits: Vec<u32>,
}

/// What [`emit`] and [`emit_hold`] make.
#[derive(Clone, Debug)]
pub(crate) struct Emitted {
    pub code: Vec<u8>,
    pub unwind: Unwind,
    /// Native offset of the first machine instruction of each plan step.
    pub native_offsets: Vec<u32>,
    /// Read by the tests (`jit_b::tests`); `BEmitter` keeps only the frozen `FunctionCode`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub frame: FrameOffsets,
}

fn disp(off: usize) -> Result<i32, Refusal> {
    i32::try_from(off).map_err(|_| Refusal::Limit(format!("Cx or table offset {off}")))
}

fn here(ops: &Ops) -> u32 {
    ops.offset().0 as u32
}

/// Calls the table function at `entry` with rdi = cx, rsi = fp, edx = `off` (the value
/// operands are already in rcx, r8, r9). Result in rax.
fn call_table(ops: &mut Ops, entry: usize, off: u32) -> Result<(), Refusal> {
    let table = disp(layout::CX_RUNTIME_TABLE)?;
    let entry = disp(entry)?;
    let off = off as i32;
    dynasm!(ops
        ; .arch x64
        ; mov rdi, r15
        ; mov rsi, r13
        ; mov edx, DWORD off
        ; mov rax, QWORD [r15 + table]
        ; call QWORD [rax + entry]
    );
    Ok(())
}

/// The prologue of `frame`: its pushes, `lea rbp, [rsp + d]` (rbp then points at the saved
/// rbp), then r15 = cx (rdi) and r13 = fp (rdx). Returns the offset after each push and after
/// the `lea`.
fn prologue(ops: &mut Ops, frame: &[u8]) -> Vec<u32> {
    let mut at = Vec::with_capacity(frame.len() + 1);
    dynasm!(ops ; .arch x64 ; push rbp);
    at.push(here(ops));
    dynasm!(ops ; .arch x64 ; push r15);
    at.push(here(ops));
    dynasm!(ops ; .arch x64 ; push r13);
    at.push(here(ops));
    if frame == HOLD_FRAME {
        dynasm!(ops ; .arch x64 ; push rbx);
        at.push(here(ops));
        dynasm!(ops ; .arch x64 ; push r12);
        at.push(here(ops));
        dynasm!(ops ; .arch x64 ; lea rbp, [rsp + 32]);
    } else {
        dynasm!(ops ; .arch x64 ; lea rbp, [rsp + 16]);
    }
    at.push(here(ops));
    dynasm!(ops ; .arch x64 ; mov r15, rdi ; mov r13, rdx);
    at
}

/// The exits every (b) function shares: the throw stubs (one per bytecode offset that can
/// throw) and the unwind call. `leave` is the common exit.
struct Exits {
    throws: BTreeMap<u32, DynamicLabel>,
    unwind: DynamicLabel,
    leave: DynamicLabel,
    /// [`FrameOffsets::exits`].
    starts: Vec<u32>,
}

impl Exits {
    fn new(ops: &mut Ops) -> Exits {
        Exits {
            throws: BTreeMap::new(),
            unwind: ops.new_dynamic_label(),
            leave: ops.new_dynamic_label(),
            starts: Vec::new(),
        }
    }

    /// The label of "an exception is pending at bytecode offset `off`".
    fn throw(&mut self, ops: &mut Ops, off: u32) -> DynamicLabel {
        *self
            .throws
            .entry(off)
            .or_insert_with(|| ops.new_dynamic_label())
    }

    fn mark(&mut self, ops: &Ops) {
        self.starts.push(here(ops));
    }

    /// `ret`: rcx = the accumulator (rax), rsi = the frame's return pc, rdx = its caller fp;
    /// then `leave`: rdi = cx, eax = *rsi; then the epilogue that restores the frame and jumps
    /// to `cx.table[*rsi]`. `frame` is [`FRAME`] or [`HOLD_FRAME`]. Returns the epilogue
    /// offsets ([`FrameOffsets::epilogue`]).
    fn emit_ret_and_leave(&mut self, ops: &mut Ops, frame: &[u8]) -> Result<Vec<u32>, Refusal> {
        let ret_pc = i32::try_from(layout::FP_RETURN_PC).map_err(|_| Refusal::Arch)?;
        let caller = i32::try_from(layout::FP_CALLER_FP).map_err(|_| Refusal::Arch)?;
        if layout::NATIVE_WORD_OFFSET != 1 << 49 {
            return Err(Refusal::Limit(format!(
                "native word offset {:#x}",
                layout::NATIVE_WORD_OFFSET
            )));
        }
        let table = layout::CX_TABLE;
        if table != 0 {
            return Err(Refusal::Limit(format!("Cx::table at {table}, not 0")));
        }
        for step in 0..5 {
            self.mark(ops);
            match step {
                0 => dynasm!(ops ; .arch x64 ; mov rcx, rax),
                1 => dynasm!(ops ; .arch x64 ; mov rsi, QWORD [r13 + ret_pc]),
                2 => dynasm!(ops ; .arch x64 ; mov rdx, QWORD [r13 + caller]),
                3 => dynasm!(ops ; .arch x64 ; btc rsi, 49),
                _ => dynasm!(ops ; .arch x64 ; btc rdx, 49),
            }
        }
        let leave = self.leave;
        dynasm!(ops ; .arch x64 ; =>leave);
        self.mark(ops);
        dynasm!(ops ; .arch x64 ; mov rdi, r15);
        self.mark(ops);
        dynasm!(ops ; .arch x64 ; movzx eax, BYTE [rsi]);
        let mut ep = vec![here(ops)];
        if frame == HOLD_FRAME {
            dynasm!(ops ; .arch x64 ; lea rsp, [rbp - 32]);
            ep.push(here(ops));
            dynasm!(ops ; .arch x64 ; pop r12);
            ep.push(here(ops));
            dynasm!(ops ; .arch x64 ; pop rbx);
            ep.push(here(ops));
        } else {
            dynasm!(ops ; .arch x64 ; lea rsp, [rbp - 16]);
            ep.push(here(ops));
        }
        dynasm!(ops ; .arch x64 ; pop r13);
        ep.push(here(ops));
        dynasm!(ops ; .arch x64 ; pop r15);
        ep.push(here(ops));
        dynasm!(ops ; .arch x64 ; pop rbp);
        ep.push(here(ops));
        dynasm!(ops ; .arch x64 ; .bytes EXIT_JMP.iter());
        self.starts.extend_from_slice(&ep);
        Ok(ep)
    }

    /// The out-of-line throw stubs and the unwind call (after the epilogue).
    fn emit_unwind(&mut self, ops: &mut Ops) -> Result<(), Refusal> {
        let unwind = self.unwind;
        let throws: Vec<(u32, DynamicLabel)> = self.throws.iter().map(|(&o, &l)| (o, l)).collect();
        for (off, l) in throws {
            dynasm!(ops ; .arch x64 ; =>l);
            self.mark(ops);
            dynasm!(ops ; .arch x64 ; mov edx, DWORD off as i32);
            self.mark(ops);
            dynasm!(ops ; .arch x64 ; jmp =>unwind);
        }
        let table = disp(layout::CX_RUNTIME_TABLE)?;
        let entry = disp(layout::RT_UNWIND)?;
        let out_pc = disp(layout::CX_OUT_PC)?;
        let out_fp = disp(layout::CX_OUT_FP)?;
        let leave = self.leave;
        dynasm!(ops ; .arch x64 ; =>unwind);
        self.mark(ops);
        dynasm!(ops
            ; .arch x64
            ; mov rdi, r15
            ; mov rsi, r13
            ; mov rax, QWORD [r15 + table]
            ; call QWORD [rax + entry]
            ; mov rcx, rax
            ; mov rsi, QWORD [r15 + out_pc]
            ; mov rdx, QWORD [r15 + out_fp]
            ; jmp =>leave
        );
        Ok(())
    }
}

/// The frame of `drive`, `id` and every other (b) function: rbp, r15, r13 (x86 register
/// numbers, the Win64 UNWIND_INFO numbers too; push order).
const FRAME: [u8; 3] = [5, 15, 13];
/// The frame of the check-(g) probe: rbp, r15, r13, rbx, r12 (rbx holds the object; r12 keeps
/// rsp 16-byte aligned at the calls).
const HOLD_FRAME: [u8; 5] = [5, 15, 13, 3, 12];

/// Compiles `plan`: the code, its unwind data, the native offset of each step and the frame
/// offsets.
pub(crate) fn emit(plan: &Plan) -> Result<Emitted, Refusal> {
    let mut ops = Ops::new(0);
    let labels: BTreeMap<u32, DynamicLabel> = plan
        .steps
        .iter()
        .map(|s| (s.offset, ops.new_dynamic_label()))
        .collect();
    let ret = ops.new_dynamic_label();
    let overflow = ops.new_dynamic_label();
    let mut exits = Exits::new(&mut ops);
    let native_limit = disp(layout::CX_NATIVE_LIMIT)?;
    let budget = disp(layout::CX_BUDGET)?;
    let label = |t: u32| labels[&t];
    let tag = NUMBER_TAG as i64;
    let f = VALUE_FALSE as i32;

    // ---- prologue ----
    let at = prologue(&mut ops, &FRAME);
    // The incoming accumulator (rcx) is the body's accumulator (rax): a `Return` or an `Inc`
    // can read it before any instruction writes it. After the prologue, so the UNWIND_INFO
    // prologue still ends at the `lea rbp` (offset 10).
    dynasm!(ops
        ; .arch x64
        ; mov rax, rcx
        ; cmp rsp, QWORD [r15 + native_limit]
        ; jbe =>overflow
    );

    // ---- body ----
    let mut slows: Vec<(DynamicLabel, Slow, u32, DynamicLabel)> = Vec::new();
    let mut native_offsets = Vec::with_capacity(plan.steps.len());
    for (i, s) in plan.steps.iter().enumerate() {
        dynasm!(ops ; .arch x64 ; =>label(s.offset));
        native_offsets.push(here(&ops));
        let next_emitted = plan.steps.get(i + 1).map(|n| n.offset);
        match s.op {
            Op::LdaZero => {
                dynasm!(ops ; .arch x64 ; mov rax, QWORD tag);
            }
            Op::Ldar(o) => {
                dynasm!(ops ; .arch x64 ; mov rax, QWORD [r13 + o]);
            }
            Op::Star(o) => {
                dynasm!(ops ; .arch x64 ; mov QWORD [r13 + o], rax);
            }
            Op::Add(o) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov rcx, QWORD [r13 + o]
                    ; mov r11, QWORD tag
                    ; mov r10, rcx
                    ; and r10, rax
                    ; cmp r10, r11
                    ; jb =>slow
                    ; mov r10d, ecx
                    ; add r10d, eax
                    ; jo =>slow
                    ; or r10, r11
                    ; mov rax, r10
                );
                slows.push((
                    slow,
                    Slow::Add {
                        lhs: Some(o),
                        imm: 0,
                    },
                    s.offset,
                    label(s.next),
                ));
            }
            Op::AddSmi(imm) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov r11, QWORD tag
                    ; cmp rax, r11
                    ; jb =>slow
                    ; mov r10d, eax
                    ; add r10d, DWORD imm
                    ; jo =>slow
                    ; or r10, r11
                    ; mov rax, r10
                );
                slows.push((slow, Slow::Add { lhs: None, imm }, s.offset, label(s.next)));
            }
            Op::Inc => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov r11, QWORD tag
                    ; cmp rax, r11
                    ; jb =>slow
                    ; mov r10d, eax
                    ; add r10d, 1
                    ; jo =>slow
                    ; or r10, r11
                    ; mov rax, r10
                );
                slows.push((slow, Slow::Inc, s.offset, label(s.next)));
            }
            Op::TestLessThan(o) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov rcx, QWORD [r13 + o]
                    ; mov r11, QWORD tag
                    ; mov r10, rcx
                    ; and r10, rax
                    ; cmp r10, r11
                    ; jb =>slow
                    ; cmp ecx, eax
                    ; setl al
                    ; movzx eax, al
                    ; add eax, f
                );
                slows.push((slow, Slow::Less { lhs: o }, s.offset, label(s.next)));
            }
            Op::JumpIfFalse(t) => {
                dynasm!(ops
                    ; .arch x64
                    ; cmp rax, f
                    ; je =>label(t)
                );
            }
            Op::JumpLoop(t) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; sub QWORD [r15 + budget], 1
                    ; jle =>slow
                    ; jmp =>label(t)
                );
                slows.push((slow, Slow::Poll, s.offset, label(t)));
            }
            Op::CallUndefinedReceiver1 => {
                let throw = exits.throw(&mut ops, s.offset);
                dynasm!(ops ; .arch x64 ; mov rcx, rax);
                call_table(&mut ops, layout::RT_CALL_ANY, s.offset)?;
                dynasm!(ops
                    ; .arch x64
                    ; test rax, rax
                    ; jz =>throw
                );
            }
            Op::Return => {
                if next_emitted.is_some() {
                    dynasm!(ops ; .arch x64 ; jmp =>ret);
                }
            }
        }
        if s.falls_through() && next_emitted != Some(s.next) {
            dynasm!(ops ; .arch x64 ; jmp =>label(s.next));
        }
    }

    // ---- return and the common exit ----
    dynasm!(ops ; .arch x64 ; =>ret);
    let ep = exits.emit_ret_and_leave(&mut ops, &FRAME)?;

    // ---- out of line ----
    let throw0 = exits.throw(&mut ops, 0);
    dynasm!(ops ; .arch x64 ; =>overflow);
    exits.mark(&ops);
    call_table(&mut ops, layout::RT_STACK_OVERFLOW, 0)?;
    dynasm!(ops ; .arch x64 ; jmp =>throw0);
    for (slow, kind, off, cont) in slows {
        let throw = exits.throw(&mut ops, off);
        dynasm!(ops ; .arch x64 ; =>slow);
        exits.mark(&ops);
        let entry = match kind {
            Slow::Add { lhs: Some(o), .. } => {
                dynasm!(ops
                    ; .arch x64
                    ; mov r8, rax
                    ; mov rcx, QWORD [r13 + o]
                );
                layout::RT_ADD
            }
            Slow::Add { lhs: None, imm } => {
                let r = (NUMBER_TAG | u64::from(imm as u32)) as i64;
                dynasm!(ops
                    ; .arch x64
                    ; mov rcx, rax
                    ; mov r8, QWORD r
                );
                layout::RT_ADD
            }
            Slow::Inc => {
                dynasm!(ops ; .arch x64 ; mov rcx, rax);
                layout::RT_INC
            }
            Slow::Less { lhs } => {
                let lt = cmp::LT as i32;
                dynasm!(ops
                    ; .arch x64
                    ; mov r9, rax
                    ; mov r8, QWORD [r13 + lhs]
                    ; mov ecx, DWORD lt
                );
                layout::RT_COMPARE
            }
            Slow::Poll => layout::RT_POLL,
        };
        call_table(&mut ops, entry, off)?;
        dynasm!(ops
            ; .arch x64
            ; test rax, rax
            ; jz =>throw
            ; jmp =>cont
        );
    }
    exits.emit_unwind(&mut ops)?;

    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let unwind = unwind(&FRAME, &at, &ep);
    Ok(Emitted {
        code,
        unwind,
        native_offsets,
        frame: FrameOffsets {
            prologue: at,
            epilogue: ep,
            exits: exits.starts,
        },
    })
}

/// The DWARF number of a pushed register (dynasm number).
fn dwarf(r: u8) -> u16 {
    match r {
        3 => RBX,
        5 => RBP,
        12 => R12,
        13 => R13,
        15 => R15,
        _ => unreachable!("the (b) frames push rbp, r15, r13, rbx, r12 only"),
    }
}

/// CFI (code alignment 1, data alignment -8) and the Win64 UNWIND_INFO of a (b) frame that
/// pushes `pushes` (dynasm numbers, rbp first). `at`: the offsets after each push and after
/// the `lea rbp`. `ep`: the offsets of the epilogue's `lea rsp`, each `pop` and the exit jump.
/// The rows are placement (a)'s (`jit::x64`), except that the last epilogue row covers the
/// 4-byte exit jump, not a 1-byte `ret`; the rows sit between remember/restore state, so the
/// out-of-line code after the jump has the body's rule again.
fn unwind(pushes: &[u8], at: &[u32], ep: &[u32]) -> Unwind {
    let n = pushes.len();
    debug_assert!(pushes[0] == 5 && at.len() == n + 1 && ep.len() == n + 2);
    let mut p = CfiProgram::new();
    let mut prev = 0;
    for (i, &r) in pushes.iter().enumerate() {
        p.advance_loc(at[i] - prev)
            .def_cfa_offset(8 * (i as u64 + 2))
            .offset(dwarf(r), i as u64 + 2);
        prev = at[i];
    }
    // lea rbp: CFA = rbp + 16 (rbp points at the saved rbp, the first push).
    p.advance_loc(at[n] - prev).def_cfa(RBP, 16);
    // The epilogue: lea rsp keeps CFA = rbp + 16; each pop restores its register; after
    // `pop rbp` CFA = rsp + 8 until the jump.
    p.advance_loc(ep[1] - at[n]).remember_state();
    for (j, &r) in pushes.iter().rev().enumerate() {
        p.advance_loc(ep[j + 2] - ep[j + 1]);
        if r == 5 {
            p.def_cfa(RSP, 8).restore(RBP);
        } else {
            p.restore(dwarf(r));
        }
    }
    p.advance_loc(EXIT_JMP.len() as u32).restore_state();
    let mut w = UnwindInfoBuilder::new();
    for (i, &r) in pushes.iter().enumerate() {
        w.push_nonvol(at[i] as u8, r);
    }
    w.set_fpreg(at[n] as u8, wreg::RBP, 8 * (n as u8 - 1));
    Unwind {
        cfi: p,
        win64: Some(w.encode()),
    }
}

/// The (b) probe of check (g) (see `a64::emit_hold`): `hold(mk, scrub, gc)` calls `mk()`,
/// keeps the result only in the callee-saved rbx (XORed with `mask` when it is not 0), calls
/// `scrub()` and `gc()` (each through `call_any`), and returns rbx with the (b) exit. `calls`
/// are, per call: its bytecode offset, the frame slot of its callee register, and the frame
/// slot of the parameter the bytecode moves there first.
///
/// ```text
/// push rbp ; push r15 ; push r13 ; push rbx ; push r12 ; lea rbp, [rsp + 32]
/// ```
pub(crate) fn emit_hold(calls: [(u32, i32, i32); 3], mask: u64) -> Result<Emitted, Refusal> {
    let mut ops = Ops::new(0);
    let mut exits = Exits::new(&mut ops);
    let at = prologue(&mut ops, &HOLD_FRAME);
    let undef = ep9_value::encoding::VALUE_UNDEFINED as i64;
    for (i, &(off, callee, param)) in calls.iter().enumerate() {
        let throw = exits.throw(&mut ops, off);
        dynasm!(ops
            ; .arch x64
            ; mov rcx, QWORD [r13 + param]
            ; mov QWORD [r13 + callee], rcx
            ; mov rcx, QWORD undef
        );
        call_table(&mut ops, layout::RT_CALL_ANY, off)?;
        dynasm!(ops ; .arch x64 ; test rax, rax ; jz =>throw);
        if i == 0 {
            if mask == 0 {
                dynasm!(ops ; .arch x64 ; mov rbx, rax);
            } else {
                let m = mask as i64;
                dynasm!(ops ; .arch x64 ; mov rbx, QWORD m ; xor rbx, rax);
            }
        }
    }
    dynasm!(ops ; .arch x64 ; mov rax, rbx);
    let ep = exits.emit_ret_and_leave(&mut ops, &HOLD_FRAME)?;
    exits.emit_unwind(&mut ops)?;
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let unwind = unwind(&HOLD_FRAME, &at, &ep);
    Ok(Emitted {
        code,
        unwind,
        native_offsets: Vec::new(),
        frame: FrameOffsets {
            prologue: at,
            epilogue: ep,
            exits: exits.starts,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CFI of the 3-push frame equals placement (a)'s rows (`jit::x64::unwind`) with the
    /// last advance over the 4-byte exit jump, not a 1-byte `ret`.
    #[test]
    fn the_cfi_of_the_3_push_frame_is_placement_a_with_a_4_byte_exit() {
        let (at, ep) = ([1, 3, 5, 10], [100, 104, 106, 108, 109]);
        let mut want = CfiProgram::new();
        want.advance_loc(1)
            .def_cfa_offset(16)
            .offset(RBP, 2)
            .advance_loc(2)
            .def_cfa_offset(24)
            .offset(R15, 3)
            .advance_loc(2)
            .def_cfa_offset(32)
            .offset(R13, 4)
            .advance_loc(5)
            .def_cfa(RBP, 16)
            .advance_loc(104 - 10)
            .remember_state()
            .advance_loc(2)
            .restore(R13)
            .advance_loc(2)
            .restore(R15)
            .advance_loc(1)
            .def_cfa(RSP, 8)
            .restore(RBP)
            .advance_loc(4)
            .restore_state();
        let u = unwind(&FRAME, &at, &ep);
        assert_eq!(u.cfi, want);
        // UNWIND_INFO: version 1, no flags; prologue 10 bytes; 4 codes; rbp, frame offset 1
        // (x 16); codes last prologue instruction first: set_fpreg at 10, push r13 at 5, push
        // r15 at 3, push rbp at 1.
        assert_eq!(
            u.win64.as_deref(),
            Some(&[1, 10, 4, 0x15, 10, 3, 5, 0xd0, 3, 0xf0, 1, 0x50][..])
        );
    }
}
