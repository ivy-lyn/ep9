//! Scope pass v0: for each function, where every binding lives and how every name resolves.
//!
//! ```text
//! oxc AST + oxc_semantic (reference ids, scope ids)
//!   |  collect.rs: one recursive walk (layer-3 depth check at every cycle point)
//!   v     scopes (preorder), bindings, references, direct-eval sites, function ids (preorder)
//!   |  resolve.rs: no recursion
//!   |     1. direct eval: mark scopes, make `this`/`arguments`/... visible to the eval code
//!   |     2. resolve every reference: Binding (static) | Dynamic (with, sloppy eval) | Global
//!   |     3. allocate every binding: Register | Parameter | Frame | Context | Global | Module
//!   |     4. context hops for every static reference to a context slot
//!   v
//! ScopeAnalysis   plain Rust data: no GC object, no oxc type, deterministic (H2)
//! ```
//!
//! Eval code (`analyze_eval`, P2-EVAL-FE) uses the same walk. The root is then a
//! [`ScopeKind::Eval`] scope (made after collection: the walk builds a Script root), and
//! resolution follows ECMA-262 PerformEval and EvalDeclarationInstantiation (§19.2.1.1,
//! §19.2.1.3):
//!
//! | Eval code | Free names (also `#x`) | `this`, `new.target`, `super` | Root `var` and function declarations |
//! |---|---|---|---|
//! | direct | Dynamic (C4): looked up by name from the caller's context | Dynamic, by the names `this`, `new.target`, `.home_object`, `.callee` of the caller's context slots | strict: local; sloppy: Dynamic, declared by `DeclareEvalVar` / `DeclareEvalFunction` |
//! | indirect | Global | `this`: the frame receiver (the global this, as a script) | strict: local; sloppy: Global, declared by `DeclareEvalVar` / `DeclareEvalFunction` |
//!
//! Lexical declarations at the eval top level are always local. The completion value `.result`
//! and its backups (names written by `eval.rs`) are root bindings that no lookup can make
//! dynamic.
//!
//! The pass resolves names itself, on its own scope tree. It does not take the context layout
//! from the oxc scope tree, because that tree differs from the ECMA-262 environments in places
//! that change where a binding must live (checked in `tests.rs`,
//! `oxc_scope_tree_differs_from_the_spec`):
//!
//! | ECMA-262 | oxc 0.155 tree | Effect on ep9 if it used the oxc tree |
//! |---|---|---|
//! | Parameters and body are 2 environments when a parameter has an expression | 1 scope (oxc still resolves the names right) | eval in a default value and closures need 2 contexts |
//! | A class has a scope with an inner, immutable name binding | no inner binding for declarations; methods see the outer one | `C = 1` in a method must throw; reassigning the outer `C` must not change the methods' `C` |
//! | A named function expression has its own name environment | the name sits with the parameters | a parameter `g` and the name `g` collide |
//! | Annex B block functions have a block binding and a var binding | one binding, moved to the var scope | the block binding and its TDZ-free copy semantics are lost |
//!
//! oxc ids are used only as lookup keys for BytecodeGen (`ScopeAnalysis::reference`,
//! `ScopeAnalysis::scope_for`); they never leave this crate (H1).

use std::fmt;

use crate::{ByteSpan, CompileError, DepthLayer, FrontEnd, RangeError};

mod collect;
mod resolve;
#[cfg(test)]
pub(crate) mod tests;

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            /// The id as a vector index.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }

            fn of(index: usize) -> Self {
                Self(u32::try_from(index).expect("fewer than 2^32 items"))
            }
        }
    };
}

define_id!(
    /// A function: its preorder index in the source tree (the H2 stable function id).
    /// Function 0 is the script or module. Synthetic class functions get their ids when the walk
    /// enters the class body: the default constructor, the instance initializer and the
    /// functions in the instance fields, then the static initializer and the functions in the
    /// static elements, then the methods (and the functions in computed keys) in element order.
    /// This is a preorder of the parent links (the verifier Tree rule; see [`ClassInfo`]).
    FunctionId
);
define_id!(
    /// A scope. Ids are in preorder: a parent scope has a smaller id than its children.
    ScopeId
);
define_id!(
    /// A binding: a declared name, or an implicit one (`this`, `arguments`, ...).
    BindingId
);
define_id!(
    /// A reference: one use of a name (or of `this`, `super`, `new.target`, `#x`).
    RefId
);
define_id!(
    /// An interned name. See [`ScopeAnalysis::name`].
    NameId
);

