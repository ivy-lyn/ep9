//! Host (Rust) functions callable from JS: the names of the week-3 API. The builtin ABI v1
//! (H20) lives in [`crate::abi`]; this module keeps the old names for one release.
//!
//! | Old name | Now |
//! |---|---|
//! | `HostFn` | [`crate::abi::HostFn`]: `for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>` |
//! | `HostCx<'a>` | an alias of [`crate::abi::Cx`] (deprecated name; removed after one release) |
//! | `Args<'a>` | [`crate::abi::Args`]: one word, the host frame pointer |
//!
//! It also holds the host hook HostEnsureCanCompileStrings (§19.2.1.2; M6-P2 REALM-CLOSE,
//! plan v9 V9-A12): a fn pointer ([`CompileStringsHook`]), set per isolate by
//! [`Isolate::set_compile_strings_hook`]. The VM calls it at its one compile point of code
//! from strings (`Cx::compile_with`, `abi/script.rs`):
//!
//! ```text
//! eval(x), x a string (direct or indirect)   hook(cx, [], x, direct)        PerformEval
//! Function(p1, .., pn, body) and its kinds   hook(cx, [p1..pn], body, false) CreateDynamicFunction
//!   (after ToString of every argument)
//! eval(x), x not a string                    no call (PerformEval step 2 returns x)
//! Cx::evaluate_script, Cx::run_script        no call (a host's own script, not eval code)
//! ```
//!
//! `cx` is in the callee realm (§19.2.1.2 calleeRealm: the realm of `%eval%` or of the
//! `Function` constructor, which a builtin runs in, H20 R5; for a direct eval the realm of the
//! code that calls it), so `cx.global()` is that realm's global object. The hook runs before
//! the compiler hook, so a refused text is never parsed. No hook (the default) allows every
//! text, as the spec's default host does.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use ep9_value::{JsResult, Throw};

use crate::Isolate;
use crate::abi::Cx;
pub use crate::abi::HostFn;
use crate::runtime::{ErrorKind, Runtime, Shared};

/// The week-3 name of [`crate::abi::Cx`]. Kept for one release (H20); new code names `Cx`.
pub type HostCx<'s> = crate::abi::Cx<'s>;

/// The type of a HostEnsureCanCompileStrings hook (the parameter of
/// [`Isolate::set_compile_strings_hook`], which spells it out, so an embedder needs no name
/// for it).
pub(crate) type CompileStringsHook = for<'s> fn(&mut Cx<'s>, &[String], &str, bool) -> JsResult<()>;

/// The message of the `EvalError` for a hook's `Err` with no pending exception.
pub(crate) const DISALLOWED: &str = "code generation from strings disallowed for this realm";

/// One isolate's hook: the isolate's `Runtime::id`, its shared state (weak: it tells whether
/// the isolate still lives) and the hook.
struct Entry {
    id: u64,
    alive: Weak<Shared>,
    hook: CompileStringsHook,
}

