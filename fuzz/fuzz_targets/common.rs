//! The checks every front-end fuzz target makes (docs/plan/10-quality.md section 4: "No panics,
//! no aborts; the bytecode verifier passes").
//!
//! ```text
//! input bytes -> UTF-8 text (else skip) -> ep9_frontend compile (target-specific entry)
//!   Ok(unit)                 -> to_bytes -> CompiledUnit::from_bytes (decode + verify) must
//!                               pass and give the same bytes
//!   Err(Internal)            -> panic: the builder or the verifier rejected BytecodeGen output
//!   Err(Compile / Unsupported / Limit) -> fine (SyntaxError, RangeError, not built yet)
//!   a panic or an abort anywhere (also on the front-end worker thread) -> libFuzzer crash
//! ```

use ep9_bytecode::{CompiledUnit, VerifiedUnit};
use ep9_frontend::CodegenError;

/// The text of `data`, or `None` when it is not UTF-8 (the engine's compile entries take
/// `&str`; lone surrogates are a VM-side matter, eval.rs E8).
pub fn text(data: &[u8]) -> Option<&str> {
    std::str::from_utf8(data).ok()
}

/// Check one compile result (see the module doc).
pub fn check(result: Result<VerifiedUnit, CodegenError>) {
    match result {
        Ok(unit) => {
            let bytes = unit.to_bytes();
            match CompiledUnit::from_bytes(&bytes) {
                Ok(again) => assert!(again.to_bytes() == bytes, "unit bytes changed on reload"),
                Err(e) => panic!("a compiled unit does not load: {e:?}"),
            }
        }
        Err(CodegenError::Internal(m)) => panic!("internal BytecodeGen error: {m}"),
        Err(CodegenError::Compile(_) | CodegenError::Unsupported(_) | CodegenError::Limit(_)) => {}
    }
}

/// The ASan options of a fuzz target that runs the VM (route FUZZ-UL-M8P5; decisions log
/// M9-PLAN-D21): `vm_asan_options!();` at the top level of the target defines
/// `__asan_default_options`, which the ASan runtime calls once at start, before `main`. It
/// turns `detect_stack_use_after_return` off (on by default on Linux).
///
/// ```text
/// use-after-return on: an address-taken local (a `Local`: on_stack::check takes its address)
///   lives in an ASan fake frame (heap memory), not on the thread stack
///   -> ep9_vm abi/local.rs on_stack::check (debug only) panics on a correct program
///   -> the conservative stack scan (ep9_heap css.rs) reads only [sp, stack high), so a
///      Local whose only copy is in a fake frame is not a root: a GC can free a live cell
/// use-after-return off (this hook): every local is on the thread stack again
///   -> the check and the scan see what a build without ASan sees
/// ```
///
/// Why the check keeps its value: the engine does not change (local.rs and css.rs are as
/// before; the symbol is only in the fuzz binaries that call this macro). A build without ASan
/// has no fake stack, so there the check and the scan never saw a difference. In the fuzz
/// binary the check still panics on a `Local` in heap memory (a `Vec` or a `Box`: a real H20 R2
/// fault), because that memory is still outside the thread stack. Only ASan's own
/// stack-use-after-return reports stop in this target (safe Rust cannot return a dangling
/// stack reference). The front-end targets do not call the macro, so they keep them. The
/// `ASAN_OPTIONS` environment variable is read after this hook and wins over it; the fuzz
/// scripts and workflows do not set this flag.
#[macro_export]
macro_rules! vm_asan_options {
    () => {
        /// The ASan default options of this target (`common.rs`, `vm_asan_options`).
        #[unsafe(no_mangle)]
        pub extern "C" fn __asan_default_options() -> *const ::core::ffi::c_char {
            c"detect_stack_use_after_return=0".as_ptr()
        }
    };
}
