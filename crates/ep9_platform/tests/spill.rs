//! Spill trampolines: every callee-saved register of the ABI lands in the spill area, the
//! registers survive the call, and the safe wrapper behaves.
//!
//! A naked helper per ABI loads a unique pattern into every callee-saved register, calls the
//! trampoline directly, and returns how many registers changed. The callback copies the spill
//! area, and the test looks for each pattern there.

use std::ffi::c_void;

use ep9_platform::{
    SpillAbi, current_thread_stack_bounds, load_word, stack_pointer, with_spilled_registers,
    with_spilled_registers_abi,
};

/// What the callback saw.
struct Seen {
    abi: SpillAbi,
    sp: usize,
    area: Vec<u64>,
}

fn record(ctx: *mut c_void, sp: usize) {
    // SAFETY: the tests pass a `*mut Seen` that lives across the helper call.
    let seen = unsafe { &mut *ctx.cast::<Seen>() };
    seen.sp = sp;
    let words = seen.abi.spill_area_bytes() / 8;
    // SAFETY: the spill area is part of the live stack above `sp` and 8-byte aligned.
    seen.area = (0..words)
        .map(|i| unsafe { load_word(sp + 8 * i) } as u64)
        .collect();
}

/// Patterns: 0xE9E9_0000_0000_00NN for general registers, 0xE9D8_0000_0000_00NN for vector
/// registers. NN reads like the register name (x19 -> 0x19, rbx -> 0xB0, xmm6 -> 0x06).
const fn gp(nn: u64) -> u64 {
    0xE9E9_0000_0000_0000 | nn
}
const fn fp(nn: u64) -> u64 {
    0xE9D8_0000_0000_0000 | nn
}

fn check(seen: &Seen, expected: &[(&str, u64)], mismatches: u64) {
    println!(
        "{}: sp {:#x}, spill area {} B, {} registers expected",
        seen.abi.name(),
        seen.sp,
        seen.abi.spill_area_bytes(),
        expected.len()
    );
    assert_eq!(
        mismatches, 0,
        "{} registers changed across the trampoline",
        mismatches
    );
    assert_eq!(
        seen.sp % 16,
        0,
        "sp must be 16-byte aligned at the callback"
    );
    for (name, v) in expected {
        let n = seen.area.iter().filter(|w| *w == v).count();
        assert!(
            n >= 1,
            "{name} = {v:#x} is not in the {} spill area",
            seen.abi.name()
        );
    }
}

#[cfg(target_arch = "aarch64")]
mod a64 {
    use super::*;
    use ep9_platform::spill::ep9_spill_aapcs64;

    pub(super) unsafe extern "C" fn cb(ctx: *mut c_void, sp: usize) {
        record(ctx, sp)
    }

