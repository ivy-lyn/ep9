#![allow(dead_code)] // m4-stub
//! `ep9 --reprl`: the Fuzzilli REPRL loop (read-eval-print-reset-loop).
//!
//! Not built yet: the REPRL role (M4-P3) fills this file. The plan for it: the control and data
//! fds 100-103 (HELO, the script from the shared buffer, a status word), a fresh isolate per
//! script, `fuzzilli("FUZZILLI_CRASH", n)` and `fuzzilli("FUZZILLI_PRINT", s)`, and the sancov
//! callbacks. This is the one module of ep9_cli that the main.rs lint allows to opt out.
//! Windows keeps this stub.

/// The REPRL entry. Returns the process exit status.
pub fn main() -> u8 {
    eprintln!("--reprl: not built yet");
    2
}
