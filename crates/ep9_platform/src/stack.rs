//! Stack bounds of the current thread, for the conservative scan and stack-limit checks.

use std::io;

use crate::sys;

/// `[low, high)` of a thread stack. The stack grows down from `high`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackBounds {
    /// Lowest stack address the OS reserved (guard pages may lie at or above it).
    pub low: usize,
    /// First byte above the stack. A conservative scan reads `[sp, high)`.
    pub high: usize,
}

impl StackBounds {
    pub fn contains(&self, addr: usize) -> bool {
        self.low <= addr && addr < self.high
    }

    pub fn size(&self) -> usize {
        self.high - self.low
    }
}

/// Bounds of the calling thread's stack: `pthread_get_stackaddr_np` on macOS,
/// `pthread_getattr_np` on Linux, `GetCurrentThreadStackLimits` on Windows.
pub fn current_thread_stack_bounds() -> io::Result<StackBounds> {
    let (low, high) = sys::stack_bounds()?;
    Ok(StackBounds { low, high })
}

/// The stack pointer at the call site.
#[inline(always)]
pub fn stack_pointer() -> usize {
    let sp: usize;
    // SAFETY: the asm only copies the stack pointer into a register.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags))
    };
    // SAFETY: the asm only copies the stack pointer into a register.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags))
    };
    sp
}
