//! Naked helpers per ABI for the deterministic probes and the no-spill control.
//!
//! | Helper | What it does |
//! |---|---|
//! | `no_spill(ctx, cb)` | calls `cb(ctx, entry sp)` with no register spill: the negative control |
//! | `hold(ctx, cb, value, tramp)` | puts `value` only in a callee-saved register (x19, rbx, or rsi on Win64), calls `tramp(ctx, cb)`, returns the register |
//!
//! The GC callbacks `gc_cb_*` run the collection on the `Mutator` behind `ctx`.

use std::ffi::c_void;

use ep9_platform::SpillAbi;

use crate::mutator::Mutator;

/// Runs a collection for an asm helper.
///
/// # Safety
/// `ctx` is the `&mut Mutator` that the caller passed to the helper.
unsafe fn gc_from_asm(ctx: *mut c_void, sp: usize) {
    // SAFETY: the caller guarantees `ctx` is a live `Mutator`; nothing else uses it during the
    // helper call.
    let m = unsafe { &mut *ctx.cast::<Mutator>() };
    m.gc_from_asm(sp);
}

#[cfg(target_arch = "aarch64")]
pub mod a64 {
    use super::*;

    /// # Safety
    /// See [`gc_from_asm`].
    pub unsafe extern "C" fn gc_cb(ctx: *mut c_void, sp: usize) {
        // SAFETY: forwarded caller contract.
        unsafe { gc_from_asm(ctx, sp) }
    }

    /// # Safety
    /// `cb` may be called with `ctx`.
    #[unsafe(naked)]
    pub unsafe extern "C" fn no_spill(
        ctx: *mut c_void,
        cb: unsafe extern "C" fn(*mut c_void, usize),
    ) {
        core::arch::naked_asm!(
            "stp x29, x30, [sp, #-16]!",
            "mov x29, sp",
            "mov x9, x1",
            // The entry sp: our own fp/lr pair is not scanned, nor is `cb`'s frame.
            "add x1, sp, #16",
            "blr x9",
            "ldp x29, x30, [sp], #16",
            "ret",
        )
    }

    /// # Safety
    /// `tramp(ctx, cb)` is safe to call.
    #[unsafe(naked)]
    pub unsafe extern "C" fn hold(
        ctx: *mut c_void,
        cb: unsafe extern "C" fn(*mut c_void, usize),
        value: usize,
        tramp: unsafe extern "C" fn(*mut c_void, unsafe extern "C" fn(*mut c_void, usize)),
    ) -> usize {
        core::arch::naked_asm!(
            "stp x29, x30, [sp, #-32]!",
            "mov x29, sp",
            "str x19, [sp, #16]",
            "mov x19, x2",
            "mov x2, #0",
            "blr x3",
            "mov x0, x19",
            "ldr x19, [sp, #16]",
            "ldp x29, x30, [sp], #32",
            "ret",
        )
    }

    /// `value` kept only in x19 across a collection that uses `abi`'s trampoline
    /// (`spill = true`) or none (`spill = false`).
    pub fn hold_in_register(m: &mut Mutator, value: usize, spill: bool) -> usize {
        m.set_asm_spill(spill.then_some(SpillAbi::Aapcs64));
        let tramp = if spill {
            ep9_platform::spill::ep9_spill_aapcs64
        } else {
            no_spill
        };
        // SAFETY: `gc_cb` matches `ctx` = this `Mutator`; both trampolines call it once.
        unsafe { hold((m as *mut Mutator).cast(), gc_cb, value, tramp) }
    }
}

#[cfg(all(target_arch = "x86_64", not(windows)))]
pub mod sysv {
    use super::*;

    /// # Safety
    /// See [`gc_from_asm`].
    pub unsafe extern "sysv64" fn gc_cb(ctx: *mut c_void, sp: usize) {
        // SAFETY: forwarded caller contract.
        unsafe { gc_from_asm(ctx, sp) }
    }

    /// # Safety
    /// `cb` may be called with `ctx`.
    #[unsafe(naked)]
    pub unsafe extern "sysv64" fn no_spill(
        ctx: *mut c_void,
        cb: unsafe extern "sysv64" fn(*mut c_void, usize),
    ) {
        core::arch::naked_asm!(
            "push rbp",
            "mov rax, rsi",
            // The entry sp (the return-address slot).
            "lea rsi, [rsp + 8]",
            "call rax",
            "pop rbp",
            "ret",
        )
    }

    /// # Safety
    /// `tramp(ctx, cb)` is safe to call.
    #[unsafe(naked)]
    pub unsafe extern "sysv64" fn hold(
        ctx: *mut c_void,
        cb: unsafe extern "sysv64" fn(*mut c_void, usize),
        value: usize,
        tramp: unsafe extern "sysv64" fn(
            *mut c_void,
            unsafe extern "sysv64" fn(*mut c_void, usize),
        ),
    ) -> usize {
        core::arch::naked_asm!(
            "push rbx",
            "mov rbx, rdx",
            "xor edx, edx",
            "call rcx",
            "mov rax, rbx",
            "pop rbx",
            "ret",
        )
    }

    pub fn hold_in_register(m: &mut Mutator, value: usize, spill: bool) -> usize {
        m.set_asm_spill(spill.then_some(SpillAbi::SysV64));
        let tramp = if spill {
            ep9_platform::spill::ep9_spill_sysv64
        } else {
            no_spill
        };
        // SAFETY: `gc_cb` matches `ctx` = this `Mutator`; both trampolines call it once.
        unsafe { hold((m as *mut Mutator).cast(), gc_cb, value, tramp) }
    }
}

