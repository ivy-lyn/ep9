//! A process that compiled JS on a thread must exit cleanly from that thread (M2-P1-WIN-TLSJOIN).
//! `harness = false`, so `main` runs on the process main thread, as in an embedder or a doctest.
//!
//! ```text
//! parent (this main) --spawns--> child "return"       compile on main, return from main
//!                                child "exit"         compile on main, std::process::exit(0)
//!                                child "thread-exit"  compile on a thread, exit from that thread
//! each child: exit status 0, no "panicked" on stderr, ends before DEADLINE
//! then the stress loop: EP9_EXIT_STRESS more "exit" and "thread-exit" children, each with one
//! spinning load thread per CPU; a child that does not end before DEADLINE is killed (a hit)
//! ```
//!
//! On Windows the thread that exits the process runs its thread-local destructors after the OS
//! has ended every other thread, the front-end worker included. A worker that joined its thread
//! in `Drop` made std panic there ("threads should not terminate unexpectedly"), and a panic in a
//! thread-local destructor aborts: status 0xC0000409 and "thread local panicked on drop". Other
//! hosts keep the worker thread alive until the destructor ends, so there the children pass with
//! either `Drop`; the worker's unit test `dropping_a_worker_does_not_wait_for_its_thread` covers
//! the cause on every host.
//!
//! WIN-EXIT-HANG (CI 42484485512): the OS can also end the worker while it holds a lock. A drop
//! that then takes the same lock waits for ever, and the child never ends. The parent kills a
//! child at DEADLINE and goes on, so a hang costs seconds, not the job timeout. The worker's unit
//! test `dropping_a_worker_takes_no_lock_its_thread_may_hold` covers this cause on every host.
//! The stderr of a "thread-exit" child shows where it stopped:
//!
//! ```text
//! no "compiled"                   the hang is before the exit (in the compile)
//! "compiled", no "marker inner"   the hang is in the exit, before the thread-local destructors
//! "marker inner", no "outer"      the hang is in a destructor between them (WORKER, the cause)
//! ```

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};

const MODE: &str = "EP9_EXIT_AFTER_COMPILE";
/// Set in a stress child: start one spinning thread per CPU before the compile.
const LOAD: &str = "EP9_EXIT_LOAD";
/// The number of stress children per mode (any count overrides [`STRESS_DEFAULT`]).
const STRESS: &str = "EP9_EXIT_STRESS";
/// The stress loop is a smoke test. The deterministic gate of WIN-EXIT-HANG is the worker's
/// unit test `dropping_a_worker_takes_no_lock_its_thread_may_hold`. Windows probe A (the test
/// without the fix) hung 9 of 8000 children (about 0.11 % per exit); probe B (with the fix)
/// hung 0 of 8000. At that rate, 500 per mode show a regression in about 42 % of the modes of
/// one run (M6-ORCH-283). The OS ends the worker at an exit on Windows only, so 5 elsewhere.
const STRESS_DEFAULT: usize = if cfg!(windows) { 500 } else { 5 };
/// A child that runs longer than this is killed and counts as a hit.
const DEADLINE: Duration = Duration::from_secs(10);
/// The stress loop stops at this many hits, so a probe run ends in bounded time.
const MAX_HITS: usize = 50;

fn compile_here() {
    let unit = ep9_frontend::compile(
        "var a = [1, 2, 3].length + 1;",
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    );
    assert!(unit.is_ok(), "{:?}", unit.err());
}

