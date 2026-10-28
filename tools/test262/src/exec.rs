//! `ep9-test262 exec`: run every test through the engine (test262 runner v0, M0 week 3).
//!
//! ```text
//! parent (exec)                                   worker process (exec-worker), one per thread
//!   discover test/**/*.js                           reads one test path per stdin line
//!   one thread per worker: "rel\n" ---stdin--->     front matter -> runs by flags (as `parse`)
//!                                                   per run: compile -> fresh Isolate
//!                                                     -> $262 prelude, assert.js, sta.js,
//!                                                        doneprintHandle.js (async), includes
//!                                                     -> the test (watchdog: terminate)
//!   <---------- one result line per test ------     "rel \t status \t ok \t runs \t cause \t detail"
//!   no line within the hard timeout: kill, respawn, record an infra error
//!   worker exits (crash, abort): record an infra error with its stderr tail, respawn
//!   counts per directory, top failure causes, expectations
//! ```
//!
//! - A test passes if every run passes. Panics, timeouts (also a run that returns after its
//!   deadline), worker crashes and internal BytecodeGen errors are infrastructure errors: they
//!   are never failures, no expectation can list them, and the run exits 2 before it compares
//!   or writes the expectation file (as `parse` does).
//! - A worker process isolates crashes: one crash loses one test, not the run.
//! - Each run gets a fresh isolate (D14: one per thread; the worker runs one at a time).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Exception, HostFn, Isolate,
    IsolateConfig, JsResult, Local, PropertyDescriptor, TerminateHandle,
};

use crate::frontmatter::{self, Meta};
use crate::parse::{Mode, STRICT_PREFIX, modes};
use crate::suite::{self, TestFile};
use crate::{expect, json, provenance};

/// Native stack of the thread that runs the isolates in a worker (the CLI uses the same).
const RUN_STACK_BYTES: usize = 64 << 20;

/// The `$262` host object (INTERPRETING.md "Host-Defined Functions"): `global`, `gc`,
/// `evalScript` (M2-P4, through `Cx::evaluate_script`) and `createRealm` (M3-P1, through
/// `Cx::create_realm`; each realm runs this prelude over its own host functions, so its
/// `evalScript` evaluates in that realm). `detachArrayBuffer` and `agent` are missing, so
/// tests that use them fail.
const PRELUDE: &str = "var $262 = { global: globalThis, gc: __ep9_gc, \
                       evalScript: __ep9_eval_script, createRealm: __ep9_create_realm };\n";

/// The host functions of every realm of a test isolate, as global names (`register`).
const HOST_FUNCTIONS: [(&str, u32, HostFn); 4] = [
    ("print", 1, print),
    ("__ep9_gc", 0, gc),
    ("__ep9_eval_script", 1, eval_script),
    ("__ep9_create_realm", 0, create_realm),
];

pub struct Options {
    pub test262: PathBuf,
    pub expect: PathBuf,
    pub update: bool,
    pub json: Option<PathBuf>,
    /// Worker processes.
    pub threads: usize,
    /// Soft timeout per run: the watchdog terminates the isolate.
    pub timeout: Duration,
    /// GC-stress for every isolate: collect before every n-th allocation, verify after each GC.
    pub gc_stress: u32,
    /// Tests that are neither run nor counted (only with `gc_stress`; tests that GC-stress
    /// makes too slow).
    pub skip_list: Option<PathBuf>,
    /// `IsolateConfig::feedback_threshold` of every isolate.
    pub feedback_threshold: u32,
    pub verbose: bool,
    pub progress: bool,
    /// Number of failure causes in the report.
    pub causes: usize,
    /// Path prefixes relative to the test262 root. Empty = everything.
    pub filters: Vec<String>,
    pub triple: &'static str,
}

pub struct WorkerOptions {
    pub test262: PathBuf,
    pub timeout: Duration,
    pub gc_stress: u32,
    pub feedback_threshold: u32,
}

// ---------------------------------------------------------------------------------------------
// Results

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    /// Panic, timeout, crash, unreadable file or bad front matter: not a test result.
    Infra,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Infra => "infra",
        }
    }

    fn from_name(s: &str) -> Option<Status> {
        match s {
            "pass" => Some(Status::Pass),
            "fail" => Some(Status::Fail),
            "infra" => Some(Status::Infra),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    pub status: Status,
    pub runs_ok: u32,
    pub runs: u32,
    /// Short grouping key of the first failing run (empty on a pass).
    pub cause: String,
    /// Every failing run: `[mode] text; ...`.
    pub detail: String,
}

impl TestResult {
    fn infra(cause: &str, detail: String) -> TestResult {
        TestResult {
            status: Status::Infra,
            runs_ok: 0,
            runs: 0,
            cause: cause.to_string(),
            detail,
        }
    }
}

pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// One protocol line (without the newline).
pub fn encode(rel: &str, r: &TestResult) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        escape(rel),
        r.status.name(),
        r.runs_ok,
        r.runs,
        escape(&r.cause),
        escape(&r.detail)
    )
}

/// Parses a protocol line: `(rel, result)`.
pub fn decode(line: &str) -> Option<(String, TestResult)> {
    let mut parts = line.splitn(6, '\t');
    let rel = unescape(parts.next()?);
    let status = Status::from_name(parts.next()?)?;
    let runs_ok = parts.next()?.parse().ok()?;
    let runs = parts.next()?.parse().ok()?;
    let cause = unescape(parts.next()?);
    let detail = unescape(parts.next()?);
    Some((
        rel,
        TestResult {
            status,
            runs_ok,
            runs,
            cause,
            detail,
        },
    ))
}

// ---------------------------------------------------------------------------------------------
// Worker: one test at a time

/// Outcome of one run (one mode) of a test.
#[derive(Debug)]
enum Run {
    Pass,
    Fail {
        cause: String,
        detail: String,
    },
    /// Infrastructure error: `cause` is `timeout`, `panic`, `harness` or `internal`.
    Infra {
        cause: &'static str,
        detail: String,
    },
}

fn infra(cause: &'static str, detail: impl Into<String>) -> Run {
    Run::Infra {
        cause,
        detail: detail.into(),
    }
}

fn fail(cause: impl Into<String>, detail: impl Into<String>) -> Run {
    Run::Fail {
        cause: cause.into(),
        detail: detail.into(),
    }
}

thread_local! {
    /// Output of `print` in the current run (async tests report through it).
    static OUT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// The last panic message (set by the worker's panic hook, from any thread).
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

fn take_panic() -> String {
    LAST_PANIC
        .lock()
        .map_or(None, |mut p| p.take())
        .unwrap_or_else(|| "panic without a message".into())
}

fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut line = String::new();
    for (i, v) in args.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(&cx.to_rust_string(v)?);
    }
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.push_str(&line);
        o.push('\n');
    });
    Ok(Local::undefined())
}

/// `$262.evalScript(source)` (INTERPRETING.md): ParseScript and ScriptEvaluation of
/// ToString(source) in the current realm, through `Cx::evaluate_script` (the compiler hook
/// `compile_script`; an early error throws a SyntaxError of this realm). v0 returns what the
/// script's entry function returns (undefined), not its completion value. Source text with a
/// lone surrogate throws the v0 not-implemented error (it cannot be `&str`).
fn eval_script<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let s = cx.to_string(args.get(0))?;
    let flat = cx.flatten(s)?;
    let units: Vec<u16> = (0..cx.string_len(s))
        .map(|i| cx.code_unit_at(flat, i).unwrap_or(0))
        .collect();
    let Ok(text) = String::from_utf16(&units) else {
        return Err(cx.not_implemented("$262.evalScript of source text with a lone surrogate"));
    };
    cx.evaluate_script(&text, "evalScript")
}

fn gc<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.collect_garbage();
    Ok(Local::undefined())
}

