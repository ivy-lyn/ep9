//! Unwind registration for generated code (`unwind` feature; H28). Validated by SP-JIT-mem.
//!
//! The system unwinder must step through every generated frame: for Rust panics that cross
//! JIT or AOT code (`extern "C-unwind"`), for `std::backtrace`, for debuggers and profilers.
//!
//! | Target | Format (encoder) | Registration | Unwinder that reads it |
//! |---|---|---|---|
//! | Linux, macOS | DWARF CFI, `.eh_frame` ([`cfi`]) | `__register_frame` in the mode the self-test chose: whole section (libgcc) or once per FDE (LLVM and Apple libunwind) | the linked `_Unwind_*` |
//! | Windows x64 | UNWIND_INFO + RUNTIME_FUNCTION ([`win64`]) | `RtlAddGrowableFunctionTable` | `RtlLookupFunctionEntry` / `RtlVirtualUnwind` |
//!
//! - unix: the unwinder is a link-time fact (a linux-gnu build links libgcc_s, zig cc links
//!   LLVM libunwind), so a self-test picks the mode at run time: register a probe, call it,
//!   walk through it ([`RegistrationMode`], [`self_test`]). If no mode works, every
//!   registration fails and native code does not start. Windows has one form only.
//! - The encoders build on every host, so their tests run everywhere.
//! - [`walk_native_stack`] walks the current thread with the same system unwinder. Tests and
//!   SP-JIT-mem use it to prove that a walk passes through generated code.

pub mod cfi;
pub mod win64;

#[cfg(unix)]
mod mode;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use mode::{
    ProbeCallback, ProbeFn, RegistrationMode, SelfTestProbe, dlsym_guess, registration_mode,
    self_test,
};
#[cfg(unix)]
use unix as os;
#[cfg(unix)]
pub use unix::EhFrameRegistration;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;
#[cfg(windows)]
pub use windows::FunctionTableRegistration;

#[cfg(test)]
mod tests;

/// Walks the calling thread's stack with the system unwinder and calls `f` with each frame's
/// instruction pointer (a return address for every frame but the first), innermost first.
/// The walk stops when `f` returns `false` or the unwinder finds no further frame. A panic in
/// `f` stops the walk and is resumed after it.
pub fn walk_native_stack(f: impl FnMut(usize) -> bool) {
    os::walk(f)
}

/// [`walk_native_stack`] into a vector of at most `max` instruction pointers.
pub fn native_return_addresses(max: usize) -> Vec<usize> {
    let mut v = Vec::new();
    if max == 0 {
        return v;
    }
    walk_native_stack(|ip| {
        v.push(ip);
        v.len() < max
    });
    v
}