/// Prints its text to stderr when its thread-local is destroyed.
struct Mark(&'static str);

impl Drop for Mark {
    fn drop(&mut self) {
        eprintln!("{}", self.0);
    }
}

thread_local! {
    // Thread-local destructors run in reverse order of the first access: inner, then the
    // front-end WORKER (and the std channel CONTEXT), then outer.
    static OUTER: Mark = const { Mark("marker outer") };
    static INNER: Mark = const { Mark("marker inner") };
}

/// One spinning thread per CPU. They hold no lock; the process exit ends them.
fn start_load() {
    if std::env::var_os(LOAD).is_none() {
        return;
    }
    let cpus = thread::available_parallelism().map_or(1, usize::from);
    for _ in 0..cpus {
        thread::spawn(|| {
            loop {
                std::hint::spin_loop();
            }
        });
    }
}

fn main() {
    match std::env::var(MODE).as_deref() {
        Ok("return") => compile_here(),
        Ok("exit") => {
            start_load();
            compile_here();
            eprintln!("compiled");
            std::process::exit(0);
        }
        Ok("thread-exit") => {
            start_load();
            // The process ends on the spawned thread, which owns the worker; main never wakes.
            let _ = thread::spawn(|| {
                OUTER.with(|_| ());
                compile_here();
                INNER.with(|_| ());
                eprintln!("compiled");
                std::process::exit(0);
            })
            .join();
            unreachable!("the spawned thread exits the process");
        }
        Ok(other) => panic!("unknown {MODE}={other}"),
        Err(_) => parent(),
    }
}

/// The end of one child.
struct Run {
    /// `None`: the child did not end before [`DEADLINE`] and was killed.
    status: Option<ExitStatus>,
    /// For a `None` run it ends with the kill result and the state after the kill.
    stderr: String,
}

impl Run {
    fn passed(&self) -> bool {
        self.status.is_some_and(|s| s.success()) && !self.stderr.contains("panicked")
    }
}

/// Run one child with stdout to null and stderr read on a thread. Every wait has a bound.
fn run_child(exe: &std::path::Path, mode: &str, load: bool) -> Run {
    let mut command = Command::new(exe);
    command
        .env(MODE, mode)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if load {
        command.env(LOAD, "1");
    }
    let mut child = command.spawn().expect("spawn the child");
    let mut err = child.stderr.take().expect("piped stderr");
    let (text_tx, text_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut text = Vec::new();
        let _ = err.read_to_end(&mut text);
        let _ = text_tx.send(String::from_utf8_lossy(&text).into_owned());
    });
    let start = Instant::now();
    let (status, killed) = loop {
        if let Some(status) = child.try_wait().expect("wait for the child") {
            break (Some(status), None);
        }
        if start.elapsed() > DEADLINE {
            // TerminateProcess or SIGKILL: the child's pipe closes and the reader ends.
            let kill = child.kill();
            let killed = Instant::now();
            while killed.elapsed() < DEADLINE && matches!(child.try_wait(), Ok(None)) {
                thread::sleep(Duration::from_millis(10));
            }
            // A child that the kill did not end can hold the pipes of this process (std
            // spawns with inherited handles on Windows): stop here with a clear line.
            let after = child.try_wait();
            if matches!(after, Ok(None)) {
                panic!("{mode}: the child survived the kill (kill: {kill:?}, after: {after:?})");
            }
            break (None, Some(format!("\nkill: {kill:?}, after: {after:?}")));
        }
        thread::sleep(Duration::from_millis(2));
    };
    let mut stderr = text_rx
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| "(stderr did not close after the end of the child)".into());
    if let Some(killed) = killed {
        stderr.push_str(&killed);
    }
    Run { status, stderr }
}

fn parent() {
    let exe = std::env::current_exe().expect("the test binary path");
    for mode in ["return", "exit", "thread-exit"] {
        let run = run_child(&exe, mode, false);
        assert!(
            run.passed(),
            "{mode}: {:?} (None: killed after {DEADLINE:?})\n{}",
            run.status,
            run.stderr
        );
        println!("exit_after_compile {mode}: ok");
    }

    let count = std::env::var(STRESS)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(STRESS_DEFAULT);
    let mut total_hits = 0;
    let mut texts: Vec<String> = Vec::new();
    let mut failed = false;
    for mode in ["exit", "thread-exit"] {
        let (mut runs, mut hits, mut fails) = (0, 0, 0);
        while runs < count && total_hits < MAX_HITS {
            let run = run_child(&exe, mode, true);
            runs += 1;
            if run.status.is_none() {
                hits += 1;
                total_hits += 1;
            } else if !run.passed() {
                fails += 1;
            } else {
                continue;
            }
            if texts.len() < 3 {
                texts.push(format!(
                    "{mode} run {runs}: {:?}\n{}",
                    run.status, run.stderr
                ));
            }
        }
        println!("exit_after_compile hits: {hits} of {runs} ({mode}; {fails} other failures)");
        failed |= hits > 0 || fails > 0;
    }
    if total_hits >= MAX_HITS {
        println!("exit_after_compile: stopped at {MAX_HITS} hits");
    }
    assert!(
        !failed,
        "a stress child hung (status None: killed after {DEADLINE:?}) or failed; the first {}:\n{}",
        texts.len(),
        texts.join("\n---\n")
    );
}
