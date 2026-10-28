//! The embedding API: one `Isolate` per thread (D14).
//!
//! ```text
//! let mut iso = Isolate::new(IsolateConfig::default())?;      heap, VM stack, realm
//! iso.register_host_function("print", 1, print);              a global host function
//! match iso.with_cx_checkpoint(|cx| cx.run_script(unit, "main.js").map(|v| cx.display(v))) {
//!     Ok(text) => println!("{text}"),                         a Cx scope, then the jobs
//!     Err(e) => eprintln!("{e}"),                             "main.js:3:7: TypeError: ..."
//! }
//! ```
//!
//! JS values reach the embedder only as `Local`s inside [`Isolate::with_cx`] or
//! [`Isolate::with_cx_checkpoint`], which cannot leave the call, so no value outlives a
//! collection (H20-Q4): copy what you need (a `String`, a number) out of the scope. The
//! scoped forms (M2-P3, C11) are `Cx` methods:
//!
//! | Old form (raw `Value`, removed in M3-P3) | Scoped form inside `with_cx` (`Local<'s>`) |
//! |---|---|
//! | `run_script(unit, file)` | [`abi::Cx::run_script`] in [`Isolate::with_cx_checkpoint`] |
//! | `call(f, this, args)` | `abi::Cx::call` in [`Isolate::with_cx_checkpoint`] |
//! | `global(name)` | [`abi::Cx::global_binding`] (runs getters; TDZ throws) |
//! | `display(v)` | [`abi::Cx::display`] |
//!
//! `import()` uses the hooks of [`Isolate::with_module_host`] (or of `abi::Cx::run_module`).
//!
//! Other threads reach a running isolate through two `Send` handles: [`TerminateHandle`]
//! (stop the script) and [`InterruptHandle`] (run a callback on the isolate thread at the
//! next interrupt poll).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError};

use ep9_bytecode::VerifiedUnit;
use ep9_heap::{GcStats, Heap, HeapConfig, HeapError, VerifyError, VerifyReport};
use ep9_value::{JsResult, Throw};

use crate::abi::{JsKey, JsObject, Local};
use crate::builtins::{self, BuiltinTable};
use crate::compiler::{CompileResult, CompilerHooks};
use crate::conv::{Res, describe};
use crate::exec::ExecOptions;
use crate::frame::{HOLE, UNDEFINED};
use crate::host::HostFn;
use crate::interp::{BUDGET, Cx, Dispatch};
use crate::object::{self, attr, is_error, is_string, own_define, own_get};
use crate::runtime::{Atom, ErrorKind, Root, Runtime, Site, WeakInterruptQueue};
use crate::unit::{FunctionInfo, LoadedUnit};
use crate::{abi, atoms, call, error, property, realm, shape, string};

/// Isolate settings.
#[derive(Clone, Debug)]
pub struct IsolateConfig {
    pub heap: HeapConfig,
    /// VM register stack size in 8-byte slots (D21). Frames past it throw a RangeError.
    pub vm_stack_slots: usize,
    /// The dispatch expansion (H15). Default: [`Dispatch::DEFAULT`].
    pub dispatch: Dispatch,
    /// Frames in `Error.prototype.stack` (V8: `Error.stackTraceLimit` = 10).
    pub stack_trace_limit: usize,
    /// Calls of a function before the interpreter gives it a feedback vector (H16): 0 = at
    /// the first call (eager), `u32::MAX` = never. Default
    /// [`IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD`].
    pub feedback_threshold: u32,
    /// The builtin table to install into the realm (H20), after the VM's own table:
    /// `ep9_builtins::TABLE` for the ECMAScript builtins. `None`: only the VM's builtins.
    pub builtins: Option<&'static BuiltinTable>,
    /// The compiler entry points for eval, `Function()` and run-time scripts (C2; ep9_vm
    /// does not link the compiler). `None`: those paths throw their v0 errors.
    pub compiler: Option<&'static CompilerHooks>,
    /// HostPromiseRejectionTracker (§27.2.1.9): called when a promise is rejected while it
    /// has no handler ("reject") and when such a promise gets one ("handle"). `None`: no hook.
    /// Either way the isolate keeps the unhandled ones for
    /// [`Isolate::take_unhandled_rejections`].
    pub promise_rejection_tracker: Option<abi::PromiseRejectionTracker>,
}

impl IsolateConfig {
    /// The default of [`IsolateConfig::feedback_threshold`].
    pub const DEFAULT_FEEDBACK_THRESHOLD: u32 = 8;
}

impl Default for IsolateConfig {
    fn default() -> Self {
        IsolateConfig {
            heap: HeapConfig::default(),
            vm_stack_slots: 1 << 20,
            dispatch: Dispatch::DEFAULT,
            stack_trace_limit: 10,
            feedback_threshold: IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD,
            builtins: None,
            compiler: None,
            promise_rejection_tracker: None,
        }
    }
}

/// Why `Isolate::new` failed.
#[derive(Debug)]
pub enum IsolateError {
    Heap(HeapError),
    Config(&'static str),
    /// A builtin table breaks a table rule, or installing it failed (H20).
    BuiltinTable(String),
}

impl std::fmt::Display for IsolateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolateError::Heap(e) => write!(f, "{e}"),
            IsolateError::Config(m) => f.write_str(m),
            IsolateError::BuiltinTable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for IsolateError {}

/// A source position: 1-based line and 1-based UTF-16 column (0 when the unit has no source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            f.write_str(&self.file)
        } else {
            write!(f, "{}:{}:{}", self.file, self.line, self.column)
        }
    }
}

