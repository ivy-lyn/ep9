//! Source text -> ep9_frontend::compile -> verified unit -> serialize, reload, verify again.
//!
//! Input: byte 0 picks the goal (0 sloppy script, 1 strict script, 2 module; mod 3), the rest
//! is the source text. scripts/seed-test262.sh writes test262 files in this form.
#![no_main]

#[path = "common.rs"]
mod common;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&goal, rest)) = data.split_first() else {
        return;
    };
    #[cfg(feature = "fixture-crash")]
    if rest.starts_with(b"EP9-FIXTURE-CRASH") {
        panic!("fixture crash, input {:?}", String::from_utf8_lossy(rest));
    }
    let Some(source) = common::text(rest) else {
        return;
    };
    let opts = match goal % 3 {
        0 => SourceOpts::script(Strictness::Sloppy),
        1 => SourceOpts::script(Strictness::Strict),
        _ => SourceOpts::module(),
    };
    common::check(ep9_frontend::compile(
        source,
        opts,
        CompileOptions::default(),
    ));
});
