//! Executable memory with W^X in the code range (`jit` feature; H28, 04-execution sections 5
//! and 12). Validated by SP-JIT-mem (`spikes/sp-jit-mem`).
//!
//! ```text
//!   CodeRange::process() ──> the process code range (one per process, H07-Q6)
//!     alloc(len) ──> CodeChunk                               pages owned by one user
//!       commit(mode) ──> CodeMemory                          pages are executable and zero
//!         open_write(off, len) ──> WriteWindow               pages are writable, not executable
//!           bytes() ──> &mut [u8]                            write the code
//!         close()                                            executable again + icache flush
//!       exec_addr(off) ──> call it (the caller transmutes the address to a fn pointer)
//! ```
//!
//! `CodeChunk::commit` is the safe entry. The unsafe `CodeMemory::commit(range, offset, len,
//! mode)` takes any code-range [`Reservation`] (also a plain one).
//!
//! | Mode | Targets | While a window is open | Toggle cost |
//! |---|---|---|---|
//! | [`WxMode::MapJit`] | macOS arm64 (default) | this thread sees every `MAP_JIT` page as RW, other threads still RX | per-thread register write, no syscall |
//! | [`WxMode::Protect`] | all (default except macOS arm64) | the window's pages are RW for all threads | 2 syscalls (`mprotect` / `VirtualProtect`) |
//! | [`WxMode::DualMap`] | Linux | nothing changes: one memfd, an RX view in the code range, an RW view elsewhere | none |
//!
//! - **MapJit** needs a `MAP_JIT` code range: on macOS arm64 [`crate::CodeRange::process`]
//!   maps it with `MAP_JIT`, see [`crate::va`]. The other modes need a plain range. `commit`
//!   checks this. [`WxMode::DEFAULT`] fits the process code range on every target.
//! - **DualMap** saves the two syscalls per window, but a binary translator (Rosetta for
//!   Linux) does not see writes through the RW view and runs stale code. Protect works there.
//! - **Protect on macOS arm64** works for a binary without the hardened runtime (measured on
//!   macOS 27.0.1). The hardened runtime needs `MAP_JIT`. SP-JIT-mem uses it only to compare costs.
//! - **icache.** `close` flushes the written range on every target: `sys_icache_invalidate` on
//!   macOS, `DC CVAU` + `IC IVAU` on Linux arm64 (as compiler-rt `__clear_cache`),
//!   `FlushInstructionCache` on Windows. x86-64 Linux needs no flush for the writing thread.
//! - Other threads that run code which a window changes need a context-synchronizing event
//!   (membarrier or a lock handshake) before they run it. That is ep9_masm's job.

use std::cell::Cell;
use std::fmt;
use std::io;
use std::marker::PhantomData;

use crate::va::{CodeChunk, Reservation, page_size};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as os;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

#[cfg(test)]
mod tests;

/// How a [`CodeMemory`] keeps W^X.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WxMode {
    /// macOS arm64: `MAP_JIT` pages and `pthread_jit_write_protect_np`, per thread.
    MapJit,
    /// `mprotect` (unix) or `VirtualProtect` (Windows): RW while a window is open, else RX.
    Protect,
    /// Linux: a memfd mapped twice, RX in the code range and RW elsewhere. Not coherent under
    /// a binary translator: x86-64 Linux on Rosetta ran stale code after a rewrite through
    /// the RW view (SP-JIT-mem). So it is not the default.
    DualMap,
}

impl WxMode {
    /// The mode ep9 uses on this target.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    pub const DEFAULT: WxMode = WxMode::MapJit;
    /// The mode ep9 uses on this target.
    #[cfg(not(all(target_vendor = "apple", target_arch = "aarch64")))]
    pub const DEFAULT: WxMode = WxMode::Protect;

    /// The modes built for this target. The first one is [`WxMode::DEFAULT`].
    pub const fn available() -> &'static [WxMode] {
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        return &[WxMode::MapJit, WxMode::Protect];
        #[cfg(any(target_os = "linux", target_os = "android"))]
        return &[WxMode::Protect, WxMode::DualMap];
        #[cfg(not(any(
            all(target_vendor = "apple", target_arch = "aarch64"),
            target_os = "linux",
            target_os = "android"
        )))]
        return &[WxMode::Protect];
    }

    /// True if the mode needs a `MAP_JIT` code range ([`Reservation::is_map_jit`]).
    pub const fn needs_map_jit(self) -> bool {
        matches!(self, WxMode::MapJit)
    }

    pub const fn name(self) -> &'static str {
        match self {
            WxMode::MapJit => "map-jit",
            WxMode::Protect => "protect",
            WxMode::DualMap => "dual-map",
        }
    }
}

