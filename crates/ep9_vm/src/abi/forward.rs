//! Forwarded calls and the human fast-path switch (H20 part 2, M7-P0 CALLHOST; H20-Q3 rest,
//! plan V12-D30).
//!
//! ```text
//! Function.prototype.apply ── IsCallable(this) ──> Cx::forward_apply(args)
//! Function.prototype.call  ── IsCallable(this) ──> Cx::forward_call(args)
//!   each: (1) record the running builtin as the realm's %Function.prototype.apply% or
//!             %Function.prototype.call% (IntrinsicId 93, 94; no table makes them)
//!         (2) the builtin's remaining steps, as before (CreateListFromArrayLike and
//!             call_built; the H20-Q3 tail call)
//! slow::call (interp/slow/call.rs `forward`): a later call of that function object from
//!   bytecode, whose `this` is a bytecode function of the realm and whose list is undefined,
//!   null, an unmodified arguments object or a packed Array, pushes the callee frame in the
//!   same dispatch run; the builtin does not run
//! ```
//!
//! The record is safe because the builtin's whole body after its IsCallable test is the
//! method: the VM then knows every step that the function object takes, and `slow::call`
//! takes the same steps. The record is made only for the running host function's own
//! [`Args`] (its host frame is the top frame), so a call with other values records nothing.
//!
//! [`Cx::set_human_fast_paths`] is doc-hidden: the FASTDIFF tests
//! (`ep9_builtins/tests/fastpath_diff.rs`) turn the human fast paths off with it, to compare
//! each fast path with its spec steps (`call::human_fast_paths`).

use ep9_value::JsResult;

use super::{Args, Cx, Local};
use crate::builtins::{self, IntrinsicId};
use crate::{call, realm};

impl<'s> Cx<'s> {
    /// Function.prototype.apply ( thisArg, argArray ) (§20.2.3.1) steps 3-6 for the builtin
    /// whose `args` these are, after its step 2 (IsCallable(this) is true): Call(func,
    /// thisArg) when argArray is undefined or null, else Call(func, thisArg,
    /// ? CreateListFromArrayLike(argArray)), with func = `args.this()`, thisArg = `args.get(0)`
    /// and argArray = `args.get(1)`. The list is built on the VM stack (an [`ArgsBuilder`])
    /// and copied once, into the callee frame (H20-Q3).
    ///
    /// It also records `args.callee()` as `%Function.prototype.apply%` of its realm
    /// ([`IntrinsicId::FunctionPrototypeApply`]), so the interpreter runs these steps itself
    /// for a later call of that function from bytecode (module doc). So a host function calls
    /// this only as `Function.prototype.apply` does: its IsCallable test, then
    /// `return cx.forward_apply(args)`.
    ///
    /// [`ArgsBuilder`]: super::ArgsBuilder
    pub fn forward_apply(&mut self, args: Args<'s>) -> JsResult<Local<'s>> {
        self.record_forwarder(&args, IntrinsicId::FunctionPrototypeApply);
        let (func, this_arg, arg_array) = (args.this(), args.get(0), args.get(1));
        // 3. If argArray is either undefined or null, then
        if arg_array.is_undefined_or_null() {
            // a. Perform PrepareForTailCall().
            // b. Return ? Call(func, thisArg).
            return self.call(func, this_arg, &[]);
        }
        // 4. Let argList be ? CreateListFromArrayLike(argArray).
        let arg_list = self.create_list_from_array_like(arg_array)?;
        // 5. Perform PrepareForTailCall() (H20-Q3: not here; a tail form of the list copy gave
        //    raytrace +0.93%, H20:175).
        // 6. Return ? Call(func, thisArg, argList).
        self.call_built(func, this_arg, arg_list)
    }

    /// Function.prototype.call ( thisArg, ...args ) (§20.2.3.3) steps 3-4 for the builtin
    /// whose `args` these are, after its step 2 (IsCallable(this) is true): PrepareForTailCall,
    /// then Call(func, thisArg, args) with func = `args.this()`, thisArg = `args.get(0)` and
    /// the arguments after it ([`Cx::tail_call_args`]: the host function must return the
    /// result at once).
    ///
    /// It also records `args.callee()` as `%Function.prototype.call%` of its realm
    /// ([`IntrinsicId::FunctionPrototypeCall`]), as [`Cx::forward_apply`] does.
    pub fn forward_call(&mut self, args: Args<'s>) -> JsResult<Local<'s>> {
        self.record_forwarder(&args, IntrinsicId::FunctionPrototypeCall);
        self.tail_call_args(args.this(), args.get(0), args.rest(1))
    }

    /// Turns every human fast path of this isolate on (the default) or off, for the FASTDIFF
    /// tests only (plan V12-D30): with them off, the VM and the builtins that read the switch
    /// take their spec steps (`call::human_fast_paths`). Not an embedder API.
    #[doc(hidden)]
    pub fn set_human_fast_paths(&mut self, on: bool) {
        call::set_human_fast_paths(self.rt(), on);
    }

    /// Records the function of the running host call `args` as the intrinsic `id` of the
    /// current realm (its own realm: `call::call_host` switched to it), if `args` belongs to
    /// the top frame and the intrinsic is not that function already. Not a GC point.
    fn record_forwarder(&mut self, args: &Args<'s>, id: IntrinsicId) {
        let rt = self.rt();
        if rt.stack().fp().cast_const() != args.fp {
            return;
        }
        let f = args.callee().bits();
        if builtins::intrinsic(rt, id) != f && crate::object::function_realm(f) == self.cx.realm {
            builtins::load_intrinsic(rt, id, f);
            realm::record_intrinsic(rt, id, f);
        }
    }
}
