//! Run one script: compile with ep9_frontend, run in a fresh isolate, report the result.

use std::io::Write as _;

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, Position, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Exception, Isolate, IsolateConfig,
    JsResult, Local,
};

use crate::args::{Options, Source};

/// Exit status of a run (see the usage text).
pub const EXIT_OK: u8 = 0;
pub const EXIT_JS_ERROR: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
/// The engine panicked: an ep9 bug, never a JS result.
pub const EXIT_INTERNAL: u8 = 70;

/// The file name of `-e` code in locations and stack traces (as node prints it).
pub const EVAL_NAME: &str = "[eval]";

/// `print(...args)`: ToString of each argument, joined by a space, then a newline (d8, jsc).
/// A symbol prints as its descriptive string, `Symbol(desc)`, as node's console.log and qjs
/// print it (ToString of a symbol throws; d8 prints only the description).
fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let line = print_line(cx, args.iter())?;
    let mut out = std::io::stdout().lock();
    // A closed stdout (for example `ep9 x.js | head -1`) is not a JS error.
    let _ = out.write_all(line.as_bytes());
    Ok(Local::undefined())
}

/// The line `print` writes for `args`, with its newline.
fn print_line<'s>(cx: &mut Cx<'s>, args: impl Iterator<Item = Local<'s>>) -> JsResult<String> {
    let mut line = String::new();
    for (i, v) in args.enumerate() {
        if i > 0 {
            line.push(' ');
        }
        let text = match v.as_symbol() {
            Some(s) => {
                let d = cx.descriptive_string(s)?;
                cx.to_rust_string(d)?
            }
            None => cx.to_rust_string(v)?,
        };
        line.push_str(&text);
    }
    line.push('\n');
    Ok(line)
}

/// `gc()`: a full collection now.
fn gc<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.collect_garbage();
    Ok(Local::undefined())
}

/// `file:line:col: ` for a compile error position (1-based line, 1-based UTF-16 column).
fn at(file: &str, p: Option<Position>) -> String {
    match p {
        Some(p) => format!("{file}:{}:{}: ", p.line, p.column),
        None => format!("{file}: "),
    }
}

/// The text of a compile error, with the file name in front of the position.
pub fn compile_error_text(file: &str, e: &CodegenError) -> String {
    match e {
        CodegenError::Compile(CompileError::Syntax(errors)) => {
            let first = errors.first();
            format!("{}SyntaxError: {}", at(file, first.position), first.message)
        }
        CodegenError::Compile(CompileError::Range(r)) => {
            format!("{}RangeError: {}", at(file, r.position), r.message)
        }
        CodegenError::Unsupported(u) => format!(
            "{}SyntaxError: ep9 BytecodeGen v0 does not support {} yet",
            at(file, Some(u.position)),
            u.feature
        ),
        CodegenError::Limit(m) => format!("{}RangeError: {m}", at(file, None)),
        CodegenError::Internal(m) => format!("{}internal BytecodeGen error: {m}", at(file, None)),
    }
}

/// The report of an uncaught exception: `file:line:col: Uncaught Name: message`, then the
/// `at` lines of the error's stack.
pub fn uncaught_text(e: &Exception) -> String {
    let mut s = match &e.location {
        Some(l) => format!("{l}: Uncaught {}", e.message),
        None => format!("Uncaught {}", e.message),
    };
    if let Some(stack) = &e.stack {
        for line in stack.lines().filter(|l| l.trim_start().starts_with("at ")) {
            s.push('\n');
            s.push_str(line);
        }
    }
    s
}

