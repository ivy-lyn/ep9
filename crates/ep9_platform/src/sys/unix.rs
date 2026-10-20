//! unix backend: macOS and Linux are tier 1. Other unix systems get the VA calls only.

use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;

// Linux counts writable private mappings against the commit limit unless MAP_NORESERVE is set.
// macOS ignores the flag, so it is left out there.
#[cfg(any(target_os = "linux", target_os = "android"))]
const NORESERVE: libc::c_int = libc::MAP_NORESERVE;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const NORESERVE: libc::c_int = 0;

const ANON_PRIVATE: libc::c_int = libc::MAP_PRIVATE | libc::MAP_ANON | NORESERVE;

pub(crate) fn page_size() -> usize {
    static PAGE: OnceLock<usize> = OnceLock::new();
    *PAGE.get_or_init(|| {
        // SAFETY: sysconf has no preconditions.
        let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(v > 0, "sysconf(_SC_PAGESIZE) failed");
        v as usize
    })
}

/// unix maps at page granularity.
pub(crate) fn granularity() -> usize {
    page_size()
}

/// Maps `size` bytes of inaccessible address space. `hint` = 0 lets the kernel choose.
/// The kernel may place the mapping somewhere else than `hint`.
pub(crate) fn reserve(hint: usize, size: usize) -> io::Result<usize> {
    // SAFETY: without MAP_FIXED, mmap only uses free address space, so no existing
    // mapping changes. The result is checked for MAP_FAILED.
    let p = unsafe {
        libc::mmap(
            hint as *mut c_void,
            size,
            libc::PROT_NONE,
            ANON_PRIVATE,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        Err(io::Error::last_os_error())
    } else {
        Ok(p as usize)
    }
}

/// Reserves exactly `[addr, addr + size)`. Returns `Ok(false)` if that range is not free.
pub(crate) fn reserve_at(addr: usize, size: usize) -> io::Result<bool> {
    let got = reserve(addr, size)?;
    if got == addr {
        return Ok(true);
    }
    // The range at `addr` was not free (or is above the user VA limit), so the kernel picked
    // another address. Give that mapping back.
    // SAFETY: `got` is the mapping that `reserve` created just now; nothing else knows it.
    unsafe { release(got, size)? };
    Ok(false)
}

/// # Safety
/// `[addr, addr + size)` is address space this process reserved, and no live reference
/// points into it.
pub(crate) unsafe fn release(addr: usize, size: usize) -> io::Result<()> {
    // SAFETY: the caller guarantees the range is ours and unused.
    let rc = unsafe { libc::munmap(addr as *mut c_void, size) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Makes the pages readable and writable. Contents stay; untouched pages read as zero.
///
/// # Safety
/// `[addr, addr + size)` is page-aligned and inside a reservation of this process.
pub(crate) unsafe fn commit(addr: usize, size: usize) -> io::Result<()> {
    // SAFETY: the caller guarantees the range is page-aligned and inside our reservation.
    let rc = unsafe {
        libc::mprotect(
            addr as *mut c_void,
            size,
            libc::PROT_READ | libc::PROT_WRITE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Drops the pages and makes the range inaccessible. The range stays reserved.
/// A later commit gives zero-filled pages.
///
/// # Safety
/// `[addr, addr + size)` is page-aligned, inside a reservation of this process, and no live
/// reference points into it.
pub(crate) unsafe fn decommit(addr: usize, size: usize) -> io::Result<()> {
    // A fresh MAP_FIXED mapping replaces the old pages in one call, on Linux and on macOS.
    // SAFETY: MAP_FIXED replaces only pages inside our own reservation (caller contract).
    let p = unsafe {
        libc::mmap(
            addr as *mut c_void,
            size,
            libc::PROT_NONE,
            ANON_PRIVATE | libc::MAP_FIXED,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        Err(io::Error::last_os_error())
    } else if p as usize != addr {
        Err(io::Error::other("mmap(MAP_FIXED) returned another address"))
    } else {
        Ok(())
    }
}

/// `(low, high)` of the calling thread's stack. `high` is the first byte above the stack.
#[cfg(target_vendor = "apple")]
pub(crate) fn stack_bounds() -> io::Result<(usize, usize)> {
    // SAFETY: pthread_self is the calling thread, which is alive. The two _np calls only read
    // its attributes.
    let (high, size) = unsafe {
        let t = libc::pthread_self();
        (
            libc::pthread_get_stackaddr_np(t) as usize,
            libc::pthread_get_stacksize_np(t),
        )
    };
    Ok((high - size, high))
}

/// `(low, high)` of the calling thread's stack. `high` is the first byte above the stack.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn stack_bounds() -> io::Result<(usize, usize)> {
    use std::mem::MaybeUninit;
    let mut attr = MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: pthread_getattr_np initializes `attr` when it returns 0.
    let rc = unsafe { libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    let mut addr: *mut c_void = std::ptr::null_mut();
    let mut size: libc::size_t = 0;
    // SAFETY: `attr` was initialized above; the out pointers are valid locals.
    let rc = unsafe { libc::pthread_attr_getstack(attr.as_ptr(), &mut addr, &mut size) };
    // SAFETY: `attr` is initialized and is destroyed exactly once.
    unsafe { libc::pthread_attr_destroy(attr.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    Ok((addr as usize, addr as usize + size))
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
pub(crate) fn stack_bounds() -> io::Result<(usize, usize)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stack bounds are implemented for macOS, Linux and Windows only",
    ))
}