impl fmt::Display for WxMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// Committed code pages inside a code-range reservation. Drop gives the pages back; the range
/// stays reserved.
pub struct CodeMemory<'r> {
    /// Where the code runs (inside the reservation).
    exec: usize,
    /// Where the code is written: `exec`, except for [`WxMode::DualMap`].
    write: usize,
    len: usize,
    mode: WxMode,
    os: os::State,
    _range: PhantomData<&'r Reservation>,
}

impl fmt::Debug for CodeMemory<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeMemory")
            .field("exec", &format_args!("{:#x}", self.exec))
            .field("write", &format_args!("{:#x}", self.write))
            .field("len", &self.len)
            .field("mode", &self.mode)
            .finish()
    }
}

impl<'r> CodeMemory<'r> {
    /// Commits `[range.base() + offset, + len)` for code. `offset` and `len` are non-zero page
    /// multiples. The pages start executable and not writable. New pages read as zero; pages
    /// that a dropped [`WxMode::MapJit`] memory used before hold unspecified bytes.
    ///
    /// # Safety
    /// No other `CodeMemory`, and nothing else, uses these pages while the result lives.
    pub unsafe fn commit(
        range: &'r Reservation,
        offset: usize,
        len: usize,
        mode: WxMode,
    ) -> io::Result<Self> {
        let exec = range.subrange(offset, len)?;
        if !WxMode::available().contains(&mode) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this W^X mode is not built for this target",
            ));
        }
        if mode.needs_map_jit() != range.is_map_jit() {
            return Err(invalid(if mode.needs_map_jit() {
                "MapJit needs a MAP_JIT code range (CodeRange::process on macOS arm64)"
            } else {
                "a MAP_JIT code range works only with WxMode::MapJit"
            }));
        }
        // SAFETY: `subrange` checked that the page-aligned range lies inside `range`, which
        // outlives the result; the caller guarantees nothing else uses these pages.
        let (write, os) = unsafe { os::commit(exec, len, mode)? };
        Ok(Self {
            exec,
            write,
            len,
            mode,
            os,
            _range: PhantomData,
        })
    }

    /// First byte of the memory, as code runs it.
    pub fn exec_base(&self) -> usize {
        self.exec
    }

    /// `exec_base() + offset`. Panics if `offset` is outside the memory.
    pub fn exec_addr(&self, offset: usize) -> usize {
        assert!(
            offset < self.len,
            "offset {offset} is outside the code memory"
        );
        self.exec + offset
    }

    /// True if `addr` lies in the executable view.
    pub fn contains(&self, addr: usize) -> bool {
        addr.wrapping_sub(self.exec) < self.len
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false: `commit` rejects an empty range.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn mode(&self) -> WxMode {
        self.mode
    }

    /// Makes `[offset, offset + len)` writable until the window closes (see the mode table in
    /// the module doc). Do not run code of a `MapJit` memory on this thread while a window is
    /// open: this thread sees all `MAP_JIT` pages as not executable then.
    pub fn open_write(&mut self, offset: usize, len: usize) -> io::Result<WriteWindow<'_, 'r>> {
        if len == 0 || offset > self.len || len > self.len - offset {
            return Err(invalid("window is empty or outside the code memory"));
        }
        let ps = page_size();
        let first = offset / ps * ps;
        let pages = (offset + len).div_ceil(ps) * ps - first;
        // SAFETY: the pages are inside this memory (checked above); `&mut self` keeps every
        // other window of this memory closed.
        unsafe { os::begin_write(self.mode, self.exec + first, pages)? };
        Ok(WriteWindow {
            mem: self,
            offset,
            len,
            first,
            pages,
            dirty: false,
            open: true,
            _thread: PhantomData,
        })
    }
}