fn run_on_this_thread(opts: &Options, file: &str, source: &str) -> u8 {
    // Test hook (crates/ep9_cli/tests/cli.rs): a panic on the isolate thread.
    if std::env::var_os("EP9_CLI_FAULT").is_some_and(|v| v == "panic") {
        panic!("injected panic (EP9_CLI_FAULT=panic)");
    }
    let source_opts = if opts.module {
        SourceOpts::module()
    } else if opts.strict {
        SourceOpts::script(Strictness::Strict)
    } else {
        SourceOpts::script(Strictness::Sloppy)
    };
    let unit = match ep9_frontend::compile(source, source_opts, CompileOptions::default()) {
        Ok(unit) => unit,
        Err(e) => {
            eprintln!("{}", compile_error_text(file, &e));
            return if matches!(e, CodegenError::Internal(_)) {
                EXIT_INTERNAL
            } else {
                EXIT_JS_ERROR
            };
        }
    };
    if opts.print_bytecode {
        println!("{}", ep9_bytecode::disassemble(&unit));
    }
    let config = IsolateConfig {
        heap: HeapConfig {
            gc_stress: opts.gc_stress,
            verify_after_gc: opts.verify_heap,
            ..HeapConfig::default()
        },
        dispatch: opts.dispatch,
        feedback_threshold: opts.feedback_threshold,
        builtins: Some(&ep9_builtins::TABLE),
        compiler: Some(&COMPILER),
        ..IsolateConfig::default()
    };
    let mut isolate = match Isolate::new(config) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("ep9: cannot create the isolate: {e}");
            return EXIT_INTERNAL;
        }
    };
    isolate.register_host_function("print", 1, print);
    isolate.register_host_function("gc", 0, gc);
    // `with_cx_checkpoint` drains the microtask queue (promise jobs) before it returns; a
    // module run does the same (`run_module`).
    let result = if opts.module {
        run_module(&mut isolate, unit, file)
    } else {
        isolate.with_cx_checkpoint(|cx| cx.run_script(unit, file).map(|_| ()))
    };
    let _ = std::io::stdout().flush();
    let status = match result {
        Ok(_) => EXIT_OK,
        Err(e) => {
            eprintln!("{}", uncaught_text(&e));
            EXIT_JS_ERROR
        }
    };
    // A promise rejected with no handler after the final checkpoint is reported like an
    // uncaught exception, and the run exits 1 (node's default `--unhandled-rejections=throw`
    // reports the first one). An uncaught exception of the script itself comes first.
    let rejections = isolate.take_unhandled_rejections();
    let status = match rejections.first() {
        Some(e) if status == EXIT_OK => {
            let _ = std::io::stdout().flush();
            eprintln!("{}", uncaught_text(e));
            EXIT_JS_ERROR
        }
        _ => status,
    };
    if opts.gc_stats {
        let st = isolate.heap().stats();
        eprintln!(
            "ep9: gc collections={} freed_cells={} allocated_bytes={}",
            st.collections, st.freed_cells, st.allocated_bytes
        );
    }
    if opts.verify_heap {
        isolate.collect_garbage();
        if let Err(e) = isolate.verify_heap() {
            eprintln!("ep9: heap verifier failed after the run:");
            for line in &e.errors {
                eprintln!("  {line}");
            }
            return EXIT_INTERNAL;
        }
    }
    status
}

// ---- the file loader for `--module` (M3-P2 MODULE) ------------------------------------------

/// Runs the module `unit` (the file `file`) with the modules it imports, then a microtask
/// checkpoint (`Isolate::with_cx_checkpoint`): the module's error wins over a job's, but not
/// over a termination in a job. The module is named by its canonical path (so an import of
/// the same file is the same module); `-e` code is named [`EVAL_NAME`].
fn run_module(
    isolate: &mut Isolate,
    unit: ep9_bytecode::VerifiedUnit,
    file: &str,
) -> Result<(), Exception> {
    let name = if file == EVAL_NAME {
        file.to_string()
    } else {
        std::fs::canonicalize(file).map_or_else(|_| file.to_string(), |p| p.display().to_string())
    };
    isolate.with_cx_checkpoint(|cx| {
        cx.run_module(unit, &name, &mut resolve_module, &mut load_module)
            .map(|_| ())
    })
}

/// The name of the module that `specifier` names in the module `referrer` (a file path, or
/// [`EVAL_NAME`] for the current directory): a relative (`./`, `../`) or absolute path,
/// canonical. A bare specifier (a package name) is an error: the CLI has no package lookup
/// yet (M4 runtime).
fn resolve_module(referrer: &str, specifier: &str) -> Result<String, String> {
    let relative = specifier.starts_with("./") || specifier.starts_with("../");
    if !relative && !specifier.starts_with('/') {
        return Err(format!(
            "Cannot find module '{specifier}' imported from {referrer} (only relative and absolute paths are supported)"
        ));
    }
    let base = if referrer == EVAL_NAME {
        std::path::PathBuf::from(".")
    } else {
        std::path::Path::new(referrer).parent().map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        )
    };
    let path = base.join(specifier);
    std::fs::canonicalize(&path)
        .map(|p| p.display().to_string())
        .map_err(|e| format!("Cannot find module '{specifier}' imported from {referrer}: {e}"))
}

/// The module unit of the file `name` (a canonical path).
fn load_module(name: &str) -> CompileResult {
    let source = match std::fs::read(name).map(String::from_utf8) {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => {
            return Err(CompileFailure::Syntax(format!(
                "{name}: the file is not valid UTF-8"
            )));
        }
        Err(e) => return Err(CompileFailure::Internal(format!("cannot read {name}: {e}"))),
    };
    ep9_frontend::compile(&source, SourceOpts::module(), CompileOptions::default())
        .map_err(hook_failure)
}

// ---- the compiler hooks (C2): ep9_frontend behind `IsolateConfig::compiler` ----------------

/// The compiler entry points of the CLI's isolates: ep9_vm does not link the front end, so eval,
/// `Function()` and run-time scripts reach it through these. Eval code (`compile_eval`, M2-P2)
/// and the `Function()` family (`compile_function_ctor`, P3-FNCTOR; the async kinds answer
/// `Unsupported`) compile.
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