/// What kind of code a function is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    /// Function 0 of a Script.
    Script,
    /// Function 0 of a Module.
    Module,
    /// A `function` declaration or expression (also generator and async).
    Normal,
    /// An arrow function. It has no own `this`, `arguments`, `new.target` or home object.
    Arrow,
    /// A method in an object literal or a class.
    Method,
    /// A getter in an object literal or a class.
    Getter,
    /// A setter in an object literal or a class.
    Setter,
    /// The `constructor` of a class without `extends`.
    ClassConstructor,
    /// The `constructor` of a class with `extends`. `this` has a TDZ until `super()`.
    DerivedConstructor,
    /// Synthetic: the constructor of a class without `extends` and without `constructor`.
    DefaultConstructor,
    /// Synthetic: `constructor(...args) { super(...args); }` of a derived class.
    DefaultDerivedConstructor,
    /// Synthetic: runs the instance field initializers (`this` = the new object).
    InstanceMembersInitializer,
    /// Synthetic: runs static field initializers and static blocks in order (`this` = the class).
    StaticMembersInitializer,
}

impl FunctionKind {
    /// No source text of its own: made by the scope pass for a class.
    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        matches!(
            self,
            Self::DefaultConstructor
                | Self::DefaultDerivedConstructor
                | Self::InstanceMembersInitializer
                | Self::StaticMembersInitializer
        )
    }

    /// A constructor of a derived class: `super()` binds `this`.
    #[must_use]
    pub const fn is_derived_constructor(self) -> bool {
        matches!(
            self,
            Self::DerivedConstructor | Self::DefaultDerivedConstructor
        )
    }

    /// The function has a `[[HomeObject]]` (for `super.x`).
    #[must_use]
    pub const fn has_home_object(self) -> bool {
        matches!(
            self,
            Self::Method
                | Self::Getter
                | Self::Setter
                | Self::ClassConstructor
                | Self::DerivedConstructor
                | Self::DefaultConstructor
                | Self::DefaultDerivedConstructor
                | Self::InstanceMembersInitializer
                | Self::StaticMembersInitializer
        )
    }

    /// The function has its own `new.target` (undefined in the synthetic initializers).
    #[must_use]
    pub const fn has_new_target(self) -> bool {
        !matches!(self, Self::Script | Self::Module | Self::Arrow)
    }

    /// The function can have an `arguments` object (ECMA-262 FunctionDeclarationInstantiation;
    /// `arguments` is an early error in class field initializers and static blocks).
    #[must_use]
    pub const fn may_have_arguments(self) -> bool {
        matches!(
            self,
            Self::Normal
                | Self::Method
                | Self::Getter
                | Self::Setter
                | Self::ClassConstructor
                | Self::DerivedConstructor
        )
    }
}

/// One formal parameter (one slot in the caller's argument list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// The `FormalParameter` (or the rest element).
    pub span: ByteSpan,
    /// The binding, if the parameter is a plain identifier and no later parameter has the same
    /// name (sloppy duplicates: the last one wins).
    pub binding: Option<BindingId>,
    /// `...rest`. It is not a formal parameter slot: its value comes from `CreateRestParameter`.
    pub rest: bool,
}

