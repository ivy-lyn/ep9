//! A process that compiled JS on a thread must exit cleanly from that thread (M2-P1-WIN-TLSJOIN).
//! `harness = false`, so `main` runs on the process main thread, as in an embedder or a doctest.
//!
//! ```text
//! parent (this main) --spawns--> child "return"       compile on main, return from main
//!                                child "exit"         compile on main, std::process::exit(0)
//!                                child "thread-exit"  compile on a thread, exit from that thread
//! each child: exit status 0, no "panicked" on stderr
//! ```
//!
//! On Windows the thread that exits the process runs its thread-local destructors after the OS
//! has ended every other thread, the front-end worker included. A worker that joined its thread
//! in `Drop` made std panic there ("threads should not terminate unexpectedly"), and a panic in a
//! thread-local destructor aborts: status 0xC0000409 and "thread local panicked on drop". Other
//! hosts keep the worker thread alive until the destructor ends, so there the children pass with
//! either `Drop`; the worker's unit test `dropping_a_worker_does_not_wait_for_its_thread` covers
//! the cause on every host.

use std::process::Command;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};

const MODE: &str = "EP9_EXIT_AFTER_COMPILE";

fn compile_here() {
    let unit = ep9_frontend::compile(
        "var a = [1, 2, 3].length + 1;",
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    );
    assert!(unit.is_ok(), "{:?}", unit.err());
}

fn main() {
    match std::env::var(MODE).as_deref() {
        Ok("return") => compile_here(),
        Ok("exit") => {
            compile_here();
            std::process::exit(0);
        }
        Ok("thread-exit") => {
            // The process ends on the spawned thread, which owns the worker; main never wakes.
            let _ = std::thread::spawn(|| {
                compile_here();
                std::process::exit(0);
            })
            .join();
            unreachable!("the spawned thread exits the process");
        }
        Ok(other) => panic!("unknown {MODE}={other}"),
        Err(_) => parent(),
    }
}

fn parent() {
    let exe = std::env::current_exe().expect("the test binary path");
    for mode in ["return", "exit", "thread-exit"] {
        let out = Command::new(&exe)
            .env(MODE, mode)
            .output()
            .expect("spawn the child");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && !stderr.contains("panicked"),
            "{mode}: {:?}\n{stderr}",
            out.status
        );
        println!("exit_after_compile {mode}: ok");
    }
}
