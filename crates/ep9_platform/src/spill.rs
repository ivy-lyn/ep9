//! Spill trampolines for the conservative stack scan (H10, H28; 03-gc section 2).
//!
//! A GC pointer can live only in a callee-saved register at a safepoint [R05 probe S1-S3].
//! A trampoline pushes every callee-saved register of its ABI and then calls `cb(ctx, sp)`.
//! `[sp, stack high)` then holds every value the suspended frames can still use:
//!
//! ```text
//!   high  +------------------------+
//!         | caller frames          |  values saved by prologues and spills
//!         +------------------------+
//!         | trampoline spill area  |  callee-saved registers at the safepoint
//!   sp -> +------------------------+  <- cb(ctx, sp) gets this
//!         | cb frame (not scanned) |
//! ```
//!
//! | ABI | Spilled registers | Spill area at `sp` |
//! |---|---|---|
//! | AAPCS64 | x19-x28, x29 (fp), x30 (lr), d8-d15 | 160 B |
//! | SysV x86-64 | rbx, rbp, r12-r15 | 56 B (8 B pad) |
//! | Win64 | rbx, rbp, rdi, rsi, r12-r15, xmm6-xmm15 | 264 B (32 B shadow space, 8 B pad) |
//!
//! - Caller-saved registers need no spill. Safepoints are calls, so the compiler has already
//!   saved every live caller-saved value in a frame.
//! - The trampolines do not reload callee-saved registers after the callback: the callback
//!   keeps them by ABI rule. So the heap must not move objects found this way (S0-S2 do not).
//! - Unwind info: DWARF CFI on ELF and Mach-O, SEH on Windows x64. Windows on arm64 is not
//!   tier 1 and has none.
//! - Naked functions (`#[unsafe(naked)]` with `naked_asm!`) build on stable 1.99.1 for all
//!   five targets. rustc adds no CFI or SEH data to them, so the bodies carry it.
//! - x86-64 builds also contain the Win64 trampoline (`extern "win64"`), so tests on macOS and
//!   Linux run its code too. SysV is not built on Windows.

use std::ffi::c_void;
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};

/// The callee-saved register set a trampoline spills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpillAbi {
    /// AArch64 procedure call standard (macOS, Linux).
    Aapcs64,
    /// System V x86-64 (macOS, Linux).
    SysV64,
    /// Microsoft x64 (Windows).
    Win64,
}

impl SpillAbi {
    /// The C ABI of this target.
    #[cfg(target_arch = "aarch64")]
    pub const HOST: SpillAbi = SpillAbi::Aapcs64;
    /// The C ABI of this target.
    #[cfg(all(target_arch = "x86_64", windows))]
    pub const HOST: SpillAbi = SpillAbi::Win64;
    /// The C ABI of this target.
    #[cfg(all(target_arch = "x86_64", not(windows)))]
    pub const HOST: SpillAbi = SpillAbi::SysV64;

    /// The trampolines built for this target.
    pub const fn available() -> &'static [SpillAbi] {
        #[cfg(target_arch = "aarch64")]
        return &[SpillAbi::Aapcs64];
        #[cfg(all(target_arch = "x86_64", windows))]
        return &[SpillAbi::Win64];
        #[cfg(all(target_arch = "x86_64", not(windows)))]
        return &[SpillAbi::SysV64, SpillAbi::Win64];
    }

    /// Bytes from `sp` that the trampoline wrote: registers, padding and shadow space.
    pub const fn spill_area_bytes(self) -> usize {
        match self {
            SpillAbi::Aapcs64 => 160,
            SpillAbi::SysV64 => 56,
            SpillAbi::Win64 => 264,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            SpillAbi::Aapcs64 => "aapcs64",
            SpillAbi::SysV64 => "sysv64",
            SpillAbi::Win64 => "win64",
        }
    }
}

/// What the callback sees: the stack pointer just below the spill area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpilledRegisters {
    sp: usize,
    abi: SpillAbi,
}

impl SpilledRegisters {
    /// Scan `[sp, stack high)`. 16-byte aligned.
    pub fn sp(&self) -> usize {
        self.sp
    }

    pub fn abi(&self) -> SpillAbi {
        self.abi
    }

    /// The bytes the trampoline wrote. A hit here came from a register.
    pub fn spill_area(&self) -> Range<usize> {
        self.sp..self.sp + self.abi.spill_area_bytes()
    }
}