/// `$262.createRealm()` (INTERPRETING.md): a new realm (`Cx::create_realm`, every builtin
/// table), with the host functions of [`HOST_FUNCTIONS`] made in it and [`PRELUDE`] run in it.
/// Returns that realm's `$262`.
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let global = cx.create_realm()?;
    cx.with_realm(global, |cx| {
        for (name, length, f) in HOST_FUNCTIONS {
            // A function of the current realm, as `Isolate::register_host_function` makes:
            // writable, configurable, not enumerable.
            let func = cx.new_builtin_closure(f, name, length, Local::undefined());
            let desc = PropertyDescriptor::data(func.into(), true, false, true);
            cx.define_property_or_throw(global, name, desc)?;
        }
        cx.evaluate_script(PRELUDE, "harness/$262 prelude")?;
        cx.get(global, "$262")
    })
}

/// The run that the watchdog watches.
#[derive(Default)]
struct Watch {
    /// Deadline and isolate of the armed run.
    armed: Option<(Instant, TerminateHandle)>,
    /// The deadline of the armed run passed.
    expired: bool,
}

/// Terminates the armed isolate when its deadline passes, and remembers that it passed: a
/// native loop that does not poll termination can outlast the deadline and then return.
struct Watchdog {
    slot: Arc<Mutex<Watch>>,
}

impl Watchdog {
    fn start() -> Watchdog {
        let slot: Arc<Mutex<Watch>> = Arc::default();
        let watched = Arc::clone(&slot);
        std::thread::Builder::new()
            .name("ep9-test262-watchdog".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(20));
                    let Ok(mut s) = watched.lock() else { return };
                    if let Some((deadline, handle)) = s.armed.as_ref()
                        && Instant::now() >= *deadline
                    {
                        handle.terminate();
                        s.armed = None;
                        s.expired = true;
                    }
                }
            })
            .expect("spawn the watchdog thread");
        Watchdog { slot }
    }

    fn arm(&self, handle: TerminateHandle, timeout: Duration) {
        if let Ok(mut s) = self.slot.lock() {
            *s = Watch {
                armed: Some((Instant::now() + timeout, handle)),
                expired: false,
            };
        }
    }

    /// Stops the clock. True if the deadline of the last armed run passed, whether or not the
    /// run saw the termination. The next call returns false until the next `arm`.
    fn disarm(&self) -> bool {
        let Ok(mut s) = self.slot.lock() else {
            return false;
        };
        let late = s
            .armed
            .take()
            .is_some_and(|(deadline, _)| Instant::now() >= deadline);
        std::mem::take(&mut s.expired) || late
    }
}

/// Harness include sources, read once per worker.
#[derive(Default)]
struct Harness {
    files: HashMap<String, Result<Arc<str>, String>>,
}

impl Harness {
    fn get(&mut self, root: &Path, name: &str) -> Result<Arc<str>, String> {
        self.files
            .entry(name.to_string())
            .or_insert_with(|| {
                let path = root.join("harness").join(name);
                match std::fs::read(&path).map(String::from_utf8) {
                    Ok(Ok(s)) => Ok(Arc::from(s)),
                    Ok(Err(_)) => Err(format!("harness/{name} is not valid UTF-8")),
                    Err(e) => Err(format!("cannot read harness/{name}: {e}")),
                }
            })
            .clone()
    }
}

/// The grouping key of an uncaught exception message.
pub fn cause_of_message(msg: &str) -> String {
    if let Some(rest) = msg.strip_prefix("Error: not implemented in ep9 v0: ") {
        return format!("not implemented: {rest}");
    }
    if msg.starts_with("Test262Error") {
        return "Test262Error (an assertion failed)".into();
    }
    let mut s: String = msg.lines().next().unwrap_or("").chars().take(100).collect();
    if s.is_empty() {
        s = "(empty message)".into();
    }
    s
}

/// The dotted name (`Object.defineProperty`) around 1-based `line` and UTF-16 `column` of
/// `src`: the VM reports a failed call at the property name of the callee.
pub fn callee_at(src: &str, line: u32, column: u32) -> Option<String> {
    let text = src.split('\n').nth(line.checked_sub(1)? as usize)?;
    // Column -> char index (UTF-16 units; astral characters count 2).
    let mut units = 0u32;
    let chars: Vec<char> = text.chars().collect();
    let mut at = None;
    for (i, c) in chars.iter().enumerate() {
        if units + 1 >= column {
            at = Some(i);
            break;
        }
        units += c.len_utf16() as u32;
    }
    let at = at?;
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let mut start = at;
    while start > 0 && (word(chars[start - 1]) || chars[start - 1] == '.') {
        start -= 1;
    }
    let mut end = at;
    while end < chars.len() && word(chars[end]) {
        end += 1;
    }
    let name: String = chars[start..end].iter().collect();
    let name = name.trim_matches('.');
    (!name.is_empty() && name.len() <= 60).then(|| name.to_string())
}

/// The grouping key and text of a compile error.
fn compile_failure(e: &CodegenError, what: &str) -> Run {
    match e {
        CodegenError::Unsupported(u) => fail(
            format!("unsupported: {}", u.feature),
            format!("{what}: {e}"),
        ),
        CodegenError::Compile(CompileError::Syntax(errs)) => fail(
            "unexpected SyntaxError at compile time",
            format!("{what}: {}", errs.first()),
        ),
        CodegenError::Compile(CompileError::Range(r)) => {
            fail("RangeError at compile time", format!("{what}: {r}"))
        }
        CodegenError::Limit(m) => fail(
            "bytecode limit at compile time",
            format!("{what}: RangeError: {m}"),
        ),
        // A BytecodeGen bug (builder or verifier rejection), not a test result: an
        // expectation file must not list it.
        CodegenError::Internal(m) => infra(
            "internal",
            format!("{what}: internal BytecodeGen error: {m}"),
        ),
    }
}

/// Compiles `src`; `name` is the test path or `harness/<file>` (for the `internal` test hook).
fn compile(
    src: &str,
    opts: SourceOpts,
    name: &str,
) -> Result<ep9_bytecode::VerifiedUnit, CodegenError> {
    if fault_for("internal", name) {
        return Err(CodegenError::Internal(format!("injected for {name}")));
    }
    ep9_frontend::compile(src, opts, CompileOptions::default())
}

/// Does an uncaught exception match the negative `error_type`? INTERPRETING.md: the name of
/// the thrown value's constructor must be `error_type`. The text does not count: a thrown
/// string `"TypeError: x"` has no constructor name, so it never matches.
fn exception_matches(e: &Exception, error_type: &str) -> bool {
    e.constructor.as_deref() == Some(error_type)
}

/// Test hook (only for tools/test262/tests): `EP9_TEST262_FAULT=<kind>:<substr>,...`. Is an
/// item of `kind` set for `name`? Kinds:
/// - `panic`, `abort`, `hang`, `nounwind-panic` (a panic that aborts): the worker fails.
/// - `late`: the run of a matching test returns normally, but only after the soft timeout
///   (as a native loop that does not poll termination does).
/// - `internal`: BytecodeGen reports an internal error for a matching test path or
///   `harness/<file>`. The `parse-*` kinds are the parse worker's (crate::parse).
pub(crate) fn fault_for(kind: &str, name: &str) -> bool {
    std::env::var("EP9_TEST262_FAULT").is_ok_and(|spec| {
        spec.split(',').any(|item| {
            item.split_once(':')
                .is_some_and(|(k, needle)| k == kind && !needle.is_empty() && name.contains(needle))
        })
    })
}