impl Drop for CodeMemory<'_> {
    fn drop(&mut self) {
        // SAFETY: this value owns the pages (commit contract); no window is open (`&mut`).
        let r = unsafe { os::release(self.exec, self.write, self.len, self.mode, &mut self.os) };
        debug_assert!(r.is_ok(), "code memory release failed: {r:?}");
    }
}

/// An open write window of a [`CodeMemory`]. `close` makes the pages executable again and
/// flushes the instruction cache for the bytes written. Drop closes too, but ignores errors.
/// Not `Send`: with [`WxMode::MapJit`] the writable state belongs to the opening thread.
pub struct WriteWindow<'m, 'r> {
    mem: &'m mut CodeMemory<'r>,
    offset: usize,
    len: usize,
    /// Page-aligned offset and length of the pages the window opened.
    first: usize,
    pages: usize,
    dirty: bool,
    open: bool,
    _thread: PhantomData<*const ()>,
}

impl WriteWindow<'_, '_> {
    /// The window's bytes, at the write address.
    pub fn bytes(&mut self) -> &mut [u8] {
        self.dirty = true;
        let p = std::ptr::with_exposed_provenance_mut::<u8>(self.mem.write + self.offset);
        // SAFETY: the bytes are committed and writable while the window is open, which
        // `&mut self` guarantees; `&mut CodeMemory` excludes every other Rust reference.
        unsafe { std::slice::from_raw_parts_mut(p, self.len) }
    }

    /// Ends the window: executable again, then an icache flush if [`WriteWindow::bytes`] was
    /// called.
    pub fn close(mut self) -> io::Result<()> {
        self.finish()
    }

    fn finish(&mut self) -> io::Result<()> {
        if !self.open {
            return Ok(());
        }
        self.open = false;
        let m = &*self.mem;
        // SAFETY: the same pages that `open_write` opened.
        unsafe { os::end_write(m.mode, m.exec + self.first, self.pages)? };
        if self.dirty {
            flush_icache(m.exec + self.offset, self.len);
        }
        Ok(())
    }
}

impl Drop for WriteWindow<'_, '_> {
    fn drop(&mut self) {
        // On error the pages stay writable and not executable: running them faults, so W^X
        // still holds.
        let _ = self.finish();
    }
}

impl CodeChunk<'_> {
    /// Commits the whole chunk as code memory ([`CodeMemory::commit`] at the chunk's offset).
    /// The result borrows the chunk, so the chunk outlives it, and only one code memory uses
    /// the chunk at a time. Errors as `CodeMemory::commit` (on macOS arm64 the process code
    /// range takes [`WxMode::MapJit`] only).
    pub fn commit(&mut self, mode: WxMode) -> io::Result<CodeMemory<'_>> {
        let range = self.code_range().reservation();
        // SAFETY: the chunk's pages belong to this chunk alone (`CodeRange::alloc` hands out
        // disjoint runs, and nothing outside this crate can reach the reservation). `&mut
        // self` is held for the result's lifetime, so no second code memory uses them, and the
        // range outlives the chunk.
        unsafe { CodeMemory::commit(range, self.offset(), self.len(), mode) }
    }
}

/// Makes the instruction fetch of this thread see bytes written to `[addr, addr + len)`.
/// `addr` is the executable address.
pub fn flush_icache(addr: usize, len: usize) {
    if len != 0 {
        os::flush_icache(addr, len);
    }
}

thread_local! {
    /// Open `MapJit` windows on this thread. The first opens write access, the last closes it.
    static MAP_JIT_WINDOWS: Cell<u32> = const { Cell::new(0) };
}

/// Counts nested `MapJit` windows; `toggle(true)` runs when the first opens, `toggle(false)`
/// when the last closes.
#[cfg_attr(
    not(all(target_vendor = "apple", target_arch = "aarch64")),
    allow(dead_code)
)]
fn map_jit_nest(open: bool, toggle: impl FnOnce(bool)) {
    MAP_JIT_WINDOWS.with(|n| {
        let v = n.get();
        if open {
            if v == 0 {
                toggle(true);
            }
            n.set(v + 1);
        } else {
            debug_assert!(v > 0, "MapJit window closed twice");
            if v == 1 {
                toggle(false);
            }
            n.set(v.saturating_sub(1));
        }
    });
}
