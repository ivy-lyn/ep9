//! Unit text in the process code range (H35 S4 step 5 "map text RX"; H28 unit mapping; DL
//! M6-ORCH-298). Always built: the jitless build loads AOT units too (H27-Q3).
//!
//! ```text
//!   UnitText::map(range, text)
//!     range.alloc(text.len()) ──> CodeChunk (whole pages; H07:52: JIT code and units share
//!                                            the one process code range, no new mapping)
//!     form, picked by the build and by CodeRange::is_map_jit(), never by a target name:
//!       `jit` on, or a MAP_JIT range ──> Window   jit::CodeMemory: commit, one write window,
//!                                                 close (RX again + icache flush)
//!       otherwise (a plain range) ────> Protect  commit RW, copy, icache flush, protect RX
//!   drop ──> pages decommitted, chunk back to the range
//! ```
//!
//! | Build | Process code range | Form |
//! |---|---|---|
//! | `jit` on macOS arm64 | one `MAP_JIT` RWX mapping | [`Form::Window`], `WxMode::MapJit` |
//! | `jit` on the other targets | plain | [`Form::Window`], `WxMode::Protect` (`CodeChunk::commit`) |
//! | no `jit` (the jitless ep9), every target | plain | [`Form::Protect`] (no `MAP_JIT`) |
//! | this crate's `cfg(test)` on macOS arm64 | `MAP_JIT` (`va.rs`) | [`Form::Window`] |
//!
//! - The text is written once, before the first byte of it runs, and never again: unit text
//!   is per process and shared (H35 S2). The A2 loader keeps one mapping per process for one
//!   text, keyed by SHA-256 of the text bytes and their unwind data, and maps a text only when
//!   a function picks unit code (`ep9_vm` `unit_loader.rs`; M7-P5 A2-RW, routes A2-UNIT-SHARE
//!   and MC-w34-4). Per-isolate data goes to the RW copy (H35 S4 step 6).
//! - **Hardened runtime (macOS arm64).** The protect form makes plain anonymous pages
//!   executable with `mprotect`. That works for a binary without the hardened runtime (the
//!   same fact as `jit::WxMode::Protect`, measured on macOS 27.0.1). A hardened binary needs
//!   `MAP_JIT` (the `com.apple.security.cs.allow-jit` entitlement) or a statically linked
//!   `.o` unit (H35, not built yet). The H28 unit-mapping row names this limit.
//! - **Other threads.** The flush is for this thread. A thread that runs text that another
//!   thread mapped (a shared text) first runs [`context_sync`]: the A2 loader calls it on each
//!   text it returns, mapped or shared, before the dispatch slot write that makes it reachable.

use std::fmt;
use std::io;
use std::ops::Range;

use crate::sys;
use crate::va::{CodeChunk, CodeRange};

/// How a [`UnitText`] got its bytes (see the module table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Form {
    /// Through a `jit::CodeMemory` write window (`jit` on, or a `MAP_JIT` range).
    Window,
    /// Commit RW, copy, flush the icache, protect RX (a plain range in a build without `jit`).
    Protect,
}

impl Form {
    /// The form name, as reports write it.
    pub const fn name(self) -> &'static str {
        match self {
            Form::Window => "window",
            Form::Protect => "protect",
        }
    }
}

impl fmt::Display for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Unit text copied into a chunk of a code range: executable, never written again. Drop
/// gives the pages back; nothing may run in them then (the caller's rule: a loaded unit's text
/// lives as long as its isolate).
pub struct UnitText<'r> {
    /// The code memory of the window form. Declared first, so it drops (decommits, or with
    /// `MapJit` drops the contents lazily) before `chunk` gives the pages back.
    #[cfg(any(feature = "jit", test))]
    _mem: Option<crate::jit::CodeMemory<'r>>,
    chunk: CodeChunk<'r>,
    len: usize,
    form: Form,
}

impl fmt::Debug for UnitText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnitText")
            .field("base", &format_args!("{:#x}", self.base()))
            .field("len", &self.len)
            .field("form", &self.form)
            .finish()
    }
}