/// An uncaught exception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exception {
    /// `Name: message` for an Error object; the value's text otherwise.
    pub message: String,
    /// The `name` of the thrown value's `constructor` (`TypeError`), or `None` for a
    /// primitive. test262 negative tests name the constructor of the expected error.
    pub constructor: Option<String>,
    /// The `stack` property of an Error object (message line plus `at` lines).
    pub stack: Option<String>,
    /// Where the Error was created, or where a non-Error value was thrown.
    pub location: Option<SourceLocation>,
    /// True if [`TerminateHandle::terminate`] stopped the run.
    pub terminated: bool,
}

impl std::fmt::Display for Exception {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.location {
            Some(l) => write!(f, "{l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Stops the running script of an isolate from any thread. The isolate polls it at loop back
/// edges and function entries, and in builtin loops that have neither (`Cx::poll_interrupt`);
/// the exception it raises cannot be caught by JS.
#[derive(Clone, Debug)]
pub struct TerminateHandle(Arc<AtomicU32>);

impl TerminateHandle {
    pub fn terminate(&self) {
        self.0
            .fetch_or(crate::interp::interrupt::TERMINATE, Ordering::Release);
    }
}

/// Runs a callback on the isolate's thread from any thread (H23-Q6): [`InterruptHandle::request`]
/// queues it, and the isolate runs it at its next interrupt poll, where [`TerminateHandle`] stops
/// a script (loop back edges, function entries, builtin loops; JIT code polls the same way). A
/// poll comes once per interrupt budget (10,000 back edges, bytecode calls or other budget units)
/// while JS runs; a call of a host function that is not bound takes no budget unit (a call of a
/// bound function takes one unit per bound level), so code that only calls host functions does
/// not poll. An idle isolate runs the callback when it next runs JS and polls.
///
/// The callback gets no `Cx`: it runs no JS and allocates nothing on the JS heap, so it is safe at
/// any poll. It may call [`TerminateHandle::terminate`] (the same poll then stops the script) or
/// [`InterruptHandle::request`] (the new callback runs at a later poll). Callbacks run in request
/// order. A callback that panics ends the process in a release build (`panic = "abort"`, H23); in
/// a debug build the panic unwinds out of the running script and the callbacks queued behind it
/// are dropped. Callbacks still queued when the isolate is dropped are dropped with it, never run
/// (the handle holds the queue weakly, so a callback that holds a handle keeps nothing alive).
/// After that, a request is never run: the callback is dropped at once.
///
/// ```
/// use std::sync::atomic::{AtomicBool, Ordering};
/// use std::sync::Arc;
/// use ep9_vm::{Isolate, IsolateConfig};
/// let iso = Isolate::new(IsolateConfig::default()).unwrap();
/// let ran = Arc::new(AtomicBool::new(false));
/// let (h, r) = (iso.interrupt_handle(), Arc::clone(&ran));
/// std::thread::spawn(move || h.request(Box::new(move || r.store(true, Ordering::SeqCst))))
///     .join()
///     .unwrap();
/// // Not run yet: the isolate has not polled.
/// assert!(!ran.load(Ordering::SeqCst));
/// ```
#[derive(Clone)]
pub struct InterruptHandle {
    word: Arc<AtomicU32>,
    queue: WeakInterruptQueue,
}

impl InterruptHandle {
    /// Queues `callback` and asks the isolate to run it at its next poll. It never waits for
    /// the isolate: the queue lock is held only to push or to take the queue, never while a
    /// callback runs. Once the isolate is dropped, the callback is dropped here, unrun.
    pub fn request(&self, callback: Box<dyn FnOnce() + Send>) {
        // No isolate: `callback` drops at the end of this function, outside any lock.
        let Some(queue) = self.queue.upgrade() else {
            return;
        };
        queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback);
        // After the push: the poll that sees the bit finds the callback.
        self.word
            .fetch_or(crate::interp::interrupt::REQUEST, Ordering::Release);
        // If the isolate went meanwhile, this is the last strong reference: the queue and its
        // callbacks drop here, on this thread, after the lock is released (callbacks are
        // `Send`).
        drop(queue);
    }
}

impl std::fmt::Debug for InterruptHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queued = self
            .queue
            .upgrade()
            .map_or(0, |q| q.lock().map_or(0, |v| v.len()));
        f.debug_struct("InterruptHandle")
            .field("word", &self.word)
            .field("queued", &queued)
            .finish()
    }
}

/// One JS engine instance: heap, VM stack, realm and loaded units. Not `Send` (D14).
pub struct Isolate {
    // Drop order: `cx` points into `rt`, so it goes first.
    pub(crate) cx: Box<Cx>,
    pub(crate) rt: Box<Runtime>,
}

/// Native stack kept free below the lowest JS entry or host call (debug frames are big).
fn native_margin(stack_size: usize) -> usize {
    (stack_size / 4).clamp(128 << 10, 1 << 20)
}

impl Isolate {
    /// Creates an isolate on the current thread: the heap, the VM stack, and a realm with the
    /// VM's builtins and those of `config.builtins` (checked first, once per table). It runs
    /// the default options of the build ([`ExecOptions::default`]); [`Isolate::new_with`]
    /// takes others.
    pub fn new(config: IsolateConfig) -> Result<Isolate, IsolateError> {
        Isolate::new_with(config, ExecOptions::default())
    }

