//! `ep9 --reprl`: the Fuzzilli REPRL loop (read-eval-print-reset-loop, plan v9 §5.5, GFF-REPRL).
//!
//! Fuzzilli (Google Project Zero's JavaScript fuzzer) starts `ep9 --reprl` as a child, with four
//! fixed file descriptors, and runs many scripts in it. The protocol is the one of Fuzzilli's
//! `Sources/libreprl/libreprl-posix.c` and `Targets/README.md` at the pinned commit
//! (`third_party/fuzzilli.commit`):
//!
//! ```text
//!  Fuzzilli (parent, libreprl)                      ep9 --reprl (child)
//!  ───────────────────────────                      ───────────────────
//!  fd 100 (control, to child)    <── "HELO" ──      fd 101 (control, to parent)
//!                                ── "HELO" ──>
//!  per script:
//!    script bytes into the data file (child fd 102), file offset 0
//!                                ── "exec" + u64 size ──>
//!                                                   read size bytes from fd 102
//!                                                   a fresh isolate: compile, run, microtasks,
//!                                                   drop; flush stdout and stderr
//!                                <── status word ──  (status & 0xff) << 8: 0 ok, 0x100 JS error
//!                                                   reset the sancov edge guards
//!    fuzzilli('FUZZILLI_PRINT', s) writes s + "\n" to fd 103 (the parent reads it after the run)
//!
//!  endings with no status word (the parent reads EOF on fd 101, then waitpid):
//!    fuzzilli('FUZZILLI_CRASH', 0)   a panic -> the REPRL panic hook -> abort   SIGABRT   crash
//!    fuzzilli('FUZZILLI_CRASH', 1)   a store to address 0x41414141              SIGSEGV   crash
//!    fuzzilli('FUZZILLI_CRASH', 2)   a trap instruction (ud2 / brk)   SIGILL x86-64, SIGTRAP arm64
//!    any panic of the engine         the same hook                              SIGABRT   crash
//!    GC heap at its limit            near-limit callback: exit 71       "failed", not a crash
//!    a timeout                       the parent kills the child (SIGKILL) and counts a timeout
//! ```
//!
//! Out of memory (P3-ORCH-5, replaces the V9-A11 status-word text): H23 stays as it is (out of
//! memory is fatal after the near-limit callback). The REPRL host sets `HeapConfig::near_limit`
//! to a callback that exits the child with [`unix::EXIT_OUT_OF_MEMORY`] and writes no status word.
//! libreprl then reads EOF, waits for the child and returns `71 << 8`, which Fuzzilli counts as a
//! failed run (not a crash, no retry delay) and starts a fresh child for the next script.
//!
//! Settings come from the environment (main.rs is frozen and passes no arguments):
//! `EP9_REPRL_HEAP_MB` (GC heap limit, default 512), `EP9_REPRL_STACK_MB` (native stack of the
//! isolate thread, default 64, as `--stack-size`), `EP9_REPRL_GC_STRESS` (0 = off, n = collect
//! before every n-th allocation and verify the heap after each collection, as `--gc-stress=n`).
//! The isolate config is the CLI's: no fuzz-only `IsolateConfig` field (doc 13, MC-w14).
//!
//! Coverage: built with SanitizerCoverage (`infra/fuzz/build-ep9.sh`: stable rustc,
//! `-Cpasses=sancov-module` with trace-pc-guard), every edge calls
//! `__sanitizer_cov_trace_pc_guard`, defined here as in Fuzzilli's `Targets/coverage.c`: the
//! edge sets its bit in the shared memory `SHM_ID` (2 MiB) and turns its guard off until the
//! next reset. In a build without the flags nothing calls them.
//!
//! This is the one module of ep9_cli that the main.rs lint allows to use `unsafe`: fixed fd
//! numbers, the shared memory map, the exported coverage callbacks and the injected faults.
//! Windows has no REPRL ([`main`] prints a message and exits 2).

/// The REPRL entry (`ep9 --reprl`). Returns the process exit status: 0 when the parent closes
/// the control channel, 2 for a bad setting or a broken protocol.
pub fn main() -> u8 {
    #[cfg(unix)]
    {
        unix::main()
    }
    #[cfg(not(unix))]
    {
        eprintln!("ep9 --reprl: the Fuzzilli REPRL mode needs a unix target");
        crate::run::EXIT_USAGE
    }
}

#[cfg(unix)]
mod unix {
    //! The REPRL child (unix only): the settings, one fresh isolate per script, the `fuzzilli`
    //! builtin and its injected faults, and the loop on fds 100-103.

    use std::fs::File;
    use std::io::{ErrorKind, Read as _, Write as _};
    use std::os::fd::{FromRawFd as _, RawFd};
    use std::sync::OnceLock;