    /// Fills x19-x29 and d8-d15, calls the trampoline, returns the count of changed registers.
    #[unsafe(naked)]
    pub(super) unsafe extern "C" fn fill_and_spill(
        ctx: *mut c_void,
        cb: unsafe extern "C" fn(*mut c_void, usize),
    ) -> u64 {
        core::arch::naked_asm!(
            "stp x29, x30, [sp, #-160]!",
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
            "mov x19, #0x19", "movk x19, #0xE9E9, lsl #48",
            "mov x20, #0x20", "movk x20, #0xE9E9, lsl #48",
            "mov x21, #0x21", "movk x21, #0xE9E9, lsl #48",
            "mov x22, #0x22", "movk x22, #0xE9E9, lsl #48",
            "mov x23, #0x23", "movk x23, #0xE9E9, lsl #48",
            "mov x24, #0x24", "movk x24, #0xE9E9, lsl #48",
            "mov x25, #0x25", "movk x25, #0xE9E9, lsl #48",
            "mov x26, #0x26", "movk x26, #0xE9E9, lsl #48",
            "mov x27, #0x27", "movk x27, #0xE9E9, lsl #48",
            "mov x28, #0x28", "movk x28, #0xE9E9, lsl #48",
            "mov x29, #0x29", "movk x29, #0xE9E9, lsl #48",
            "mov x9, #0x08", "movk x9, #0xE9D8, lsl #48", "fmov d8, x9",
            "mov x9, #0x09", "movk x9, #0xE9D8, lsl #48", "fmov d9, x9",
            "mov x9, #0x10", "movk x9, #0xE9D8, lsl #48", "fmov d10, x9",
            "mov x9, #0x11", "movk x9, #0xE9D8, lsl #48", "fmov d11, x9",
            "mov x9, #0x12", "movk x9, #0xE9D8, lsl #48", "fmov d12, x9",
            "mov x9, #0x13", "movk x9, #0xE9D8, lsl #48", "fmov d13, x9",
            "mov x9, #0x14", "movk x9, #0xE9D8, lsl #48", "fmov d14, x9",
            "mov x9, #0x15", "movk x9, #0xE9D8, lsl #48", "fmov d15, x9",
            // x0 = ctx, x1 = cb are still the arguments.
            "bl {spill}",
            "mov x10, #0",
            "mov x9, #0x19", "movk x9, #0xE9E9, lsl #48", "cmp x19, x9", "cinc x10, x10, ne",
            "mov x9, #0x20", "movk x9, #0xE9E9, lsl #48", "cmp x20, x9", "cinc x10, x10, ne",
            "mov x9, #0x21", "movk x9, #0xE9E9, lsl #48", "cmp x21, x9", "cinc x10, x10, ne",
            "mov x9, #0x22", "movk x9, #0xE9E9, lsl #48", "cmp x22, x9", "cinc x10, x10, ne",
            "mov x9, #0x23", "movk x9, #0xE9E9, lsl #48", "cmp x23, x9", "cinc x10, x10, ne",
            "mov x9, #0x24", "movk x9, #0xE9E9, lsl #48", "cmp x24, x9", "cinc x10, x10, ne",
            "mov x9, #0x25", "movk x9, #0xE9E9, lsl #48", "cmp x25, x9", "cinc x10, x10, ne",
            "mov x9, #0x26", "movk x9, #0xE9E9, lsl #48", "cmp x26, x9", "cinc x10, x10, ne",
            "mov x9, #0x27", "movk x9, #0xE9E9, lsl #48", "cmp x27, x9", "cinc x10, x10, ne",
            "mov x9, #0x28", "movk x9, #0xE9E9, lsl #48", "cmp x28, x9", "cinc x10, x10, ne",
            "mov x9, #0x29", "movk x9, #0xE9E9, lsl #48", "cmp x29, x9", "cinc x10, x10, ne",
            "mov x9, #0x08", "movk x9, #0xE9D8, lsl #48", "fmov x11, d8", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x09", "movk x9, #0xE9D8, lsl #48", "fmov x11, d9", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x10", "movk x9, #0xE9D8, lsl #48", "fmov x11, d10", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x11", "movk x9, #0xE9D8, lsl #48", "fmov x11, d11", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x12", "movk x9, #0xE9D8, lsl #48", "fmov x11, d12", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x13", "movk x9, #0xE9D8, lsl #48", "fmov x11, d13", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x14", "movk x9, #0xE9D8, lsl #48", "fmov x11, d14", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x9, #0x15", "movk x9, #0xE9D8, lsl #48", "fmov x11, d15", "cmp x11, x9", "cinc x10, x10, ne",
            "mov x0, x10",
            "ldp x19, x20, [sp, #16]",
            "ldp x21, x22, [sp, #32]",
            "ldp x23, x24, [sp, #48]",
            "ldp x25, x26, [sp, #64]",
            "ldp x27, x28, [sp, #80]",
            "ldp d8, d9, [sp, #96]",
            "ldp d10, d11, [sp, #112]",
            "ldp d12, d13, [sp, #128]",
            "ldp d14, d15, [sp, #144]",
            "ldp x29, x30, [sp], #160",
            "ret",
            spill = sym ep9_spill_aapcs64,
        )
    }

    pub(super) const EXPECTED: [(&str, u64); 19] = [
        ("x19", gp(0x19)),
        ("x20", gp(0x20)),
        ("x21", gp(0x21)),
        ("x22", gp(0x22)),
        ("x23", gp(0x23)),
        ("x24", gp(0x24)),
        ("x25", gp(0x25)),
        ("x26", gp(0x26)),
        ("x27", gp(0x27)),
        ("x28", gp(0x28)),
        ("x29", gp(0x29)),
        ("d8", fp(0x08)),
        ("d9", fp(0x09)),
        ("d10", fp(0x10)),
        ("d11", fp(0x11)),
        ("d12", fp(0x12)),
        ("d13", fp(0x13)),
        ("d14", fp(0x14)),
        ("d15", fp(0x15)),
    ];
}

#[cfg(target_arch = "aarch64")]
#[test]
fn aapcs64_spills_every_callee_saved_register() {
    let mut seen = Seen {
        abi: SpillAbi::Aapcs64,
        sp: 0,
        area: vec![],
    };
    // SAFETY: `a64::cb` accepts `&mut seen` as ctx; the helper keeps the AAPCS64 contract.
    let n = unsafe { a64::fill_and_spill((&raw mut seen).cast(), a64::cb) };
    check(&seen, &a64::EXPECTED, n);
}

#[cfg(all(target_arch = "x86_64", not(windows)))]
mod sysv {
    use super::*;
    use ep9_platform::spill::ep9_spill_sysv64;

    pub(super) unsafe extern "sysv64" fn cb(ctx: *mut c_void, sp: usize) {
        record(ctx, sp)
    }

