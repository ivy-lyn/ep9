//! Form D keep-calls: the barrier slow call of T0 code on x86-64 (plan v14 V14-D7; DL M8-P3-D3,
//! M9-PRE-FORMD-KIND; route X86-STORE-ALLOC-SAVES; plan rule 81).
//!
//! An x86-64 handler gets `cx`, `pc`, `fp` and `acc` in rdi, rsi, rdx and rcx (H15). These
//! registers are caller-saved. When a handler body makes a plain call that returns (a barrier
//! slow path) and then dispatches, LLVM keeps the values that live across the call in
//! callee-saved registers. So the handler saves those registers on entry, on every run, also
//! when the slow path does not run. A keep-call changes only the slow call:
//!
//! ```text
//!  handler (sysv64)                   ep9_keep_barrier_slow (naked)      barrier_slow (C ABI)
//!  values in rdi, rsi, rdx, rcx, ...
//!  r8, r9, r10 = host, slot, v
//!  asm!("call wrapper") ────────────► push rcx, rdx, rsi, rdi, r8-r11
//!    changes only: rax, xmm0-15,      (Win64: + 32 B shadow space)
//!    st(0)-st(7), mm0-7, flags        C argument registers <- r8, r9, r10
//!                                     call ───────────────────────────► store, then log
//!                                     pop r11-r8, rdi, rsi, rdx, rcx ◄─
//!  values still in place ◄─────────── ret
//! ```
//!
//! | Build | The barrier slow call of `context_set` |
//! |---|---|
//! | x86-64, `panic = "abort"` (the release profile and its artifacts, H23) | the keep-call (`context_set_keep`) |
//! | x86-64, `panic = "unwind"` (the dev profile, test harnesses) | a plain call (`object::context_set`): `asm!` with `may_unwind` is not stable, and a panic must not unwind through an `asm!` block. The unit tests call `context_set_keep` directly |
//! | other targets | a plain call (`object::context_set`; aarch64 has 18 free caller-saved registers) |
//!
//! - **Clobbers.** The `asm!` call names rax, xmm0-xmm15, st(0)-st(7), mm0-mm7 and the flags
//!   (no `preserves_flags`). That is the whole caller-saved set of the SysV and Win64 ABIs on
//!   the baseline x86-64 target, less the 8 GPRs that the wrapper saves. With `avx512f`,
//!   zmm16-zmm31 and k0-k7 are caller-saved too, so such a build stops at a `compile_error!`
//!   (X2 of DL M8-P3-D3).
//!   The guard is crate-wide: it does not see a function-level `#[target_feature]`. Never call
//!   a keep path from such a function: an `avx512f` one may keep values in zmm16-zmm31 or
//!   k0-k7 across the call. Today `ep9_math/src/exact.rs:68` and `:522` are such functions
//!   (fma), and they call no keep path (ep9_math does not depend on ep9_vm).
//! - **Stack.** No `nostack` option: the `asm!` block starts with rsp 16-byte aligned and with
//!   nothing live below rsp. The wrapper keeps that alignment for its own call (SysV: 8 pushes
//!   and 8 bytes; Win64: 8 pushes, 32 bytes of shadow space and 8 bytes).
//! - **GC.** The barrier slow path never allocates and never reaches a safepoint (H11). The
//!   wrapper pushes the caller-saved registers onto the native stack, which the conservative
//!   scan reads (H10), as it reads the callee-saved spills of a plain call.
//! - **Barrier contract (H11).** `slot_set_keep` is `Slot::set` with the same check (the
//!   new-value filter, then `gc_state <= threshold`), the same Relaxed store on the fast path
//!   (`Slot::init` is the store of the `else` arm of `Slot::set`) and the same slow-path entry
//!   (`barrier_slow`, which does the store). Only the call instruction differs. A const check
//!   ties this copy of the fast path to `BARRIER_VERSION` 2.
//! - **Unwind info.** DWARF CFI on ELF and Mach-O, SEH on Windows (the form of
//!   `ep9_platform::spill`): rustc adds none to a naked function.
//! - **Not handlers.** The wrapper is an `ep9_keep_*` symbol, not an `ep9h__` one, so
//!   `cargo xtask disasm-check` counts the same handlers.

#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
use ep9_heap::{BarrierState, CellPtr, Heap, Slot};
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
use ep9_value::{Value, Word0};

use crate::object;
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
use crate::object::ctxf;
use crate::runtime::Runtime;
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
use crate::runtime::val;