    /// [`Isolate::new`] with the execution options `exec` (C-23): the preset and the
    /// type-feedback mode. A preset this build cannot run is
    /// [`IsolateError::Config`], never a silent downgrade (M-R6): the jit preset needs the
    /// `jit` feature. The feedback mode is stored; it takes effect in M5-P3.
    ///
    /// ```
    /// use ep9_vm::{ExecOptions, Feedback, Isolate, IsolateConfig};
    /// let exec = ExecOptions::jitless().feedback(Feedback::FULL);
    /// assert!(Isolate::new_with(IsolateConfig::default(), exec).is_ok());
    /// ```
    pub fn new_with(config: IsolateConfig, exec: ExecOptions) -> Result<Isolate, IsolateError> {
        if !exec.preset().available() {
            return Err(IsolateError::Config(
                "the jit preset needs a build with the jit feature",
            ));
        }
        if config.vm_stack_slots < 1024 || config.vm_stack_slots > (1 << 28) {
            return Err(IsolateError::Config(
                "vm_stack_slots must be in 1024..=2^28",
            ));
        }
        builtins::check(config.builtins).map_err(IsolateError::BuiltinTable)?;
        let heap = Heap::new(shape::heap_config(config.heap)).map_err(IsolateError::Heap)?;
        let bounds = ep9_platform::current_thread_stack_bounds()
            .map_err(|e| IsolateError::Heap(HeapError::Stack(e)))?;
        let native_limit = bounds.low + native_margin(bounds.size());
        let rt = Runtime::new(
            heap,
            config.vm_stack_slots,
            native_limit,
            config.stack_trace_limit,
            config.compiler,
        );
        builtins::prepare(&rt, config.builtins);
        rt.exec.set(exec);
        realm::init(&rt);
        let mut cx = Box::new(Cx {
            table: *config.dispatch.table(),
            rt: &*rt,
            stack_limit: rt.stack().end(),
            budget: BUDGET,
            global_this: rt.global(),
            out_fp: core::ptr::null_mut(),
            out_pc: core::ptr::null(),
            dispatch: config.dispatch,
            feedback_threshold: config.feedback_threshold,
            interrupt: Arc::as_ptr(&rt.terminate),
            native_limit: rt.native_limit,
            #[cfg(feature = "spike-native")]
            runtime_table: &crate::native::RUNTIME_TABLE,
            #[cfg(not(feature = "spike-native"))]
            runtime_table: core::ptr::null(),
            realm: rt.root(Root::Realm),
            unit_data: 0,
        });
        realm::install(&mut cx).map_err(IsolateError::BuiltinTable)?;
        rt.jobs.set_tracker(config.promise_rejection_tracker);
        Ok(Isolate { cx, rt })
    }

    /// Makes a new realm (H22-Q3) with the isolate's builtin tables, and runs `f` in it: its
    /// global object is `f`'s second argument, and the code and objects that `f` makes belong
    /// to the new realm ([`abi::Cx::with_realm`]). The isolate's first realm is current again
    /// when it returns. The realm lives while something reachable holds one of its objects or
    /// functions; [`abi::Cx::create_realm`] is the same step inside [`Isolate::with_cx`] or a
    /// host function. An `Err(Throw)` from `f` becomes the [`Exception`] that is pending.
    ///
    /// ```
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let r = iso.create_realm(|cx, global| {
    ///     // The new realm is the current one.
    ///     let here = cx.global();
    ///     Ok(cx.same_value(here, global))
    /// });
    /// assert_eq!(r, Ok(true));
    /// ```
    pub fn create_realm<R>(
        &mut self,
        f: impl for<'s> FnOnce(&mut abi::Cx<'s>, Local<'s, JsObject>) -> JsResult<R>,
    ) -> Result<R, Exception> {
        self.with_cx(|cx| {
            let global = cx.create_realm()?;
            cx.with_realm(global, |cx| f(cx, global))
        })
    }

    /// The builtin table installed after the VM's own (`IsolateConfig::builtins`).
    pub fn builtin_table(&self) -> Option<&'static BuiltinTable> {
        self.rt.builtins.table()
    }

    /// The compiler hooks of this isolate (`IsolateConfig::compiler`, C2).
    pub fn compiler(&self) -> Option<&'static CompilerHooks> {
        self.rt.compiler
    }