/// A function and the things BytecodeGen needs to emit its prologue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionInfo {
    /// This function.
    pub id: FunctionId,
    /// What kind of code it is.
    pub kind: FunctionKind,
    /// `async`.
    pub is_async: bool,
    /// `function*` or `async function*`.
    pub is_generator: bool,
    /// A `static` class element (method, accessor, or the static initializer).
    pub is_static: bool,
    /// The enclosing function. `None` only for function 0.
    pub parent: Option<FunctionId>,
    /// Source range of the function node (the class body for synthetic functions).
    pub span: ByteSpan,
    /// Strict mode code.
    pub strict: bool,
    /// The scope in which the closure is created: its context is the closure's outer context.
    pub outer_scope: Option<ScopeId>,
    /// The function scope: parameters, `this`, `arguments` and, unless split, the body.
    pub scope: ScopeId,
    /// The scope of the body's `var` and top-level lexical declarations. It is `scope` unless
    /// a parameter has an expression (ECMA-262 FunctionDeclarationInstantiation step 28).
    pub body_scope: ScopeId,
    /// The name scope of a named function expression (holds [`FunctionInfo::self_name`]).
    pub name_scope: Option<ScopeId>,
    /// Formal parameters in order, then the rest parameter.
    pub params: Vec<Param>,
    /// No defaults, no patterns, no rest.
    pub simple_params: bool,
    /// A default value or a computed key in a parameter pattern.
    pub has_parameter_expressions: bool,
    /// `this` of this function, if used (also from inner arrows or a direct eval).
    pub this: Option<BindingId>,
    /// `new.target`, if used.
    pub new_target: Option<BindingId>,
    /// The `arguments` object, if used. It can be a `var arguments` binding (the spec shares it).
    pub arguments: Option<BindingId>,
    /// The arguments object is mapped (sloppy function with simple parameters): parameters
    /// and `arguments[i]` alias, so every parameter is in a context slot.
    pub mapped_arguments: bool,
    /// `[[HomeObject]]`, if `super.x` is used.
    pub home_object: Option<BindingId>,
    /// The function object itself, for `super()` in a derived constructor.
    pub callee: Option<BindingId>,
    /// The name binding of a named function expression.
    pub self_name: Option<BindingId>,
    /// A direct `eval(...)` call in this function's own code (not in nested functions).
    pub calls_eval: bool,
    /// A sloppy direct eval can add `var` bindings to a scope of this function.
    pub calls_sloppy_eval: bool,
    /// Registers for named locals (`r0..`). BytecodeGen adds its temporaries after them.
    pub register_count: u32,
}

/// What kind of scope. The runtime scope kinds (`ep9_bytecode::ScopeKind`) map as: `Function`,
/// `Script`, `Module`, `Eval`, `Catch`, `With`, `Class` to themselves; `FunctionName`,
/// `FunctionBody`, `For`, `Switch`, `StaticBlock` and `Block` to `Block`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    /// Script top level (global code).
    Script,
    /// Module top level.
    Module,
    /// Eval code top level: the new lexical environment of PerformEval (and, for strict eval
    /// code, its new variable environment). Function 0 of an eval unit keeps
    /// [`FunctionKind::Script`]: it has no own `new.target`, home object or `arguments`.
    Eval,
    /// The own name of a named function expression (outside the parameters).
    FunctionName,
    /// Parameters (and the body, unless split).
    Function,
    /// The body's var scope when a parameter has an expression.
    FunctionBody,
    /// `{ ... }`, a synthetic block around `if (x) function f() {}`, or a `catch` body.
    Block,
    /// The head of a `for`, `for-in` or `for-of` statement.
    For,
    /// The case block of a `switch`. It is non-linear: a case can skip a declaration.
    Switch,
    /// A `catch` parameter.
    Catch,
    /// The body of a `with` statement (an object environment).
    With,
    /// A class: the inner class name, private names, heritage and computed keys.
    Class,
    /// A class static block: a var scope inside the static initializer.
    StaticBlock,
}

/// How a loop head scope makes new bindings per iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PerIteration {
    /// Not a loop head, or no lexical declaration in it.
    No,
    /// `for (let ...;;)`: copy the bindings into a new environment each iteration.
    Copy,
    /// `for (let/const ... in/of ...)`: a new environment each iteration.
    Fresh,
}