/// The panic hook of the `ep9` binary. A panic on any thread is an ep9 bug: print one line and
/// exit [`EXIT_INTERNAL`] at once. So the exit status is 70 whether the build unwinds or
/// aborts on a panic (`panic = "abort"`, P2-INFRA2, would give 134 after the default hook).
/// `RUST_BACKTRACE=1` adds the backtrace.
fn internal_error(info: &std::panic::PanicHookInfo<'_>) {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(ToString::to_string)
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".into());
    // Write errors are ignored: a panic inside the hook would abort.
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "ep9: internal error: {msg} (this is an ep9 bug)");
    if let Some(at) = info.location() {
        let thread = std::thread::current();
        let _ = writeln!(
            err,
            "ep9: panicked at {at} on thread '{}'",
            thread.name().unwrap_or("?")
        );
    }
    let trace = std::backtrace::Backtrace::capture();
    if trace.status() == std::backtrace::BacktraceStatus::Captured {
        let _ = writeln!(err, "{trace}");
    }
    drop(err);
    // Flushes stdout (what the script printed so far), then ends every thread.
    std::process::exit(i32::from(EXIT_INTERNAL));
}

/// Runs the script of `opts` on a new thread with a `stack_mib` MiB stack. Returns the exit status.
pub fn main(opts: Options) -> u8 {
    let (file, source) = match &opts.source {
        Source::Eval(code) => (EVAL_NAME.to_string(), code.clone()),
        Source::File(path) => match std::fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => (path.display().to_string(), s),
                Err(_) => {
                    eprintln!("ep9: {}: the file is not valid UTF-8", path.display());
                    return EXIT_USAGE;
                }
            },
            Err(e) => {
                eprintln!("ep9: cannot read {}: {e}", path.display());
                return EXIT_USAGE;
            }
        },
    };
    let stack = opts.stack_mib << 20;
    std::panic::set_hook(Box::new(internal_error));
    let worker = std::thread::Builder::new()
        .name("ep9-isolate".into())
        .stack_size(stack)
        .spawn(move || run_on_this_thread(&opts, &file, &source));
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(status)) => status,
        // Not reached while the hook exits first; kept in case a panic skips the hook.
        Ok(Err(_)) => {
            eprintln!("ep9: internal error: the engine panicked (this is an ep9 bug)");
            EXIT_INTERNAL
        }
        Err(e) => {
            eprintln!("ep9: cannot start the isolate thread: {e}");
            EXIT_INTERNAL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_shows_a_symbol_as_its_descriptive_string() {
        let mut isolate = Isolate::new(IsolateConfig {
            builtins: Some(&ep9_builtins::TABLE),
            ..IsolateConfig::default()
        })
        .unwrap();
        let line = isolate.with_cx(|cx| {
            let d = cx.new_string("x")?;
            let x = cx.new_symbol(d)?;
            let none = cx.new_symbol(Local::undefined())?;
            let args: [Local<'_>; 4] = [x.into(), none.into(), d.into(), Local::int32(1)];
            print_line(cx, args.into_iter())
        });
        assert_eq!(line.unwrap(), "Symbol(x) Symbol() x 1\n");
    }

    /// The hooks reach the front end: a script, eval code (P2-EVAL-FE) and `Function()` code
    /// (P3-FNCTOR) compile, an early error is a `Syntax` failure, and an async function is
    /// `Unsupported`.
    #[test]
    fn the_compiler_hooks_reach_the_front_end() {
        let iso = Isolate::new(IsolateConfig {
            compiler: Some(&COMPILER),
            ..IsolateConfig::default()
        })
        .unwrap();
        let hooks = iso.compiler().unwrap();
        assert!((hooks.compile_script)("var a = 1;").is_ok());
        // An unsupported feature (M3-P3 IMPORT-TLA builds import()).
        assert_eq!(
            (hooks.compile_script)("import('x')").unwrap_err(),
            CompileFailure::Unsupported("import()".to_string())
        );
        assert!(matches!(
            (hooks.compile_script)("let a; let a;"),
            Err(CompileFailure::Syntax(_))
        ));
        let ctx = EvalContext::default();
        assert!((hooks.compile_eval)("1", &ctx).is_ok());
        assert!(matches!(
            (hooks.compile_eval)("return 1", &ctx),
            Err(CompileFailure::Syntax(_))
        ));
        assert!((hooks.compile_function)(FunctionCtorKind::Normal, "a", "return a").is_ok());
        assert!(matches!(
            (hooks.compile_function)(FunctionCtorKind::Normal, "/*", "*/){"),
            Err(CompileFailure::Syntax(_))
        ));
        assert!((hooks.compile_function)(FunctionCtorKind::Async, "a", "return await a").is_ok());
        assert!(
            (hooks.compile_function)(FunctionCtorKind::AsyncGenerator, "", "yield await 1").is_ok()
        );
        assert!(
            Isolate::new(IsolateConfig::default())
                .unwrap()
                .compiler()
                .is_none()
        );
    }
}