    use ep9_bytecode::{EvalContext, FunctionCtorKind};
    use ep9_frontend::{CodegenError, CompileOptions, SourceOpts, Strictness};
    use ep9_heap::HeapConfig;
    use ep9_vm::{
        Args, CompileFailure, CompileResult, CompilerHooks, Cx, Isolate, IsolateConfig, JsResult,
        Local,
    };

    use super::sancov;
    use crate::run;
    use crate::run::EXIT_USAGE;

    /// The exit status of the child when the GC heap reaches its limit (the near-limit callback).
    /// The parent sees "exited with 71", not a signal: a failed run, never a crash.
    pub const EXIT_OUT_OF_MEMORY: u8 = 71;

    /// The file name of a REPRL script in locations and stack traces.
    const SCRIPT_NAME: &str = "reprl";

    /// Largest script: libreprl's `REPRL_MAX_DATA_SIZE` (16 MiB).
    const MAX_SCRIPT_BYTES: u64 = 16 << 20;

    /// What `fuzzilli('FUZZILLI_PRINT', s)` writes to: the REPRL data channel to the parent (fd 103)
    /// once the handshake is done, stdout before (as the QJS target does without the channel).
    static FUZZ_OUT: OnceLock<std::fs::File> = OnceLock::new();

    /// The settings of a REPRL child, from the environment (see the module doc).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Settings {
        heap_bytes: usize,
        stack_bytes: usize,
        gc_stress: u32,
    }

    impl Settings {
        fn from_env() -> Result<Settings, String> {
            fn number(
                name: &str,
                default: u32,
                range: std::ops::RangeInclusive<u32>,
            ) -> Result<u32, String> {
                match std::env::var(name) {
                    Err(std::env::VarError::NotPresent) => Ok(default),
                    Ok(v) => v
                        .parse::<u32>()
                        .ok()
                        .filter(|n| range.contains(n))
                        .ok_or_else(|| {
                            format!(
                                "bad {name} value {v:?} ({}..={})",
                                range.start(),
                                range.end()
                            )
                        }),
                    Err(e) => Err(format!("bad {name} value: {e}")),
                }
            }
            Ok(Settings {
                heap_bytes: (number("EP9_REPRL_HEAP_MB", 512, 8..=65536)? as usize) << 20,
                stack_bytes: (number("EP9_REPRL_STACK_MB", 64, 1..=4096)? as usize) << 20,
                gc_stress: number("EP9_REPRL_GC_STRESS", 0, 0..=u32::MAX)?,
            })
        }
    }

    /// The near-limit callback of every REPRL isolate (`HeapConfig::near_limit`, H23): the GC heap
    /// is at its limit after a full collection. Exits the child at once with
    /// [`EXIT_OUT_OF_MEMORY`] and no status word. Touches no heap and no isolate.
    fn out_of_memory(committed: usize, limit: usize) -> usize {
        let _ = std::io::stdout().flush();
        eprintln!(
            "ep9 --reprl: out of memory: {committed} bytes committed, limit {limit} (exit {EXIT_OUT_OF_MEMORY})"
        );
        std::process::exit(i32::from(EXIT_OUT_OF_MEMORY))
    }

    /// The panic hook of a REPRL child: a panic is an ep9 bug, so the child must die by a signal
    /// for Fuzzilli to count a crash (run.rs's hook exits 70, which Fuzzilli counts as a failed
    /// run). Prints the same first lines as run.rs's hook (the triage script keys on them), the
    /// backtrace when `RUST_BACKTRACE` asks for one, then aborts (SIGABRT).
    fn abort_on_panic(info: &std::panic::PanicHookInfo<'_>) {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(ToString::to_string)
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic without a message".into());
        let _ = std::io::stdout().flush();
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
        std::process::abort();
    }

    /// The compiler hooks of every REPRL isolate: run.rs's hooks (eval, `Function()`, run-time
    /// scripts), except that an internal compiler error panics, a crash for Fuzzilli (SIGABRT
    /// through the REPRL panic hook), as on the first compile in [`execute`]. With run.rs's
    /// hooks the VM turns [`CompileFailure::Internal`] into a catchable `Error`, so a script
    /// that catches it (or a failed run) would hide the compiler bug. `ep9 <file>` keeps
    /// run.rs's hooks.
    static COMPILER: CompilerHooks = CompilerHooks {
        compile_eval,
        compile_function,
        compile_script,
    };

    fn compile_eval(source: &str, ctx: &EvalContext) -> CompileResult {
        crash_on_internal(
            test_fault(&[source]).unwrap_or_else(|| (run::COMPILER.compile_eval)(source, ctx)),
        )
    }

    fn compile_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
        crash_on_internal(
            test_fault(&[params, body])
                .unwrap_or_else(|| (run::COMPILER.compile_function)(kind, params, body)),
        )
    }

    fn compile_script(source: &str) -> CompileResult {
        crash_on_internal(
            test_fault(&[source]).unwrap_or_else(|| (run::COMPILER.compile_script)(source)),
        )
    }

    /// An internal compiler error is an ep9 bug: panic. Every other result goes to the VM.
    fn crash_on_internal(result: CompileResult) -> CompileResult {
        if let Err(CompileFailure::Internal(m)) = &result {
            panic!("internal BytecodeGen error: {m}");
        }
        result
    }

    /// The test-only fault: a run-time source text that contains this fails as if the front end
    /// had returned an internal error (the front end has no input that does it on purpose).
    #[cfg(test)]
    pub(super) const TEST_INTERNAL_FAULT: &str = "ep9-reprl-test-internal-compile-error";

    #[cfg(test)]
    fn test_fault(texts: &[&str]) -> Option<CompileResult> {
        texts
            .iter()
            .any(|t| t.contains(TEST_INTERNAL_FAULT))
            .then(|| {
                Err(CompileFailure::Internal(
                    "injected by the reprl tests".into(),
                ))
            })
    }

    /// No fault outside the tests.
    #[cfg(not(test))]
    fn test_fault(_: &[&str]) -> Option<CompileResult> {
        None
    }

    /// Runs one REPRL script in a fresh isolate and returns its status: 0, or
    /// [`run::EXIT_JS_ERROR`] for a compile error, an uncaught exception or an unhandled
    /// rejection. An internal BytecodeGen error is an ep9 bug: it panics (a crash for Fuzzilli).
    fn execute(settings: &Settings, source: &str) -> u8 {
        let opts = SourceOpts::script(Strictness::Sloppy);
        let unit = match ep9_frontend::compile(source, opts, CompileOptions::default()) {
            Ok(unit) => unit,
            Err(CodegenError::Internal(m)) => panic!("internal BytecodeGen error: {m}"),
            Err(e) => {
                eprintln!("{}", run::compile_error_text(SCRIPT_NAME, &e));
                return run::EXIT_JS_ERROR;
            }
        };
        let config = IsolateConfig {
            heap: HeapConfig {
                max_heap_bytes: settings.heap_bytes,
                near_limit: Some(out_of_memory),
                gc_stress: settings.gc_stress,
                verify_after_gc: settings.gc_stress > 0,
                ..HeapConfig::default()
            },
            builtins: Some(&ep9_builtins::TABLE),
            compiler: Some(&COMPILER),
            ..IsolateConfig::default()
        };
        let mut isolate = match Isolate::new(config) {
            Ok(i) => i,
            Err(e) => panic!("cannot create the isolate: {e}"),
        };
        isolate.register_host_function("print", 1, run::print);
        isolate.register_host_function("gc", 0, run::gc);
        isolate.register_host_function("fuzzilli", 2, fuzzilli);
        let result = isolate.with_cx_checkpoint(|cx| cx.run_script(unit, SCRIPT_NAME).map(|_| ()));
        let _ = std::io::stdout().flush();
        let status = match result {
            Ok(()) => run::EXIT_OK,
            Err(e) => {
                eprintln!("{}", run::uncaught_text(&e));
                run::EXIT_JS_ERROR
            }
        };
        // As run.rs: a promise rejected with no handler fails the run (Fuzzilli's REPRLRun checks it).
        let rejections = isolate.take_unhandled_rejections();
        match rejections.first() {
            Some(e) if status == run::EXIT_OK => {
                eprintln!("{}", run::uncaught_text(e));
                run::EXIT_JS_ERROR
            }
            _ => status,
        }
    }

    /// `fuzzilli(op, arg)`, the Fuzzilli builtin (Targets/README.md "Adding Custom fuzzilli
    /// Javascript Builtin"): `fuzzilli('FUZZILLI_CRASH', n)` kills the process in way `n` (0 panic,
    /// 1 wild store, 2 trap; other `n`: nothing), `fuzzilli('FUZZILLI_PRINT', s)` writes `s` and a
    /// newline to the REPRL data channel. Any other call does nothing. Returns undefined.
    fn fuzzilli<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        let op = cx.to_rust_string(args.get(0))?;
        match op.as_str() {
            "FUZZILLI_CRASH" => {
                let kind = cx.to_int32(args.get(1))?;
                crash(kind);
            }
            "FUZZILLI_PRINT" if args.len() > 1 => {
                let mut line = cx.to_rust_string(args.get(1))?;
                line.push('\n');
                // A closed channel is not a JS error.
                let _ = match FUZZ_OUT.get() {
                    Some(mut f) => f.write_all(line.as_bytes()),
                    None => std::io::stdout().lock().write_all(line.as_bytes()),
                };
            }
            _ => {}
        }
        Ok(Local::undefined())
    }

    /// The injected faults of `fuzzilli('FUZZILLI_CRASH', kind)`: one for each way ep9 can die.
    fn crash(kind: i32) {
        match kind {
            // A Rust panic: what an engine bug (assert, unwrap, bounds check) looks like.
            0 => panic!("fuzzilli('FUZZILLI_CRASH', 0): injected panic"),
            // A store through a wild pointer: what a memory-safety bug in unsafe code looks like.
            1 => {
                // A byte store: a wider type would fail the debug-build alignment check of
                // `write_volatile` (a panic, SIGABRT) before the store.
                let wild = 0x4141_4141_usize as *mut u8;
                // SAFETY: none, on purpose: the store faults (SIGSEGV), which is the test. The
                // address is unmapped: macOS keeps the low 4 GiB as __PAGEZERO, and Linux maps
                // nothing at 1 GiB in a 64-bit process (the address Fuzzilli's C targets use).
                unsafe { wild.write_volatile(0x37) };
                std::process::abort();
            }
            // A trap instruction: what a JIT guard trap (the Spike C exit stubs) looks like.
            2 => trap(),
            _ => {}
        }
    }

    /// Executes a trap instruction: `ud2` (SIGILL) on x86-64.
    #[cfg(target_arch = "x86_64")]
    fn trap() -> ! {
        // SAFETY: `ud2` touches no memory and raises SIGILL; it never returns.
        unsafe { core::arch::asm!("ud2", options(noreturn, nomem, nostack)) }
    }

    /// Executes a trap instruction: `brk` (SIGTRAP) on arm64.
    #[cfg(target_arch = "aarch64")]
    fn trap() -> ! {
        // SAFETY: `brk` touches no memory and raises SIGTRAP; it never returns.
        unsafe { core::arch::asm!("brk #0x1", options(noreturn, nomem, nostack)) }
    }

    /// No trap instruction here: abort (SIGABRT).
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn trap() -> ! {
        std::process::abort()
    }

    /// Control channel from the parent ("HELO", "exec" + size).
    const CTRL_IN: RawFd = 100;
    /// Control channel to the parent ("HELO", status words).
    const CTRL_OUT: RawFd = 101;
    /// Data channel from the parent (the script, read from file offset 0).
    const DATA_IN: RawFd = 102;
    /// Data channel to the parent (`FUZZILLI_PRINT`).
    const DATA_OUT: RawFd = 103;

    pub(super) fn main() -> u8 {
        let settings = match Settings::from_env() {
            Ok(s) => s,
            Err(msg) => {
                eprintln!("ep9 --reprl: {msg}");
                return EXIT_USAGE;
            }
        };
        for fd in [CTRL_IN, CTRL_OUT, DATA_IN, DATA_OUT] {
            // SAFETY: F_GETFD reads the flags of a descriptor number; it changes nothing.
            if unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
                eprintln!(
                    "ep9 --reprl: fd {fd} is not open: run ep9 --reprl under a REPRL parent such as \
                     Fuzzilli (fds 100-103; sancov edges in this build: {})",
                    sancov::edges()
                );
                return EXIT_USAGE;
            }
        }
        // SAFETY: the four fds are open (checked above). They belong to the REPRL protocol:
        // the parent set them up for this process, and nothing else in ep9 uses fixed fd
        // numbers, so these `File`s are their only owners.
        let (mut ctrl_in, mut ctrl_out, data_in, data_out) = unsafe {
            (
                File::from_raw_fd(CTRL_IN),
                File::from_raw_fd(CTRL_OUT),
                File::from_raw_fd(DATA_IN),
                File::from_raw_fd(DATA_OUT),
            )
        };
        let mut helo = [0u8; 4];
        if ctrl_out.write_all(b"HELO").is_err()
            || ctrl_in.read_exact(&mut helo).is_err()
            || &helo != b"HELO"
        {
            eprintln!("ep9 --reprl: no HELO answer from the REPRL parent");
            return EXIT_USAGE;
        }
        let _ = FUZZ_OUT.set(data_out);
        std::panic::set_hook(Box::new(abort_on_panic));
        // One isolate thread with the configured stack runs every script (a fresh isolate
        // each); the stack-limit RangeError then behaves as in `ep9 <file>`.
        let worker = std::thread::Builder::new()
            .name("ep9-reprl".into())
            .stack_size(settings.stack_bytes)
            .spawn(move || serve(&settings, ctrl_in, ctrl_out, data_in));
        match worker.map(std::thread::JoinHandle::join) {
            Ok(Ok(status)) => status,
            // The hook aborts first; kept in case a panic skips it.
            Ok(Err(_)) => std::process::abort(),
            Err(e) => {
                eprintln!("ep9 --reprl: cannot start the isolate thread: {e}");
                EXIT_USAGE
            }
        }
    }

    /// The loop: one script per "exec". Returns 0 when the parent closes the control channel.
    fn serve(settings: &Settings, mut ctrl_in: File, mut ctrl_out: File, mut data_in: File) -> u8 {
        let mut script = Vec::new();
        loop {
            let mut action = [0u8; 4];
            match ctrl_in.read_exact(&mut action) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => return 0,
                Err(e) => return broken(&format!("cannot read the control channel: {e}")),
            }
            // libreprl writes the four bytes "exec" (the C patches compare with 'cexe').
            if &action != b"exec" {
                return broken(&format!("unknown action {action:?}"));
            }
            let mut size = [0u8; 8];
            if let Err(e) = ctrl_in.read_exact(&mut size) {
                return broken(&format!("cannot read the script size: {e}"));
            }
            let size = u64::from_ne_bytes(size);
            if size > MAX_SCRIPT_BYTES {
                return broken(&format!("script size {size} is above {MAX_SCRIPT_BYTES}"));
            }
            script.clear();
            script.resize(size as usize, 0);
            if let Err(e) = data_in.read_exact(&mut script) {
                return broken(&format!("cannot read the script: {e}"));
            }
            let status = execute(settings, &String::from_utf8_lossy(&script));
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            let word = (i32::from(status) & 0xff) << 8;
            if let Err(e) = ctrl_out.write_all(&word.to_ne_bytes()) {
                return broken(&format!("cannot write the status word: {e}"));
            }
            sancov::reset();
        }
    }

    fn broken(msg: &str) -> u8 {
        eprintln!("ep9 --reprl: {msg}");
        EXIT_USAGE
    }
}