/// The synthetic functions and the inner name of a class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    /// `extends` is present.
    pub derived: bool,
    /// The inner (immutable) class name binding.
    pub inner_name: Option<BindingId>,
    /// The constructor function: explicit or synthetic.
    pub constructor: Option<FunctionId>,
    /// Synthetic default constructor, when the class has no `constructor`.
    pub default_constructor: Option<FunctionId>,
    /// Synthetic instance members initializer, when the class has instance fields, accessors
    /// or private methods.
    pub instance_initializer: Option<FunctionId>,
    /// Synthetic static members initializer, when the class has static fields or static blocks.
    pub static_initializer: Option<FunctionId>,
}

/// A scope and its context layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeInfo {
    /// This scope.
    pub id: ScopeId,
    /// What kind of scope.
    pub kind: ScopeKind,
    /// The enclosing scope (across function boundaries). `None` only for the root.
    pub parent: Option<ScopeId>,
    /// The function whose code runs in this scope.
    pub function: FunctionId,
    /// Source range of the scope node.
    pub span: ByteSpan,
    /// Strict mode.
    pub strict: bool,
    /// Bindings in declaration order.
    pub bindings: Vec<BindingId>,
    /// Context slot `i` holds `context_slots[i]`.
    pub context_slots: Vec<BindingId>,
    /// BytecodeGen pushes a context when it enters this scope.
    pub needs_context: bool,
    /// A sloppy direct eval can add `var` bindings to this scope at run time. In a function
    /// var scope, binding 0 and context slot 0 are then [`BindingKind::EvalExt`].
    pub sloppy_eval: bool,
    /// (ABI 5 `ScopeFlags::PARAMETER_ENV`.) The function scope of a sloppy function with
    /// parameter expressions, whose `sloppy_eval` comes from a direct eval in the parameter
    /// list: ECMA-262 puts the parameters in an environment of their own between that eval
    /// and the variable environment (FunctionDeclarationInstantiation step 20).
    pub parameter_env: bool,
    /// A direct eval in this scope or an inner one (also in inner functions).
    pub contains_eval: bool,
    /// Loop head per-iteration bindings.
    pub per_iteration: PerIteration,
    /// Function declarations to instantiate on entry, in source order (the last one wins).
    pub hoisted_functions: Vec<(BindingId, FunctionId)>,
    /// For a class scope: its functions and inner name.
    pub class: Option<ClassInfo>,
}

/// What a private name is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivateKind {
    /// `#x` or `#x = v`.
    Field,
    /// `#m() {}`.
    Method,
    /// `get #x() {}` only.
    Getter,
    /// `set #x(v) {}` only.
    Setter,
    /// `get #x` and `set #x`.
    GetterSetter,
    /// `accessor #x`.
    Accessor,
}

/// How a binding was declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingKind {
    /// `var` (also an Annex B var for a block function).
    Var,
    /// `let`.
    Let,
    /// `const`.
    Const,
    /// `using` or `await using`.
    Using,
    /// `class C {}` (the outer, mutable binding).
    Class,
    /// The inner class name (immutable, visible in the class body and heritage).
    ClassInner,
    /// A function declaration at the top level of a function, script, module or static block.
    Function,
    /// A function declaration in a block (lexical; initialized on block entry).
    BlockFunction,
    /// A formal parameter (or a name in a parameter pattern).
    Parameter,
    /// A `catch` parameter (or a name in its pattern).
    CatchParameter,
    /// The own name of a named function expression (immutable).
    FunctionName,
    /// An import binding.
    Import,
    /// A private name of a class.
    PrivateName {
        /// What the name is.
        kind: PrivateKind,
        /// `static`.
        is_static: bool,
    },
    /// Implicit: `this` of a function, script or module.
    This,
    /// Implicit: `new.target`.
    NewTarget,
    /// Implicit: the `arguments` object.
    Arguments,
    /// Implicit: `[[HomeObject]]` of a method.
    HomeObject,
    /// Implicit: the running function object (for `super()`).
    Callee,
    /// Implicit: the object that holds the `var` and function declarations of sloppy direct
    /// eval code ([`ep9_bytecode::EVAL_EXT_BINDING`]). Context slot 0 of each function var scope
    /// with a sloppy direct eval (never a script scope: script-level eval vars go to the
    /// global object). It starts undefined; the VM makes the object (C6).
    EvalExt,
}