fn injected_fault(rel: &str) {
    if fault_for("panic", rel) {
        panic!("injected panic for {rel}");
    }
    if fault_for("abort", rel) {
        std::process::abort();
    }
    if fault_for("hang", rel) {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Whether `EP9_GC_GENERATIONAL=1` turns on the S1 generational heap (see [`isolate_config`]).
fn generational_from_env() -> bool {
    std::env::var_os("EP9_GC_GENERATIONAL").is_some_and(|v| v == "1")
}

/// The config of every test isolate: the builtins of `ep9_builtins` (H20) on top of the VM's.
/// `EP9_GC_GENERATIONAL=1` in the environment (inherited by the workers) turns on the S1
/// generational heap (`HeapConfig::generational`): with `--gc-stress` that is the S1
/// GC-stress mode, a minor GC at every Nth allocation and a major one every 64th GC (V9-D27;
/// plan v9 §5.1). The switch lives in `HeapConfig`, never in `IsolateConfig`.
fn isolate_config(gc_stress: u32, feedback_threshold: u32) -> IsolateConfig {
    let generational = generational_from_env();
    IsolateConfig {
        heap: HeapConfig {
            gc_stress,
            verify_after_gc: gc_stress > 0,
            generational,
            ..HeapConfig::default()
        },
        feedback_threshold,
        builtins: Some(&ep9_builtins::TABLE),
        compiler: Some(&COMPILER),
        ..IsolateConfig::default()
    }
}

// ---- the compiler hooks (C2): ep9_frontend behind `IsolateConfig::compiler` ----------------

/// The compiler entry points of the test isolates: ep9_vm does not link the front end, so eval,
/// `Function()` and run-time scripts reach it through these (M2-P0: the eval and
/// `Function()` entries still answer `Unsupported`).
static COMPILER: CompilerHooks = CompilerHooks {
    compile_eval: hook_compile_eval,
    compile_function: hook_compile_function,
    compile_script: hook_compile_script,
};

fn hook_compile_eval(source: &str, ctx: &EvalContext) -> CompileResult {
    ep9_frontend::compile_eval(source, ctx, CompileOptions::default()).map_err(hook_failure)
}

fn hook_compile_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
    ep9_frontend::compile_function_ctor(kind, params, body, CompileOptions::default())
        .map_err(hook_failure)
}

fn hook_compile_script(source: &str) -> CompileResult {
    let opts = SourceOpts::script(Strictness::Sloppy);
    ep9_frontend::compile(source, opts, CompileOptions::default()).map_err(hook_failure)
}

/// A front-end error as the VM's [`CompileFailure`] (the message without a position).
fn hook_failure(e: CodegenError) -> CompileFailure {
    match e {
        CodegenError::Compile(CompileError::Syntax(errors)) => {
            CompileFailure::Syntax(errors.first().message.clone())
        }
        CodegenError::Compile(CompileError::Range(r)) => CompileFailure::Range(r.message),
        CodegenError::Unsupported(u) => CompileFailure::Unsupported(u.feature.to_string()),
        CodegenError::Limit(m) => CompileFailure::Range(m),
        CodegenError::Internal(m) => CompileFailure::Internal(m),
    }
}

/// `exec` exits 2 unless an isolate made with `config` has a builtin table: a run without
/// one would fail thousands of tests and could rewrite the expectations with them.
fn check_builtins(config: IsolateConfig) -> Result<(), String> {
    let iso = Isolate::new(config).map_err(|e| format!("cannot create an isolate: {e}"))?;
    match iso.builtin_table() {
        Some(_) => Ok(()),
        None => Err("no builtin table is installed (IsolateConfig::builtins)".to_string()),
    }
}

struct RunCtx<'a> {
    opts: &'a WorkerOptions,
    harness: &'a mut Harness,
    watchdog: &'a Watchdog,
}

impl RunCtx<'_> {
    fn isolate(&self) -> Result<Isolate, String> {
        let config = isolate_config(self.opts.gc_stress, self.opts.feedback_threshold);
        let mut iso = Isolate::new(config).map_err(|e| format!("cannot create an isolate: {e}"))?;
        for (name, length, f) in HOST_FUNCTIONS {
            iso.register_host_function(name, length, f);
        }
        Ok(iso)
    }

    /// Compiles and runs one harness script. `None` = it ran.
    fn harness_script(&mut self, iso: &mut Isolate, name: &str, src: &str) -> Option<Run> {
        let what = format!("harness/{name}");
        let unit = match compile(src, SourceOpts::script(Strictness::Sloppy), &what) {
            Ok(u) => u,
            Err(e) => return Some(compile_failure(&e, &what)),
        };
        match iso.with_cx_checkpoint(|cx| cx.run_script(unit, &what).map(|_| ())) {
            Ok(()) => None,
            Err(e) if e.terminated => Some(infra("timeout", format!("timeout in {what}"))),
            Err(e) => Some(fail(
                format!("{what} threw: {}", cause_of_message(&e.message)),
                format!("{what}: {e}"),
            )),
        }
    }

    fn run(&mut self, rel: &str, src: &str, meta: &Meta, mode: Mode) -> Run {
        let negative = meta.negative.as_ref();
        let (source, opts) = match mode {
            Mode::Strict => (
                std::borrow::Cow::Owned(format!("{STRICT_PREFIX}{src}")),
                SourceOpts::script(Strictness::Sloppy),
            ),
            Mode::Sloppy | Mode::Raw => (
                std::borrow::Cow::Borrowed(src),
                SourceOpts::script(Strictness::Sloppy),
            ),
            Mode::Module => (std::borrow::Cow::Borrowed(src), SourceOpts::module()),
        };
        let compiled = compile(&source, opts, rel);
        if let Err(e @ CodegenError::Internal(_)) = &compiled {
            return compile_failure(e, "test");
        }
        if let Some(neg) = negative
            && neg.phase == "parse"
        {
            return match compiled {
                Err(CodegenError::Compile(CompileError::Syntax(_)))
                    if neg.error_type == "SyntaxError" =>
                {
                    Run::Pass
                }
                Err(e) => fail(
                    "negative parse test: wrong error",
                    format!("expected an early {}, got {e}", neg.error_type),
                ),
                Ok(_) => fail(
                    "negative parse test: no early error",
                    format!("expected an early {}, but it compiled", neg.error_type),
                ),
            };
        }
        let unit = match compiled {
            Ok(u) => u,
            Err(e) => return compile_failure(&e, "test"),
        };
        let mut iso = match self.isolate() {
            Ok(i) => i,
            Err(e) => return infra("panic", e),
        };
        self.watchdog.arm(iso.terminate_handle(), self.opts.timeout);
        OUT.with(|o| o.borrow_mut().clear());
        if mode != Mode::Raw {
            if let Some(r) = self.harness_script(&mut iso, "$262 prelude", PRELUDE) {
                return r;
            }
            let mut names = vec!["assert.js".to_string(), "sta.js".to_string()];
            if meta.has_flag("async") {
                names.push("doneprintHandle.js".to_string());
            }
            names.extend(meta.includes.iter().cloned());
            for name in &names {
                let src = match self.harness.get(&self.opts.test262, name) {
                    Ok(s) => s,
                    Err(e) => return infra("harness", e),
                };
                if let Some(r) = self.harness_script(&mut iso, name, &src) {
                    return r;
                }
            }
        }
        // The test262 loader is the module host of the test and its jobs, so `import()`
        // works in a script and in a job too (a module run uses the same hooks).
        let root = &self.opts.test262;
        let mut resolve =
            |referrer: &str, specifier: &str| resolve_module(root, referrer, specifier);
        let mut load = |name: &str| load_module(root, name);
        let result = iso.with_module_host(&mut resolve, &mut load, |iso| {
            if mode == Mode::Module {
                run_module(iso, root, unit, rel)
            } else {
                iso.with_cx_checkpoint(|cx| cx.run_script(unit, rel).map(|_| ()))
            }
        });
        if fault_for("late", rel) {
            std::thread::sleep(self.opts.timeout + Duration::from_millis(100));
        }
        // A module's loading and linking errors (phase `resolution`) are thrown by
        // `run_module` before any module code runs; those tests start with
        // `$DONOTEVALUATE()`, so an evaluated test throws its Test262Error instead.
        let runtime_negative = negative
            .filter(|n| n.phase == "runtime" || (mode == Mode::Module && n.phase == "resolution"));
        match result {
            Err(e) if e.terminated => infra(
                "timeout",
                format!("timeout: the test ran longer than {:?}", self.opts.timeout),
            ),
            Err(e) => match runtime_negative {
                Some(n) if exception_matches(&e, &n.error_type) => Run::Pass,
                Some(n) => fail(
                    "negative runtime test: wrong error",
                    format!(
                        "expected {}, got {e} (constructor: {})",
                        n.error_type,
                        e.constructor.as_deref().unwrap_or("none")
                    ),
                ),
                None => {
                    let mut cause = cause_of_message(&e.message);
                    // "x is not a function": name the callee from the source text, so the
                    // report says which built-in is missing.
                    if e.message.ends_with(" is not a function")
                        && let Some(loc) = &e.location
                        && loc.file == rel
                        && let Some(name) = callee_at(&source, loc.line, loc.column)
                    {
                        cause = format!("{cause} ({name})");
                    }
                    fail(cause, e.to_string())
                }
            },
            Ok(()) => {
                if let Some(n) = negative {
                    return fail(
                        format!("negative {} test: no error", n.phase),
                        format!(
                            "expected {} in phase {}, but it completed",
                            n.error_type, n.phase
                        ),
                    );
                }
                if meta.has_flag("async") {
                    return OUT.with(|o| async_outcome(&o.borrow()));
                }
                Run::Pass
            }
        }
    }
}

