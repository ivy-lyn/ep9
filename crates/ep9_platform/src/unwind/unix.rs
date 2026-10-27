//! unix backend of [`super`]: `__register_frame` and `_Unwind_Backtrace`. The registration
//! mode comes from the self-test (`super::mode`).

use std::ffi::{c_int, c_void};
use std::io;
use std::panic::{self, AssertUnwindSafe};

use super::cfi::EhFrameSection;
use super::mode::{RegistrationMode, registration_mode};

#[repr(C)]
struct UnwindContext {
    _opaque: [u8; 0],
}

type TraceFn = extern "C" fn(*mut UnwindContext, *mut c_void) -> c_int;

const URC_NO_REASON: c_int = 0;
/// Value that backtrace-rs returns to stop a walk.
const URC_FAILURE: c_int = 9;

// libgcc_s (Linux) and libunwind in libSystem (macOS). std links both already.
unsafe extern "C" {
    fn __register_frame(begin: *const u8);
    fn __deregister_frame(begin: *const u8);
    fn _Unwind_Backtrace(trace: TraceFn, arg: *mut c_void) -> c_int;
    fn _Unwind_GetIP(ctx: *mut UnwindContext) -> usize;
}

/// An `.eh_frame` section registered with the system unwinder. Drop deregisters it.
#[derive(Debug)]
pub struct EhFrameRegistration {
    section: EhFrameSection,
    mode: RegistrationMode,
    /// Addresses passed to `__register_frame`.
    registered: Vec<usize>,
}

// SAFETY: the value only owns the section bytes and the addresses it registered; the unwinder
// keeps its own lock. Deregistering from another thread is allowed by both unwinders.
unsafe impl Send for EhFrameRegistration {}
// SAFETY: no method mutates through `&self`.
unsafe impl Sync for EhFrameRegistration {}

impl EhFrameRegistration {
    /// Registers every FDE of `section` in the process's [`RegistrationMode`]. The first call
    /// in a build with `jit` runs the self-test ([`super::registration_mode`]). Fails if no
    /// mode passed the self-test, or (without `jit`) if [`super::self_test`] has not run yet.
    ///
    /// # Safety
    /// - Each FDE describes code at its `pc_begin` range correctly; the unwinder trusts it.
    /// - That code stays mapped and unchanged while the registration lives. Drop the
    ///   registration before the code memory.
    /// - Drop the registration only when no thread unwinds or walks through that code (a
    ///   panic, a backtrace, a [`super::walk_native_stack`] callback). The unwinders use a
    ///   found FDE without a lock, and macOS libunwind reads it again at the next step of a
    ///   walk. A drop during such a walk reads freed memory (SP-JIT-mem review: SIGSEGV on
    ///   macOS, abort or SIGSEGV with libgcc).
    /// - The spans of two live registrations do not overlap. A span runs from the lowest
    ///   `pc_begin` to the highest FDE end of the section. libgcc finds a registration by its
    ///   span, so a walk misses frames of interleaved registrations (libgcc 12 and 14).
    pub unsafe fn register(section: EhFrameSection) -> io::Result<Self> {
        let mode = registration_mode()?;
        // SAFETY: the caller's contract.
        unsafe { Self::register_mode(section, mode) }
    }

    /// [`EhFrameRegistration::register`] in a given mode, for the self-test.
    ///
    /// # Safety
    /// As [`EhFrameRegistration::register`].
    pub(super) unsafe fn register_mode(
        section: EhFrameSection,
        mode: RegistrationMode,
    ) -> io::Result<Self> {
        let records = section.records()?;
        let base = section.bytes().as_ptr() as usize;
        let registered: Vec<usize> = match mode {
            RegistrationMode::WholeSection => vec![base],
            RegistrationMode::PerFde => records
                .iter()
                .filter(|(_, is_cie)| !is_cie)
                .map(|(o, _)| base + o)
                .collect(),
        };
        for &p in &registered {
            // SAFETY: `p` points into `section`, which the result owns and keeps at the same
            // address (boxed storage never moves); `records` checked the structure; the caller
            // vouches for the content.
            unsafe { __register_frame(p as *const u8) };
        }
        Ok(Self {
            section,
            mode,
            registered,
        })
    }

    pub fn section(&self) -> &EhFrameSection {
        &self.section
    }

    /// The mode this registration used.
    pub fn mode(&self) -> RegistrationMode {
        self.mode
    }
}

impl Drop for EhFrameRegistration {
    fn drop(&mut self) {
        for &p in self.registered.iter().rev() {
            // SAFETY: `p` was registered by `register` and is still owned by `self.section`.
            unsafe { __deregister_frame(p as *const u8) };
        }
    }
}

struct Walk<'a> {
    f: &'a mut dyn FnMut(usize) -> bool,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

extern "C" fn trace(ctx: *mut UnwindContext, arg: *mut c_void) -> c_int {
    // SAFETY: `arg` is the `Walk` that `walk` passed; only this callback uses it during the
    // walk.
    let w = unsafe { &mut *arg.cast::<Walk<'_>>() };
    // SAFETY: `ctx` is the live context that _Unwind_Backtrace passes to its callback.
    let ip = unsafe { _Unwind_GetIP(ctx) };
    if ip == 0 {
        return URC_FAILURE;
    }
    match panic::catch_unwind(AssertUnwindSafe(|| (w.f)(ip))) {
        Ok(true) => URC_NO_REASON,
        Ok(false) => URC_FAILURE,
        Err(p) => {
            w.panic = Some(p);
            URC_FAILURE
        }
    }
}

pub(super) fn walk(mut f: impl FnMut(usize) -> bool) {
    let mut w = Walk {
        f: &mut f,
        panic: None,
    };
    // SAFETY: `trace` matches the callback type and never unwinds; `w` outlives the call.
    unsafe { _Unwind_Backtrace(trace, (&raw mut w).cast()) };
    if let Some(p) = w.panic {
        panic::resume_unwind(p);
    }
}
