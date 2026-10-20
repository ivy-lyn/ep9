//! unix backend of [`super`]: mprotect, MAP_JIT (macOS arm64), memfd dual mapping (Linux).

use std::ffi::c_void;
use std::io;

use super::WxMode;
use crate::sys;

/// Per-memory OS state: the memfd of a dual mapping.
#[derive(Debug, Default)]
pub(super) struct State {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    memfd: Option<std::os::fd::OwnedFd>,
}

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// # Safety
/// `[addr, addr + len)` is page-aligned, inside a reservation of this process, and unused.
unsafe fn mprotect(addr: usize, len: usize, prot: libc::c_int) -> io::Result<()> {
    // SAFETY: the caller guarantees the range is ours and unused.
    check(unsafe { libc::mprotect(addr as *mut c_void, len, prot) })
}

const RX: libc::c_int = libc::PROT_READ | libc::PROT_EXEC;
const RW: libc::c_int = libc::PROT_READ | libc::PROT_WRITE;

/// Commits the pages. Returns the write address and the OS state.
///
/// # Safety
/// `[exec, exec + len)` is page-aligned, inside a reservation of this process that suits
/// `mode` (MAP_JIT or plain), and unused.
pub(super) unsafe fn commit(exec: usize, len: usize, mode: WxMode) -> io::Result<(usize, State)> {
    match mode {
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        WxMode::MapJit => {
            // The pages are RWX since the reservation (va module doc). Undo a REUSABLE hint
            // from an earlier release; a fresh page accepts it too (measured).
            // SAFETY: madvise only changes paging hints of our own pages.
            check(unsafe { libc::madvise(exec as *mut c_void, len, libc::MADV_FREE_REUSE) })?;
            Ok((exec, State::default()))
        }
        WxMode::Protect => {
            // SAFETY: forwarded caller contract. Untouched pages read as zero.
            unsafe { mprotect(exec, len, RX)? };
            Ok((exec, State::default()))
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        WxMode::DualMap => {
            // SAFETY: forwarded caller contract.
            unsafe { dual::map(exec, len) }
        }
        #[allow(unreachable_patterns)]
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "W^X mode not built for this target",
        )),
    }
}

/// Gives the pages back. The range stays reserved.
///
/// # Safety
/// The values come from one successful `commit`; no window is open; no code in the pages runs.
pub(super) unsafe fn release(
    exec: usize,
    write: usize,
    len: usize,
    mode: WxMode,
    state: &mut State,
) -> io::Result<()> {
    let _ = (write, &state);
    match mode {
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        WxMode::MapJit => {
            // MAP_JIT pages cannot leave RWX (EACCES), so only drop their contents lazily.
            // SAFETY: madvise only changes paging hints of our own pages.
            check(unsafe { libc::madvise(exec as *mut c_void, len, libc::MADV_FREE_REUSABLE) })
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        WxMode::DualMap => {
            // SAFETY: `write` is the RW view that `dual::map` created; nobody uses it now.
            check(unsafe { libc::munmap(write as *mut c_void, len) })?;
            // SAFETY: the RX view is inside our reservation; decommit puts an inaccessible
            // private mapping in its place. The memfd closes when `state` drops.
            let r = unsafe { sys::decommit(exec, len) };
            state.memfd = None;
            r
        }
        // SAFETY: forwarded caller contract.
        _ => unsafe { sys::decommit(exec, len) },
    }
}

/// Opens write access to the pages.
///
/// # Safety
/// The pages belong to a committed memory of `mode`, and no other window of it is open.
pub(super) unsafe fn begin_write(mode: WxMode, addr: usize, len: usize) -> io::Result<()> {
    match mode {
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        WxMode::MapJit => {
            super::map_jit_nest(true, map_jit_writable);
            let _ = (addr, len);
            Ok(())
        }
        // SAFETY: forwarded caller contract.
        WxMode::Protect => unsafe { mprotect(addr, len, RW) },
        _ => Ok(()),
    }
}

/// Closes write access to the pages.
///
/// # Safety
/// The pages are the ones that the matching `begin_write` opened.
pub(super) unsafe fn end_write(mode: WxMode, addr: usize, len: usize) -> io::Result<()> {
    match mode {
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        WxMode::MapJit => {
            super::map_jit_nest(false, map_jit_writable);
            let _ = (addr, len);
            Ok(())
        }
        // SAFETY: forwarded caller contract.
        WxMode::Protect => unsafe { mprotect(addr, len, RX) },
        _ => Ok(()),
    }
}

/// `true`: this thread may write `MAP_JIT` pages and may not run them.
#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
fn map_jit_writable(writable: bool) {
    // SAFETY: the call only switches this thread's view of MAP_JIT pages (APRR). The caller
    // keeps generated code from running on this thread while it is writable.
    unsafe { libc::pthread_jit_write_protect_np(if writable { 0 } else { 1 }) };
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    /// libkern (libSystem): makes instruction fetch see new bytes in `[start, start + len)`.
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
}

#[cfg(target_vendor = "apple")]
pub(super) fn flush_icache(addr: usize, len: usize) {
    // SAFETY: sys_icache_invalidate only runs cache maintenance on the range; it does not
    // write memory. A range that is not mapped is the caller's bug and faults.
    unsafe { sys_icache_invalidate(addr as *mut c_void, len) };
}

/// The compiler-rt `__clear_cache` sequence for AArch64 (it honors CTR_EL0.IDC and DIC).
#[cfg(all(not(target_vendor = "apple"), target_arch = "aarch64"))]
pub(super) fn flush_icache(addr: usize, len: usize) {
    use core::arch::asm;
    let ctr: u64;
    // SAFETY: reading CTR_EL0 is allowed at EL0 on Linux (the kernel traps and emulates it when
    // SCTLR_EL1.UCT is clear).
    unsafe { asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags)) };
    let end = addr + len;
    if (ctr >> 28) & 1 == 0 {
        // IDC = 0: clean the data cache to the point of unification.
        let line = 4usize << ((ctr >> 16) & 0xf);
        let mut a = addr & !(line - 1);
        while a < end {
            // SAFETY: DC CVAU only cleans a cache line; the address is mapped (caller).
            unsafe { asm!("dc cvau, {}", in(reg) a, options(nostack, preserves_flags)) };
            a += line;
        }
    }
    // SAFETY: a barrier has no memory-safety effect.
    unsafe { asm!("dsb ish", options(nostack, preserves_flags)) };
    if (ctr >> 29) & 1 == 0 {
        // DIC = 0: invalidate the instruction cache to the point of unification.
        let line = 4usize << (ctr & 0xf);
        let mut a = addr & !(line - 1);
        while a < end {
            // SAFETY: IC IVAU only invalidates a cache line; the address is mapped (caller).
            unsafe { asm!("ic ivau, {}", in(reg) a, options(nostack, preserves_flags)) };
            a += line;
        }
        // SAFETY: a barrier has no memory-safety effect.
        unsafe { asm!("dsb ish", options(nostack, preserves_flags)) };
    }
    // SAFETY: an instruction barrier has no memory-safety effect.
    unsafe { asm!("isb", options(nostack, preserves_flags)) };
}

