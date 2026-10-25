//! Code made at run time (owner P3-EVAL-VM): [`Cx::evaluate_script`], and the steps that it,
//! `eval` (direct, `interp/slow/eval.rs`; indirect, `natives.rs`) and the `Function`
//! constructor family (`natives.rs`, `generator.rs`) share. ep9_vm has no edge to the front
//! end, so every one of them reaches it through the compiler hook (`IsolateConfig::compiler`,
//! C2).
//!
//! ```text
//! source text --source_text--> &str (a lone surrogate: not implemented, D8/E8: the shim is M3)
//!   --compile (hook)--> VerifiedUnit | CompileFailure -> SyntaxError, RangeError,
//!                                                       "unsupported: <f>", internal error
//!                       no hook -> EvalError (O2)
//!   --load_entry--> LoadedUnit (Runtime::units, never freed) + a closure of function 0
//!   --Call--> direct eval: closure over the caller's current context (the receiver is not read)
//!             indirect eval, Function(): no context, this = the global this
//!             evaluate_script: no context, this = the global object (as Cx::run_script)
//! ```
//!
//! | Compile failure (D7) | Thrown |
//! |---|---|
//! | `Syntax(m)` | `SyntaxError: m` |
//! | `Range(m)` | `RangeError: m` |
//! | `Unsupported(f)` | `Error: not implemented in ep9 v0: unsupported: f` (never a SyntaxError: an `assert.throws(SyntaxError, ...)` test must not pass on a construct that is not built yet, W4-P3-FALSE-PASS) |
//! | `Internal(m)` | `Error: ep9 internal error: m` |
//! | no hook | `EvalError: code generation from strings is not available (no compiler hook)` (V8 throws an EvalError when code generation from strings is not allowed) |
//!
//! Every unit made here stays loaded until the isolate drops (`Runtime::units` only grows):
//! each eval or `Function()` call costs its unit (P3-EVAL-VM task 8).

use ep9_bytecode::{CodeKind, EvalContext, FunctionCtorKind, VerifiedUnit};
use ep9_value::{JsResult, Throw};

use super::{Args, Cx, JsFunction, JsString, Local};
use crate::builtins::{self, IntrinsicId};
use crate::compiler::{CompileFailure, CompileResult, CompilerHooks};
use crate::frame::UNDEFINED;
use crate::kinds;
use crate::object;
use crate::runtime::ErrorKind;
use crate::string::{self, JsStr};
use crate::unit::{EvalPrivateName, EvalReceiver, FunctionInfo, LoadedUnit};

/// The message of the EvalError that eval, `Function()` and [`Cx::evaluate_script`] throw on
/// an isolate without compiler hooks (O2).
pub(crate) const NO_COMPILER: &str =
    "code generation from strings is not available (no compiler hook)";

/// The file name of eval units in stack traces and locations.
pub(crate) const EVAL_FILE: &str = "<eval>";

/// The file name of `Function()` units.
const FUNCTION_FILE: &str = "<anonymous>";

