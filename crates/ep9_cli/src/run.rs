//! Run one script: compile with ep9_frontend, run in a fresh isolate, report the result.

use std::io::Write as _;

use ep9_frontend::{CodegenError, CompileError, CompileOptions, Position, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Args, Exception, HostCx, Isolate, IsolateConfig, JsResult, Value};

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
fn print(cx: &mut HostCx<'_>, _this: Value, args: Args<'_>, _nt: Value) -> JsResult<Value> {
    let mut line = String::new();
    for (i, v) in args.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(&cx.to_rust_string(v)?);
    }
    line.push('\n');
    let mut out = std::io::stdout().lock();
    // A closed stdout (for example `ep9 x.js | head -1`) is not a JS error.
    let _ = out.write_all(line.as_bytes());
    Ok(Value::UNDEFINED)
}

/// `gc()`: a full collection now.
fn gc(cx: &mut HostCx<'_>, _this: Value, _args: Args<'_>, _nt: Value) -> JsResult<Value> {
    cx.collect_garbage();
    Ok(Value::UNDEFINED)
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
    let result = isolate.run_script(unit, file);
    let _ = std::io::stdout().flush();
    let status = match result {
        Ok(_) => EXIT_OK,
        Err(e) => {
            eprintln!("{}", uncaught_text(&e));
            EXIT_JS_ERROR
        }
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
    let worker = std::thread::Builder::new()
        .name("ep9-isolate".into())
        .stack_size(stack)
        .spawn(move || run_on_this_thread(&opts, &file, &source));
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(status)) => status,
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