impl BindingKind {
    /// The binding is uninitialized until its declaration runs (ECMA-262 TDZ).
    #[must_use]
    pub const fn has_tdz(self) -> bool {
        matches!(
            self,
            Self::Let | Self::Const | Self::Using | Self::Class | Self::ClassInner | Self::Import
        )
    }

    /// An assignment through this binding throws (`const`, imports, class inner names) or is
    /// ignored in sloppy mode (function expression names).
    #[must_use]
    pub const fn is_immutable(self) -> bool {
        matches!(
            self,
            Self::Const | Self::Using | Self::ClassInner | Self::Import | Self::FunctionName
        )
    }

    /// Implicit bindings have no declaration in the source.
    #[must_use]
    pub const fn is_implicit(self) -> bool {
        matches!(
            self,
            Self::This
                | Self::NewTarget
                | Self::Arguments
                | Self::HomeObject
                | Self::Callee
                | Self::EvalExt
        )
    }
}

/// A frame slot that the VM fills on entry (not a register name, H02 F5/F11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameSlot {
    /// The receiver (`this`).
    Receiver,
    /// `new.target`.
    NewTarget,
    /// The running function object.
    Callee,
    /// `[[HomeObject]]` of the running function object.
    HomeObject,
}

/// Where a binding lives at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingLocation {
    /// Register `r<n>` of the owning function: a local that no closure captures.
    Register(u32),
    /// Parameter slot `a<n>` (simple parameter lists only).
    Parameter(u32),
    /// A frame slot the VM fills on entry.
    Frame(FrameSlot),
    /// Slot `slot` of the context of `scope` (always the binding's own scope).
    Context {
        /// The scope whose context holds the slot.
        scope: ScopeId,
        /// The slot index in [`ScopeInfo::context_slots`].
        slot: u32,
    },
    /// A property of the global object (script top-level `var` and function declarations).
    Global,
    /// A global lexical binding (script top-level `let`, `const`, `class`), looked up by name.
    GlobalLexical,
    /// A module environment cell: an import, or a top-level binding that is exported.
    Module,
}

/// Why a binding is in a context slot instead of a register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextReason {
    /// An inner function (closure) uses it.
    Captured,
    /// A direct eval in this scope or an inner scope can read it by name.
    Eval,
    /// A reference through a `with` object or a sloppy-eval scope may resolve to it at run time.
    DynamicLookup,
    /// A parameter that a mapped (sloppy) arguments object aliases.
    MappedArguments,
}

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The name (`this`, `new.target`, `.home_object`, `.callee`, `.eval_ext`, `#x` for implicit
    /// and private names; `*default*` for an anonymous default export).
    pub name: NameId,
    /// How it was declared.
    pub kind: BindingKind,
    /// The scope that declares it.
    pub scope: ScopeId,
    /// The declaring identifier. `None` for implicit bindings.
    pub span: Option<ByteSpan>,
    /// Where it lives.
    pub location: BindingLocation,
    /// Why it is in a context slot, if it is.
    pub context_reason: Option<ContextReason>,
    /// The binding is uninitialized until its declaration runs: references may need a hole check.
    pub tdz: bool,
    /// For a TDZ binding: a reference in the same function, in a linear scope, at or after this
    /// byte offset runs after the initialization (V8 rule; the end of the declarator).
    pub init_pos: u32,
    /// An inner function uses it.
    pub captured: bool,
    /// At least one reference resolves to it.
    pub referenced: bool,
    /// Module: exported (lives in a module cell).
    pub exported: bool,
    /// Annex B.3.2: the var binding that receives this block function when its declaration
    /// is evaluated.
    pub annex_b_var: Option<BindingId>,
    /// The binding starts with the value of this binding: a body `var x` with a parameter `x`
    /// when parameters and body are split, or `var arguments` with the arguments object.
    pub init_from: Option<BindingId>,
}