impl<'r> UnitText<'r> {
    /// Copies `text` into a new chunk of `range` and makes it executable (see the module doc
    /// for the form). Errors: `InvalidInput` for empty text; the errors of `CodeRange::alloc`
    /// and of the protection calls. On an error nothing stays committed.
    pub fn map(range: &'r CodeRange, text: &[u8]) -> io::Result<UnitText<'r>> {
        if text.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unit text is empty",
            ));
        }
        let chunk = range.alloc(text.len())?;
        #[cfg(any(feature = "jit", test))]
        if cfg!(feature = "jit") || range.is_map_jit() {
            return Self::map_window(chunk, text);
        }
        Self::map_protect(chunk, text)
    }

    /// The window form: `jit::CodeMemory` over the chunk, one write window.
    #[cfg(any(feature = "jit", test))]
    fn map_window(chunk: CodeChunk<'r>, text: &[u8]) -> io::Result<UnitText<'r>> {
        use crate::jit::{CodeMemory, WxMode};
        let range = chunk.code_range();
        let mode = if range.is_map_jit() {
            WxMode::MapJit
        } else {
            WxMode::Protect
        };
        // SAFETY: the chunk's pages belong to this chunk alone (`CodeRange::alloc` hands out
        // disjoint runs, and nothing outside this crate can reach the reservation); the code
        // memory is stored next to the chunk and drops first (field order), so no other user
        // of the pages exists while it lives, and the range outlives both.
        let mut mem =
            unsafe { CodeMemory::commit(range.reservation(), chunk.offset(), chunk.len(), mode) }?;
        {
            let mut w = mem.open_write(0, text.len())?;
            w.bytes().copy_from_slice(text);
            w.close()?;
        }
        Ok(UnitText {
            _mem: Some(mem),
            chunk,
            len: text.len(),
            form: Form::Window,
        })
    }

    /// The protect form: commit RW, copy, flush, protect RX.
    fn map_protect(chunk: CodeChunk<'r>, text: &[u8]) -> io::Result<UnitText<'r>> {
        let res = chunk.code_range().reservation();
        let (base, pages) = (chunk.base(), chunk.len());
        res.commit(chunk.offset(), pages)?;
        // SAFETY: the pages were committed read-write just now and belong to this chunk alone;
        // `text.len()` <= `pages` (the chunk rounds up).
        unsafe {
            core::ptr::copy_nonoverlapping(
                text.as_ptr(),
                core::ptr::with_exposed_provenance_mut::<u8>(base),
                text.len(),
            );
        }
        sys::flush_icache(base, text.len());
        // SAFETY: the pages are committed, page-aligned and ours; no reference writes them.
        if let Err(e) = unsafe { sys::protect_rx(base, pages) } {
            // SAFETY: nothing runs or points into the pages yet.
            let _ = unsafe { res.decommit(chunk.offset(), pages) };
            return Err(e);
        }
        Ok(UnitText {
            #[cfg(any(feature = "jit", test))]
            _mem: None,
            chunk,
            len: text.len(),
            form: Form::Protect,
        })
    }

    /// The address of the first text byte (page-aligned).
    pub fn base(&self) -> usize {
        self.chunk.base()
    }

    /// The text length in bytes (the chunk may be longer: whole pages).
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false: [`UnitText::map`] refuses empty text.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `base()..base() + len()`.
    pub fn range(&self) -> Range<usize> {
        self.base()..self.base() + self.len
    }

    /// The pages of the chunk: `base()..base() + chunk length` (a page multiple).
    pub fn pages(&self) -> Range<usize> {
        self.chunk.addr_range()
    }

    /// True if `addr` is a text byte.
    pub fn contains(&self, addr: usize) -> bool {
        addr.wrapping_sub(self.base()) < self.len
    }

    /// How the bytes got there.
    pub fn form(&self) -> Form {
        self.form
    }
}

impl Drop for UnitText<'_> {
    fn drop(&mut self) {
        if self.form == Form::Protect {
            // SAFETY: the pages belong to this chunk; the caller's rule is that no code runs in
            // them and no pointer into them is used after the drop.
            let r = unsafe {
                self.chunk
                    .code_range()
                    .reservation()
                    .decommit(self.chunk.offset(), self.chunk.len())
            };
            debug_assert!(r.is_ok(), "unit text decommit failed: {r:?}");
        }
        // The window form decommits in `CodeMemory`'s drop; then the chunk gives the pages back.
    }
}

/// A context-synchronizing event on this thread, before it runs unit text that another thread
/// wrote (M7-P5 Codex A1; the Spike B `unit/load.rs` form): `isb` on AArch64 (Arm ARM,
/// concurrent modification and execution of instructions: the writer's cache maintenance is
/// broadcast, the pipeline of this core is not), `cpuid` on x86-64 (a serializing instruction;
/// Intel SDM vol. 3 8.1.3, cross-modifying code). The writer flushed the icache when it wrote
/// the text; a lock (the loader's text map) orders the data. Other targets: nothing.
#[inline]
pub fn context_sync() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: `isb` only flushes this core's pipeline; it touches no memory or register.
    unsafe {
        core::arch::asm!("isb", options(nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: cpuid leaf 0 exists on every x86-64 CPU and has no side effect.
        #[allow(unused_unsafe)]
        let _ = unsafe { core::arch::x86_64::__cpuid(0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::va::{CodeRangeConfig, page_size};

    /// `return 42` (u32), in the C ABI of the host (no stack use).
    fn ret42() -> Vec<u8> {
        #[cfg(target_arch = "aarch64")]
        {
            // mov w0, #42; ret
            [0x5280_0540u32, 0xD65F_03C0]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect()
        }
        #[cfg(target_arch = "x86_64")]
        {
            // mov eax, 42; ret
            vec![0xB8, 42, 0, 0, 0, 0xC3]
        }
    }

    /// The form follows the range, not the target: a `MAP_JIT` range (macOS arm64 in this
    /// crate's test build) gets the window, a plain one (every other target here) the
    /// protect form. The bytes run.
    #[test]
    fn the_form_follows_the_range_and_the_text_runs() {
        let range = CodeRange::process().unwrap();
        let t = UnitText::map(range, &ret42()).unwrap();
        let want = if cfg!(feature = "jit") || range.is_map_jit() {
            Form::Window
        } else {
            Form::Protect
        };
        assert_eq!(t.form(), want);
        assert!(range.contains(t.base()) && t.base().is_multiple_of(page_size()));
        // SAFETY: the text is `ret42`, mapped executable until `t` drops.
        let f: extern "C" fn() -> u32 = unsafe { core::mem::transmute(t.base()) };
        assert_eq!(f(), 42);
    }

    /// A range of its own: the pages go back at drop, and empty text is refused.
    #[test]
    fn the_pages_go_back_and_empty_text_is_refused() {
        let range = CodeRange::reserve(&CodeRangeConfig::default()).unwrap();
        let free = range.free_bytes();
        let t = UnitText::map(&range, &ret42()).unwrap();
        assert_eq!(range.free_bytes(), free - page_size());
        assert_eq!(t.pages().len(), page_size());
        drop(t);
        assert_eq!(range.free_bytes(), free);
        let e = UnitText::map(&range, &[]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(range.free_bytes(), free);
    }
}
