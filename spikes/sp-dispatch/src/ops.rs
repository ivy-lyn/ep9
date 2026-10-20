//! The 20 opcodes, defined once (H15). Register + accumulator machine, Ignition style.
//!
//! Values are `i64`. Arithmetic has an int32 fast path; a result outside int32 goes to a
//! `#[cold]` slow path (in an engine it would make a heap number). Bitwise ops work on the
//! low 32 bits and give int32 results.
//!
//! Frame layout on the VM stack (`fp` points at r0):
//!
//! ```text
//! fp[-3] = slots from this frame's fp back to the caller's fp
//! fp[-2] = return pc, as a byte offset from the code start
//! fp[-1] = register count of this frame
//! fp[0..nregs] = registers; arguments are copied to r0.., the rest are set to 0
//! ```

/// Frame header size in slots (see the module docs).
pub const HEADER: usize = 3;

/// Calls `$m!` with the full opcode list. `[...]` is passed through to `$m`.
///
/// Entry syntax: `class snake_name CamelName = opcode (operand: Kind, ..) => { body } [cold { body }];`
/// - class `tail`: the handler always continues to another handler.
/// - class `exit`: the handler may return to the host.
/// - The body sees `cx: &mut Cx`, `pc` (start of this instruction), `next_pc`, `fp`, `acc`
///   and the operands. It returns a `Flow`.
/// - The optional cold block runs when the body returns `Flow::Cold`. Put every runtime call
///   there: then the fast handler has no call and no stack frame (see README, Win64).
/// - Bodies name `Flow` without a path; every expansion site imports `crate::dsl::Flow`.
macro_rules! for_each_op {
    ($m:ident [$($arg:tt)*]) => {
        $m! {
            [$($arg)*]
            state(cx, pc, next_pc, fp, acc);

            /// Return `acc` to the host.
            exit halt Halt = 0 () => { Flow::Exit(acc) };
            /// acc = imm
            tail lda_smi LdaSmi = 1 (imm: Imm) => { acc = imm; Flow::Next };
            /// acc = constant k
            tail lda_const LdaConst = 2 (k: Const) => { acc = *cx.consts.add(k); Flow::Next };
            /// acc = r
            tail ldar Ldar = 3 (r: Reg) => { acc = *fp.add(r); Flow::Next };
            /// r = acc
            tail star Star = 4 (r: Reg) => { *fp.add(r) = acc; Flow::Next };
            /// dst = src
            tail mov Mov = 5 (src: Reg, dst: Reg) => {
                #[cfg(feature = "neg-escape")]
                $crate::ops::neg_escape(&mut acc);
                *fp.add(dst) = *fp.add(src);
                Flow::Next
            };
            /// acc = r + acc
            tail add Add = 6 (r: Reg) => {
                match $crate::ops::int32(acc.wrapping_add(*fp.add(r))) {
                    Some(v) => { acc = v; Flow::Next }
                    None => Flow::Cold,
                }
            } cold {
                acc = $crate::ops::add_slow(cx, *fp.add(r), acc);
                Flow::Next
            };
            /// acc = r - acc
            tail sub Sub = 7 (r: Reg) => {
                match $crate::ops::int32((*fp.add(r)).wrapping_sub(acc)) {
                    Some(v) => { acc = v; Flow::Next }
                    None => Flow::Cold,
                }
            } cold {
                acc = $crate::ops::sub_slow(cx, *fp.add(r), acc);
                Flow::Next
            };
            /// acc = acc + imm
            tail add_smi AddSmi = 8 (imm: Imm) => {
                match $crate::ops::int32(acc.wrapping_add(imm)) {
                    Some(v) => { acc = v; Flow::Next }
                    None => Flow::Cold,
                }
            } cold {
                acc = $crate::ops::add_slow(cx, acc, imm);
                Flow::Next
            };
            /// acc = r & acc (int32)
            tail bit_and BitAnd = 9 (r: Reg) => {
                acc = ((*fp.add(r) as i32) & (acc as i32)) as i64;
                Flow::Next
            };
            /// acc = r ^ acc (int32)
            tail bit_xor BitXor = 10 (r: Reg) => {
                acc = ((*fp.add(r) as i32) ^ (acc as i32)) as i64;
                Flow::Next
            };
            /// acc = acc << (imm & 31) (int32, wraps)
            tail shl_smi ShlSmi = 11 (imm: Imm) => {
                acc = (acc as i32).wrapping_shl(imm as u32) as i64;
                Flow::Next
            };
            /// acc = acc >>> (imm & 31) on the low 32 bits, read back as int32
            tail shr_smi ShrSmi = 12 (imm: Imm) => {
                acc = (acc as u32).wrapping_shr(imm as u32) as i32 as i64;
                Flow::Next
            };
            /// acc = (r < acc) as 0 or 1
            tail test_less_than TestLessThan = 13 (r: Reg) => {
                acc = (*fp.add(r) < acc) as i64;
                Flow::Next
            };
            /// acc = (r == acc) as 0 or 1
            tail test_equal TestEqual = 14 (r: Reg) => {
                acc = (*fp.add(r) == acc) as i64;
                Flow::Next
            };
            /// pc += off
            tail jump Jump = 15 (off: Off) => { Flow::Jump(pc.offset(off)) };
            /// if acc == 0 { pc += off }
            tail jump_if_false JumpIfFalse = 16 (off: Off) => {
                if acc == 0 { Flow::Jump(pc.offset(off)) } else { Flow::Next }
            };
            /// Loop back edge: spend one unit of the interrupt budget, then pc += off.
            /// At 0 the cold block runs the interrupt check.
            tail jump_loop JumpLoop = 17 (off: Off) => {
                cx.budget -= 1;
                if cx.budget == 0 { Flow::Cold } else { Flow::Jump(pc.offset(off)) }
            } cold {
                $crate::ops::interrupt(cx);
                Flow::Jump(pc.offset(off))
            };
            /// acc = call function f with argc arguments from registers args..
            /// The call pushes a VM frame. It never recurses on the native stack.
            /// A full VM stack goes to the cold block, which throws.
            tail call Call = 18 (f: Func, args: Reg, argc: Cnt) => {
                let callee = &*cx.funcs.add(f);
                let cur = *fp.sub(1) as usize;
                let new_fp = fp.wrapping_add(cur + $crate::ops::HEADER);
                if new_fp.wrapping_add(callee.nregs) as *const i64 > cx.stack_limit {
                    Flow::Cold
                } else {
                    *new_fp.sub(3) = (cur + $crate::ops::HEADER) as i64;
                    *new_fp.sub(2) = next_pc.offset_from(cx.code) as i64;
                    *new_fp.sub(1) = callee.nregs as i64;
                    let mut i = 0;
                    while i < argc {
                        *new_fp.add(i) = *fp.add(args + i);
                        i += 1;
                    }
                    while i < callee.nregs {
                        *new_fp.add(i) = 0;
                        i += 1;
                    }
                    fp = new_fp;
                    Flow::Jump(callee.code)
                }
            } cold {
                acc = $crate::ops::stack_overflow(cx);
                Flow::Jump(cx.throw_pc)
            };
            /// Pop the frame and continue in the caller after its Call. acc is the result.
            tail ret Return = 19 () => {
                let back = *fp.sub(3) as usize;
                let ret = *fp.sub(2) as usize;
                fp = fp.sub(back);
                Flow::Jump(cx.code.add(ret))
            };
        }
    };
}
pub(crate) use for_each_op;