/// What a reference is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// An `IdentifierReference`, with the oxc read/write flags.
    Identifier {
        /// Reads the value.
        read: bool,
        /// Writes the value.
        write: bool,
    },
    /// The local name in `export { name }`.
    Export,
    /// A `var` declarator with an initializer (or in a for-in/of head). The assignment goes
    /// through the scope chain: it can hit a `catch` parameter or a `with` object.
    VarInit,
    /// `this`, or the receiver of `super.x` and `super()`.
    This,
    /// `new.target`, or the one `super()` passes on.
    NewTarget,
    /// The home object of `super.x`.
    HomeObject,
    /// The function object for `super()`.
    Callee,
    /// `#x` in `o.#x` or `#x in o`.
    PrivateName,
}

/// Where a reference goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// A statically known binding.
    Binding {
        /// The binding.
        binding: BindingId,
        /// For a context slot: how many contexts to walk out from the reference's scope.
        hops: u32,
        /// The binding may be uninitialized here: throw a ReferenceError on the hole.
        hole_check: bool,
    },
    /// No declaration in any enclosing scope: a global lookup by name.
    Global,
    /// A `with` object or a sloppy direct eval may provide the name at run time: look it up by
    /// name. `fallback` is the binding the static lookup found (`None`: global).
    Dynamic {
        /// The static result.
        fallback: Option<BindingId>,
    },
}

/// One reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The name.
    pub name: NameId,
    /// What kind of reference.
    pub kind: RefKind,
    /// The scope it appears in.
    pub scope: ScopeId,
    /// The function it appears in.
    pub function: FunctionId,
    /// The identifier (or `this`, `super`, `new.target`, `#x`).
    pub span: ByteSpan,
    /// Where it goes.
    pub resolution: Resolution,
}

/// Lookup key for references that have no oxc reference id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ImplicitRef {
    VarInit,
    This,
    NewTarget,
    HomeObject,
    Callee,
    Private,
}

/// The scope description of one source text. Plain Rust data: no GC object, no oxc type.
/// All ids index the vectors. The same source and options always give an equal value (H2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeAnalysis {
    names: Vec<Box<str>>,
    functions: Vec<FunctionInfo>,
    scopes: Vec<ScopeInfo>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    by_oxc_reference: Vec<u32>,
    by_oxc_scope: Vec<u32>,
    declared_at: Vec<(u32, BindingId)>,
    implicit: Vec<(u32, ImplicitRef, RefId)>,
    function_at: Vec<(u32, FunctionId)>,
    synthetic_blocks: Vec<(u32, ScopeId)>,
    /// (M3-P4, E9) Direct eval code: the private names of the caller's classes (with the
    /// `#`), their kind (`Field`, `Method` or `GetterSetter` for any accessor) and whether
    /// they are static, from the `EvalContext`. A `#x` reference of the eval code that no class
    /// of the eval code declares resolves `Dynamic`, and BytecodeGen compiles it by this kind.
    caller_private: Vec<(Box<str>, PrivateKind, bool)>,
}

const NONE: u32 = u32::MAX;

impl ScopeAnalysis {
    /// (E9) Sets the private names of the caller's classes (see the field).
    pub(crate) fn set_caller_private(&mut self, names: Vec<(Box<str>, PrivateKind, bool)>) {
        self.caller_private = names;
    }

    /// (E9) The kind of the caller's private name `name` (with the `#`) and whether it is
    /// static.
    #[must_use]
    pub(crate) fn caller_private(&self, name: &str) -> Option<(PrivateKind, bool)> {
        self.caller_private
            .iter()
            .find(|(n, _, _)| &**n == name)
            .map(|&(_, kind, is_static)| (kind, is_static))
    }

    /// Functions by id. `functions()[0]` is the script or module.
    #[must_use]
    pub fn functions(&self) -> &[FunctionInfo] {
        &self.functions
    }

    /// Scopes by id. `scopes()[0]` is the root.
    #[must_use]
    pub fn scopes(&self) -> &[ScopeInfo] {
        &self.scopes
    }

    /// Bindings by id.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// References by id, in source order of their walk.
    #[must_use]
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// The text of a name.
    #[must_use]
    pub fn name(&self, id: NameId) -> &str {
        &self.names[id.index()]
    }