    /// Runs `f` with a builtin-ABI context ([`abi::Cx`], H20): the embedder's way to hold
    /// values across calls that may collect garbage. Every `Local` that `f` sees lives on the
    /// native stack and is found by the stack scan, and none can leave `f`: its scope `'s` is
    /// local to the call, so the result type `R` cannot name it (H20-Q4). An `Err(Throw)` from
    /// `f` becomes the [`Exception`] that is pending.
    ///
    /// ```
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let nan = iso.with_cx(|cx| {
    ///     let g = cx.global();
    ///     let v = cx.get(g, "NaN")?;
    ///     cx.collect_garbage();
    ///     Ok(cx.to_number(v)?.is_nan())
    /// });
    /// assert_eq!(nan, Ok(true));
    /// ```
    ///
    /// A `Local` cannot leave the call, so it cannot be used after a later collection:
    ///
    /// ```compile_fail
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let global = iso.with_cx(|cx| Ok(cx.global())).unwrap();
    /// iso.collect_garbage();
    /// let _ = global;
    /// ```
    pub fn with_cx<R>(
        &mut self,
        f: impl for<'s> FnOnce(&mut abi::Cx<'s>) -> JsResult<R>,
    ) -> Result<R, Exception> {
        let r = {
            let mut cx = abi::Cx::new(&mut self.cx);
            f(&mut cx)
        };
        match r {
            Ok(v) => {
                // R4: `Ok` comes with no pending exception.
                debug_assert!(self.rt.pending() == 0, "Ok with a pending exception");
                self.rt.take_pending();
                Ok(v)
            }
            Err(Throw) => Err(self.exception()),
        }
    }

    /// [`Isolate::with_cx`], then a microtask checkpoint
    /// ([`Isolate::perform_microtask_checkpoint`]), also after a throw: the jobs that `f`
    /// queued run before it returns. This is the run of a script or a call as a host does it
    /// (`f` calls [`abi::Cx::run_script`], [`abi::Cx::run_module`] or `abi::Cx::call`).
    ///
    /// The result of `f` is made before the jobs run, so `f` copies out what it needs (a
    /// `Local` cannot leave `f`). The error rules: a termination in `f` runs no checkpoint and
    /// drops the queued jobs, so none of them runs later; a throw from `f` wins over one from
    /// a job, but a termination in a job wins over the throw; a job that throws after `f`
    /// returned `Ok` makes the result that error, and the jobs behind it stay queued.
    ///
    /// ```
    /// use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};
    /// fn job<'s>(
    ///     cx: &mut Cx<'s>,
    ///     _this: Local<'s>,
    ///     args: Args<'s>,
    ///     _new_target: Local<'s>,
    /// ) -> JsResult<Local<'s>> {
    ///     let g = cx.global();
    ///     cx.set(g, "ran", args.get(0), true)?;
    ///     Ok(Local::undefined())
    /// }
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// iso.register_host_function("job", 1, job);
    /// let r = iso.with_cx_checkpoint(|cx| {
    ///     let f = cx.global_binding("job")?.expect("a global");
    ///     cx.enqueue_job(f, Local::int32(7))?;
    ///     // The job runs after this closure returns.
    ///     Ok(cx.global_binding("ran")?.is_none())
    /// });
    /// assert_eq!(r, Ok(true));
    /// let ran = iso.with_cx(|cx| Ok(cx.global_binding("ran")?.map(|v| cx.display(v))));
    /// assert_eq!(ran, Ok(Some("7".to_string())));
    /// ```
    ///
    /// ```compile_fail
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// // The `Local` cannot leave the call.
    /// let global = iso.with_cx_checkpoint(|cx| Ok(cx.global())).unwrap();
    /// iso.collect_garbage();
    /// let _ = global;
    /// ```
    pub fn with_cx_checkpoint<R>(
        &mut self,
        f: impl for<'s> FnOnce(&mut abi::Cx<'s>) -> JsResult<R>,
    ) -> Result<R, Exception> {
        match self.with_cx(f) {
            Ok(v) => {
                self.perform_microtask_checkpoint()?;
                Ok(v)
            }
            // A termination: `exception` dropped the queue, so no job runs.
            Err(e) if e.terminated => Err(e),
            Err(e) => match self.perform_microtask_checkpoint() {
                Err(c) if c.terminated => Err(c),
                // The error of `f` wins; a job's error after it is dropped.
                _ => Err(e),
            },
        }
    }

    /// Runs `f` with `resolve` and `load` as the module host of this isolate: the hooks of
    /// `import()` (HostLoadImportedModule) in all the code that runs during `f`: scripts
    /// ([`abi::Cx::run_script`]), calls and microtask checkpoints. The hooks have the rules of
    /// the hooks of [`abi::Cx::run_module`]; a `run_module` inside `f` uses its own hooks for
    /// its run. Calls nest: the innermost host is the one in use. With no module host,
    /// `import()` throws the v0 not-implemented `Error`. Only this isolate uses the host.
    ///
    /// ```
    /// use ep9_vm::{CompileFailure, Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let mut resolve = |_referrer: &str, specifier: &str| Ok(specifier.to_string());
    /// let mut load = |name: &str| Err(CompileFailure::Internal(format!("no file {name}")));
    /// // A script run and its checkpoint, with the hooks in place for `import()`.
    /// let r = iso.with_module_host(&mut resolve, &mut load, |iso| {
    ///     iso.with_cx_checkpoint(|cx| Ok(cx.global_binding("NaN")?.is_some()))
    /// });
    /// assert_eq!(r, Ok(true));
    /// ```
    pub fn with_module_host<R>(
        &mut self,
        resolve: &mut dyn FnMut(&str, &str) -> Result<String, String>,
        load: &mut dyn FnMut(&str) -> CompileResult,
        f: impl FnOnce(&mut Isolate) -> R,
    ) -> R {
        let key = crate::module::host_key(&self.rt);
        let mut host = crate::module::Host { resolve, load };
        crate::module::with_host(key, &mut host, || f(self))
    }

    pub fn dispatch(&self) -> Dispatch {
        self.cx.dispatch
    }

    /// Defines the global `name` as a host function (writable, configurable, not enumerable,
    /// like V8's `print` in d8).
    pub fn register_host_function(&mut self, name: &str, length: u32, f: HostFn) {
        let rt = &*self.rt;
        let func = realm::new_host_function(rt, f, name, length, false);
        let key = atoms::intern_str(rt, name);
        own_define(rt, rt.global(), key, func, attr::HIDDEN);
    }

    /// Performs a microtask checkpoint: runs every queued job (promise reactions, and the jobs
    /// of `Cx::enqueue_job`) in order, and the jobs they queue, until none is left. A job that
    /// throws stops it: that exception is the `Err`, and the jobs behind it stay queued. A
    /// termination stops it and drops the queue. [`Isolate::with_cx_checkpoint`] runs one
    /// after its scope; an embedder that runs JS inside [`Isolate::with_cx`] calls this
    /// afterwards.
    pub fn perform_microtask_checkpoint(&mut self) -> Result<(), Exception> {
        let r = abi::Cx::new(&mut self.cx).perform_microtask_checkpoint();
        match r {
            Ok(()) => Ok(()),
            Err(Throw) => Err(self.exception()),
        }
    }

