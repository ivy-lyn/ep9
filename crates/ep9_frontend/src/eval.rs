//! `eval` code (ECMA-262 PerformEval, §19.2.1.1). M2 P0 stub: [`compile_eval`] returns
//! [`CodegenError::Unsupported`]. P2-EVAL-FE builds it (C3, C4):
//!
//! ```text
//! source --parse as a Script; drop only the diagnostics whose span is an allowed
//!          new.target / super / #x node (EvalContext)
//!        --post-checks: `arguments` in a field initializer, caller strictness, every #x in
//!          EvalContext::private_names
//!        --scope pass: CodeKind::Eval; direct eval: free names are Dynamic; indirect: Global
//!        --BytecodeGen --> VerifiedUnit (DeclareEvalVar / DeclareEvalFunction for sloppy vars)
//! ```
//!
//! The VM calls it through its compiler hook (`IsolateConfig::compiler`, C2): ep9_vm has no
//! edge to this crate.
#![allow(dead_code)] // m2-stub

use ep9_bytecode::{EvalContext, VerifiedUnit};

use crate::bytecodegen::{CodegenError, CompileOptions, Unsupported};
use crate::{ByteSpan, LineMap};

/// Compile `source` as eval code for the caller described by `ctx`.
///
/// # Errors
///
/// See [`CodegenError`]. M2 P0: always [`CodegenError::Unsupported`] (`"eval"`).
pub fn compile_eval(
    source: &str,
    ctx: &EvalContext,
    options: CompileOptions,
) -> Result<VerifiedUnit, CodegenError> {
    let _ = (ctx, options);
    Err(CodegenError::Unsupported(Unsupported {
        feature: "eval",
        span: ByteSpan { start: 0, end: 0 },
        position: LineMap::new(source.as_bytes()).position(0),
    }))
}
