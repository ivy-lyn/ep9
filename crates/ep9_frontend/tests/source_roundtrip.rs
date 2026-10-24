//! H05 "Units keep source" (plan-v7 D21, M2-P4-RF1-H05-SRC): with the default options a
//! compiled unit embeds its source text, and the text survives the serialized form. A script
//! and a module, ASCII and non-ASCII text, each: compile -> `to_bytes` -> `from_bytes` ->
//! `source == Some(input)`. The negative control compiles with `embed_source: false` and gets
//! `None` (and no source bytes).
//!
//! `cargo test -p ep9_frontend --test source_roundtrip`

use ep9_bytecode::{CompiledUnit, sha256};
use ep9_frontend::{
    CompileOptions, SourceOpts, Strictness, compile, compile_module, compile_script,
};

/// Non-ASCII text: Latin-1, a BMP character outside Latin-1, a non-BMP character (a UTF-16
/// surrogate pair) and a line separator, in a string, an identifier and a comment.
const SCRIPT: &str = "// caf\u{e9} \u{2028}\nvar \u{3c0} = '\u{20ac} \u{1F600}';\nfunction f(\u{e9}) { return \u{e9} + \u{3c0}; }\n";
const MODULE: &str =
    "export const \u{3c0} = '\u{1F600}';\nexport default function () { return \u{3c0}; }\n";

/// Encodes, decodes and checks the embedded text of `unit` against `input`.
fn round_trip(unit: &CompiledUnit, input: &str) {
    assert_eq!(unit.source.as_deref(), Some(input), "compiled unit");
    let bytes = unit.to_bytes();
    let back = CompiledUnit::from_bytes(&bytes).expect("the unit loads back");
    assert_eq!(back.source.as_deref(), Some(input), "loaded unit");
    assert_eq!(back.header.source_len as usize, input.len());
    assert_eq!(back.header.source_hash, sha256::sha256(input.as_bytes()));
    assert_eq!(
        back.to_bytes(),
        bytes,
        "a second write gives the same bytes"
    );
}

#[test]
fn script_keeps_its_source_by_default() {
    for src in ["var x = 1;\nx + 1;\n", SCRIPT] {
        for strictness in [Strictness::Sloppy, Strictness::Strict] {
            let unit = compile_script(src, strictness).expect("script compiles");
            round_trip(&unit, src);
        }
    }
}

#[test]
fn module_keeps_its_source_by_default() {
    for src in ["export let x = 1;\n", MODULE] {
        let unit = compile_module(src).expect("module compiles");
        round_trip(&unit, src);
        // ABI 5: a module unit also carries its module records.
        assert!(unit.module_records.is_some());
    }
}

#[test]
fn default_options_embed_the_source() {
    assert!(CompileOptions::default().embed_source);
}

/// Negative control: without `embed_source` the unit keeps only the hash and the length.
#[test]
fn embed_source_false_gives_none() {
    let options = CompileOptions {
        embed_source: false,
        ..CompileOptions::default()
    };
    for (src, opts) in [
        (SCRIPT, SourceOpts::script(Strictness::Sloppy)),
        (MODULE, SourceOpts::module()),
    ] {
        let unit = compile(src, opts, options).expect("compiles");
        assert_eq!(unit.source, None);
        let back = CompiledUnit::from_bytes(&unit.to_bytes()).expect("the unit loads back");
        assert_eq!(back.source, None);
        assert_eq!(back.header.source_len as usize, src.len());
        assert_eq!(back.header.source_hash, sha256::sha256(src.as_bytes()));
        // The same source with the default options is longer by exactly the text.
        let with = compile(src, opts, CompileOptions::default()).expect("compiles");
        assert_eq!(with.to_bytes().len(), unit.to_bytes().len() + src.len());
    }
}