    /// One function.
    #[must_use]
    pub fn function(&self, id: FunctionId) -> &FunctionInfo {
        &self.functions[id.index()]
    }

    /// One scope.
    #[must_use]
    pub fn scope(&self, id: ScopeId) -> &ScopeInfo {
        &self.scopes[id.index()]
    }

    /// One binding.
    #[must_use]
    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.index()]
    }

    /// The binding declared by the identifier that starts at byte `start` (a `BindingIdentifier`
    /// of a declaration, parameter, catch parameter, import or function name). For a class
    /// declaration it is the outer binding; see [`ClassInfo::inner_name`] for the inner one.
    #[must_use]
    pub fn declared_at(&self, start: u32) -> Option<BindingId> {
        lookup(&self.declared_at, |&(s, _)| s, start).map(|&(_, b)| b)
    }

    /// The function whose node (`Function` or `ArrowFunctionExpression`) starts at byte `start`.
    #[must_use]
    pub fn function_at(&self, start: u32) -> Option<FunctionId> {
        lookup(&self.function_at, |&(s, _)| s, start).map(|&(_, f)| f)
    }

    /// The synthetic block scope around `if (x) function f() {}` (Annex B.3.3), by the start of
    /// the function declaration.
    #[must_use]
    pub fn synthetic_block_at(&self, start: u32) -> Option<ScopeId> {
        lookup(&self.synthetic_blocks, |&(s, _)| s, start).map(|&(_, s)| s)
    }

    /// The reference of an `IdentifierReference`, by its oxc reference id.
    #[must_use]
    pub(crate) fn reference(&self, id: oxc_semantic::ReferenceId) -> Option<&Reference> {
        match self.by_oxc_reference.get(id.index()) {
            Some(&r) if r != NONE => Some(&self.references[r as usize]),
            _ => None,
        }
    }

    /// The scope made for an oxc scope node (Program, function, arrow, block, for, switch,
    /// catch, with, class, static block).
    #[must_use]
    pub(crate) fn scope_for(&self, id: oxc_semantic::ScopeId) -> Option<ScopeId> {
        match self.by_oxc_scope.get(id.index()) {
            Some(&s) if s != NONE => Some(ScopeId(s)),
            _ => None,
        }
    }

    /// A reference without an oxc reference id, by the start of its node: `this`, `super`
    /// (`This` + `HomeObject`, or `This` + `NewTarget` + `Callee` for `super()`), `new.target`,
    /// `#x`, and `var` initializers (by the identifier).
    #[must_use]
    pub(crate) fn implicit(&self, start: u32, kind: ImplicitRef) -> Option<&Reference> {
        let i = self
            .implicit
            .partition_point(|&(s, k, _)| (s, k) < (start, kind));
        match self.implicit.get(i) {
            Some(&(s, k, r)) if s == start && k == kind => Some(&self.references[r.index()]),
            _ => None,
        }
    }
}

fn lookup<T>(sorted: &[T], key: impl Fn(&T) -> u32, start: u32) -> Option<&T> {
    let i = sorted.partition_point(|t| key(t) < start);
    sorted.get(i).filter(|t| key(t) == start)
}

/// Direct or indirect eval code (ECMA-262 §19.2.1.1 PerformEval `direct`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EvalKind {
    /// `eval(...)` that calls %eval%: the code sees the caller's scope.
    Direct,
    /// Any other call of %eval%: global code.
    Indirect,
}

/// Scope pass entry point, on the front-end worker after oxc_semantic. BytecodeGen calls it
/// first and keeps the result for the whole unit.
///
/// # Errors
///
/// A layer-3 [`RangeError`] if the walk finds nesting above [`crate::AST_CAP`]. Layer 2 has
/// already checked the same tree, so this is a second guard, not an expected path.
pub(crate) fn analyze(fe: &FrontEnd<'_>) -> Result<ScopeAnalysis, CompileError> {
    analyze_with_cap(fe, crate::AST_CAP)
}

