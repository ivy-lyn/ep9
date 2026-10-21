//! The `Function` constructor family (ECMA-262 CreateDynamicFunction, §20.2.1.1.1). M2 P0
//! stub: [`compile_function_ctor`] returns [`CodegenError::Unsupported`]. P3-FNCTOR builds it:
//! parse the parameters and the body on their own (so `"a){"` cannot close the list early),
//! then the whole `<prefix> anonymous(<params>\n) {\n<body>\n}` text as one function.
//!
//! The VM calls it through its compiler hook (`IsolateConfig::compiler`, C2) after it has
//! converted the arguments to strings.
#![allow(dead_code)] // m2-stub

use ep9_bytecode::{FunctionCtorKind, VerifiedUnit};

use crate::bytecodegen::{CodegenError, CompileOptions, Unsupported};
use crate::{ByteSpan, LineMap};

/// Compile a function of `kind` from the parameter text `params` (the arguments joined with
/// `,`) and the body text `body`.
///
/// # Errors
///
/// See [`CodegenError`]. M2 P0: always [`CodegenError::Unsupported`]
/// (`"Function constructor"`).
pub fn compile_function_ctor(
    kind: FunctionCtorKind,
    params: &str,
    body: &str,
    options: CompileOptions,
) -> Result<VerifiedUnit, CodegenError> {
    let _ = (kind, params, options);
    Err(CodegenError::Unsupported(Unsupported {
        feature: "Function constructor",
        span: ByteSpan { start: 0, end: 0 },
        position: LineMap::new(body.as_bytes()).position(0),
    }))
}
