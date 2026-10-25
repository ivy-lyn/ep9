//! Plain data that the VM and the front end share for `eval` and the `Function` constructor
//! (ABI 3). No GC type and no oxc type: the VM fills an [`EvalContext`] from the running frame
//! and passes it to the compiler hook; the front end (`ep9_frontend::compile_eval`) reads it.
//!
//! ```text
//! direct eval:   VM (ResolvePossiblyDirectEval) --EvalContext--> compile_eval --> unit (CodeKind::Eval)
//! indirect eval: VM (%eval%)                    --EvalContext--> compile_eval --> unit
//! Function():    VM (CreateDynamicFunction)  --FunctionCtorKind--> compile_function_ctor --> unit
//! ```
//!
//! Sloppy direct eval code puts its `var` and function declarations into an object that the
//! [`EVAL_EXT_BINDING`] binding holds: slot 0 of the context of each function var scope that
//! has [`ScopeFlags::CALLS_SLOPPY_EVAL`](crate::ScopeFlags::CALLS_SLOPPY_EVAL). The slot starts
//! undefined; the VM makes the object on the first `DeclareEvalVar` or `DeclareEvalFunction`.
//! Script-level eval vars go to the global object, so a `Script` scope has no such binding.
//!
//! What an eval unit (`ep9_frontend::compile_eval`, code kind `Eval`) expects from the VM:
//!
//! | Item | Direct eval | Indirect eval |
//! |---|---|---|
//! | How function 0 runs | a closure over the caller's current context (the receiver is not read) | global code: the receiver is the global this, as for a script |
//! | Free names (also `#x`) | `LdaLookupSlot` and friends, by name, from the eval code's context | `LdaGlobal` / `StaGlobal` |
//! | `this`, `new.target`, `super.x`, `arguments` | looked up by the names `this`, `new.target`, `.home_object`, `arguments` in the caller's context slots. A `with` object on the way must not answer the first three (they are not identifier references); it does answer `arguments`. `super()` reads `.callee` and `new.target` by name and binds `this` with `StaLookupSlot "this"` (BindThisValue in the VM; E6, M3-P4) | `this` is the receiver; the rest are early errors |
//!
//! Sloppy eval code calls `DeclareEvalFunction(name, closure)` per function (spec order of
//! `functionsToInitialize`) and then `DeclareEvalVar(name)` per other `var` name, before the
//! body; these do the checks of EvalDeclarationInstantiation. Strict eval code keeps its
//! declarations in scope 0 (kind `Eval`, a context made by `CreateEvalContext` when it needs
//! one). Function 0 returns the completion value.

/// The name of the synthetic binding that holds the sloppy direct eval variables of a function
/// var scope (slot 0 of its context). It starts with `.`, so no JS identifier can name it.
pub const EVAL_EXT_BINDING: &str = ".eval_ext";

/// What the eval code may use, from the caller's place (ECMA-262 PerformEval and its early
/// errors, §19.2.1.1).
///
/// For indirect eval every flag is false and `private_names` is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct EvalContext {
    /// The eval code is strict: the caller is strict (direct eval) or the source has a
    /// `"use strict"` directive.
    pub strict: bool,
    /// A direct eval (`eval(...)` that calls %eval%). Indirect eval is global code.
    pub direct: bool,
    /// The direct eval is in function code (also in an arrow inside a function), not at the
    /// top level of a script or module.
    pub in_function: bool,
    /// The nearest non-arrow function is a method (it has a home object).
    pub in_method: bool,
    /// The nearest non-arrow function is a derived class constructor.
    pub in_derived_ctor: bool,
    /// The eval is inside a class field initializer: `arguments` is an early error.
    pub in_class_field_init: bool,
    /// `new.target` is allowed in the eval code.
    pub new_target_ok: bool,
    /// `super.x` is allowed in the eval code.
    pub super_property_ok: bool,
    /// `super()` is allowed in the eval code.
    pub super_call_ok: bool,
    /// The private names in scope at the eval site, without the `#` (each `#x` in the eval code
    /// must be one of them).
    pub private_names: Vec<String>,
}

/// The kind of function that the `Function` constructor family makes (ECMA-262
/// CreateDynamicFunction `kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionCtorKind {
    /// `Function(...)`.
    Normal,
    /// `GeneratorFunction(...)`.
    Generator,
    /// `AsyncFunction(...)`.
    Async,
    /// `AsyncGeneratorFunction(...)`.
    AsyncGenerator,
}

impl FunctionCtorKind {
    /// The source text prefix (CreateDynamicFunction step 7-10): `function`, `function*`,
    /// `async function` or `async function*`.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Normal => "function",
            Self::Generator => "function*",
            Self::Async => "async function",
            Self::AsyncGenerator => "async function*",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_data() {
        let c = EvalContext::default();
        assert!(!c.strict && !c.direct && c.private_names.is_empty());
        assert!(EVAL_EXT_BINDING.starts_with('.'));
        assert_eq!(FunctionCtorKind::AsyncGenerator.prefix(), "async function*");
        assert_eq!(FunctionCtorKind::Normal.prefix(), "function");
    }
}