std::thread_local! {
    /// The HostEnsureCanCompileStrings hooks of the isolates of this thread. An isolate is not
    /// `Send` (H22-Q5), so its entry is always on the thread that runs it. The key is
    /// `Runtime::id`, which no other isolate gets (as the module hosts, `module::HOSTS`).
    /// [`Isolate::set_compile_strings_hook`] drops the entries of dropped isolates. Holds no
    /// Value (rule 8): a fn pointer and a weak pointer to Rust memory.
    static HOOKS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

impl Isolate {
    /// Sets the HostEnsureCanCompileStrings hook of this isolate (§19.2.1.2; `None`: no hook,
    /// every text may compile, as the spec's default host). It holds for every realm of the
    /// isolate and replaces the hook set before.
    ///
    /// The VM calls `hook(cx, parameters, body, direct)` before it compiles code from a string
    /// (HostEnsureCanCompileStrings(calleeRealm, parameterStrings, bodyString, direct)):
    /// `cx` is in the callee realm (`cx.global()` is its global object), `parameters` are the
    /// parameter strings of a `Function` constructor call (empty for eval), `body` is the body
    /// or eval text, `direct` is true for a direct eval. `Ok(())` lets the VM compile the
    /// text. `Err` refuses it: the VM throws the hook's pending exception (the hook throws
    /// with [`Cx::throw_error`](crate::Cx::throw_error), for example an `EvalError` as the
    /// HTML host does); an `Err` with no pending exception throws `EvalError: code generation
    /// from strings disallowed for this realm`. The hook may run ABI code (and so JS) and
    /// collect garbage; code from strings that it makes asks the hook again.
    ///
    /// ```
    /// use ep9_vm::{Cx, ErrorKind, Isolate, IsolateConfig, JsResult};
    /// fn no_eval(cx: &mut Cx<'_>, _p: &[String], body: &str, _direct: bool) -> JsResult<()> {
    ///     Err(cx.throw_error(ErrorKind::EvalError, &format!("refused: {body}")))
    /// }
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// iso.set_compile_strings_hook(Some(no_eval));
    /// let e = iso.with_cx(|cx| {
    ///     let eval = cx.global_binding("eval")?.expect("%eval%");
    ///     let text = cx.new_string("1 + 1")?;
    ///     let g = cx.global();
    ///     let v = cx.call(eval, g, &[text.into()])?;
    ///     Ok(cx.display(v))
    /// });
    /// assert_eq!(e.unwrap_err().message, "EvalError: refused: 1 + 1");
    /// ```
    ///
    /// The hooks live in a per-thread table, so an isolate must stay on its thread (H22-Q5):
    ///
    /// ```compile_fail
    /// fn is_send<T: Send>() {}
    /// is_send::<ep9_vm::Isolate>();
    /// ```
    // The type is spelled out: `host` is not a public path, so a type name would be one that
    // an embedder cannot write.
    #[allow(clippy::type_complexity)]
    pub fn set_compile_strings_hook(
        &mut self,
        hook: Option<for<'s> fn(&mut Cx<'s>, &[String], &str, bool) -> JsResult<()>>,
    ) {
        let id = self.rt.id;
        let alive = Rc::downgrade(&self.rt.shared);
        HOOKS.with_borrow_mut(|t| {
            t.retain(|e| e.id != id && e.alive.strong_count() > 0);
            if let Some(hook) = hook {
                t.push(Entry { id, alive, hook });
            }
        });
    }
}

/// The hook of `rt`'s isolate, if it has one.
fn hook_of(rt: &Runtime) -> Option<CompileStringsHook> {
    HOOKS.with_borrow(|t| t.iter().find(|e| e.id == rt.id).map(|e| e.hook))
}

/// HostEnsureCanCompileStrings(the current realm, parameterStrings, bodyString, direct) for
/// the compile point `Cx::compile_with` (`abi/script.rs`): `strings` is `(parameters, body,
/// direct)` for eval and `Function()`, and `None` for a host's own script
/// ([`Cx::evaluate_script`]), which asks no hook. `Ok` if there is nothing to ask, the
/// isolate has no hook or its hook allows the text; else the hook's throw (see
/// [`Isolate::set_compile_strings_hook`]). GC point when a hook runs.
pub(crate) fn ensure_can_compile_strings(
    cx: &mut Cx<'_>,
    strings: Option<(&[String], &str, bool)>,
) -> JsResult<()> {
    let Some((parameters, body, direct)) = strings else {
        return Ok(());
    };
    let Some(hook) = hook_of(cx.inner().rt()) else {
        return Ok(());
    };
    match hook(cx, parameters, body, direct) {
        Ok(()) => {
            // R4: `Ok` comes with no pending exception.
            debug_assert!(
                cx.inner().rt().pending() == 0,
                "Ok with a pending exception"
            );
            Ok(())
        }
        Err(Throw) if cx.inner().rt().pending() == 0 => {
            Err(cx.throw_error(ErrorKind::EvalError, DISALLOWED))
        }
        Err(Throw) => Err(Throw),
    }
}

/// The number of entries in this thread's hook table (tests).
#[cfg(test)]
pub(crate) fn hook_count() -> usize {
    HOOKS.with_borrow(Vec::len)
}
