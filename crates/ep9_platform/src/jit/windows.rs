//! Windows backend of [`super`]: `VirtualAlloc` commit, `VirtualProtect` toggles,
//! `FlushInstructionCache`.

use std::ffi::c_void;
use std::io;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_DECOMMIT, PAGE_EXECUTE_READ, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
    VirtualAlloc, VirtualFree, VirtualProtect,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use super::WxMode;

#[derive(Debug, Default)]
pub(super) struct State;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "W^X mode not built for this target",
    )
}

/// # Safety
/// `[addr, addr + len)` is page-aligned and inside a reservation of this process.
unsafe fn protect(addr: usize, len: usize, prot: PAGE_PROTECTION_FLAGS) -> io::Result<()> {
    let mut old: PAGE_PROTECTION_FLAGS = 0;
    // SAFETY: the caller guarantees the range is committed and ours; `old` is a valid local.
    let ok = unsafe { VirtualProtect(addr as *const c_void, len, prot, &mut old) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// # Safety
/// `[exec, exec + len)` is page-aligned, inside a reservation of this process, and unused.
pub(super) unsafe fn commit(exec: usize, len: usize, mode: WxMode) -> io::Result<(usize, State)> {
    if mode != WxMode::Protect {
        return Err(unsupported());
    }
    // A reservation made with PAGE_NOACCESS accepts any protection at commit. New pages are
    // zero.
    // SAFETY: the caller guarantees the range is inside our reservation and unused.
    let p = unsafe { VirtualAlloc(exec as *const c_void, len, MEM_COMMIT, PAGE_EXECUTE_READ) };
    if p.is_null() {
        Err(io::Error::last_os_error())
    } else if p as usize != exec {
        Err(io::Error::other(
            "VirtualAlloc(MEM_COMMIT) returned another address",
        ))
    } else {
        Ok((exec, State))
    }
}

/// # Safety
/// The values come from one successful `commit`; no window is open; no code in the pages runs.
pub(super) unsafe fn release(
    exec: usize,
    _write: usize,
    len: usize,
    _mode: WxMode,
    _state: &mut State,
) -> io::Result<()> {
    // SAFETY: the caller guarantees the pages are ours and unused.
    let ok = unsafe { VirtualFree(exec as *mut c_void, len, MEM_DECOMMIT) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// # Safety
/// The pages belong to a committed memory of `mode`, and no other window of it is open.
pub(super) unsafe fn begin_write(mode: WxMode, addr: usize, len: usize) -> io::Result<()> {
    match mode {
        // SAFETY: forwarded caller contract.
        WxMode::Protect => unsafe { protect(addr, len, PAGE_READWRITE) },
        _ => Err(unsupported()),
    }
}

/// # Safety
/// The pages are the ones that the matching `begin_write` opened.
pub(super) unsafe fn end_write(mode: WxMode, addr: usize, len: usize) -> io::Result<()> {
    match mode {
        // SAFETY: forwarded caller contract.
        WxMode::Protect => unsafe { protect(addr, len, PAGE_EXECUTE_READ) },
        _ => Err(unsupported()),
    }
}

pub(super) fn flush_icache(addr: usize, len: usize) {
    // SAFETY: FlushInstructionCache only runs cache maintenance for our own process; it does
    // not write memory. The pseudo handle of GetCurrentProcess needs no close.
    unsafe { FlushInstructionCache(GetCurrentProcess(), addr as *const c_void, len) };
}
