//! Repo checks, run as `cargo xtask <task>` (alias in `.cargo/config.toml`). The plan lists them
//! in docs/plan/08-repo.md section 7: disassembly checks, size asserts, pin-leak guards.
//!
//! | Task | Module | Exit code |
//! |---|---|---|
//! | `disasm-check` | `disasm.rs`: tail-call disassembly and handler argument count (H15) | 0 pass, 1 fail, 2 usage or tool error |
//!
//! Other tasks are not implemented yet; they exit with code 2.

mod disasm;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().cloned().unwrap_or_default();
    let result = match task.as_str() {
        "disasm-check" => disasm::main(&args[1..]),
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