/// The SanitizerCoverage trace-pc-guard callbacks, as Fuzzilli's `Targets/coverage.c`.
///
/// The functions that the instrumentation calls are named `__sanitizer_*`, which LLVM's
/// SanitizerCoverage pass does not instrument, and their bodies use only raw loads, stores and
/// integer arithmetic (no call into instrumented code, which would recurse).
#[cfg(unix)]
mod sancov {
    use core::ptr::null_mut;

    /// The shared memory size Fuzzilli maps (`SHM_SIZE`): a u32 edge count, then one bit per
    /// edge.
    const SHM_SIZE: usize = 0x20_0000;
    const MAX_EDGES: usize = (SHM_SIZE - 4) * 8;

    // Written once by `__sanitizer_cov_trace_pc_guard_init` (a module constructor, before
    // `main` and before any other thread exists), then only read. Plain statics (no atomics):
    // `__sanitizer_cov_trace_pc_guard` must not call a function, and an atomic load is one
    // until it is inlined.
    static mut SHMEM: *mut u8 = null_mut();
    static mut EDGES_START: *mut u32 = null_mut();
    static mut EDGES_STOP: *mut u32 = null_mut();

    /// The number of edge guards: 0 in a build without coverage.
    pub(super) fn edges() -> usize {
        // SAFETY: plain reads of statics written only before `main` (see above).
        let (start, stop) = unsafe { (EDGES_START, EDGES_STOP) };
        (stop as usize - start as usize) / 4
    }