/// Spills the callee-saved registers of the host ABI, then runs `f`.
///
/// A panic in `f` is caught before it reaches the trampoline and resumed after it.
pub fn with_spilled_registers<R>(f: impl FnOnce(SpilledRegisters) -> R) -> R {
    with_spilled_registers_abi(SpillAbi::HOST, f)
}

/// Like [`with_spilled_registers`] with a chosen trampoline.
///
/// # Panics
/// If `abi` is not in [`SpillAbi::available`].
pub fn with_spilled_registers_abi<F, R>(abi: SpillAbi, f: F) -> R
where
    F: FnOnce(SpilledRegisters) -> R,
{
    let mut call = Call::<F, R> {
        f: Some(f),
        abi,
        out: None,
    };
    let ctx = (&raw mut call).cast::<c_void>();
    match abi {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: `ctx` points to `call`, which outlives the call; the thunk has the matching
        // ABI and type parameters.
        SpillAbi::Aapcs64 => unsafe { ep9_spill_aapcs64(ctx, thunk_c::<F, R>) },
        #[cfg(all(target_arch = "x86_64", not(windows)))]
        // SAFETY: as above.
        SpillAbi::SysV64 => unsafe { ep9_spill_sysv64(ctx, thunk_sysv64::<F, R>) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: as above.
        SpillAbi::Win64 => unsafe { ep9_spill_win64(ctx, thunk_win64::<F, R>) },
        #[allow(unreachable_patterns)]
        other => panic!(
            "spill trampoline {} is not built for this target",
            other.name()
        ),
    }
    match call
        .out
        .take()
        .expect("trampoline returned without calling back")
    {
        Ok(r) => r,
        Err(payload) => panic::resume_unwind(payload),
    }
}

struct Call<F, R> {
    f: Option<F>,
    abi: SpillAbi,
    out: Option<std::thread::Result<R>>,
}

/// Body of every thunk. Never unwinds: a panic in `f` is stored in `out`.
///
/// # Safety
/// `ctx` is the `Call<F, R>` of the running `with_spilled_registers_abi`.
unsafe fn run<F: FnOnce(SpilledRegisters) -> R, R>(ctx: *mut c_void, sp: usize) {
    // SAFETY: the caller guarantees `ctx` is a live `Call<F, R>`; no other reference to it
    // exists while the trampoline runs.
    let call = unsafe { &mut *ctx.cast::<Call<F, R>>() };
    let regs = SpilledRegisters { sp, abi: call.abi };
    if let Some(f) = call.f.take() {
        call.out = Some(panic::catch_unwind(AssertUnwindSafe(|| f(regs))));
    }
}

#[cfg(target_arch = "aarch64")]
unsafe extern "C" fn thunk_c<F: FnOnce(SpilledRegisters) -> R, R>(ctx: *mut c_void, sp: usize) {
    // SAFETY: only `with_spilled_registers_abi` passes this thunk, with its own `Call`.
    unsafe { run::<F, R>(ctx, sp) }
}

#[cfg(all(target_arch = "x86_64", not(windows)))]
unsafe extern "sysv64" fn thunk_sysv64<F: FnOnce(SpilledRegisters) -> R, R>(
    ctx: *mut c_void,
    sp: usize,
) {
    // SAFETY: only `with_spilled_registers_abi` passes this thunk, with its own `Call`.
    unsafe { run::<F, R>(ctx, sp) }
}

#[cfg(target_arch = "x86_64")]
unsafe extern "win64" fn thunk_win64<F: FnOnce(SpilledRegisters) -> R, R>(
    ctx: *mut c_void,
    sp: usize,
) {
    // SAFETY: only `with_spilled_registers_abi` passes this thunk, with its own `Call`.
    unsafe { run::<F, R>(ctx, sp) }
}

// Unwind directives: DWARF CFI everywhere except Windows, SEH on Windows. Each expands to one
// template string, or to an empty line where it does not apply.
#[cfg(not(windows))]
#[allow(unused_macros)]
macro_rules! cfi {
    ($($s:literal),+ $(,)?) => { concat!($($s, "\n"),+) };
}
#[cfg(windows)]
#[allow(unused_macros)]
macro_rules! cfi {
    ($($s:literal),+ $(,)?) => {
        ""
    };
}
#[cfg(windows)]
#[allow(unused_macros)]
macro_rules! seh {
    ($($s:literal),+ $(,)?) => { concat!($($s, "\n"),+) };
}
#[cfg(not(windows))]
#[allow(unused_macros)]
macro_rules! seh {
    ($($s:literal),+ $(,)?) => {
        ""
    };
}

/// AAPCS64 trampoline: `cb(ctx, sp)` after spilling x19-x28, fp, lr and d8-d15.
///
/// Frame (160 B): `[sp]` fp, lr; `[sp+16]` x19-x28; `[sp+96]` d8-d15.
///
/// # Safety
/// `cb` is safe to call with `ctx` and any `sp`.
#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn ep9_spill_aapcs64(
    ctx: *mut c_void,
    cb: unsafe extern "C" fn(*mut c_void, usize),
) {
    core::arch::naked_asm!(
        cfi!(".cfi_startproc"),
        // The frame record (fp, lr) sits at the bottom, so x29 keeps the frame chain.
        "stp x29, x30, [sp, #-160]!",
        cfi!(
            ".cfi_def_cfa_offset 160",
            ".cfi_offset w30, -152",
            ".cfi_offset w29, -160"
        ),
        "mov x29, sp",
        "stp x19, x20, [sp, #16]",
        "stp x21, x22, [sp, #32]",
        "stp x23, x24, [sp, #48]",
        "stp x25, x26, [sp, #64]",
        "stp x27, x28, [sp, #80]",
        "stp d8, d9, [sp, #96]",
        "stp d10, d11, [sp, #112]",
        "stp d12, d13, [sp, #128]",
        "stp d14, d15, [sp, #144]",
        // cb(ctx, sp): x0 already holds ctx.
        "mov x9, x1",
        "mov x1, sp",
        "blr x9",
        // The callee kept x19-x28 and d8-d15, so only fp and lr come back.
        "ldp x29, x30, [sp], #160",
        cfi!(
            ".cfi_def_cfa_offset 0",
            ".cfi_restore w30",
            ".cfi_restore w29"
        ),
        "ret",
        cfi!(".cfi_endproc"),
    )
}