// ---- the module loader for `flags: [module]` files (M3-P2 MODULE) -------------------------

/// Runs the module test `unit` (named `rel`) with `Cx::run_module` and the test262 loader,
/// then a microtask checkpoint (`Isolate::with_cx_checkpoint`): an error of the module wins
/// over one of a job, but not over a termination in a job.
fn run_module(
    iso: &mut Isolate,
    root: &Path,
    unit: ep9_bytecode::VerifiedUnit,
    rel: &str,
) -> Result<(), Exception> {
    let mut resolve = |referrer: &str, specifier: &str| resolve_module(root, referrer, specifier);
    let mut load = |name: &str| load_module(root, name);
    iso.with_cx_checkpoint(|cx| {
        cx.run_module(unit, rel, &mut resolve, &mut load)
            .map(|_| ())
    })
}

/// HostLoadImportedModule's name step for test262: a relative specifier (`./x.js`,
/// `../x.js`) names a file next to the referrer (a path relative to the test262 root, with
/// `.` and `..` removed); the name is that path. Another specifier, or a missing file, is an
/// error.
fn resolve_module(root: &Path, referrer: &str, specifier: &str) -> Result<String, String> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return Err(format!(
            "Cannot find module '{specifier}' imported from {referrer} (not a relative specifier)"
        ));
    }
    let mut parts: Vec<&str> = referrer.split('/').collect();
    parts.pop();
    for p in specifier.split('/') {
        match p {
            "." | "" => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!(
                        "Cannot find module '{specifier}' imported from {referrer}"
                    ));
                }
            }
            p => parts.push(p),
        }
    }
    let name = parts.join("/");
    if root.join(&name).is_file() {
        Ok(name)
    } else {
        Err(format!(
            "Cannot find module '{specifier}' imported from {referrer}"
        ))
    }
}

/// HostLoadImportedModule's load step for test262: the file `name` (relative to the test262
/// root) compiled as a module (the `internal` test hook applies to it).
fn load_module(root: &Path, name: &str) -> CompileResult {
    let src = match std::fs::read(root.join(name)).map(String::from_utf8) {
        Ok(Ok(s)) => s,
        _ => return Err(CompileFailure::Internal(format!("cannot read {name}"))),
    };
    compile(&src, SourceOpts::module(), name).map_err(hook_failure)
}

/// The result of an async test from its `print` output (INTERPRETING.md, flag `async`). A
/// `Test262:AsyncTestFailure:` line fails it, before or after a completion; it passes on
/// exactly one `Test262:AsyncTestComplete` line (a second one means `$DONE` ran twice).
fn async_outcome(out: &str) -> Run {
    if let Some(msg) = out
        .lines()
        .find_map(|l| l.strip_prefix("Test262:AsyncTestFailure:"))
    {
        return fail(cause_of_message(msg), format!("async: {msg}"));
    }
    match out
        .lines()
        .filter(|l| *l == "Test262:AsyncTestComplete")
        .count()
    {
        1 => Run::Pass,
        0 => fail(
            "async test: $DONE was not called",
            "async: no Test262:AsyncTestComplete",
        ),
        n => fail(
            "async test: $DONE was called more than once",
            format!("async: Test262:AsyncTestComplete {n} times"),
        ),
    }
}

fn run_test(
    opts: &WorkerOptions,
    harness: &mut Harness,
    watchdog: &Watchdog,
    rel: &str,
) -> TestResult {
    let path = opts.test262.join(rel);
    let src = match std::fs::read(&path).map(String::from_utf8) {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => {
            return TestResult::infra("unreadable test", "file is not valid UTF-8".into());
        }
        Err(e) => return TestResult::infra("unreadable test", format!("cannot read: {e}")),
    };
    let meta = match frontmatter::parse(&src) {
        Ok(m) => m,
        Err(e) => return TestResult::infra("bad front matter", format!("front matter: {e}")),
    };
    let mut result = TestResult {
        status: Status::Pass,
        runs_ok: 0,
        runs: 0,
        cause: String::new(),
        detail: String::new(),
    };
    for mode in modes(&meta) {
        result.runs += 1;
        let run = catch_unwind(AssertUnwindSafe(|| {
            injected_fault(rel);
            if fault_for("nounwind-panic", rel) {
                panic_without_unwind(format!("injected panic for {rel}"));
            }
            let mut cx = RunCtx {
                opts,
                harness: &mut *harness,
                watchdog,
            };
            cx.run(rel, &src, &meta, mode)
        }))
        .unwrap_or_else(|_| infra("panic", format!("panic: {}", take_panic())));
        // Past the deadline: a timeout even if the run returned, never a pass or a failure.
        let run = if watchdog.disarm() && !matches!(run, Run::Infra { .. }) {
            infra(
                "timeout",
                format!(
                    "timeout: the test ran longer than {:?} (it returned after the deadline)",
                    opts.timeout
                ),
            )
        } else {
            run
        };
        let (status, cause, text) = match run {
            Run::Pass => {
                result.runs_ok += 1;
                continue;
            }
            Run::Fail { cause, detail } => (Status::Fail, cause, detail),
            Run::Infra { cause, detail } => (Status::Infra, cause.to_string(), detail),
        };
        if !result.detail.is_empty() {
            result.detail.push_str("; ");
        }
        let _ = write!(result.detail, "[{}] {text}", mode.name());
        // Infra beats fail; the first cause of the winning status names the test.
        if status == Status::Infra && result.status != Status::Infra {
            result.status = Status::Infra;
            result.cause = cause;
        } else if result.status == Status::Pass {
            result.status = status;
            result.cause = cause;
        }
    }
    result
}

/// The panic hook of both worker processes (`exec-worker`, `parse-worker`). It keeps the text
/// for the result line ([`take_panic`]) and writes it to stderr. Under `panic = "abort"`
/// (P2-INFRA2) the process ends right after the hook; the parent then reports a `worker crash`
/// with the stderr tail ([`WorkerProc`]), so the stderr line is how the panic text reaches the
/// report.
pub(crate) fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(ToString::to_string)
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic".into());
        let at = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        if let Ok(mut p) = LAST_PANIC.lock() {
            *p = Some(format!("{msg}{at}"));
        }
        // A closed stderr must not panic inside the hook (that aborts).
        let _ = writeln!(io::stderr(), "ep9-test262 worker panicked: {msg}{at}");
    }));
}

/// Panics inside an `extern "C"` function. A panic cannot unwind out of one, so Rust aborts
/// the process after the panic hook ran ("panic in a function that cannot unwind"): the same
/// end as any panic in a `panic = "abort"` build. The test hooks `nounwind-panic` and
/// `parse-nounwind-panic` use it to show that the panic text still reaches the report.
pub(crate) fn panic_without_unwind(text: String) -> ! {
    thread_local! {
        static TEXT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }
    extern "C" fn panic_here() -> ! {
        panic!("{}", TEXT.with(std::cell::RefCell::take));
    }
    TEXT.with(|t| *t.borrow_mut() = text);
    panic_here()
}

