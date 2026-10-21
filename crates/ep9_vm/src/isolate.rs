//! The embedding API: one `Isolate` per thread (D14).
//!
//! ```text
//! let mut iso = Isolate::new(IsolateConfig::default())?;      heap, VM stack, realm
//! iso.register_host_function("print", 1, print);              a global host function
//! match iso.run_script(unit, "main.js") {                     VerifiedUnit from ep9_frontend
//!     Ok(v) => println!("{}", iso.display(v)),
//!     Err(e) => eprintln!("{e}"),                             "main.js:3:7: TypeError: ..."
//! }
//! ```
//!
//! A `Value` that the API returns is not a root: it stays valid until the next call into the
//! isolate (that call may collect garbage). Copy what you need out of it first, or work inside
//! [`Isolate::with_cx`], whose `Local`s cannot outlive the call (H20-Q4). The scoped forms
//! (M2-P3, C11) are `Cx` methods:
//!
//! | Old form (raw `Value`) | Scoped form inside `with_cx` (`Local<'s>`) |
//! |---|---|
//! | `Isolate::run_script` | [`abi::Cx::run_script`] |
//! | `Isolate::call` | `abi::Cx::call` |
//! | `Isolate::global` | [`abi::Cx::global_binding`] (runs getters; TDZ throws) |
//! | `Isolate::display` | [`abi::Cx::display`] |
//!
//! The old forms stay until the embedders move (M3-P3 EMBED-MIGRATE).
//!
//! Other threads reach a running isolate through two `Send` handles: [`TerminateHandle`]
//! (stop the script) and [`InterruptHandle`] (run a callback on the isolate thread at the
//! next interrupt poll).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError};

use ep9_bytecode::VerifiedUnit;
use ep9_heap::{Heap, HeapConfig, HeapError, VerifyError, VerifyReport};
use ep9_value::{JsResult, Throw, Value};

use crate::abi::{JsKey, JsObject, Local};
use crate::builtins::{self, BuiltinTable};
use crate::compiler::CompilerHooks;
use crate::conv::{Res, describe};
use crate::frame::{HOLE, UNDEFINED};
use crate::host::HostFn;
use crate::interp::{BUDGET, Cx, Dispatch};
use crate::object::{self, attr, is_error, is_string, own_define, own_get};
use crate::runtime::{Atom, ErrorKind, InterruptCallback, InterruptQueue, Root, Runtime, val};
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
/// queues it, and the isolate runs it at its next interrupt poll, the place where
/// [`TerminateHandle`] stops a script (loop back edges, function entries, builtin loops; JIT
/// code polls the same way). A poll comes once per interrupt budget (10,000 back edges or
/// calls) while JS runs; an idle isolate runs the callback when it next runs JS and polls.
///
/// The callback gets no `Cx`: it runs no JS and allocates nothing on the JS heap, so it is
/// safe at any poll. It may call [`TerminateHandle::terminate`] (the same poll then stops the
/// script) or [`InterruptHandle::request`] (the new callback runs at a later poll).
/// Callbacks run in request order. A callback that panics ends the process in a release build
/// (`panic = "abort"`, H23); in a debug build the panic unwinds out of the running script and
/// the callbacks queued behind it are dropped. After the isolate is dropped, a request is
/// never run; the callback is dropped with the last handle.
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
    queue: InterruptQueue,
}

impl InterruptHandle {
    /// Queues `callback` and asks the isolate to poll it. Never blocks on the isolate (only
    /// on the queue lock, which no callback holds).
    pub fn request(&self, callback: Box<dyn FnOnce() + Send>) {
        let callback: InterruptCallback = callback;
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback);
        // After the push: the poll that sees the bit finds the callback.
        self.word
            .fetch_or(crate::interp::interrupt::REQUEST, Ordering::Release);
    }
}

