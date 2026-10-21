//! Repo checks, run as `cargo xtask <task>` (alias in `.cargo/config.toml`). The plan lists them
//! in docs/plan/08-repo.md section 7: disassembly checks, size asserts, pin-leak guards.
//!
//! | Task | Module | What it checks |
//! |---|---|---|
//! | `disasm-check` | `disasm.rs` | tail-call disassembly and handler argument count (H15) |
//! | `fp-check` | `fpcheck.rs` | no banned libm symbol, no FMA instruction outside `ep9_math`, no second math crate, in post-LTO objects for 5 targets (H36) |
//! | `feature-matrix` | `features.rs` | default, jitless and `aot` builds: build, clippy and test, per package (H27) |
//! | `gc-lint` | `gc_lint.rs` | no stack-only GC type (`Local` and friends) in a heap container, `static` or `thread_local` (H10) |
//!
//! Each module's doc says how far it is built. Exit codes for every task: 0 pass, 1 fail,
//! 2 usage error, tool error or a task that is not implemented yet. An unknown task exits 2.

mod disasm;
mod features;
mod fpcheck;
mod gc_lint;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().cloned().unwrap_or_default();
    let result = match task.as_str() {
        "disasm-check" => disasm::main(&args[1..]),
        "fp-check" => fpcheck::main(&args[1..]),
        "feature-matrix" => features::main(&args[1..]),
        "gc-lint" => gc_lint::main(&args[1..]),
        _ => {
            eprintln!("xtask: task {task:?} is not implemented yet");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("xtask {task}: {e}");
            ExitCode::from(2)
        }
    }
}