/// x86-64 keeps instruction fetch coherent with stores of the same thread.
#[cfg(all(not(target_vendor = "apple"), target_arch = "x86_64"))]
pub(super) fn flush_icache(_addr: usize, _len: usize) {
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod dual {
    use std::ffi::c_void;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::{RW, RX, State, check};
    use crate::sys;

    /// Linux 6.3+: an executable memfd. With `vm.memfd_noexec = 2` a memfd without it cannot
    /// be mapped executable. Older kernels reject the flag (EINVAL); then we retry without it.
    const MFD_EXEC: libc::c_uint = 0x0010;

    fn memfd() -> io::Result<OwnedFd> {
        let name = c"ep9-jit";
        // SAFETY: `name` is a valid C string; the flags are plain values.
        let mut fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | MFD_EXEC) };
        if fd < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL) {
            // SAFETY: as above.
            fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        }
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// Maps a new memfd as RX over `[exec, exec + len)` and as RW somewhere else.
    ///
    /// # Safety
    /// `[exec, exec + len)` is page-aligned, inside our reservation, and unused.
    pub(super) unsafe fn map(exec: usize, len: usize) -> io::Result<(usize, State)> {
        let fd = memfd()?;
        let size = libc::off_t::try_from(len).map_err(|_| io::Error::other("length too large"))?;
        // SAFETY: `fd` is our memfd.
        check(unsafe { libc::ftruncate(fd.as_raw_fd(), size) })?;
        // SAFETY: MAP_FIXED replaces only pages of our own reservation (caller contract).
        let x = unsafe {
            libc::mmap(
                exec as *mut c_void,
                len,
                RX,
                libc::MAP_SHARED | libc::MAP_FIXED,
                fd.as_raw_fd(),
                0,
            )
        };
        if x == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: without MAP_FIXED, mmap only takes free address space.
        let w = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                RW,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if w == libc::MAP_FAILED {
            let e = io::Error::last_os_error();
            // SAFETY: put the inaccessible reservation back over the RX view we just mapped.
            let _ = unsafe { sys::decommit(exec, len) };
            return Err(e);
        }
        Ok((w as usize, State { memfd: Some(fd) }))
    }
}
