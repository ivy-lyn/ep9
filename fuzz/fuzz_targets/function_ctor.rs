//! `Function(params, body)` and its generator / async kinds (CreateDynamicFunction) ->
//! ep9_frontend::compile_function_ctor -> verified unit -> serialize, reload, verify again.
//!
//! Input: byte 0 picks the kind (Function, GeneratorFunction, AsyncFunction,
//! AsyncGeneratorFunction; mod 4); the rest is `params NUL body`, or only the body when it has
//! no NUL byte. The parameter text and the body are parsed apart first (function_ctor.rs F1),
//! so text that closes the wrapper early must give a SyntaxError, never a different function.
#![no_main]

#[path = "common.rs"]
mod common;

use ep9_bytecode::FunctionCtorKind;
use ep9_frontend::CompileOptions;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&kind, rest)) = data.split_first() else {
        return;
    };
    let Some(text) = common::text(rest) else {
        return;
    };
    let (params, body) = text.split_once('\0').unwrap_or(("", text));
    let kind = match kind % 4 {
        0 => FunctionCtorKind::Normal,
        1 => FunctionCtorKind::Generator,
        2 => FunctionCtorKind::Async,
        _ => FunctionCtorKind::AsyncGenerator,
    };
    common::check(ep9_frontend::compile_function_ctor(
        kind,
        params,
        body,
        CompileOptions::default(),
    ));
});