#[cfg(target_arch = "x86_64")]
pub mod win64 {
    use super::*;

    /// # Safety
    /// See [`gc_from_asm`].
    pub unsafe extern "win64" fn gc_cb(ctx: *mut c_void, sp: usize) {
        // SAFETY: forwarded caller contract.
        unsafe { gc_from_asm(ctx, sp) }
    }

    /// # Safety
    /// `cb` may be called with `ctx`.
    #[unsafe(naked)]
    pub unsafe extern "win64" fn no_spill(
        ctx: *mut c_void,
        cb: unsafe extern "win64" fn(*mut c_void, usize),
    ) {
        core::arch::naked_asm!(
            "push rbp",
            "sub rsp, 32",
            "mov rax, rdx",
            // The entry sp (the return-address slot).
            "lea rdx, [rsp + 40]",
            "call rax",
            "add rsp, 32",
            "pop rbp",
            "ret",
        )
    }

    /// Keeps `value` in rsi: callee-saved on Win64 only, so only the Win64 trampoline saves it.
    ///
    /// # Safety
    /// `tramp(ctx, cb)` is safe to call.
    #[unsafe(naked)]
    pub unsafe extern "win64" fn hold(
        ctx: *mut c_void,
        cb: unsafe extern "win64" fn(*mut c_void, usize),
        value: usize,
        tramp: unsafe extern "win64" fn(*mut c_void, unsafe extern "win64" fn(*mut c_void, usize)),
    ) -> usize {
        core::arch::naked_asm!(
            "push rsi",
            "sub rsp, 32",
            "mov rsi, r8",
            "xor r8d, r8d",
            "call r9",
            "mov rax, rsi",
            "add rsp, 32",
            "pop rsi",
            "ret",
        )
    }

    pub fn hold_in_register(m: &mut Mutator, value: usize, spill: bool) -> usize {
        m.set_asm_spill(spill.then_some(SpillAbi::Win64));
        let tramp = if spill {
            ep9_platform::spill::ep9_spill_win64
        } else {
            no_spill
        };
        // SAFETY: `gc_cb` matches `ctx` = this `Mutator`; both trampolines call it once.
        unsafe { hold((m as *mut Mutator).cast(), gc_cb, value, tramp) }
    }
}

/// Zeroes the 16 KiB of dead stack below the caller's sp. Frameless and uses only scratch
/// registers, so it leaves no copy of anything behind.
///
/// Why: a conservative scan also reads uninitialized slots of live frames (for example the
/// `out` payload of the trampoline call record). Such a slot can hold a stale address from a
/// frame that returned earlier at the same depth. That only keeps extra objects alive, so it
/// never hides a miss; but the "this form is not a root" probes need a clean stack.
/// The loops go from high to low addresses, so Windows guard pages are touched in order.
///
/// # Safety
/// At least 16 KiB of stack must be free below the caller's sp.
#[unsafe(naked)]
pub unsafe extern "C" fn scrub_stack() {
    #[cfg(target_arch = "aarch64")]
    core::arch::naked_asm!(
        "mov x9, sp",
        "sub x10, x9, #4, lsl #12",
        "2:",
        "stp xzr, xzr, [x9, #-16]!",
        "cmp x9, x10",
        "b.hi 2b",
        "ret",
    );
    #[cfg(target_arch = "x86_64")]
    core::arch::naked_asm!(
        "mov rax, rsp",
        "lea rcx, [rsp - 16384]",
        "2:",
        "sub rax, 8",
        "mov qword ptr [rax], 0",
        "cmp rax, rcx",
        "ja 2b",
        "ret",
    );
}

/// Keeps `value` only in a callee-saved register of `abi` across one collection.
/// Returns the register value after the collection.
#[inline(never)]
pub fn hold_in_register(m: &mut Mutator, abi: SpillAbi, value: usize, spill: bool) -> usize {
    match abi {
        #[cfg(target_arch = "aarch64")]
        SpillAbi::Aapcs64 => a64::hold_in_register(m, value, spill),
        #[cfg(all(target_arch = "x86_64", not(windows)))]
        SpillAbi::SysV64 => sysv::hold_in_register(m, value, spill),
        #[cfg(target_arch = "x86_64")]
        SpillAbi::Win64 => win64::hold_in_register(m, value, spill),
        #[allow(unreachable_patterns)]
        other => panic!("{} helpers are not built for this target", other.name()),
    }
}

/// Runs one collection through the no-spill control helper of `abi`.
#[inline(never)]
pub fn collect_without_spill(m: &mut Mutator, abi: SpillAbi) {
    m.set_asm_spill(None);
    let ctx = (m as *mut Mutator).cast::<c_void>();
    match abi {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: `gc_cb` matches `ctx` = this `Mutator`.
        SpillAbi::Aapcs64 => unsafe { a64::no_spill(ctx, a64::gc_cb) },
        #[cfg(all(target_arch = "x86_64", not(windows)))]
        // SAFETY: as above.
        SpillAbi::SysV64 => unsafe { sysv::no_spill(ctx, sysv::gc_cb) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: as above.
        SpillAbi::Win64 => unsafe { win64::no_spill(ctx, win64::gc_cb) },
        #[allow(unreachable_patterns)]
        other => panic!("{} helpers are not built for this target", other.name()),
    }
}