#[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
compile_error!(
    "ep9: the x86-64 keep-call (interp/keepcall.rs) names only rax, xmm0-xmm15, x87 and mm0-mm7 \
     as changed; with avx512f, zmm16-zmm31 and k0-k7 are caller-saved too (DL M8-P3-D3 X2)"
);

// The fast path below is a copy of `Slot::set` of barrier version 2. A new version may change
// it, so a bump stops the build here.
const _: () = assert!(ep9_heap::BARRIER_VERSION == 2);

// Unwind directives: DWARF CFI everywhere except Windows, SEH on Windows. Each expands to one
// template string, or to an empty string where it does not apply.
#[cfg(all(target_arch = "x86_64", not(windows)))]
macro_rules! cfi {
    ($($s:literal),+ $(,)?) => { concat!($($s, "\n"),+) };
}
#[cfg(all(target_arch = "x86_64", windows))]
macro_rules! cfi {
    ($($s:literal),+ $(,)?) => {
        ""
    };
}
#[cfg(all(target_arch = "x86_64", windows))]
macro_rules! seh {
    ($($s:literal),+ $(,)?) => { concat!($($s, "\n"),+) };
}
#[cfg(all(target_arch = "x86_64", not(windows)))]
macro_rules! seh {
    ($($s:literal),+ $(,)?) => {
        ""
    };
}
// `.seh_proc <symbol>`: SEH needs the symbol name.
#[cfg(all(target_arch = "x86_64", windows))]
macro_rules! seh_proc {
    ($name:ident) => {
        concat!(".seh_proc ", stringify!($name), "\n")
    };
}
#[cfg(all(target_arch = "x86_64", not(windows)))]
macro_rules! seh_proc {
    ($name:ident) => {
        ""
    };
}