    /// The promises that were rejected while they had no handler and still have none, as
    /// [`Exception`]s of their rejection reasons, in rejection order; the isolate forgets
    /// them. A host reports these after its last checkpoint (the ep9 CLI prints the first one
    /// as an uncaught error and exits 1, as node does). The list drops a promise as soon as it
    /// gets a handler.
    ///
    /// It runs ABI code, so it is an H20 core entry point (Codex W4-3): the message of a
    /// reason that is an object but not an Error is ToString(reason), and that can call JS
    /// or a host function (a `toString` method). A throw from that ToString is dropped (the
    /// message is then the plain description). Jobs that this code queues stay queued: call
    /// [`Isolate::perform_microtask_checkpoint`] after it to run them.
    ///
    /// ```
    /// use std::sync::atomic::{AtomicU32, Ordering};
    /// use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};
    /// static CALLS: AtomicU32 = AtomicU32::new(0);
    /// fn to_string<'s>(
    ///     cx: &mut Cx<'s>,
    ///     _this: Local<'s>,
    ///     _args: Args<'s>,
    ///     _new_target: Local<'s>,
    /// ) -> JsResult<Local<'s>> {
    ///     CALLS.fetch_add(1, Ordering::SeqCst);
    ///     Ok(cx.new_string("a reason from a host function")?.into())
    /// }
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// iso.register_host_function("hostToString", 0, to_string);
    /// iso.with_cx_checkpoint(|cx| {
    ///     let f = cx.global_binding("hostToString")?.expect("a global");
    ///     let reason = cx.new_object();
    ///     cx.set(reason, "toString", f, true)?;
    ///     let p = cx.new_promise(Local::null());
    ///     cx.reject_promise(p, reason);
    ///     Ok(())
    /// })
    /// .unwrap();
    /// assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    /// let list = iso.take_unhandled_rejections();
    /// // The host function ran in this call.
    /// assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    /// assert_eq!(list[0].message, "a reason from a host function");
    /// ```
    pub fn take_unhandled_rejections(&mut self) -> Vec<Exception> {
        let rt: &Runtime = self.cx.rt();
        let reasons = rt.heap.scratch_roots();
        abi::Cx::new(&mut self.cx).take_unhandled(&reasons);
        let mut out = Vec::with_capacity(reasons.len());
        for i in 0..reasons.len() {
            let v = reasons.get(i).to_bits();
            out.push(self.describe(v, None));
        }
        out
    }

    /// A handle that stops the running script from another thread.
    pub fn terminate_handle(&self) -> TerminateHandle {
        TerminateHandle(Arc::clone(&self.rt.terminate))
    }

    /// A handle that runs callbacks on this isolate's thread from another thread.
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle {
            word: Arc::clone(&self.rt.terminate),
            queue: Arc::downgrade(&self.rt.interrupt_requests),
        }
    }

    /// Runs a full garbage collection.
    pub fn collect_garbage(&mut self) {
        self.rt.heap.collect();
    }

    /// Runs the heap verifier.
    pub fn verify_heap(&self) -> Result<VerifyReport, VerifyError> {
        self.rt.heap.verify()
    }

    /// A copy of the heap counters (collections, freed and allocated bytes, the last marked
    /// bytes, ...). The default build gives no `&Heap` (H20-Q4), so this is the embedder's
    /// way to read them.
    ///
    /// ```
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let before = iso.gc_stats().collections;
    /// iso.collect_garbage();
    /// assert_eq!(iso.gc_stats().collections, before + 1);
    /// ```
    ///
    /// The raw heap is not in the default build (H20-Q4): see the `compile_fail` tests in
    /// `isolate.rs` (`NoRawHeap`).
    pub fn gc_stats(&self) -> GcStats {
        self.rt.heap.stats()
    }

    /// The pending exception (taken) as an API [`Exception`]. A termination also drops the
    /// job queue before it resets: no job queued before it runs on a later run, so a promise
    /// loop cannot outlive [`TerminateHandle`].
    fn exception(&mut self) -> Exception {
        let rt = &*self.rt;
        let terminated = || Exception {
            message: "execution terminated".into(),
            constructor: None,
            stack: None,
            location: None,
            terminated: true,
        };
        let exc = rt.take_pending();
        let site = rt.shared.throw_site.take();
        if end_termination(rt) {
            return terminated();
        }
        debug_assert!(exc != 0, "Err(Throw) without a pending exception");
        if exc == 0 {
            return Exception {
                message: "ep9 internal error: no pending exception".into(),
                constructor: None,
                stack: None,
                location: None,
                terminated: false,
            };
        }
        self.describe(exc, site)
    }

    /// The API [`Exception`] of the thrown value `exc` (`site`: where a non-Error value was
    /// thrown). `exc` stays live in this frame (CSS) while its text is read.
    fn describe(&mut self, exc: u64, site: Option<Site>) -> Exception {
        let rt = &*self.rt;
        let terminated = || Exception {
            message: "execution terminated".into(),
            constructor: None,
            stack: None,
            location: None,
            terminated: true,
        };
        let constructor = constructor_name(&mut self.cx, exc);
        let (err_site, stack) = error::describe_exception(rt, exc);
        let site = if is_error(exc) { err_site } else { site };
        let message = if is_error(exc) {
            error::error_header(&mut self.cx, exc)
        } else if object::is_object(exc) {
            // A thrown object that is not an Error (for example a test262 `Test262Error`):
            // ToString(value), as d8 reports it. If ToString throws, drop that second
            // exception and use the plain description.
            match crate::conv::to_rust_string(&mut self.cx, exc) {
                Ok(s) => s,
                Err(Throw) => {
                    rt.take_pending();
                    rt.shared.throw_site.take();
                    if end_termination(rt) {
                        return terminated();
                    }
                    display_bits(exc)
                }
            }
        } else {
            display_bits(exc)
        };
        let location = site.map(|s| {
            // SAFETY: sites name units of this isolate, which live until it drops.
            let u = unsafe { &*rt.unit(s.unit) };
            let (line, column) = u.line_column(s.offset).unwrap_or((0, 0));
            SourceLocation {
                file: u.file.clone(),
                line,
                column,
            }
        });
        Exception {
            message,
            constructor,
            stack,
            location,
            terminated: false,
        }
    }
}

