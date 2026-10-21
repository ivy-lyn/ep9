//! The compiler hook (C2): ep9_vm has no edge to ep9_frontend (guard 12 checks that the ep9
//! binary links the front end, and that ep9_vm does not), so the VM reaches the compiler
//! through function pointers that the embedder passes in [`IsolateConfig::compiler`].
//!
//! ```text
//! embedder (ep9_cli run.rs, ep9_test262 exec.rs)       ep9_vm
//!   static HOOKS: CompilerHooks { .. } ---------------> IsolateConfig::compiler
//!     compile_eval      -> ep9_frontend::compile_eval        PerformEval (P3-EVAL-VM)
//!     compile_function  -> ep9_frontend::compile_function_ctor  CreateDynamicFunction (P3-FNCTOR)
//!     compile_script    -> ep9_frontend::compile (sloppy Script)  script entry (ABI script.rs)
//! ```
//!
//! M2-P0: nothing calls the hooks yet. With no hooks, or with a
//! [`CompileFailure::Unsupported`] result, `Function()` keeps its v0 Error and `%eval%` stays
//! uninstalled (C19), so no test262 result changes.
//!
//! [`IsolateConfig::compiler`]: crate::IsolateConfig::compiler

use ep9_bytecode::{EvalContext, FunctionCtorKind, VerifiedUnit};

/// Why a compiler hook gave no unit. The VM turns it into the thrown error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileFailure {
    /// An early error: the VM throws a `SyntaxError` with this message.
    Syntax(String),
    /// A compile-time limit (depth, registers, code size): the VM throws a `RangeError` with
    /// this message.
    Range(String),
    /// The compiler does not build this construct yet (BytecodeGen v0 `Unsupported`); the text
    /// names the construct.
    Unsupported(String),
    /// A compiler bug (the builder or the verifier rejected its output).
    Internal(String),
}

/// The result of a compiler hook.
pub type CompileResult = Result<VerifiedUnit, CompileFailure>;

/// Compiler entry points for code made at run time (C2). Every unit a hook returns is
/// verified; the VM loads it like a script unit.
#[derive(Clone, Copy, Debug)]
pub struct CompilerHooks {
    /// Eval code (PerformEval, ECMA-262 §19.2.1.1): `source` for the caller that `ctx`
    /// describes (strictness, direct or indirect, what `new.target`, `super` and `#x` may
    /// name).
    pub compile_eval: fn(source: &str, ctx: &EvalContext) -> CompileResult,
    /// The `Function` constructor family (CreateDynamicFunction, §20.2.1.1.1): a function of
    /// `kind` from the parameter text `params` (the arguments joined with `,`) and `body`.
    pub compile_function: fn(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult,
    /// A sloppy-mode Script from source text (an embedder or `$262.evalScript` entry).
    pub compile_script: fn(source: &str) -> CompileResult,
}