/// Defines `$name`, a naked keep-call wrapper of the C-ABI function `$target`: it takes up to 3
/// integer arguments in r8, r9 and r10, saves rcx, rdx, rsi, rdi and r8-r11, calls `$target`
/// with the arguments in the C argument registers of the target (SysV rdi, rsi, rdx; Win64 rcx,
/// rdx, r8, with 32 bytes of shadow space), and restores the 8 registers. rax holds the result
/// of `$target`. A wrapper is called only by `keep_call!`.
#[cfg(target_arch = "x86_64")]
macro_rules! keep_wrapper {
    ($(#[$m:meta])* $name:ident => $target:path) => {
        $(#[$m])*
        ///
        /// # Safety
        /// Call it only through `keep_call!`, with the arguments of its target in r8, r9 and
        /// r10, and only with a target that does not unwind.
        #[unsafe(no_mangle)]
        #[unsafe(naked)]
        pub(crate) unsafe extern "C" fn $name() {
            core::arch::naked_asm!(
                seh_proc!($name),
                cfi!(".cfi_startproc"),
                "push rcx",
                seh!(".seh_pushreg rcx"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rcx, 0"),
                "push rdx",
                seh!(".seh_pushreg rdx"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rdx, 0"),
                "push rsi",
                seh!(".seh_pushreg rsi"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rsi, 0"),
                "push rdi",
                seh!(".seh_pushreg rdi"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rdi, 0"),
                "push r8",
                seh!(".seh_pushreg r8"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r8, 0"),
                "push r9",
                seh!(".seh_pushreg r9"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r9, 0"),
                "push r10",
                seh!(".seh_pushreg r10"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r10, 0"),
                "push r11",
                seh!(".seh_pushreg r11"),
                cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r11, 0"),
                // Entry rsp is 8 mod 16, and 8 pushes keep it so. SysV: 8 bytes of pad make it
                // 16-aligned for the call. Win64: 32 bytes of shadow space and 8 bytes of pad.
                seh!("sub rsp, 40", ".seh_stackalloc 40", ".seh_endprologue"),
                cfi!("sub rsp, 8", ".cfi_adjust_cfa_offset 8"),
                // The C argument registers (Win64 reads r8 first, then writes it).
                seh!("mov rcx, r8", "mov rdx, r9", "mov r8, r10"),
                cfi!("mov rdi, r8", "mov rsi, r9", "mov rdx, r10"),
                "call {target}",
                seh!("add rsp, 40"),
                cfi!("add rsp, 8", ".cfi_adjust_cfa_offset -8"),
                "pop r11",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r11"),
                "pop r10",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r10"),
                "pop r9",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r9"),
                "pop r8",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r8"),
                "pop rdi",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rdi"),
                "pop rsi",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rsi"),
                "pop rdx",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rdx"),
                "pop rcx",
                cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rcx"),
                "ret",
                cfi!(".cfi_endproc"),
                seh!(".seh_endproc"),
                target = sym $target,
            )
        }
    };
}

/// Calls the keep-call wrapper `$wrapper` (made by `keep_wrapper!`) with 3 integer arguments.
/// The `asm!` block names only rax, xmm0-xmm15, st(0)-st(7), mm0-mm7 and the flags as changed,
/// so LLVM keeps every other value in its register across the call. It expands to an `asm!`
/// statement: use it in an `unsafe` block whose safety comment names the target.
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
macro_rules! keep_call {
    ($wrapper:path; $a:expr, $b:expr, $c:expr) => {
        core::arch::asm!(
            "call {w}",
            w = sym $wrapper,
            in("r8") $a,
            in("r9") $b,
            in("r10") $c,
            out("rax") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _,
            out("xmm4") _, out("xmm5") _, out("xmm6") _, out("xmm7") _,
            out("xmm8") _, out("xmm9") _, out("xmm10") _, out("xmm11") _,
            out("xmm12") _, out("xmm13") _, out("xmm14") _, out("xmm15") _,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
            out("mm0") _, out("mm1") _, out("mm2") _, out("mm3") _,
            out("mm4") _, out("mm5") _, out("mm6") _, out("mm7") _,
        )
    };
}
// For the unit tests (crate::tests::keepcall).
#[cfg(all(target_arch = "x86_64", test))]
pub(crate) use keep_call;

#[cfg(target_arch = "x86_64")]
keep_wrapper! {
    /// The keep-call wrapper of `ep9_heap::barrier_slow(host, slot, v)`.
    ep9_keep_barrier_slow => ep9_heap::barrier_slow
}

/// `object::context_set` for the T0 context stores (`StaContextSlot`, `StaCurrentContextSlot`;
/// `ops::ctx_store`): stores `v` into slot `i` of the context `c`, with the barrier. False if
/// `i` is out of range; nothing is stored then. On x86-64 abort builds the barrier slow call is
/// a keep-call (`context_set_keep`); elsewhere this is `object::context_set`.
#[cfg(all(target_arch = "x86_64", panic = "abort"))]
#[inline(always)]
pub(crate) fn context_set(rt: &Runtime, c: u64, i: u32, v: u64) -> bool {
    context_set_keep(rt, c, i, v)
}

/// `object::context_set` (see the x86-64 abort form above).
#[cfg(not(all(target_arch = "x86_64", panic = "abort")))]
#[inline(always)]
pub(crate) fn context_set(rt: &Runtime, c: u64, i: u32, v: u64) -> bool {
    object::context_set(rt, c, i, v)
}

/// The body of `object::context_set` with `wr_keep` for its store. Built on x86-64 abort
/// builds, and in the unit tests (which unwind) so that `cargo test` runs it.
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
#[inline(always)]
pub(crate) fn context_set_keep(rt: &Runtime, c: u64, i: u32, v: u64) -> bool {
    let cc = object::cell(c);
    if (i as usize) < object::rd_i32(cc, ctxf::LENGTH) as usize {
        wr_keep(&rt.heap, cc, ctxf::SLOTS + 8 * i as usize, v);
        true
    } else {
        false
    }
}

/// `object::wr` with `slot_set_keep`: the barriered store of `v` into the Value slot at `off`
/// of the live cell `c`.
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
#[inline(always)]
pub(crate) fn wr_keep(heap: &Heap, c: CellPtr, off: usize, v: u64) {
    // SAFETY: `c` is live (crate invariant, object.rs) and every caller passes an offset of a
    // Value slot inside the cell's layout.
    let (h, slot) = unsafe { (c.header(), c.slot(off)) };
    slot_set_keep(slot, h, val(v), heap.barrier());
}

/// `Slot::set` with the keep-call slow path: stores `v` into `slot` of the cell whose header
/// is `host`, with the check and the stores of `Slot::set`, and the slow call through
/// `ep9_keep_barrier_slow`.
#[cfg(all(target_arch = "x86_64", any(test, panic = "abort")))]
#[inline(always)]
pub(crate) fn slot_set_keep(slot: &Slot, host: &Word0, v: Value, state: &BarrierState) {
    if ep9_heap::barrier::is_cell_bits(v.to_bits()) && host.gc_state() <= state.threshold() {
        // SAFETY: the wrapper calls `barrier_slow(host, slot, v)` with the 3 arguments in the
        // C argument registers. `host` and `slot` are live references, `v` is a valid Value,
        // and `barrier_slow` does not unwind: it only stores and logs, and a panic in an
        // abort build aborts.
        unsafe {
            keep_call!(
                ep9_keep_barrier_slow;
                core::ptr::from_ref(host),
                core::ptr::from_ref(slot),
                v.to_bits()
            );
        }
    } else {
        // The `else` arm of `Slot::set`: the same Relaxed store, with no barrier.
        slot.init(v);
    }
}

/// A keep-call target and its wrapper for the GPR-fill tests (`crate::tests::keepcall`; DL
/// M8-P3-D3 X3).
#[cfg(all(target_arch = "x86_64", test))]
pub(crate) mod test_target {
    use core::sync::atomic::AtomicU64;

    /// What `clobber` saw: its 3 arguments, then rsp mod 16 at its entry.
    pub(crate) static SEEN: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

    /// A C-ABI target that records its 3 arguments and rsp mod 16, then writes junk into every
    /// register that the C ABI lets it change (SysV: rax, rcx, rdx, rsi, rdi, r8-r11 and
    /// xmm0-xmm15; Win64: rax, rcx, rdx, r8-r11, xmm0-xmm5 and its 32-byte shadow space).
    #[cfg(not(windows))]
    #[unsafe(naked)]
    unsafe extern "C" fn clobber() {
        core::arch::naked_asm!(
            "mov qword ptr [rip + {seen}], rdi",
            "mov qword ptr [rip + {seen} + 8], rsi",
            "mov qword ptr [rip + {seen} + 16], rdx",
            "mov rax, rsp",
            "and rax, 15",
            "mov qword ptr [rip + {seen} + 24], rax",
            "mov rax, -1",
            "mov rcx, rax",
            "mov rdx, rax",
            "mov rsi, rax",
            "mov rdi, rax",
            "mov r8, rax",
            "mov r9, rax",
            "mov r10, rax",
            "mov r11, rax",
            "pcmpeqd xmm0, xmm0",
            "pcmpeqd xmm1, xmm1",
            "pcmpeqd xmm2, xmm2",
            "pcmpeqd xmm3, xmm3",
            "pcmpeqd xmm4, xmm4",
            "pcmpeqd xmm5, xmm5",
            "pcmpeqd xmm6, xmm6",
            "pcmpeqd xmm7, xmm7",
            "pcmpeqd xmm8, xmm8",
            "pcmpeqd xmm9, xmm9",
            "pcmpeqd xmm10, xmm10",
            "pcmpeqd xmm11, xmm11",
            "pcmpeqd xmm12, xmm12",
            "pcmpeqd xmm13, xmm13",
            "pcmpeqd xmm14, xmm14",
            "pcmpeqd xmm15, xmm15",
            "ret",
            seen = sym SEEN,
        )
    }

    #[cfg(windows)]
    #[unsafe(naked)]
    unsafe extern "C" fn clobber() {
        core::arch::naked_asm!(
            "mov qword ptr [rip + {seen}], rcx",
            "mov qword ptr [rip + {seen} + 8], rdx",
            "mov qword ptr [rip + {seen} + 16], r8",
            "mov rax, rsp",
            "and rax, 15",
            "mov qword ptr [rip + {seen} + 24], rax",
            "mov rax, -1",
            // The shadow space is the callee's: [rsp + 8, rsp + 40).
            "mov qword ptr [rsp + 8], rax",
            "mov qword ptr [rsp + 16], rax",
            "mov qword ptr [rsp + 24], rax",
            "mov qword ptr [rsp + 32], rax",
            "mov rcx, rax",
            "mov rdx, rax",
            "mov r8, rax",
            "mov r9, rax",
            "mov r10, rax",
            "mov r11, rax",
            "pcmpeqd xmm0, xmm0",
            "pcmpeqd xmm1, xmm1",
            "pcmpeqd xmm2, xmm2",
            "pcmpeqd xmm3, xmm3",
            "pcmpeqd xmm4, xmm4",
            "pcmpeqd xmm5, xmm5",
            "ret",
            seen = sym SEEN,
        )
    }

    keep_wrapper! {
        /// A keep-call wrapper of `clobber`, for the tests.
        ep9_keep_test_clobber => clobber
    }
}
