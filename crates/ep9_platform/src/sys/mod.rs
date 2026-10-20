//! OS primitives. Each backend gives the same small set of functions:
//!
//! | Function | unix | Windows |
//! |---|---|---|
//! | `page_size` | `sysconf(_SC_PAGESIZE)` | `GetSystemInfo().dwPageSize` |
//! | `granularity` | page size | `dwAllocationGranularity` (64 KiB) |
//! | `reserve` | `mmap(PROT_NONE)` | `VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` |
//! | `release` | `munmap` | `VirtualFree(MEM_RELEASE)` |
//! | `commit` | `mprotect(RW)` | `VirtualAlloc(MEM_COMMIT, PAGE_READWRITE)` |
//! | `decommit` | `mmap(MAP_FIXED, PROT_NONE)` | `VirtualFree(MEM_DECOMMIT)` |
//! | `stack_bounds` | `pthread_get_stackaddr_np` / `pthread_getattr_np` | `GetCurrentThreadStackLimits` |

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(not(any(unix, windows)))]
compile_error!("ep9_platform supports unix and Windows only");
