//! The `ep9` binary: run, repl, eval, check, snapshot build, test262 host, Fuzzilli REPRL and
//! the JSC-shell API, with the flags `--jitless`, `--unit` and `--profile-out`.
//!
//! M0 stub: it prints its version.
#![forbid(unsafe_code)]

const VERSION_LINE: &str = concat!("ep9 ", env!("CARGO_PKG_VERSION"));

fn main() {
    println!("{VERSION_LINE}");
}