use crate::vm::{Cx, ERR_STACK_OVERFLOW};

/// Interrupt budget per back edge count. The C baseline uses the same value.
pub const BUDGET: i64 = 10_000;

/// `Some(v)` if `v` fits in int32 (the fast path), else `None`.
#[inline(always)]
pub fn int32(v: i64) -> Option<i64> {
    if v as i32 as i64 == v { Some(v) } else { None }
}

#[cold]
#[inline(never)]
pub fn add_slow(cx: &mut Cx, a: i64, b: i64) -> i64 {
    cx.slow_paths += 1;
    a.wrapping_add(b)
}

#[cold]
#[inline(never)]
pub fn sub_slow(cx: &mut Cx, a: i64, b: i64) -> i64 {
    cx.slow_paths += 1;
    a.wrapping_sub(b)
}

/// Back-edge interrupt check: an engine would poll GC, tier-up and termination here.
#[cold]
#[inline(never)]
pub fn interrupt(cx: &mut Cx) {
    cx.interrupts += 1;
    cx.budget = BUDGET;
}

/// The VM stack is full. Records the error; the caller jumps to the throw stub.
#[cold]
#[inline(never)]
pub fn stack_overflow(cx: &mut Cx) -> i64 {
    cx.error = ERR_STACK_OVERFLOW;
    i64::MIN
}

/// Negative test only: makes the address of a local escape into an opaque call. LLVM then
/// may not turn the dispatch call into a tail jump.
#[cfg(feature = "neg-escape")]
#[inline(never)]
pub fn neg_escape(x: &mut i64) {
    core::hint::black_box(x);
}