    /// Fills rbx, rbp, r12-r15, calls the trampoline, returns the count of changed registers.
    #[unsafe(naked)]
    pub(super) unsafe extern "sysv64" fn fill_and_spill(
        ctx: *mut c_void,
        cb: unsafe extern "sysv64" fn(*mut c_void, usize),
    ) -> u64 {
        core::arch::naked_asm!(
            "push rbp",
            "push rbx",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "sub rsp, 8",
            "mov rbx, 0xE9E90000000000B0",
            "mov rbp, 0xE9E90000000000B5",
            "mov r12, 0xE9E9000000000012",
            "mov r13, 0xE9E9000000000013",
            "mov r14, 0xE9E9000000000014",
            "mov r15, 0xE9E9000000000015",
            // rdi = ctx, rsi = cb are still the arguments.
            "call {spill}",
            "xor eax, eax",
            "xor edx, edx",
            "mov rcx, 0xE9E90000000000B0", "cmp rbx, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E90000000000B5", "cmp rbp, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000012", "cmp r12, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000013", "cmp r13, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000014", "cmp r14, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000015", "cmp r15, rcx", "setne dl", "add rax, rdx",
            "add rsp, 8",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbx",
            "pop rbp",
            "ret",
            spill = sym ep9_spill_sysv64,
        )
    }

    pub(super) const EXPECTED: [(&str, u64); 6] = [
        ("rbx", gp(0xB0)),
        ("rbp", gp(0xB5)),
        ("r12", gp(0x12)),
        ("r13", gp(0x13)),
        ("r14", gp(0x14)),
        ("r15", gp(0x15)),
    ];
}

#[cfg(all(target_arch = "x86_64", not(windows)))]
#[test]
fn sysv64_spills_every_callee_saved_register() {
    let mut seen = Seen {
        abi: SpillAbi::SysV64,
        sp: 0,
        area: vec![],
    };
    // SAFETY: `sysv::cb` accepts `&mut seen` as ctx; the helper keeps the SysV contract.
    let n = unsafe { sysv::fill_and_spill((&raw mut seen).cast(), sysv::cb) };
    check(&seen, &sysv::EXPECTED, n);
}

#[cfg(target_arch = "x86_64")]
mod win64 {
    use super::*;
    use ep9_platform::spill::ep9_spill_win64;

    pub(super) unsafe extern "win64" fn cb(ctx: *mut c_void, sp: usize) {
        record(ctx, sp)
    }

    /// Fills rbx, rbp, rdi, rsi, r12-r15 and xmm6-xmm15, calls the trampoline, returns the
    /// count of changed registers.
    #[unsafe(naked)]
    pub(super) unsafe extern "win64" fn fill_and_spill(
        ctx: *mut c_void,
        cb: unsafe extern "win64" fn(*mut c_void, usize),
    ) -> u64 {
        core::arch::naked_asm!(
            "push rbp",
            "push rbx",
            "push rdi",
            "push rsi",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "sub rsp, 200",
            "movaps xmmword ptr [rsp + 32], xmm6",
            "movaps xmmword ptr [rsp + 48], xmm7",
            "movaps xmmword ptr [rsp + 64], xmm8",
            "movaps xmmword ptr [rsp + 80], xmm9",
            "movaps xmmword ptr [rsp + 96], xmm10",
            "movaps xmmword ptr [rsp + 112], xmm11",
            "movaps xmmword ptr [rsp + 128], xmm12",
            "movaps xmmword ptr [rsp + 144], xmm13",
            "movaps xmmword ptr [rsp + 160], xmm14",
            "movaps xmmword ptr [rsp + 176], xmm15",
            "mov rbx, 0xE9E90000000000B0",
            "mov rbp, 0xE9E90000000000B5",
            "mov rdi, 0xE9E90000000000D1",
            "mov rsi, 0xE9E9000000000051",
            "mov r12, 0xE9E9000000000012",
            "mov r13, 0xE9E9000000000013",
            "mov r14, 0xE9E9000000000014",
            "mov r15, 0xE9E9000000000015",
            "mov rax, 0xE9D8000000000006", "movq xmm6, rax",
            "mov rax, 0xE9D8000000000007", "movq xmm7, rax",
            "mov rax, 0xE9D8000000000008", "movq xmm8, rax",
            "mov rax, 0xE9D8000000000009", "movq xmm9, rax",
            "mov rax, 0xE9D8000000000010", "movq xmm10, rax",
            "mov rax, 0xE9D8000000000011", "movq xmm11, rax",
            "mov rax, 0xE9D8000000000012", "movq xmm12, rax",
            "mov rax, 0xE9D8000000000013", "movq xmm13, rax",
            "mov rax, 0xE9D8000000000014", "movq xmm14, rax",
            "mov rax, 0xE9D8000000000015", "movq xmm15, rax",
            // rcx = ctx, rdx = cb are still the arguments; [rsp, rsp + 32) is shadow space.
            "call {spill}",
            "xor eax, eax",
            "xor edx, edx",
            "mov rcx, 0xE9E90000000000B0", "cmp rbx, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E90000000000B5", "cmp rbp, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E90000000000D1", "cmp rdi, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000051", "cmp rsi, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000012", "cmp r12, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000013", "cmp r13, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000014", "cmp r14, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9E9000000000015", "cmp r15, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000006", "movq r8, xmm6", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000007", "movq r8, xmm7", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000008", "movq r8, xmm8", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000009", "movq r8, xmm9", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000010", "movq r8, xmm10", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000011", "movq r8, xmm11", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000012", "movq r8, xmm12", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000013", "movq r8, xmm13", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000014", "movq r8, xmm14", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "mov rcx, 0xE9D8000000000015", "movq r8, xmm15", "cmp r8, rcx", "setne dl", "add rax, rdx",
            "movaps xmm6, xmmword ptr [rsp + 32]",
            "movaps xmm7, xmmword ptr [rsp + 48]",
            "movaps xmm8, xmmword ptr [rsp + 64]",
            "movaps xmm9, xmmword ptr [rsp + 80]",
            "movaps xmm10, xmmword ptr [rsp + 96]",
            "movaps xmm11, xmmword ptr [rsp + 112]",
            "movaps xmm12, xmmword ptr [rsp + 128]",
            "movaps xmm13, xmmword ptr [rsp + 144]",
            "movaps xmm14, xmmword ptr [rsp + 160]",
            "movaps xmm15, xmmword ptr [rsp + 176]",
            "add rsp, 200",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rsi",
            "pop rdi",
            "pop rbx",
            "pop rbp",
            "ret",
            spill = sym ep9_spill_win64,
        )
    }

