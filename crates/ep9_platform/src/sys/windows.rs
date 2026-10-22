//! Windows backend (x86_64-pc-windows-msvc is tier 1).

use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{ERROR_INVALID_ADDRESS, GetLastError};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READWRITE,
    VirtualAlloc, VirtualFree,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits;

/// `(page size, allocation granularity, highest user address)`.
fn system_info() -> (usize, usize, usize) {
    static INFO: OnceLock<(usize, usize, usize)> = OnceLock::new();
    *INFO.get_or_init(|| {
        let mut si = SYSTEM_INFO::default();
        // SAFETY: GetSystemInfo writes the struct behind a valid pointer and cannot fail.
        unsafe { GetSystemInfo(&mut si) };
        (
            si.dwPageSize as usize,
            si.dwAllocationGranularity as usize,
            si.lpMaximumApplicationAddress as usize,
        )
    })
}

pub(crate) fn page_size() -> usize {
    system_info().0
}

/// Reservation base addresses are multiples of this (64 KiB on all current Windows).
pub(crate) fn granularity() -> usize {
    system_info().1
}

/// Highest byte a reservation can cover (`lpMaximumApplicationAddress`): 0x7FFF_FFFE_FFFF on
/// x64 Windows 8.1 and later, 64 KiB + 1 byte below 2^47.
fn max_address() -> usize {
    system_info().2
}

/// Reserves `size` bytes. `hint` = 0 lets the system choose. With a hint, the call fails
/// unless the whole range at `hint` is free.
pub(crate) fn reserve(hint: usize, size: usize) -> io::Result<usize> {
    // SAFETY: MEM_RESERVE only takes free address space; it never changes existing pages.
    let p = unsafe { VirtualAlloc(hint as *const c_void, size, MEM_RESERVE, PAGE_NOACCESS) };
    if p.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(p as usize)
    }
}

/// Reserves exactly `[addr, addr + size)`. Returns `Ok(false)` if that range is not free or
/// ends above `max_address`.
///
/// `addr` and `size` are non-zero granularity multiples and do not wrap (`va::check_at`).
pub(crate) fn reserve_at(addr: usize, size: usize) -> io::Result<bool> {
    // VirtualAlloc fails a range that ends above the user address space with
    // ERROR_INVALID_PARAMETER, the same code as for a bad argument. A 64 GiB cage in the top
    // 4 GiB-aligned slot below 2^47 is such a range (CI run 39632519735). It is not free here.
    if addr + (size - 1) > max_address() {
        return Ok(false);
    }
    // SAFETY: as in `reserve`.
    let p = unsafe { VirtualAlloc(addr as *const c_void, size, MEM_RESERVE, PAGE_NOACCESS) };
    if p.is_null() {
        // SAFETY: GetLastError has no preconditions.
        let e = unsafe { GetLastError() };
        return if e == ERROR_INVALID_ADDRESS {
            Ok(false)
        } else {
            Err(io::Error::from_raw_os_error(e as i32))
        };
    }
    if p as usize == addr {
        return Ok(true);
    }
    // Not expected for an aligned `addr`, but never keep a range we did not ask for.
    // SAFETY: `p` is the reservation made just now.
    unsafe { release(p as usize, size)? };
    Ok(false)
}

/// # Safety
/// `addr` is the base of a reservation of this process, and no live reference points into it.
pub(crate) unsafe fn release(addr: usize, _size: usize) -> io::Result<()> {
    // SAFETY: the caller guarantees `addr` is the base of our reservation. MEM_RELEASE needs
    // size 0 and frees the whole reservation.
    let ok = unsafe { VirtualFree(addr as *mut c_void, 0, MEM_RELEASE) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Commits the pages as read-write. Contents of pages already committed stay.
///
/// # Safety
/// `[addr, addr + size)` is page-aligned and inside a reservation of this process.
pub(crate) unsafe fn commit(addr: usize, size: usize) -> io::Result<()> {
    // SAFETY: the caller guarantees the range is inside our reservation.
    let p = unsafe { VirtualAlloc(addr as *const c_void, size, MEM_COMMIT, PAGE_READWRITE) };
    if p.is_null() {
        Err(io::Error::last_os_error())
    } else if p as usize != addr {
        Err(io::Error::other(
            "VirtualAlloc(MEM_COMMIT) returned another address",
        ))
    } else {
        Ok(())
    }
}

/// Decommits the pages. The range stays reserved; a later commit gives zero-filled pages.
///
/// # Safety
/// `[addr, addr + size)` is page-aligned, inside a reservation of this process, and no live
/// reference points into it.
pub(crate) unsafe fn decommit(addr: usize, size: usize) -> io::Result<()> {
    // SAFETY: the caller guarantees the range is ours and unused.
    let ok = unsafe { VirtualFree(addr as *mut c_void, size, MEM_DECOMMIT) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `(low, high)` of the calling thread's stack. `high` is the first byte above the stack.
pub(crate) fn stack_bounds() -> io::Result<(usize, usize)> {
    let (mut low, mut high) = (0usize, 0usize);
    // SAFETY: both out pointers are valid locals. The call cannot fail (Windows 8 and later).
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    Ok((low, high))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::va::{CAGE_LIMIT, DEFAULT_CAGE_SIZE};

    /// x64 user space ends 64 KiB below 2^47, so a range that ends at 2^47 is never free.
    /// VirtualAlloc itself fails it with ERROR_INVALID_PARAMETER (87), not
    /// ERROR_INVALID_ADDRESS.
    #[test]
    fn range_ending_at_2_47_is_not_free() {
        assert!(max_address() < CAGE_LIMIT, "{:#x}", max_address());
        let g = granularity();
        for size in [g, DEFAULT_CAGE_SIZE] {
            assert!(
                !reserve_at(CAGE_LIMIT - size, size).unwrap(),
                "{size:#x} bytes below 2^47"
            );
        }
    }
}