/// The raw heap is not in the default build, so safe code cannot get a raw `Value` of a
/// heap cell from it (H20-Q4, Codex W4-1). No new cell (`Heap::alloc`, then
/// `CellPtr::to_value`):
///
/// ```compile_fail,E0599
/// use ep9_vm::{Isolate, IsolateConfig};
/// let iso = Isolate::new(IsolateConfig::default()).unwrap();
/// let c = iso.heap().alloc(ep9_heap::KindId::new(1).unwrap(), 16).unwrap();
/// let _raw = c.to_value();
/// ```
///
/// no cell of a `Local` from the conservative-scan log
/// (`Heap::last_conservative_roots`):
///
/// ```compile_fail,E0599
/// use ep9_vm::{Isolate, IsolateConfig};
/// let mut cfg = IsolateConfig::default();
/// cfg.heap.record_conservative_roots = true;
/// let mut iso = Isolate::new(cfg).unwrap();
/// iso.with_cx(|cx| {
///     let _s = cx.new_string("a string")?;
///     cx.collect_garbage();
///     Ok(())
/// })
/// .unwrap();
/// let _raw: Vec<_> = iso.heap().last_conservative_roots().iter().map(|c| c.to_value()).collect();
/// ```
///
/// and no cell from a heap walk (`Heap::find_cell`):
///
/// ```compile_fail,E0599
/// use ep9_vm::{Isolate, IsolateConfig};
/// let iso = Isolate::new(IsolateConfig::default()).unwrap();
/// let base = iso.heap().cage_base();
/// let _raw = iso.heap().find_cell(base).map(|c| c.to_value());
/// ```
///
/// (Rustdoc runs these tests on this private item; with spike-native, `heap` exists, so the
/// item and its tests are left out.)
#[cfg(not(feature = "spike-native"))]
#[allow(dead_code)]
struct NoRawHeap;

/// The raw heap: only for the in-crate tests and the spike-native API, never in the default
/// build (H20-Q4, Codex W4-1). Its safe methods give raw cells and `Value`s that no scope
/// keeps live (`Heap::alloc`, `Heap::find_cell`, `Heap::last_conservative_roots`, then
/// `CellPtr::to_value`). A block of its own, so the H20 golden (`tests/h20_core.rs`) has no
/// line for it.
#[cfg(any(test, feature = "spike-native"))]
impl Isolate {
    pub fn heap(&self) -> &Heap {
        &self.rt.heap
    }
}

/// Loads `unit` (a new entry of `Runtime::units`, never freed) and calls its entry function:
/// `this` is the global object for a script and undefined for module code. The body of
/// [`abi::Cx::run_script`] and of the test helper `run_raw`. Works from Rust with no frame and from
/// a host call (a nested run on top of the caller's frames).
fn run_unit(cx: &mut Cx, unit: VerifiedUnit, file: &str) -> Res<u64> {
    let rt = cx.rt();
    let module = unit.header.code_kind == ep9_bytecode::CodeKind::Module;
    let index = u32::try_from(rt.units.borrow().len()).expect("fewer than 2^32 units");
    let lu = LoadedUnit::load(rt, unit, file.to_string(), index);
    let entry: *const FunctionInfo = &lu.infos[0];
    rt.units.borrow_mut().push(lu);
    let f = object::new_function(rt, entry, UNDEFINED);
    let this = if module { UNDEFINED } else { rt.global() };
    call::call_function(cx, f, this, &[])
}

/// The raw read of the global binding `name` (the tests and `NativeCx::global_value`): the
/// script lexical record first (`None` in its TDZ), then an own data property of the global
/// object (`None` for an accessor). Runs no JS; interning `name` may allocate.
#[cfg(any(test, feature = "spike-native"))]
fn global_data(rt: &Runtime, name: &str) -> Option<u64> {
    let key = atoms::intern_str(rt, name);
    match own_get(rt.root(Root::Lexical), key) {
        // The TDZ hole never leaves the VM.
        Some((v, _)) => (v != HOLE).then_some(v),
        None => own_get(rt.global(), key)
            // An accessor's slot holds its ACCESSOR_PAIR cell, which is not a JS value.
            .filter(|&(_, a)| a & attr::ACCESSOR == 0)
            .map(|(v, _)| v),
    }
}

// The raw `Value` reads of the in-crate tests and of the spike-native API.
#[cfg(any(test, feature = "spike-native"))]
use {crate::runtime::val, ep9_value::Value};

/// Test support for the in-crate tests, which drive the VM with raw values (objects they make
/// with `object::*`, atoms, bits they read back from the heap). The embedding API has no raw
/// form (H20-Q4); a test that checks that API uses the scoped forms. A raw `Value` here is
/// not a root: it lives while the test's Rust frame holds it (the conservative stack scan,
/// H10) or something reachable holds it, never in a Rust heap container.
#[cfg(test)]
impl Isolate {
    /// The global binding `name` as `global_data` reads it (no JS).
    pub(crate) fn global_raw(&self, name: &str) -> Option<Value> {
        global_data(&self.rt, name).map(val)
    }