/// System V x86-64 trampoline: `cb(ctx, sp)` after spilling rbx, rbp and r12-r15.
///
/// Frame (56 B from `sp`): pad, r15, r14, r13, r12, rbx, rbp.
///
/// # Safety
/// `cb` is safe to call with `ctx` and any `sp`.
#[cfg(all(target_arch = "x86_64", not(windows)))]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "sysv64" fn ep9_spill_sysv64(
    ctx: *mut c_void,
    cb: unsafe extern "sysv64" fn(*mut c_void, usize),
) {
    core::arch::naked_asm!(
        ".cfi_startproc",
        "push rbp",
        ".cfi_def_cfa_offset 16",
        ".cfi_offset rbp, -16",
        "mov rbp, rsp",
        ".cfi_def_cfa_register rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        ".cfi_offset rbx, -24",
        ".cfi_offset r12, -32",
        ".cfi_offset r13, -40",
        ".cfi_offset r14, -48",
        ".cfi_offset r15, -56",
        // Entry rsp is 8 mod 16; 6 pushes and 8 bytes of pad make it 16-aligned at the call.
        "sub rsp, 8",
        // cb(ctx, sp): rdi already holds ctx.
        "mov rax, rsi",
        "mov rsi, rsp",
        "call rax",
        "add rsp, 8",
        "pop r15",
        ".cfi_restore r15",
        "pop r14",
        ".cfi_restore r14",
        "pop r13",
        ".cfi_restore r13",
        "pop r12",
        ".cfi_restore r12",
        "pop rbx",
        ".cfi_restore rbx",
        "pop rbp",
        ".cfi_def_cfa rsp, 8",
        ".cfi_restore rbp",
        "ret",
        ".cfi_endproc",
    )
}

