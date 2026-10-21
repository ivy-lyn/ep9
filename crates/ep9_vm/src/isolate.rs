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
//! [`Isolate::with_cx`], whose `Local`s cannot outlive the call (H20-Q4).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ep9_bytecode::VerifiedUnit;
use ep9_heap::{Heap, HeapConfig, HeapError, VerifyError, VerifyReport};
use ep9_value::{JsResult, Throw, Value};

use crate::builtins::{self, BuiltinTable};
use crate::conv::{Res, describe};
use crate::frame::{HOLE, UNDEFINED};
use crate::host::HostFn;
use crate::interp::{BUDGET, Cx, Dispatch};
use crate::object::{self, attr, is_error, is_string, own_define, own_get};
use crate::runtime::{Atom, Root, Runtime, val};
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
/// edges and function entries; the exception it raises cannot be caught by JS.
#[derive(Clone, Debug)]
pub struct TerminateHandle(Arc<AtomicBool>);

impl TerminateHandle {
    pub fn terminate(&self) {
        self.0.store(true, Ordering::Release);
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
        });
        builtins::install(&mut cx).map_err(IsolateError::BuiltinTable)?;
        Ok(Isolate { cx, rt })
    }

    /// The builtin table installed after the VM's own (`IsolateConfig::builtins`).
    pub fn builtin_table(&self) -> Option<&'static BuiltinTable> {
        self.rt.builtins.table()
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
        let rt = &*self.rt;
        let module = unit.header.code_kind == ep9_bytecode::CodeKind::Module;
        let index = u32::try_from(rt.units.borrow().len()).expect("fewer than 2^32 units");
        let lu = LoadedUnit::load(rt, unit, file.to_string(), index);
        let entry: *const FunctionInfo = &lu.infos[0];
        rt.units.borrow_mut().push(lu);
        let f = object::new_function(rt, entry, UNDEFINED);
        let this = if module { UNDEFINED } else { rt.global() };
        let r = call::call_function(&mut self.cx, f, this, &[]);
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
    /// if there is none, and also if it is an accessor property of the global object (reading
    /// it would call the getter, as `[[Get]]` through [`Isolate::with_cx`] does).
    pub fn global(&self, name: &str) -> Option<Value> {
        let rt = &*self.rt;
        let key = atoms::intern_str(rt, name);
        own_get(rt.root(Root::Lexical), key)
            .or_else(|| own_get(rt.global(), key))
            // An accessor's slot holds its ACCESSOR_PAIR cell, which is not a JS value.
            .filter(|&(_, a)| a & attr::ACCESSOR == 0)
            .map(|(v, _)| val(v))
    }

    /// Text of `v` for a REPL or test output, without running JS: strings as they are,
    /// numbers in JS form, other values as `describe` prints them.
    pub fn display(&self, v: Value) -> String {
        let b = v.to_bits();
        if is_string(b) {
            string::to_rust_string(b)
        } else {
            describe(b)
        }
    }

    /// A handle that stops the running script from another thread.
    pub fn terminate_handle(&self) -> TerminateHandle {
        TerminateHandle(Arc::clone(&self.rt.terminate))
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