/// `exec-worker`: reads test paths from stdin, writes one result line per test to stdout.
pub fn worker(opts: WorkerOptions) -> ExitCode {
    install_panic_hook();
    let runner = std::thread::Builder::new()
        .name("ep9-test262-run".into())
        .stack_size(RUN_STACK_BYTES)
        .spawn(move || {
            let watchdog = Watchdog::start();
            let mut harness = Harness::default();
            let stdin = io::stdin();
            let mut out = io::stdout().lock();
            for line in stdin.lock().lines() {
                let Ok(rel) = line else { return 2 };
                let rel = rel.trim_end_matches('\r');
                if rel.is_empty() {
                    continue;
                }
                let r = run_test(&opts, &mut harness, &watchdog, rel);
                if writeln!(out, "{}", encode(rel, &r))
                    .and_then(|()| out.flush())
                    .is_err()
                {
                    return 2;
                }
            }
            0
        });
    match runner.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => ExitCode::from(code),
        _ => ExitCode::from(3),
    }
}

// ---------------------------------------------------------------------------------------------
// Parent: worker processes, report, expectations

/// One worker process (`ep9-test262 <subcommand> <flags>`): request lines in on stdin, one
/// reply line out on stdout per request, in order. A crash ends only this process.
pub(crate) struct WorkerProc {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    stderr_tail: Arc<Mutex<String>>,
}

/// Bytes of a worker's stderr kept for a crash report.
const STDERR_TAIL: usize = 4096;

/// Sends each line a worker writes to stdout, without its newline (`\n` or `\r\n`). A last
/// piece with no newline is a reply cut short (the worker died while it wrote): it is dropped,
/// as is everything after a line that is not UTF-8, so the parent sees the worker gone (a
/// crash), never a short reply that might decode.
fn forward_lines(mut r: impl BufRead, tx: &mpsc::Sender<String>) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if !matches!(r.read_until(b'\n', &mut buf), Ok(n) if n > 0) || buf.pop() != Some(b'\n') {
            return;
        }
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        let Ok(line) = String::from_utf8(core::mem::take(&mut buf)) else {
            return;
        };
        if tx.send(line).is_err() {
            return;
        }
    }
}

/// A worker that is gone while it had a request: it exited (crash, abort, signal), wrote a bad
/// reply, or gave no reply in time and was killed. Never a test result.
pub(crate) struct Gone {
    /// `worker crash` or `timeout`.
    pub(crate) cause: &'static str,
    pub(crate) detail: String,
}

impl WorkerProc {
    fn spawn(subcommand: &str, args: &[String]) -> io::Result<WorkerProc> {
        let exe = std::env::current_exe()?;
        let mut child = Command::new(exe)
            .arg(subcommand)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || forward_lines(BufReader::new(stdout), &tx));
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let tail = Arc::clone(&stderr_tail);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if let Ok(mut t) = tail.lock() {
                            t.push_str(&String::from_utf8_lossy(&buf[..n]));
                            if t.len() > STDERR_TAIL {
                                let mut cut = t.len() - STDERR_TAIL;
                                while !t.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                t.drain(..cut);
                            }
                        }
                    }
                }
            }
        });
        Ok(WorkerProc {
            child,
            stdin,
            lines,
            stderr_tail,
        })
    }

    /// Writes request lines (each ends in a newline) in one write. `Err` = the worker closed
    /// its input.
    fn send(&mut self, lines: &str) -> io::Result<()> {
        self.stdin
            .write_all(lines.as_bytes())
            .and_then(|()| self.stdin.flush())
    }

    /// The next reply line. `Err` = the worker is gone: drop it and spawn a new one.
    fn recv(&mut self, hard: Duration) -> Result<String, Gone> {
        match self.lines.recv_timeout(hard) {
            Ok(line) => Ok(line),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
                Err(Gone {
                    cause: "timeout",
                    detail: format!(
                        "hard timeout: no result after {hard:?}; the worker was killed"
                    ),
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(self.crashed("the worker exited")),
        }
    }

    fn crashed(&mut self, what: &str) -> Gone {
        let _ = self.child.kill();
        let status = self
            .child
            .wait()
            .map_or_else(|e| e.to_string(), |s| s.to_string());
        // Let the stderr reader finish.
        std::thread::sleep(Duration::from_millis(50));
        let tail = self
            .stderr_tail
            .lock()
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        Gone {
            cause: "worker crash",
            detail: format!("{what} ({status}); stderr: {tail}"),
        }
    }
}

impl Drop for WorkerProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How [`in_workers`] runs its jobs.
pub(crate) struct Workers<'a> {
    /// The worker command (`exec-worker`, `parse-worker`) and its flags.
    pub(crate) subcommand: &'static str,
    pub(crate) args: &'a [String],
    /// Worker processes, each driven by one parent thread.
    pub(crate) threads: usize,
    /// Jobs a parent thread takes at a time. With 1, a worker has one job at a time. With more,
    /// the parent takes the next `chunk` jobs while fewer than `chunk` are unanswered, so the
    /// worker never waits for a round trip (it may then answer in groups).
    pub(crate) chunk: usize,
    /// No reply within this time: kill the worker (a `timeout`).
    pub(crate) hard: Duration,
}

/// The callbacks of [`in_workers`].
struct Jobs<'a, T, Q, P, G, D> {
    items: &'a [T],
    request: Q,
    reply: P,
    gone: G,
    done: D,
}

/// Runs every job in worker processes and returns the results in job order.
///
/// ```text
/// parent thread (one per worker)                worker process
///   take jobs ----------- request lines ---->     one reply line per request, in order
///   reply(job, line) <--- reply lines -------     (single, or a group at a time)
///   worker gone, 1 job unanswered: gone(job, Gone); a new worker takes the jobs after it
///   worker gone, n > 1 unanswered: run those n again one at a time, so the job that stops
///     the worker again gets the Gone; if none does, the first of them gets the first Gone
/// ```
///
/// `request` gives a job's request line (no newline). `reply` reads its reply line; `None` (a
/// bad line) counts as a crash. `done(n)` runs after each job, `n` = jobs done so far.
/// `Err` = a worker process could not be started.
pub(crate) fn in_workers<T, R>(
    w: &Workers<'_>,
    items: &[T],
    request: impl Fn(&T) -> String + Sync,
    reply: impl Fn(&T, &str) -> Option<R> + Sync,
    gone: impl Fn(&T, Gone) -> R + Sync,
    done: impl Fn(usize) + Sync,
) -> Result<Vec<R>, String>
where
    T: Sync,
    R: Send,
{
    let jobs = Jobs {
        items,
        request,
        reply,
        gone,
        done,
    };
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let threads = w.threads.clamp(1, items.len().max(1));
    let spawn_error: Mutex<Option<String>> = Mutex::new(None);
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| scope.spawn(|| drive(w, &jobs, &next, &finished, &spawn_error)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("runner thread"))
            .collect()
    });
    if let Some(e) = spawn_error.lock().ok().and_then(|mut s| s.take()) {
        return Err(e);
    }
    let mut slots: Vec<Option<R>> = std::iter::repeat_with(|| None).take(items.len()).collect();
    for (i, r) in parts.into_iter().flatten() {
        slots[i] = Some(r);
    }
    Ok(slots
        .into_iter()
        .map(|r| r.expect("every job ran once"))
        .collect())
}