    pub(super) const EXPECTED: [(&str, u64); 18] = [
        ("rbx", gp(0xB0)),
        ("rbp", gp(0xB5)),
        ("rdi", gp(0xD1)),
        ("rsi", gp(0x51)),
        ("r12", gp(0x12)),
        ("r13", gp(0x13)),
        ("r14", gp(0x14)),
        ("r15", gp(0x15)),
        ("xmm6", fp(0x06)),
        ("xmm7", fp(0x07)),
        ("xmm8", fp(0x08)),
        ("xmm9", fp(0x09)),
        ("xmm10", fp(0x10)),
        ("xmm11", fp(0x11)),
        ("xmm12", fp(0x12)),
        ("xmm13", fp(0x13)),
        ("xmm14", fp(0x14)),
        ("xmm15", fp(0x15)),
    ];
}

/// Runs natively on Windows, and on macOS and Linux x86-64 through `extern "win64"`.
#[cfg(target_arch = "x86_64")]
#[test]
fn win64_spills_every_callee_saved_register() {
    let mut seen = Seen {
        abi: SpillAbi::Win64,
        sp: 0,
        area: vec![],
    };
    // SAFETY: `win64::cb` accepts `&mut seen` as ctx; the helper keeps the Win64 contract.
    let n = unsafe { win64::fill_and_spill((&raw mut seen).cast(), win64::cb) };
    check(&seen, &win64::EXPECTED, n);
}

#[test]
fn wrapper_returns_value_and_sp_is_on_this_stack() {
    let bounds = current_thread_stack_bounds().unwrap();
    for &abi in SpillAbi::available() {
        let outer = stack_pointer();
        let (sp, area) = with_spilled_registers_abi(abi, |regs| (regs.sp(), regs.spill_area()));
        assert!(
            bounds.contains(sp),
            "{}: sp {sp:#x} is not on this stack",
            abi.name()
        );
        assert!(
            sp < outer,
            "{}: the spill area must be below the caller",
            abi.name()
        );
        assert_eq!(area.start, sp);
        assert_eq!(area.len(), abi.spill_area_bytes());
        assert_eq!(sp % 16, 0);
    }
    assert_eq!(with_spilled_registers(|r| r.abi()), SpillAbi::HOST);
}

#[test]
fn wrapper_nests() {
    let (a, b) = with_spilled_registers(|outer| {
        let inner = with_spilled_registers(|inner| inner.sp());
        (outer.sp(), inner)
    });
    assert!(b < a, "the inner spill area is deeper");
}

#[test]
fn wrapper_resumes_panics() {
    for &abi in SpillAbi::available() {
        let r = std::panic::catch_unwind(|| {
            with_spilled_registers_abi(abi, |_| -> u32 { panic!("boom from {}", abi.name()) })
        });
        let msg = r.unwrap_err().downcast::<String>().unwrap();
        assert_eq!(*msg, format!("boom from {}", abi.name()));
    }
}
