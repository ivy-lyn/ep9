//! Unwind registration for generated code (`unwind` feature; H28). Validated by SP-JIT-mem.
//!
//! The system unwinder must step through every generated frame: for Rust panics that cross
//! JIT or AOT code (`extern "C-unwind"`), for `std::backtrace`, for debuggers and profilers.
//!
//! | Target | Format (encoder) | Registration | Unwinder that reads it |
//! |---|---|---|---|
//! | Linux (glibc) | DWARF CFI, `.eh_frame` ([`cfi`]) | `__register_frame(section)`: libgcc takes the whole section | libgcc `_Unwind_*` |
//! | macOS | DWARF CFI, `.eh_frame` ([`cfi`]) | `__register_frame(fde)` once per FDE: libunwind takes one FDE | libunwind `_Unwind_*` |
//! | Windows x64 | UNWIND_INFO + RUNTIME_FUNCTION ([`win64`]) | `RtlAddGrowableFunctionTable` | `RtlLookupFunctionEntry` / `RtlVirtualUnwind` |
//!
//! - The encoders build on every host, so their tests run everywhere.
//! - [`walk_native_stack`] walks the current thread with the same system unwinder. Tests and
//!   SP-JIT-mem use it to prove that a walk passes through generated code.

pub mod cfi;
pub mod win64;

#[cfg(unix)]
mod unix;
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
    walk_native_stack(|ip| {
        v.push(ip);
        v.len() < max
    });
    v
}
