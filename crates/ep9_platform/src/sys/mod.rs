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
//! | `protect_rx` (unit text, M7-P4) | `mprotect(RX)` | `VirtualProtect(PAGE_EXECUTE_READ)` |
//! | `protect_rx_bti` (unit text with landing pads, M8-P2) | Linux/Android arm64: `mprotect(RX \| PROT_BTI)`, plain RX on `EINVAL`; else `protect_rx` | `protect_rx` |
//! | `flush_icache` (unit text, M7-P4) | `sys_icache_invalidate`; arm64 `DC CVAU` + `IC IVAU`; x86-64 none | `FlushInstructionCache` |
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