impl std::fmt::Debug for InterruptHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queued = self.queue.lock().map_or(0, |q| q.len());
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
    /// VM's builtins and those of `config.builtins` (checked first, once per table).
    pub fn new(config: IsolateConfig) -> Result<Isolate, IsolateError> {
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
            proto_epoch: &rt.shapes.proto_epoch,
            interrupt: Arc::as_ptr(&rt.terminate),
            #[cfg(feature = "spike-native")]
            native_limit: rt.native_limit,
            #[cfg(feature = "spike-native")]
            runtime_table: &crate::native::RUNTIME_TABLE,
        });
        builtins::install(&mut cx).map_err(IsolateError::BuiltinTable)?;
        Ok(Isolate { cx, rt })
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

    /// Loads `unit` (from ep9_frontend, or a code cache after verification) and runs its entry
    /// function: `this` is the global object for a script and undefined for module code
    /// (§16.2.1.6.4 step 9; v0 runs a module unit alone, with no linking). `file` names the
    /// unit in stack traces and locations.
    pub fn run_script(&mut self, unit: VerifiedUnit, file: &str) -> Result<Value, Exception> {
        let r = run_unit(&mut self.cx, unit, file);
        self.finish(r)
    }

    /// Calls `f` with `this` and `args` (Call, §7.3.14).
    pub fn call(&mut self, f: Value, this: Value, args: &[Value]) -> Result<Value, Exception> {
        // The callee frame copies the arguments onto the VM stack before the first GC point.
        let bits: Vec<u64> = args.iter().map(|v| v.to_bits()).collect();
        let r = call::call_function(&mut self.cx, f.to_bits(), this.to_bits(), &bits);
        self.finish(r)
    }

    /// The global binding `name` (the script lexical record first), without running JS: `None`
    /// if there is none, if it is a lexical binding in its TDZ (not initialized yet; reading
    /// it would throw a ReferenceError), and if it is an accessor property of the global
    /// object (reading it would call the getter). An own data property of the global object
    /// only: [`abi::Cx::global_binding`] does the full lookup, getters and TDZ included.
    pub fn global(&self, name: &str) -> Option<Value> {
        let rt = &*self.rt;
        let key = atoms::intern_str(rt, name);
        match own_get(rt.root(Root::Lexical), key) {
            // The TDZ hole never leaves the VM.
            Some((v, _)) => (v != HOLE).then(|| val(v)),
            None => own_get(rt.global(), key)
                // An accessor's slot holds its ACCESSOR_PAIR cell, which is not a JS value.
                .filter(|&(_, a)| a & attr::ACCESSOR == 0)
                .map(|(v, _)| val(v)),
        }
    }

    /// Text of `v` for a REPL or test output, without running JS: strings as they are,
    /// numbers in JS form, other values as `describe` prints them.
    pub fn display(&self, v: Value) -> String {
        display_bits(v.to_bits())
    }

    /// A handle that stops the running script from another thread.
    pub fn terminate_handle(&self) -> TerminateHandle {
        TerminateHandle(Arc::clone(&self.rt.terminate))
    }

    /// A handle that runs callbacks on this isolate's thread from another thread.
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle {
            word: Arc::clone(&self.rt.terminate),
            queue: Arc::clone(&self.rt.interrupt_requests),
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

    pub fn heap(&self) -> &Heap {
        &self.rt.heap
    }

    /// Turns the result of a run into the API result; takes the pending exception.
    fn finish(&mut self, r: Res<u64>) -> Result<Value, Exception> {
        match r {
            // The hole never leaves the VM.
            Ok(b) => Ok(val(if b == HOLE { UNDEFINED } else { b })),
            Err(Throw) => Err(self.exception()),
        }
    }

    /// The pending exception (taken) as an API [`Exception`].
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
        if rt.terminating.replace(false) {
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
                    if rt.terminating.replace(false) {
                        return terminated();
                    }
                    self.display(val(exc))
                }
            }
        } else {
            self.display(val(exc))
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

/// Loads `unit` (a new entry of `Runtime::units`, never freed) and calls its entry function:
/// `this` is the global object for a script and undefined for module code. The shared body of
/// [`Isolate::run_script`] and [`abi::Cx::run_script`]. Works from Rust with no frame and from
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

/// The text of [`Isolate::display`] and [`abi::Cx::display`]. Runs no JS.
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
    /// Loads `unit` and runs it, as [`Isolate::run_script`] does: `this` is the global object
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

    /// The text of `v` for a REPL or test output, as [`Isolate::display`]: strings as they
    /// are, numbers in JS form, other values described. Runs no JS (no `toString` call).
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