    /// The text of `v` as [`abi::Cx::display`] gives it (no JS).
    pub(crate) fn display_raw(&self, v: Value) -> String {
        display_bits(v.to_bits())
    }

    /// [`abi::Cx::run_script`] in a [`Isolate::with_cx`] scope, with no microtask checkpoint
    /// after it (as [`Isolate::call_raw`]): the completion value as a raw `Value`.
    pub(crate) fn run_raw(&mut self, unit: VerifiedUnit, file: &str) -> Result<Value, Exception> {
        self.with_cx(|cx| cx.run_script(unit, file).map(|v| val(v.bits())))
    }

    /// Call(`f`, `this`, `args`) in a [`Isolate::with_cx`] scope, with no microtask
    /// checkpoint after it (these tests queue no jobs; `tests::jobs` checks the checkpoint
    /// through [`Isolate::with_cx_checkpoint`]). The arguments are copied into the callee
    /// frame before its first GC point (`call::call_with`).
    pub(crate) fn call_raw(
        &mut self,
        f: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Value, Exception> {
        let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
        self.with_cx(|cx| {
            let b = call::call_with(
                cx.inner(),
                f.to_bits(),
                this.to_bits(),
                argc,
                |i| args[i as usize].to_bits(),
                UNDEFINED,
            )?;
            // The hole never leaves the VM.
            Ok(val(if b == HOLE { UNDEFINED } else { b }))
        })
    }
}

/// The raw-`Value` reads of the spike-native API (feature `spike-native` only, never in the
/// ep9 graph, guard 11). That API takes and returns raw `Value`s by design
/// ([`crate::install_native`], [`crate::native_target`], [`crate::call_from_rust`],
/// [`NativeCx::intrinsic`](crate::NativeCx::intrinsic)), so Spike B and its tests get the
/// functions it installs here. The embedding API has no raw-`Value` read (H20-Q4); this one
/// sits in isolate.rs because the global lookup is here.
#[cfg(feature = "spike-native")]
impl crate::native::NativeCx {
    /// The global binding `name` of `iso`, without running JS: the script lexical record
    /// first, then an own data property of the global object. `None` if there is none, for a
    /// lexical binding in its TDZ, and for an accessor property. The `Value` is not a root: it
    /// stays valid while something reachable holds it (a global binding keeps its function;
    /// the heap does not move objects, H10).
    pub fn global_value(iso: &Isolate, name: &str) -> Option<Value> {
        global_data(&iso.rt, name).map(val)
    }
}

/// Resets the termination state after a terminated run and gives true if there was one; then
/// it drops the job queue (`JobState::clear_queue`), so no job queued before the termination
/// runs at a later run or checkpoint of the isolate, and makes each module whose async
/// evaluation waited for those jobs aborted (`module::abort_async_modules`).
fn end_termination(rt: &Runtime) -> bool {
    let t = rt.terminating.replace(false);
    if t {
        rt.jobs.clear_queue();
        crate::module::abort_async_modules(rt);
    }
    t
}

/// The text of [`abi::Cx::display`]. Runs no JS.
fn display_bits(b: u64) -> String {
    if is_string(b) {
        string::to_rust_string(b)
    } else {
        describe(b)
    }
}

/// The embedder forms scoped to [`Isolate::with_cx`] (H20-Q4, C11): they take and return
/// `Local`s, which cannot leave the `with_cx` call, so no raw `Value` outlives a collection.
/// They are `Cx` methods, so a host function can use them too.
impl<'s> abi::Cx<'s> {
    /// Loads `unit` and runs it (a script or a module unit alone): `this` is the global object
    /// for a script and undefined for module code; `file` names the unit in stack traces. The
    /// completion value comes back as a `Local`; a throw is the pending exception (`Err`),
    /// which `with_cx` turns into its [`Exception`].
    ///
    /// ```compile_fail
    /// use ep9_vm::Isolate;
    /// fn escape(iso: &mut Isolate, unit: ep9_bytecode::VerifiedUnit) {
    ///     // The `Local` cannot leave `with_cx`.
    ///     let v = iso.with_cx(|cx| cx.run_script(unit, "a.js")).unwrap();
    ///     iso.collect_garbage();
    ///     let _ = v;
    /// }
    /// ```
    pub fn run_script(&mut self, unit: VerifiedUnit, file: &str) -> JsResult<Local<'s>> {
        let b = run_unit(self.inner(), unit, file)?;
        // The hole never leaves the VM.
        Ok(Local::from_bits(if b == HOLE { UNDEFINED } else { b }))
    }