    /// Turns every edge guard back on (after each script).
    pub(super) fn reset() {
        // SAFETY: the guard section [EDGES_START, EDGES_STOP) as the linker gave it (or empty).
        unsafe { __sanitizer_cov_reset_edgeguards() }
    }

    /// Gives guard `i` the value `i + 1` (0 = off), up to the bitmap size.
    ///
    /// # Safety
    /// `EDGES_START..EDGES_STOP` is the guard section (or both are null).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __sanitizer_cov_reset_edgeguards() {
        // SAFETY: see the function contract; the loop stays inside the section.
        unsafe {
            let mut n: usize = 0;
            let mut x = EDGES_START as usize;
            let stop = EDGES_STOP as usize;
            while x < stop && n < MAX_EDGES {
                n += 1;
                *(x as *mut u32) = n as u32;
                x += 4;
            }
        }
    }

    /// Called by each instrumented module's constructor with the whole guard section.
    ///
    /// # Safety
    /// `start..stop` is the sancov guard section (the instrumentation's contract).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __sanitizer_cov_trace_pc_guard_init(start: *mut u32, stop: *mut u32) {
        // SAFETY: `start..stop` is the guard section; the statics are written before any
        // other thread exists (module constructors run before `main`).
        unsafe {
            // Each module of the binary calls this with the same section: only the first does
            // the work (its guards are nonzero after it).
            if start == stop || *start != 0 {
                return;
            }
            if !EDGES_START.is_null() || !EDGES_STOP.is_null() {
                fail(b"ep9 sancov: coverage is only supported for one guard section\n");
            }
            EDGES_START = start;
            EDGES_STOP = stop;
            let key = libc::getenv(c"SHM_ID".as_ptr());
            let map = if key.is_null() {
                // No Fuzzilli parent: a private bitmap, so the guards still work.
                libc::mmap(
                    null_mut(),
                    SHM_SIZE,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            } else {
                // Open only (no O_CREAT), so no mode argument is needed; macOS declares
                // shm_open variadic, Linux with a mode_t.
                #[cfg(target_vendor = "apple")]
                let fd = libc::shm_open(key, libc::O_RDWR);
                #[cfg(not(target_vendor = "apple"))]
                let fd = libc::shm_open(key, libc::O_RDWR, 0);
                if fd < 0 {
                    fail(b"ep9 sancov: cannot open the shared memory SHM_ID\n");
                }
                libc::mmap(
                    null_mut(),
                    SHM_SIZE,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            if map == libc::MAP_FAILED {
                fail(b"ep9 sancov: cannot map the coverage bitmap\n");
            }
            SHMEM = map.cast::<u8>();
            __sanitizer_cov_reset_edgeguards();
            let edges = (stop as usize - start as usize) / 4;
            *(SHMEM as *mut u32) = edges as u32;
        }
    }

    /// Called on each instrumented edge: sets the edge's bit once, then turns the guard off.
    ///
    /// # Safety
    /// `guard` is a guard of the section given to `__sanitizer_cov_trace_pc_guard_init`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __sanitizer_cov_trace_pc_guard(guard: *mut u32) {
        // SAFETY: `guard` is in the section; a nonzero guard means the init mapped SHMEM, and
        // the reset keeps every index below MAX_EDGES, so the byte is inside the bitmap. Two
        // threads on one edge at once may both set the bit (same result), as in coverage.c.
        unsafe {
            let index = *guard;
            if index == 0 {
                return;
            }
            let byte = (SHMEM as usize + 4 + (index / 8) as usize) as *mut u8;
            *byte |= 1 << (index % 8);
            *guard = 0;
        }
    }

    /// Writes `msg` to stderr and exits at once (`_exit(-1)`, as coverage.c's CHECK).
    fn fail(msg: &[u8]) -> ! {
        // SAFETY: write(2) of a valid buffer, then _exit; no Rust runtime state is touched.
        unsafe {
            libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(-1)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    //! A fake REPRL parent: the libreprl side of the protocol on fds 100-103 against a child
    //! that is this test binary, re-run with only [`child_entry`] and `EP9_REPRL_TEST_CHILD`
    //! set (unit tests have no path to the `ep9` binary). The child runs [`super::main`].

    use std::fs::File;
    use std::io::{PipeReader, PipeWriter, Read as _, Seek as _, SeekFrom, Write as _};
    use std::os::fd::{AsRawFd as _, RawFd};
    use std::os::unix::fs::FileExt as _;
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::unix::EXIT_OUT_OF_MEMORY;

    const CHILD_ENV: &str = "EP9_REPRL_TEST_CHILD";

    /// The child end of the fake parent. Without `EP9_REPRL_TEST_CHILD` it does nothing.
    #[test]
    fn child_entry() {
        if std::env::var_os(CHILD_ENV).is_some() {
            let code = super::main();
            std::process::exit(i32::from(code));
        }
    }

    /// How one "exec" ended, as libreprl reports it.
    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        /// A status word arrived (`exit status << 8`).
        Status(i32),
        /// EOF on the control channel, then the child exited with this code.
        Exited(i32),
        /// EOF on the control channel, then the child died by this signal.
        Signaled(i32),
    }

    fn temp_file() -> File {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "ep9-reprl-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let f = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        f
    }

    struct FakeParent {
        child: Option<Child>,
        to_child: PipeWriter,
        from_child: PipeReader,
        data_to_child: File,
        data_from_child: File,
        stderr: File,
    }

    impl FakeParent {
        /// Starts a child with `env` added, and does the HELO handshake.
        fn spawn(env: &[(&str, &str)]) -> FakeParent {
            let (ctrl_in_r, to_child) = std::io::pipe().unwrap();
            let (from_child, ctrl_out_w) = std::io::pipe().unwrap();
            let data_to_child = temp_file();
            let data_from_child = temp_file();
            let stderr = temp_file();
            let sources: [RawFd; 4] = [
                ctrl_in_r.as_raw_fd(),
                ctrl_out_w.as_raw_fd(),
                data_to_child.as_raw_fd(),
                data_from_child.as_raw_fd(),
            ];
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "--exact",
                "reprl::tests::child_entry",
                "--nocapture",
                "--test-threads=1",
                "-q",
            ])
            .env(CHILD_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr.try_clone().unwrap());
            for (k, v) in env {
                cmd.env(k, v);
            }
            // SAFETY: the closure runs in the forked child before exec and calls only fcntl
            // and dup2 (async-signal-safe) on fds that the parent keeps open until spawn
            // returns.
            unsafe {
                cmd.pre_exec(move || {
                    // Move each source above 103 first, so a dup2 never overwrites a source
                    // that happens to sit on 100-103 (libreprl reserves those fds instead).
                    let mut high = [0; 4];
                    for (h, fd) in high.iter_mut().zip(sources) {
                        *h = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 200);
                        if *h < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                    for (i, h) in high.iter().enumerate() {
                        if libc::dup2(*h, 100 + i as RawFd) < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                    Ok(())
                });
            }
            let child = cmd.spawn().unwrap();
            drop((ctrl_in_r, ctrl_out_w));
            let mut p = FakeParent {
                child: Some(child),
                to_child,
                from_child,
                data_to_child,
                data_from_child,
                stderr,
            };
            let mut helo = [0u8; 4];
            p.wait_readable();
            p.from_child.read_exact(&mut helo).unwrap();
            assert_eq!(&helo, b"HELO");
            p.to_child.write_all(b"HELO").unwrap();
            p
        }

        /// Polls the control channel (60 s), so a hung child fails the test instead of
        /// hanging it.
        fn wait_readable(&mut self) {
            let mut fds = libc::pollfd {
                fd: self.from_child.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            let n = unsafe { libc::poll(&mut fds, 1, 60_000) };
            if n != 1 {
                let _ = self.child.as_mut().unwrap().kill();
                panic!("the REPRL child gave no answer in 60 s");
            }
        }

        /// One "exec", as libreprl's `reprl_execute` does it.
        fn exec(&mut self, script: &str) -> Outcome {
            for f in [
                &mut self.data_to_child,
                &mut self.data_from_child,
                &mut self.stderr,
            ] {
                f.seek(SeekFrom::Start(0)).unwrap();
            }
            self.data_to_child.write_all(script.as_bytes()).unwrap();
            self.data_to_child.seek(SeekFrom::Start(0)).unwrap();
            self.to_child.write_all(b"exec").unwrap();
            self.to_child
                .write_all(&(script.len() as u64).to_ne_bytes())
                .unwrap();
            self.wait_readable();
            let mut word = [0u8; 4];
            let mut got = 0;
            while got < 4 {
                match self.from_child.read(&mut word[got..]).unwrap() {
                    0 => break,
                    n => got += n,
                }
            }
            if got == 4 {
                return Outcome::Status(i32::from_ne_bytes(word));
            }
            let status = self.child.take().unwrap().wait().unwrap();
            match (status.code(), status.signal()) {
                (Some(code), _) => Outcome::Exited(code),
                (None, Some(sig)) => Outcome::Signaled(sig),
                _ => panic!("odd exit status {status:?}"),
            }
        }

        /// What the child wrote to `file` during the last exec (up to the shared offset).
        fn written(file: &File) -> String {
            let mut f = file;
            let end = f.stream_position().unwrap() as usize;
            let mut buf = vec![0u8; end];
            file.read_exact_at(&mut buf, 0).unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        }

        fn fuzz_out(&self) -> String {
            Self::written(&self.data_from_child)
        }

        fn stderr(&self) -> String {
            Self::written(&self.stderr)
        }

        /// Closes the control channel: the child must exit 0.
        fn finish(mut self) {
            let child = self.child.take().unwrap();
            drop(self);
            let status = child.wait_with_output().unwrap().status;
            assert_eq!(status.code(), Some(0), "{status:?}");
        }
    }

    impl Drop for FakeParent {
        fn drop(&mut self) {
            if let Some(c) = self.child.as_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    const OK: Outcome = Outcome::Status(0);
    const FAILED: Outcome = Outcome::Status(1 << 8);

    /// 200 scripts in one child: results, JS errors, compile errors, FUZZILLI_PRINT, print,
    /// gc, promise jobs, and state that must not survive into the next script (a fresh
    /// isolate per exec). Each status word is checked.
    #[test]
    fn two_hundred_scripts_with_status_words() {
        let mut p = FakeParent::spawn(&[]);
        for i in 0..200 {
            let (script, want) = match i % 10 {
                0 => (
                    "var a = [1, 2, 3].map(x => x * 2); a.join()".to_string(),
                    OK,
                ),
                1 => ("throw new Error('e' + 1)".to_string(), FAILED),
                2 => ("let a; let a;".to_string(), FAILED),
                3 => (format!("fuzzilli('FUZZILLI_PRINT', 'line ' + {i})"), OK),
                4 => (
                    "if (globalThis.leak !== undefined) throw new Error('leak');\
                     globalThis.leak = 1; Object.prototype.polluted = 1;"
                        .to_string(),
                    OK,
                ),
                5 => (
                    "if (({}).polluted !== undefined || typeof leak !== 'undefined') throw 1"
                        .to_string(),
                    OK,
                ),
                6 => ("Promise.reject(42)".to_string(), FAILED),
                7 => (
                    "async function f() { throw 1 } f().catch(() => {}); 1".to_string(),
                    OK,
                ),
                8 => (
                    "gc(); print('out', 1); fuzzilli('nothing', 0)".to_string(),
                    OK,
                ),
                _ => (
                    // A crash kind out of range does nothing.
                    "fuzzilli('FUZZILLI_CRASH', 7); undefinedName".to_string(),
                    FAILED,
                ),
            };
            assert_eq!(p.exec(&script), want, "script {i}: {script}");
            if i % 10 == 3 {
                assert_eq!(p.fuzz_out(), format!("line {i}\n"), "script {i}");
            } else {
                assert_eq!(p.fuzz_out(), "", "script {i}");
            }
            if want == FAILED {
                assert!(!p.stderr().is_empty(), "script {i}: no error text");
            }
        }
        p.finish();
    }

    /// The three crash kinds end the child by a signal, with no status word. Kind 0 (a
    /// panic) prints the panic text the triage script keys on.
    #[test]
    fn three_crash_kinds_end_by_a_signal() {
        let trap = if cfg!(target_arch = "x86_64") {
            libc::SIGILL
        } else if cfg!(target_arch = "aarch64") {
            libc::SIGTRAP
        } else {
            libc::SIGABRT
        };
        for (kind, signals) in [
            (0, vec![libc::SIGABRT]),
            (1, vec![libc::SIGSEGV, libc::SIGBUS]),
            (2, vec![trap]),
        ] {
            let mut p = FakeParent::spawn(&[]);
            assert_eq!(p.exec("1 + 1"), OK);
            match p.exec(&format!("fuzzilli('FUZZILLI_CRASH', {kind})")) {
                Outcome::Signaled(sig) => assert!(signals.contains(&sig), "kind {kind}: {sig}"),
                other => panic!("kind {kind}: {other:?}"),
            }
            if kind == 0 {
                let err = p.stderr();
                assert!(
                    err.contains(
                        "ep9: internal error: fuzzilli('FUZZILLI_CRASH', 0): injected panic"
                    ),
                    "{err}"
                );
            }
        }
    }

    /// An internal compiler error at run time (direct and indirect eval, `Function()`) ends the
    /// child by SIGABRT with no status word, whether the script catches the error or not (the
    /// test-only fault of the REPRL compiler hooks makes the error). A run-time SyntaxError
    /// stays a catchable JS error.
    #[test]
    fn runtime_internal_compile_errors_abort() {
        let fault = super::unix::TEST_INTERNAL_FAULT;
        for script in [
            format!("eval('{fault}')"),
            format!("try {{ eval('{fault}') }} catch (e) {{}} 1"),
            format!("try {{ (0, eval)('{fault}') }} catch (e) {{}} 1"),
            format!("Function('{fault}')"),
            format!("try {{ new Function('a', '{fault}') }} catch (e) {{}} 1"),
        ] {
            let mut p = FakeParent::spawn(&[]);
            assert_eq!(
                p.exec(
                    "try { eval('let a; let a;') } catch (e) {} eval('1') + Function('return 1')()"
                ),
                OK
            );
            match p.exec(&script) {
                Outcome::Signaled(sig) => assert_eq!(sig, libc::SIGABRT, "{script}"),
                other => panic!("{script}: {other:?} (stderr: {})", p.stderr()),
            }
            let err = p.stderr();
            assert!(
                err.contains("ep9: internal error: internal BytecodeGen error: injected"),
                "{script}: {err}"
            );
        }
    }

    /// A script that fills the GC heap ends the child by the near-limit callback: exit 71
    /// and no status word (a failed run for Fuzzilli, not a crash). A fresh child then runs.
    #[test]
    fn out_of_memory_exits_71_without_a_status_word() {
        let mut p = FakeParent::spawn(&[("EP9_REPRL_HEAP_MB", "16")]);
        assert_eq!(p.exec("var small = []; small.push({}); 1"), OK);
        let script = "var keep = []; for (;;) keep.push([1.5, 2.5, 3.5, {}, 'x' + keep.length]);";
        assert_eq!(
            p.exec(script),
            Outcome::Exited(i32::from(EXIT_OUT_OF_MEMORY))
        );
        assert!(
            p.stderr().contains("ep9 --reprl: out of memory"),
            "{}",
            p.stderr()
        );
        let mut fresh = FakeParent::spawn(&[("EP9_REPRL_HEAP_MB", "16")]);
        assert_eq!(fresh.exec("1"), OK);
        fresh.finish();
    }

    /// A bad setting stops the child before the handshake.
    #[test]
    fn a_bad_setting_is_a_usage_error() {
        let run = |v: &str| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "reprl::tests::child_entry", "--nocapture", "-q"])
                .env(CHILD_ENV, "1")
                .env("EP9_REPRL_HEAP_MB", v)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .unwrap()
        };
        let out = run("0");
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("bad EP9_REPRL_HEAP_MB value"));
    }
}
