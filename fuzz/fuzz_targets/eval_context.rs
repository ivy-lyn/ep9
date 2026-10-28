//! eval code under a random caller context -> ep9_frontend::compile_eval -> verified unit ->
//! serialize, reload, verify again.
//!
//! Input: bytes 0-1 (little endian) are the context bits, the rest is the eval text. The bits
//! only make contexts the VM can make (ep9_bytecode EvalContext):
//!
//! ```text
//! bit 0 strict          bit 1 direct (else indirect: no other flag, no private name)
//! bit 2 in_function     bit 3 in_method (needs in_function)   bit 4 in_derived_ctor (needs in_method)
//! bit 5 in_class_field_init (direct; new.target and super.x are allowed there)
//! bit 6 private names #x, #y in scope   bit 7 #y is a method   bit 8 #x is an accessor
//! bit 9 #y is static
//! new_target_ok = in_function | field init; super_property_ok = in_method | field init;
//! super_call_ok = in_derived_ctor
//! ```
#![no_main]

#[path = "common.rs"]
mod common;

use ep9_bytecode::EvalContext;
use ep9_frontend::CompileOptions;

fn context(bits: u16) -> EvalContext {
    let on = |b: u16| bits & (1 << b) != 0;
    let direct = on(1);
    let in_function = direct && on(2);
    let in_method = in_function && on(3);
    let in_derived_ctor = in_method && on(4);
    let in_class_field_init = direct && on(5);
    let names = direct && on(6);
    let list = |yes: bool, name: &str| {
        if names && yes {
            vec![name.to_owned()]
        } else {
            Vec::new()
        }
    };
    EvalContext {
        strict: on(0),
        direct,
        in_function,
        in_method,
        in_derived_ctor,
        in_class_field_init,
        new_target_ok: in_function || in_class_field_init,
        super_property_ok: in_method || in_class_field_init,
        super_call_ok: in_derived_ctor,
        private_names: if names {
            vec!["x".to_owned(), "y".to_owned()]
        } else {
            Vec::new()
        },
        private_methods: list(on(7), "y"),
        private_accessors: list(on(8), "x"),
        private_static: list(on(9), "y"),
    }
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let [b0, b1, rest @ ..] = data else {
        return;
    };
    let Some(source) = common::text(rest) else {
        return;
    };
    let ctx = context(u16::from_le_bytes([*b0, *b1]));
    common::check(ep9_frontend::compile_eval(
        source,
        &ctx,
        CompileOptions::default(),
    ));
});