/// Microsoft x64 trampoline: `cb(ctx, sp)` after spilling rbx, rbp, rdi, rsi, r12-r15 and
/// xmm6-xmm15.
///
/// Frame (264 B from `sp`): 32 B shadow space for `cb`, xmm6-xmm15 (160 B), 8 B pad,
/// r15, r14, r13, r12, rsi, rdi, rbx, rbp.
///
/// # Safety
/// `cb` is safe to call with `ctx` and any `sp`.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "win64" fn ep9_spill_win64(
    ctx: *mut c_void,
    cb: unsafe extern "win64" fn(*mut c_void, usize),
) {
    core::arch::naked_asm!(
        seh!(".seh_proc ep9_spill_win64"),
        cfi!(".cfi_startproc"),
        "push rbp",
        seh!(".seh_pushreg rbp"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rbp, 0"),
        "push rbx",
        seh!(".seh_pushreg rbx"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rbx, 0"),
        "push rdi",
        seh!(".seh_pushreg rdi"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rdi, 0"),
        "push rsi",
        seh!(".seh_pushreg rsi"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset rsi, 0"),
        "push r12",
        seh!(".seh_pushreg r12"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r12, 0"),
        "push r13",
        seh!(".seh_pushreg r13"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r13, 0"),
        "push r14",
        seh!(".seh_pushreg r14"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r14, 0"),
        "push r15",
        seh!(".seh_pushreg r15"),
        cfi!(".cfi_adjust_cfa_offset 8", ".cfi_rel_offset r15, 0"),
        // Entry rsp is 8 mod 16; 8 pushes keep it 8 mod 16; 200 = 32 shadow + 160 xmm + 8 pad
        // makes it 16-aligned for movaps and for the call.
        "sub rsp, 200",
        seh!(".seh_stackalloc 200"),
        cfi!(".cfi_adjust_cfa_offset 200"),
        "movaps xmmword ptr [rsp + 32], xmm6",
        seh!(".seh_savexmm xmm6, 32"),
        "movaps xmmword ptr [rsp + 48], xmm7",
        seh!(".seh_savexmm xmm7, 48"),
        "movaps xmmword ptr [rsp + 64], xmm8",
        seh!(".seh_savexmm xmm8, 64"),
        "movaps xmmword ptr [rsp + 80], xmm9",
        seh!(".seh_savexmm xmm9, 80"),
        "movaps xmmword ptr [rsp + 96], xmm10",
        seh!(".seh_savexmm xmm10, 96"),
        "movaps xmmword ptr [rsp + 112], xmm11",
        seh!(".seh_savexmm xmm11, 112"),
        "movaps xmmword ptr [rsp + 128], xmm12",
        seh!(".seh_savexmm xmm12, 128"),
        "movaps xmmword ptr [rsp + 144], xmm13",
        seh!(".seh_savexmm xmm13, 144"),
        "movaps xmmword ptr [rsp + 160], xmm14",
        seh!(".seh_savexmm xmm14, 160"),
        "movaps xmmword ptr [rsp + 176], xmm15",
        seh!(".seh_savexmm xmm15, 176"),
        seh!(".seh_endprologue"),
        // cb(ctx, sp): rcx already holds ctx; [rsp, rsp + 32) is the callee's shadow space.
        "mov rax, rdx",
        "mov rdx, rsp",
        "call rax",
        // The callee kept the non-volatile registers; the epilogue only pops (SEH epilogue form).
        "add rsp, 200",
        cfi!(".cfi_adjust_cfa_offset -200"),
        "pop r15",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r15"),
        "pop r14",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r14"),
        "pop r13",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r13"),
        "pop r12",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore r12"),
        "pop rsi",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rsi"),
        "pop rdi",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rdi"),
        "pop rbx",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rbx"),
        "pop rbp",
        cfi!(".cfi_adjust_cfa_offset -8", ".cfi_restore rbp"),
        "ret",
        cfi!(".cfi_endproc"),
        seh!(".seh_endproc"),
    )
}

/// Reads the word at `addr` with one load that the optimizer cannot see into.
///
/// A conservative scan reads stack slots that Rust may treat as uninitialized or borrowed.
/// A Rust read of such a slot is undefined behavior. An `asm!` load is a plain machine load
/// and returns whatever bits are there; the scan treats every word only as a candidate.
///
/// # Safety
/// `addr` is 8-byte aligned and readable for 8 bytes.
#[inline(always)]
pub unsafe fn load_word(addr: usize) -> usize {
    let w: usize;
    // SAFETY: the caller guarantees `addr` is aligned and readable. The asm only loads.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "ldr {w}, [{a}]",
            a = in(reg) addr,
            w = lateout(reg) w,
            options(nostack, readonly, preserves_flags),
        )
    };
    // SAFETY: the caller guarantees `addr` is aligned and readable. The asm only loads.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "mov {w}, qword ptr [{a}]",
            a = in(reg) addr,
            w = lateout(reg) w,
            options(nostack, readonly, preserves_flags),
        )
    };
    w
}
