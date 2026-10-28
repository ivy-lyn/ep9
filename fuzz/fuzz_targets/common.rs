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