/// One parent thread of [`in_workers`]: its worker process and its jobs.
fn drive<T, R, Q, P, G, D>(
    w: &Workers<'_>,
    jobs: &Jobs<'_, T, Q, P, G, D>,
    next: &AtomicUsize,
    finished: &AtomicUsize,
    spawn_error: &Mutex<Option<String>>,
) -> Vec<(usize, R)>
where
    Q: Fn(&T) -> String,
    P: Fn(&T, &str) -> Option<R>,
    G: Fn(&T, Gone) -> R,
    D: Fn(usize),
{
    let chunk = w.chunk.max(1);
    let mut out: Vec<(usize, R)> = Vec::new();
    let mut worker: Option<WorkerProc> = None;
    // Jobs taken and not answered yet, in order. The first `sent` went to `worker`.
    let mut pending: VecDeque<usize> = VecDeque::new();
    let mut sent = 0;
    // After a worker stopped with several jobs unanswered: how many jobs at the front of
    // `pending` still run one at a time, and the first Gone with its job while no job has
    // stopped a worker again.
    let mut alone = 0;
    let mut unexplained: Option<(usize, Gone)> = None;
    loop {
        if pending.len() < chunk {
            let start = next.fetch_add(chunk, Ordering::Relaxed);
            if start < jobs.items.len() {
                pending.extend(start..jobs.items.len().min(start + chunk));
            } else if pending.is_empty() {
                return out;
            }
        }
        if worker.is_none() {
            match WorkerProc::spawn(w.subcommand, w.args) {
                Ok(p) => {
                    worker = Some(p);
                    sent = 0;
                }
                Err(e) => {
                    if let Ok(mut s) = spawn_error.lock() {
                        *s = Some(e.to_string());
                    }
                    return out;
                }
            }
        }
        let proc = worker.as_mut().expect("spawned above");
        let limit = if alone > 0 { 1 } else { pending.len() };
        if sent < limit {
            let mut lines = String::new();
            for &i in pending.range(sent..limit) {
                lines.push_str(&(jobs.request)(&jobs.items[i]));
                lines.push('\n');
            }
            // On an error the worker is gone; the next reply says how.
            if proc.send(&lines).is_ok() {
                sent = limit;
            }
        }
        let i = *pending.front().expect("taken above");
        let result = if sent == 0 {
            Err(proc.crashed("the worker closed its input"))
        } else {
            proc.recv(w.hard).and_then(|line| {
                (jobs.reply)(&jobs.items[i], &line)
                    .ok_or_else(|| proc.crashed(&format!("bad result line {line:?}")))
            })
        };
        let r = match result {
            Ok(r) => {
                sent -= 1;
                r
            }
            Err(g) => {
                let unanswered = sent;
                worker = None;
                sent = 0;
                if alone == 0 && unanswered > 1 {
                    // Not known which of them stopped the worker: run each alone.
                    alone = unanswered;
                    unexplained = Some((i, g));
                    continue;
                }
                if alone > 0 {
                    unexplained = None;
                }
                (jobs.gone)(&jobs.items[i], g)
            }
        };
        pending.pop_front();
        out.push((i, r));
        (jobs.done)(finished.fetch_add(1, Ordering::Relaxed) + 1);
        if alone > 0 {
            alone -= 1;
            if alone == 0
                && let Some((j, g)) = unexplained.take()
            {
                // No job stopped a worker again (a crash from outside, or one that needs other
                // jobs in the same worker): still an infrastructure error, on the first of them.
                let g = Gone {
                    cause: g.cause,
                    detail: format!(
                        "{} (the worker stopped with several files unanswered; none stopped it \
                         again when each ran alone)",
                        g.detail
                    ),
                };
                let slot = out
                    .iter_mut()
                    .rev()
                    .find(|(k, _)| *k == j)
                    .expect("the job ran alone");
                slot.1 = (jobs.gone)(&jobs.items[j], g);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Count {
    passed: usize,
    total: usize,
}

impl Count {
    fn add(&mut self, passed: bool) {
        self.total += 1;
        self.passed += usize::from(passed);
    }

    fn add_n(&mut self, passed: u32, total: u32) {
        self.total += total as usize;
        self.passed += passed as usize;
    }

    fn percent(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            100.0 * self.passed as f64 / self.total as f64
        }
    }
}

/// The header of an exec expectation file.
fn header(triple: &str) -> String {
    format!(
        "# ep9-test262 exec (mode jit): known failures on {triple}. Paths are relative to the\n\
         # test262 root. Infrastructure errors (panic, timeout, crash) are never listed here.\n\
         # Regenerate: cargo run --release -p ep9_test262 -- exec --update-expectations\n"
    )
}

/// Run the `exec` command. Exit code: 0 = matches the expectations, 1 = differs, 2 = error.
pub fn run(opts: &Options) -> ExitCode {
    let started = Instant::now();
    // The source facts before any test runs (CX5-C1): the summary compares them with a second
    // read when it is written.
    let source = provenance::source(&crate::repo_root(), &provenance::process_env);
    if let Err(e) = check_builtins(isolate_config(0, opts.feedback_threshold)) {
        eprintln!("ep9-test262: {e}");
        return ExitCode::from(2);
    }
    let root = &opts.test262;
    let dirs = match suite::all_dirs(root) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "ep9-test262: cannot list {}: {e}",
                root.join("test").display()
            );
            return ExitCode::from(2);
        }
    };
    let dir_refs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let mut files: Vec<TestFile> = match suite::discover(root, &dir_refs) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("ep9-test262: cannot list tests in {}: {e}", root.display());
            return ExitCode::from(2);
        }
    };
    let full = opts.filters.is_empty();
    if !full {
        files.retain(|f| opts.filters.iter().any(|p| f.rel.starts_with(p.as_str())));
        if files.is_empty() {
            eprintln!("ep9-test262: no test matches {:?}", opts.filters);
            return ExitCode::from(2);
        }
    }
    if opts.update && !full {
        eprintln!("ep9-test262: --update-expectations needs a full run (no path filters)");
        return ExitCode::from(2);
    }
    // Skipped tests are neither run nor counted, and the expectation compare ignores them.
    let mut skipped: BTreeSet<String> = BTreeSet::new();
    if let Some(path) = &opts.skip_list {
        let list = match expect::read(path) {
            Ok(list) => list,
            Err(e) => {
                eprintln!(
                    "ep9-test262: cannot read the skip list {}: {e}",
                    path.display()
                );
                return ExitCode::from(2);
            }
        };
        files.retain(|f| {
            let skip = list.contains(&f.rel);
            if skip {
                skipped.insert(f.rel.clone());
            }
            !skip
        });
        if full {
            for p in list.difference(&skipped) {
                eprintln!("ep9-test262: warning: skip list entry {p} names no test");
            }
        }
        if files.is_empty() {
            eprintln!("ep9-test262: the skip list leaves no test to run");
            return ExitCode::from(2);
        }
    }

    let worker_args: Vec<String> = vec![
        "--test262".into(),
        root.display().to_string(),
        "--timeout".into(),
        opts.timeout.as_millis().to_string(),
        "--gc-stress".into(),
        opts.gc_stress.to_string(),
        "--feedback-threshold".into(),
        opts.feedback_threshold.to_string(),
    ];
    // A test has at most 2 runs; each gets the soft timeout, then a margin. A worker that
    // gives no result by then is stuck where the interrupt check does not reach: kill it.
    let hard = opts.timeout * 2 + Duration::from_secs(5);
    let workers = Workers {
        subcommand: "exec-worker",
        args: &worker_args,
        threads: opts.threads.clamp(1, files.len()),
        // One test at a time: exec tests are slow enough, and the hard timeout is per test.
        chunk: 1,
        hard,
    };
    let threads = workers.threads;
    let results = in_workers(
        &workers,
        &files,
        |f| f.rel.clone(),
        |f, line| {
            decode(line)
                .filter(|(rel, _)| *rel == f.rel)
                .map(|(_, r)| r)
        },
        |_, g| TestResult::infra(g.cause, g.detail),
        |n| {
            if opts.progress && n.is_multiple_of(2000) {
                eprintln!(
                    "ep9-test262: {n}/{} tests, {:.0} s",
                    files.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        },
    );
    let results = match results {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ep9-test262: cannot start a worker process: {e}");
            return ExitCode::from(2);
        }
    };
    let results: Vec<(&str, TestResult)> = files
        .iter()
        .zip(results)
        .map(|(f, r)| (f.rel.as_str(), r))
        .collect();
    let wall = started.elapsed();

    // Counts.
    let mut by_dir: BTreeMap<String, (Count, Count)> = BTreeMap::new();
    let mut files_all = Count::default();
    let mut runs_all = Count::default();
    let mut causes: BTreeMap<&str, usize> = BTreeMap::new();
    let mut infra: Vec<(&str, &TestResult)> = Vec::new();
    for (rel, r) in &results {
        let entry = by_dir.entry(suite::top_dir(rel).to_string()).or_default();
        let passed = r.status == Status::Pass;
        entry.0.add(passed);
        entry.1.add_n(r.runs_ok, r.runs);
        files_all.add(passed);
        runs_all.add_n(r.runs_ok, r.runs);
        match r.status {
            Status::Pass => {}
            Status::Fail => *causes.entry(r.cause.as_str()).or_default() += 1,
            Status::Infra => infra.push((rel, r)),
        }
    }
    let mut top: Vec<(&str, usize)> = causes.iter().map(|(c, n)| (*c, *n)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    println!(
        "ep9-test262 exec: {} tests, {} worker processes, target {}, mode jit, timeout {:?}{}{}",
        results.len(),
        threads,
        opts.triple,
        opts.timeout,
        if opts.gc_stress > 0 {
            format!(", GC-stress {}", opts.gc_stress)
        } else {
            String::new()
        },
        if opts.feedback_threshold == IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD {
            String::new()
        } else {
            format!(", feedback threshold {}", opts.feedback_threshold)
        }
    );
    println!("  test262: {}", root.display());
    if let Some(path) = &opts.skip_list {
        println!(
            "  skipped: {} (listed in {}; not run, not counted)",
            skipped.len(),
            path.display()
        );
    }
    println!(
        "  {:<14}{:>8} / {:<8}{:>8}   {:>8} / {:<8}",
        "files", "passed", "total", "", "runs ok", "runs"
    );
    let line = |label: &str, f: Count, r: Count| {
        println!(
            "  {label:<14}{:>8} / {:<8}{:>7.2}%   {:>8} / {:<8}{:>7.2}%",
            f.passed,
            f.total,
            f.percent(),
            r.passed,
            r.total,
            r.percent()
        );
    };
    for (dir, (f, r)) in &by_dir {
        line(dir, *f, *r);
    }
    line("all", files_all, runs_all);
    println!(
        "  infrastructure errors: {} (panic {}, timeout {}, worker crash {}, other {})",
        infra.len(),
        infra.iter().filter(|(_, r)| r.cause == "panic").count(),
        infra.iter().filter(|(_, r)| r.cause == "timeout").count(),
        infra
            .iter()
            .filter(|(_, r)| r.cause == "worker crash")
            .count(),
        infra
            .iter()
            .filter(|(_, r)| !matches!(r.cause.as_str(), "panic" | "timeout" | "worker crash"))
            .count(),
    );
    println!(
        "  top {} failure causes (test files):",
        opts.causes.min(top.len())
    );
    for (cause, n) in top.iter().take(opts.causes) {
        println!("    {n:>7}  {cause}");
    }

    let failing: BTreeSet<String> = results
        .iter()
        .filter(|(_, r)| r.status == Status::Fail)
        .map(|(rel, _)| (*rel).to_string())
        .collect();
    let checked: BTreeSet<String> = results.iter().map(|(rel, _)| (*rel).to_string()).collect();

    let mut status = ExitCode::SUCCESS;
    let mut diff = expect::Diff::default();
    if infra.is_empty() {
        if opts.update {
            if let Err(e) = expect::write(&opts.expect, &header(opts.triple), &failing) {
                eprintln!("ep9-test262: cannot write {}: {e}", opts.expect.display());
                return ExitCode::from(2);
            }
        } else {
            match expect::read(&opts.expect) {
                Ok(expected) => {
                    let expected: BTreeSet<String> =
                        expected.difference(&skipped).cloned().collect();
                    diff = expect::diff(&expected, &failing, &checked, full);
                }
                Err(e) => {
                    eprintln!(
                        "ep9-test262: cannot read expectations {}: {e}\n  (create it with --update-expectations)",
                        opts.expect.display()
                    );
                    status = ExitCode::from(2);
                }
            }
        }
        if !diff.is_clean() && status == ExitCode::SUCCESS {
            status = ExitCode::from(1);
        }
        println!(
            "  failures: {} (new: {}, listed but passing: {}){}",
            failing.len(),
            diff.new_failures.len(),
            diff.now_passing.len(),
            if opts.update {
                format!("; wrote {}", opts.expect.display())
            } else {
                String::new()
            }
        );
        let details: BTreeMap<&str, &str> = results
            .iter()
            .map(|(rel, r)| (*rel, r.detail.as_str()))
            .collect();
        for path in &diff.new_failures {
            println!(
                "  NEW FAILURE {path}: {}",
                details.get(path.as_str()).copied().unwrap_or("")
            );
        }
        for path in &diff.now_passing {
            println!(
                "  NOW PASSING (remove from {}): {path}",
                opts.expect.display()
            );
        }
    } else {
        for (rel, r) in &infra {
            eprintln!("ep9-test262: {rel}: {}", r.detail);
        }
        eprintln!(
            "ep9-test262: {} tests hit an infrastructure error (panic, timeout or crash); \
             the expectations were not compared or written",
            infra.len()
        );
        status = ExitCode::from(2);
    }
    if opts.verbose {
        for (rel, r) in &results {
            if r.status == Status::Fail {
                println!("  failure {rel}: {}", r.detail);
            }
        }
    }
    println!("  wall time: {:.1} s", wall.as_secs_f64());

    if let Some(path) = &opts.json {
        let text = summary_json(
            opts, &by_dir, files_all, runs_all, &top, &results, &diff, wall, &skipped, source,
        );
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("ep9-test262: cannot write {}: {e}", path.display());
            return ExitCode::from(2);
        }
    }
    status
}