impl<'s> Cx<'s> {
    /// Runs `source` as a sloppy Script (ScriptEvaluation, §16.1.6) through the compiler hook
    /// `compile_script`: the unit is loaded as [`Cx::run_script`](crate::Cx::run_script)
    /// loads one, and runs with `this` = the global object. `file` names the unit in stack
    /// traces. The result is the value the script's entry function returns (undefined for a
    /// script).
    ///
    /// # Errors
    ///
    /// A pending exception: an early error (`SyntaxError`), a compile limit (`RangeError`), a
    /// construct that is not built yet (the v0 not-implemented `Error`), `EvalError` on an
    /// isolate without compiler hooks, or whatever the script throws.
    pub fn evaluate_script(&mut self, source: &str, file: &str) -> JsResult<Local<'s>> {
        let unit = self.compile_with(|h| (h.compile_script)(source))?;
        let f = self.load_entry(unit, file, UNDEFINED, EvalReceiver::default(), Box::new([]));
        let global = self.global();
        self.call(f, global, &[])
    }

    /// The text of the string `s` for the compiler hook (which takes `&str`). Source text with
    /// a lone surrogate cannot be UTF-8: it throws the v0 not-implemented `Error` (the shim is
    /// milestone-M3 work, D8; H05-Q3).
    pub(crate) fn source_text(&mut self, s: Local<'s, JsString>) -> JsResult<String> {
        let b = string::flatten(self.cx, s.bits());
        let text = match string::view(self.cx, b) {
            JsStr::Latin1(bytes) => Some(bytes.iter().map(|&c| char::from(c)).collect()),
            JsStr::Utf16(units) => String::from_utf16(units).ok(),
        };
        match text {
            Some(t) => Ok(t),
            None => Err(self.not_implemented("source text with a lone surrogate")),
        }
    }

    /// Runs `compile` with the isolate's compiler hooks and turns a failure into the thrown
    /// error (see the module doc).
    pub(crate) fn compile_with(
        &mut self,
        compile: impl FnOnce(&CompilerHooks) -> CompileResult,
    ) -> JsResult<VerifiedUnit> {
        let Some(hooks) = self.rt().compiler else {
            return Err(self.throw_error(ErrorKind::EvalError, NO_COMPILER));
        };
        compile(hooks).map_err(|f| self.throw_compile_failure(f))
    }

    fn throw_compile_failure(&mut self, f: CompileFailure) -> Throw {
        match f {
            CompileFailure::Syntax(m) => self.throw_error(ErrorKind::SyntaxError, &m),
            CompileFailure::Range(m) => self.throw_error(ErrorKind::RangeError, &m),
            CompileFailure::Unsupported(what) => {
                self.not_implemented(&format!("unsupported: {what}"))
            }
            CompileFailure::Internal(m) => {
                self.throw_error(ErrorKind::Error, &format!("ep9 internal error: {m}"))
            }
        }
    }

    /// Loads `unit` as [`Cx::run_script`](crate::Cx::run_script) does, with the
    /// receiver flags `receiver` and the caller's private names `private` (direct eval code;
    /// the default and empty for everything else), and
    /// returns a closure of its function 0 over `context` (undefined: no context). GC point
    /// (the unit's strings and constants, the closure).
    pub(crate) fn load_entry(
        &mut self,
        unit: VerifiedUnit,
        file: &str,
        context: u64,
        receiver: EvalReceiver,
        private: Box<[EvalPrivateName]>,
    ) -> Local<'s, JsFunction> {
        let rt = self.rt();
        let index = u32::try_from(rt.units.borrow().len()).expect("fewer than 2^32 units");
        let mut lu = LoadedUnit::load(rt, unit, file.to_string(), index);
        lu.eval_receiver = receiver;
        lu.eval_private = private;
        if lu.unit.header.code_kind == CodeKind::Eval {
            // The ScriptOrModule of the code that runs eval or `Function()` now: `import()`
            // in the new code and in its functions resolves from there (M3-P3).
            lu.script_or_module = Some(crate::module::eval_referrer(rt, context));
        }
        let entry: *const FunctionInfo = &lu.infos[0];
        rt.units.borrow_mut().push(lu);
        Self::local(object::new_function(rt, entry, context))
    }

    /// PerformEval (§19.2.1.1) of direct eval code with the caller's `ctx`: compiles `source`
    /// and returns a closure of the unit's function 0 over `context`, the caller's current
    /// context (`interp/slow/eval.rs` calls it). The unit keeps the receiver flags of `ctx`
    /// for the direct evals inside it, and `private`, where the caller's private names are
    /// (CX2-E9-1). GC point.
    pub(crate) fn direct_eval(
        &mut self,
        source: Local<'s, JsString>,
        ctx: &EvalContext,
        context: u64,
        private: Box<[EvalPrivateName]>,
    ) -> JsResult<Local<'s, JsFunction>> {
        let text = self.source_text(source)?;
        let unit = self.compile_with(|h| (h.compile_eval)(&text, ctx))?;
        Ok(self.load_entry(unit, EVAL_FILE, context, EvalReceiver::of(ctx), private))
    }

    /// PerformEval (§19.2.1.1) of indirect eval code: `x` is the argument of `%eval%`. A value
    /// that is not a string comes back unchanged (step 2). The code runs as global code: no
    /// context, `this` = the global this, every [`EvalContext`] flag false (steps 4-8); its
    /// sloppy `var` and function declarations become configurable global properties
    /// (`DeclareEvalVar` / `DeclareEvalFunction`, M2-P2-EVAL-SCOPE).
    pub(crate) fn indirect_eval(&mut self, x: Local<'s>) -> JsResult<Local<'s>> {
        let Some(s) = x.as_string() else {
            return Ok(x);
        };
        let text = self.source_text(s)?;
        let ctx = EvalContext::default();
        let unit = self.compile_with(|h| (h.compile_eval)(&text, &ctx))?;
        let f = self.load_entry(
            unit,
            EVAL_FILE,
            UNDEFINED,
            EvalReceiver::default(),
            Box::new([]),
        );
        let this: Local<'s> = Self::local(self.cx.global_this);
        self.call(f, this, &[])
    }

    /// CreateDynamicFunction (§20.2.1.1.1) for the constructor `args.callee()` and
    /// `new_target` (undefined for a `[[Call]]`).
    ///
    /// ```text
    /// ToString of each parameter argument, then of the body (the last argument, or "")
    /// compile_function(kind, params joined by ",", body)    the FNCTOR unit contract (D8):
    ///   a CodeKind::Eval unit whose function 0 returns the new closure
    /// GetPrototypeFromConstructor(newTarget, fallback)      step 27 (a proxy trap may run)
    /// function 0: no context, this = the global this        the closure is over the global env
    /// F.[[Prototype]] = proto
    /// ```
    ///
    /// `kind` is `Normal` (`%Function%`) or `Generator` (`%GeneratorFunction%`); the async
    /// kinds answer `unsupported` through the hook until month 3.
    pub(crate) fn create_dynamic_function(
        &mut self,
        args: Args<'s>,
        new_target: Local<'s>,
        kind: FunctionCtorKind,
    ) -> JsResult<Local<'s>> {
        let constructor = args.callee();
        let new_target = match new_target.as_object() {
            Some(nt) => nt,
            None => constructor.into(),
        };
        let n = args.len();
        let mut params = String::new();
        for i in 0..n.saturating_sub(1) {
            let p = self.to_string(args.get(i))?;
            if i > 0 {
                params.push(',');
            }
            params.push_str(&self.source_text(p)?);
        }
        let body = if n == 0 {
            String::new()
        } else {
            let b = self.to_string(args.get(n - 1))?;
            self.source_text(b)?
        };
        let unit = self.compile_with(|h| (h.compile_function)(kind, &params, &body))?;
        let fallback = match kind {
            FunctionCtorKind::Generator => IntrinsicId::GeneratorFunctionPrototype,
            // §20.2.1.1.1 step 6: fallbackProto of kind async generator is
            // %AsyncGeneratorFunction.prototype%.
            FunctionCtorKind::AsyncGenerator => IntrinsicId::AsyncGeneratorFunctionPrototype,
            // §20.2.1.1.1 step 5: fallbackProto of kind async is %AsyncFunction.prototype%.
            FunctionCtorKind::Async => IntrinsicId::AsyncFunctionPrototype,
            FunctionCtorKind::Normal => IntrinsicId::FunctionPrototype,
        };
        let proto = self.get_prototype_from_constructor(new_target, fallback)?;
        let entry = self.load_entry(
            unit,
            FUNCTION_FILE,
            UNDEFINED,
            EvalReceiver::default(),
            Box::new([]),
        );
        let this: Local<'s> = Self::local(self.cx.global_this);
        let f = self.call(entry, this, &[])?;
        let Some(fo) = f.as_function() else {
            return Err(self.throw_error(
                ErrorKind::Error,
                "ep9 internal error: the Function() unit did not return a function",
            ));
        };
        if object::proto_of(fo.bits()) != proto.bits() {
            self.set_prototype_of(fo, proto)?;
        }
        Ok(fo.into())
    }

    /// `%eval%` of this realm (undefined if no table made it).
    pub(crate) fn eval_intrinsic(&self) -> u64 {
        let v = builtins::intrinsic(self.rt(), IntrinsicId::Eval);
        debug_assert!(v == UNDEFINED || kinds::is_object(v));
        v
    }
}