    /// Runs the module unit `unit` (a module goal unit from ep9_frontend) as the module named
    /// `name`, with the modules it imports (the module loader API, M3-P2; §16.2.1.5): the
    /// steps of a host's import of an entry module, with a synchronous loader.
    ///
    /// ```text
    /// the record of `name` in the current realm's module map, or a new one from `unit`
    /// load:     for each import or re-export specifier of each module, depth first:
    ///             resolve(referrer name, specifier) -> a name (the map key)
    ///             not in the map: load(name) -> a module unit -> a new record
    /// link:     imports bound to the exporting modules' bindings (live), namespaces
    /// evaluate: each module's body once, dependencies first; a thrown error is kept and
    ///           thrown again by each later import of that module
    /// ```
    ///
    /// The names are the host's: `resolve` decides what two specifiers mean the same module
    /// (the test262 runner and the CLI give a normalized path), and a name loads once per
    /// realm. `Err(text)` from `resolve` throws an `Error` with that text; a `load` failure
    /// throws as eval code's compile failures do (`SyntaxError` for an early error), and a
    /// later run that needs the module tries the load again. If a [`TerminateHandle`] stops
    /// a module's evaluation, a later run that reaches that module throws an `Error`; it
    /// never takes the module as evaluated. An
    /// import that no module exports, or that two `export *` give, is a `SyntaxError`.
    /// An import attribute key other than `type` is a `SyntaxError` (in `import()` a
    /// `TypeError`); `type: "json"` throws the v0 not-implemented `Error`, another type a
    /// `TypeError`. The result is undefined; the module's code runs with `this` undefined.
    /// After the evaluation, `run_module` runs a microtask checkpoint (also with no `await`
    /// in the graph), so a top-level `await` settles in the call: a rejection is the throw,
    /// and an `Error` if it never settles. Called with JS on the stack (from a host function
    /// that JS calls), it runs no checkpoint, as a checkpoint needs an empty JS stack: the
    /// jobs of the run wait for the caller's checkpoint, and a top-level `await` that is still
    /// pending after the evaluation throws an `Error`. `import()` in code that runs during the
    /// call (also in that checkpoint: in a promise job, or in a module that `import()`
    /// evaluates) uses `resolve` and `load`, and resolves from the module or script whose
    /// code calls it (for a function made by eval code or `Function()`, the code that made
    /// it); its module goes into the module map of that referrer's realm. After the call,
    /// `import()` uses the host of [`Isolate::with_module_host`], or throws the v0
    /// not-implemented `Error`. Run a microtask checkpoint after it
    /// ([`Isolate::with_cx_checkpoint`]).
    ///
    /// ```compile_fail
    /// use ep9_vm::Isolate;
    /// fn escape(iso: &mut Isolate, unit: ep9_bytecode::VerifiedUnit) {
    ///     // The `Local` cannot leave `with_cx`.
    ///     let v = iso
    ///         .with_cx(|cx| cx.run_module(unit, "a.js", &mut |_, s| Ok(s.into()), &mut |_| todo!()))
    ///         .unwrap();
    ///     iso.collect_garbage();
    ///     let _ = v;
    /// }
    /// ```
    pub fn run_module(
        &mut self,
        unit: VerifiedUnit,
        name: &str,
        resolve: &mut dyn FnMut(&str, &str) -> Result<String, String>,
        load: &mut dyn FnMut(&str) -> CompileResult,
    ) -> JsResult<Local<'s>> {
        let mut host = crate::module::Host { resolve, load };
        let b = crate::module::run_module(self.inner(), unit, name, &mut host)?;
        Ok(Local::from_bits(b))
    }

    /// The value of the global binding `name` (GetBindingValue of the global environment,
    /// §9.1.1.4.6): the script lexical record first, where a binding in its TDZ throws a
    /// ReferenceError, then the global object and its prototype chain (HasProperty, then
    /// `[[Get]]`, which runs a getter). `Ok(None)` if no binding has the name (an
    /// unresolvable reference).
    ///
    /// ```
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let r = iso.with_cx(|cx| {
    ///     let nan = cx.global_binding("NaN")?.expect("a global");
    ///     cx.collect_garbage();
    ///     Ok((cx.display(nan), cx.global_binding("nope")?.is_none()))
    /// });
    /// assert_eq!(r, Ok(("NaN".to_string(), true)));
    /// ```
    ///
    /// ```compile_fail
    /// use ep9_vm::{Isolate, IsolateConfig};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// // The `Local` cannot leave `with_cx`.
    /// let nan = iso.with_cx(|cx| cx.global_binding("NaN")).unwrap();
    /// iso.collect_garbage();
    /// let _ = nan;
    /// ```
    pub fn global_binding(&mut self, name: &str) -> JsResult<Option<Local<'s>>> {
        let cx = self.inner();
        let rt = cx.rt();
        // GC point; the key stays live in this Rust local (CSS).
        let key = atoms::intern_str(rt, name);
        if let Some((v, _)) = own_get(rt.root(Root::Lexical), key) {
            if v == HOLE {
                let msg = format!("Cannot access '{name}' before initialization");
                return Err(error::throw_error(cx, ErrorKind::ReferenceError, &msg));
            }
            return Ok(Some(Local::from_bits(v)));
        }
        let global: Local<'s, JsObject> = Local::from_bits(rt.global());
        let key: Local<'s, JsKey> = Local::from_bits(key);
        if !self.has_property(global, key)? {
            return Ok(None);
        }
        self.get(global, key).map(Some)
    }

    /// The text of `v` for a REPL or test output: strings as they are, numbers in JS form,
    /// other values described. Runs no JS (no `toString` call).
    pub fn display(&self, v: impl Into<Local<'s>>) -> String {
        display_bits(v.into().bits())
    }
}

/// `v.constructor.name` for an object `v`, if it is a string; `None` for a primitive. It
/// reads data properties only (an accessor counts as absent), so it runs no JS and cannot
/// throw.
fn constructor_name(cx: &mut Cx, v: u64) -> Option<String> {
    if !object::is_object(v) {
        return None;
    }
    let rt = cx.rt();
    let ctor = property::get_data_from(cx, v, rt.atom(Atom::Constructor))?;
    if !object::is_object(ctor) {
        return None;
    }
    let name = property::get_data_from(cx, ctor, rt.atom(Atom::Name))?;
    is_string(name).then(|| string::to_rust_string(name))
}