/// [`analyze`] for eval code (a Script-goal `fe`): the root becomes a [`ScopeKind::Eval`]
/// scope after collection, and resolution and allocation follow the module doc table.
///
/// # Errors
///
/// As [`analyze`].
pub(crate) fn analyze_eval(
    fe: &FrontEnd<'_>,
    kind: EvalKind,
) -> Result<ScopeAnalysis, CompileError> {
    analyze_full(fe, crate::AST_CAP, Some(kind))
}

/// [`analyze`] with a given depth cap (tests use a small one to reach layer 3).
pub(crate) fn analyze_with_cap(fe: &FrontEnd<'_>, cap: u32) -> Result<ScopeAnalysis, CompileError> {
    analyze_full(fe, cap, None)
}

/// The scope pass for a script or module (`eval` = `None`) or for eval code.
fn analyze_full(
    fe: &FrontEnd<'_>,
    cap: u32,
    eval: Option<EvalKind>,
) -> Result<ScopeAnalysis, CompileError> {
    let mut builder = collect::Builder::new(fe, cap);
    builder.collect(fe.program);
    if let Some(at) = builder.too_deep {
        return Err(RangeError::too_deep(DepthLayer::Pass, fe.source, Some(at)).into());
    }
    if eval.is_some() {
        // The walk made a Script root (collect.rs `visit_program`); its function 0 keeps
        // FunctionKind::Script.
        debug_assert_eq!(builder.scopes[0].kind, ScopeKind::Script);
        builder.scopes[0].kind = ScopeKind::Eval;
    }
    builder.resolve(eval);
    builder.allocate();
    Ok(builder.finish())
}

impl fmt::Display for ScopeAnalysis {
    /// A stable text dump (tests and debugging).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for func in &self.functions {
            write!(
                f,
                "fn{} {:?}{}{}{} scope=s{} body=s{} regs={}",
                func.id.0,
                func.kind,
                if func.strict { " strict" } else { "" },
                if func.is_async { " async" } else { "" },
                if func.is_generator { " generator" } else { "" },
                func.scope.0,
                func.body_scope.0,
                func.register_count,
            )?;
            if let Some(args) = func.arguments {
                let mapped = if func.mapped_arguments {
                    "mapped"
                } else {
                    "unmapped"
                };
                write!(f, " arguments=b{}({mapped})", args.0)?;
            }
            writeln!(f)?;
        }
        for scope in &self.scopes {
            write!(f, "s{} {:?} fn{}", scope.id.0, scope.kind, scope.function.0)?;
            if let Some(p) = scope.parent {
                write!(f, " parent=s{}", p.0)?;
            }
            if scope.needs_context {
                write!(f, " context[{}]", scope.context_slots.len())?;
            }
            if scope.sloppy_eval {
                write!(f, " sloppy-eval")?;
            }
            if scope.parameter_env {
                write!(f, " parameter-env")?;
            }
            writeln!(f)?;
            for &b in &scope.bindings {
                let binding = &self.bindings[b.index()];
                write!(
                    f,
                    "  b{} {} {:?} {:?}",
                    b.0,
                    self.name(binding.name),
                    binding.kind,
                    binding.location
                )?;
                if let Some(reason) = binding.context_reason {
                    write!(f, " ({reason:?})")?;
                }
                if binding.tdz {
                    write!(f, " tdz@{}", binding.init_pos)?;
                }
                if let Some(var) = binding.annex_b_var {
                    write!(f, " annex-b=b{}", var.0)?;
                }
                if let Some(from) = binding.init_from {
                    write!(f, " init-from=b{}", from.0)?;
                }
                writeln!(f)?;
            }
        }
        for r in &self.references {
            write!(f, "{}@{} {:?} -> ", self.name(r.name), r.span.start, r.kind)?;
            match r.resolution {
                Resolution::Binding {
                    binding,
                    hops,
                    hole_check,
                } => write!(
                    f,
                    "b{} hops={hops}{}",
                    binding.0,
                    if hole_check { " hole-check" } else { "" }
                )?,
                Resolution::Global => write!(f, "global")?,
                Resolution::Dynamic { fallback } => match fallback {
                    Some(b) => write!(f, "dynamic(b{})", b.0)?,
                    None => write!(f, "dynamic(global)")?,
                },
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