#[allow(clippy::too_many_arguments)]
fn summary_json(
    opts: &Options,
    by_dir: &BTreeMap<String, (Count, Count)>,
    files_all: Count,
    runs_all: Count,
    top: &[(&str, usize)],
    results: &[(&str, TestResult)],
    diff: &expect::Diff,
    wall: Duration,
    skipped: &BTreeSet<String>,
    source: provenance::Source,
) -> String {
    let count = |out: &mut String, c: Count| {
        let _ = write!(out, "{{\"passed\":{},\"total\":{}}}", c.passed, c.total);
    };
    let mut out = String::from("{\"command\":\"exec\",\"mode\":\"jit\",\"target\":");
    json::string(&mut out, opts.triple);
    out.push_str(",\"test262\":");
    json::string(&mut out, &opts.test262.display().to_string());
    // Where the run came from (`cargo xtask m1-exit` items 3 and 4: freshness and provenance):
    // the facts read before the run, dirty when a second read now differs (CX5-C1).
    let now = provenance::source(&crate::repo_root(), &provenance::process_env);
    provenance::write_fields(&mut out, &provenance::settle(source, &now));
    // The runner's build commit and the test262 checkout HEAD (C2, item 8; m1-exit X4).
    provenance::write_runner_fields(
        &mut out,
        provenance::runner_commit(),
        provenance::test262_head(&opts.test262).as_deref(),
    );
    let _ = write!(
        out,
        ",\"workers\":{},\"timeout_ms\":{},\"gc_stress\":{},\"feedback_threshold\":{},\"skipped\":{}",
        opts.threads,
        opts.timeout.as_millis(),
        opts.gc_stress,
        opts.feedback_threshold,
        skipped.len(),
    );
    // The skipped tests by path, sorted (`cargo xtask m0-exit` item 6 compares them with the
    // committed skip list); "skipped" stays their count.
    out.push_str(",\"skipped_paths\":");
    json::string_array(&mut out, &skipped.iter().collect::<Vec<_>>());
    // A run with path filters (a GC-stress shard) also names every file it ran, sorted:
    // `cargo xtask m0-exit` item 6 checks that each shard ran exactly the files of its shard
    // (M3-P3 fix round 2, C-G1). A full run runs every file of the checkout, so it leaves the
    // list out (it would add about 4 MB).
    if !opts.filters.is_empty() {
        out.push_str(",\"run_paths\":");
        json::string_array(
            &mut out,
            &results.iter().map(|(rel, _)| *rel).collect::<Vec<_>>(),
        );
    }
    // `cargo xtask m1-exit` item 4 reads these two (TOOLS F6, GC4 need 3): the S1 generational
    // heap (`EP9_GC_GENERATIONAL=1`, `isolate_config`) and how often the heap verifier runs
    // (after every collection under GC-stress, else never).
    let _ = write!(
        out,
        ",\"generational\":{},\"verify_cadence\":{}",
        generational_from_env(),
        u32::from(opts.gc_stress > 0),
    );
    let _ = write!(out, ",\"wall_seconds\":{:.3}", wall.as_secs_f64());
    out.push_str(",\"files\":");
    count(&mut out, files_all);
    out.push_str(",\"runs\":");
    count(&mut out, runs_all);
    out.push_str(",\"by_dir\":{");
    for (i, (dir, (f, r))) in by_dir.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json::string(&mut out, dir);
        out.push_str(":{\"files\":");
        count(&mut out, *f);
        out.push_str(",\"runs\":");
        count(&mut out, *r);
        out.push('}');
    }
    out.push_str("},\"causes\":[");
    for (i, (cause, n)) in top.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("\n{\"cause\":");
        json::string(&mut out, cause);
        let _ = write!(out, ",\"files\":{n}}}");
    }
    out.push_str("],\"results\":[");
    let mut first = true;
    for (rel, r) in results {
        if r.status == Status::Pass {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str("\n{\"path\":");
        json::string(&mut out, rel);
        out.push_str(",\"status\":");
        json::string(&mut out, r.status.name());
        out.push_str(",\"cause\":");
        json::string(&mut out, &r.cause);
        out.push_str(",\"detail\":");
        json::string(&mut out, &r.detail);
        out.push('}');
    }
    out.push_str("],\"expectations\":{\"file\":");
    json::string(&mut out, &opts.expect.display().to_string());
    out.push_str(",\"new_failures\":");
    json::string_array(&mut out, &diff.new_failures);
    out.push_str(",\"now_passing\":");
    json::string_array(&mut out, &diff.now_passing);
    out.push_str("}}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_round_trip() {
        let r = TestResult {
            status: Status::Fail,
            runs_ok: 1,
            runs: 2,
            cause: "Test262Error (an assertion failed)".into(),
            detail: "[strict] a\tb\nc \\ d".into(),
        };
        let line = encode("test/a b.js", &r);
        assert!(!line.contains('\n'));
        assert_eq!(decode(&line), Some(("test/a b.js".to_string(), r)));
        assert_eq!(decode("x\tbogus\t0\t0\t\t"), None);
    }

    #[test]
    fn a_reply_cut_by_the_end_of_the_output_is_dropped() {
        let (tx, rx) = mpsc::channel();
        forward_lines(&b"a\tb\nc\r\n\nd\te"[..], &tx);
        drop(tx);
        assert_eq!(rx.iter().collect::<Vec<_>>(), ["a\tb", "c", ""]);
        let (tx, rx) = mpsc::channel();
        forward_lines(&b"ok\n\xff\nlost\n"[..], &tx);
        drop(tx);
        assert_eq!(rx.iter().collect::<Vec<_>>(), ["ok"]);
    }

    #[test]
    fn causes_group_messages() {
        assert_eq!(
            cause_of_message("Error: not implemented in ep9 v0: GetIterator"),
            "not implemented: GetIterator"
        );
        assert_eq!(
            cause_of_message("Test262Error: Expected SameValue(«1», «2») to be true"),
            "Test262Error (an assertion failed)"
        );
        assert_eq!(
            cause_of_message("ReferenceError: Symbol is not defined"),
            "ReferenceError: Symbol is not defined"
        );
        assert_eq!(cause_of_message(""), "(empty message)");
    }

    #[test]
    fn callee_names_from_source() {
        let src = "var a;\nvar obj = Object.create(null);\n  x.y.z(1);";
        assert_eq!(callee_at(src, 2, 18).as_deref(), Some("Object.create"));
        assert_eq!(callee_at(src, 2, 11).as_deref(), Some("Object"));
        assert_eq!(callee_at(src, 3, 7).as_deref(), Some("x.y.z"));
        assert_eq!(callee_at("/*😀*/f()", 1, 7).as_deref(), Some("f"));
        assert_eq!(callee_at(src, 9, 1), None);
    }

    fn exception(message: &str, constructor: Option<&str>) -> Exception {
        Exception {
            message: message.into(),
            constructor: constructor.map(Into::into),
            stack: None,
            location: None,
            terminated: false,
        }
    }

    #[test]
    fn negative_types_match_the_constructor() {
        let matches = |constructor, ty| exception_matches(&exception("text", constructor), ty);
        assert!(matches(Some("TypeError"), "TypeError"));
        assert!(matches(Some("Test262Error"), "Test262Error"));
        assert!(!matches(Some("RangeError"), "TypeError"));
        assert!(!matches(Some("TypeErrorX"), "TypeError"));
        // The text does not count: a thrown string, or an object whose ToString looks right.
        assert!(!exception_matches(
            &exception("TypeError: x", None),
            "TypeError"
        ));
        assert!(!exception_matches(
            &exception("TypeError: x", Some("Object")),
            "TypeError"
        ));
    }

    #[test]
    fn async_failure_beats_completion() {
        let pass = |out: &str| matches!(async_outcome(out), Run::Pass);
        assert!(pass("Test262:AsyncTestComplete\n"));
        assert!(pass("debug line\nTest262:AsyncTestComplete\n"));
        assert!(!pass(""));
        assert!(!pass("Test262:AsyncTestFailure:Test262Error: x\n"));
        assert!(!pass(
            "Test262:AsyncTestFailure:Error: x\nTest262:AsyncTestComplete\n"
        ));
        assert!(!pass(
            "Test262:AsyncTestComplete\nTest262:AsyncTestFailure:Error: late\n"
        ));
        assert!(!pass(
            "Test262:AsyncTestComplete\nTest262:AsyncTestComplete\n"
        ));
        let Run::Fail { cause, .. } = async_outcome("Test262:AsyncTestFailure:Test262Error: x\n")
        else {
            panic!("not a failure");
        };
        assert_eq!(cause, "Test262Error (an assertion failed)");
    }

    #[test]
    fn exec_needs_a_builtin_table() {
        let config = isolate_config(0, IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD);
        assert!(config.builtins.is_some());
        assert_eq!(check_builtins(config), Ok(()));
        let err = check_builtins(IsolateConfig::default()).unwrap_err();
        assert!(err.contains("no builtin table is installed"), "{err}");
    }

    #[test]
    fn watchdog_remembers_an_expired_deadline() {
        // The isolate never polls here, as in a native loop: the expiry must still show.
        let iso = Isolate::new(IsolateConfig::default()).unwrap();
        let w = Watchdog::start();
        w.arm(iso.terminate_handle(), Duration::from_secs(60));
        assert!(!w.disarm(), "in time");
        // Past the deadline before the watchdog thread looks (it looks every 20 ms).
        w.arm(iso.terminate_handle(), Duration::ZERO);
        assert!(w.disarm(), "late, seen at disarm");
        // The watchdog thread fires first: it disarms and records the expiry.
        w.arm(iso.terminate_handle(), Duration::from_millis(1));
        let waited = Instant::now();
        while w.slot.lock().unwrap().armed.is_some() {
            assert!(
                waited.elapsed() < Duration::from_secs(10),
                "the watchdog never fired"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(w.slot.lock().unwrap().expired);
        assert!(w.disarm(), "late, fired by the watchdog");
        // Taken: no stale expiry for a later run that compiles but is never armed.
        assert!(!w.disarm());
    }
}
