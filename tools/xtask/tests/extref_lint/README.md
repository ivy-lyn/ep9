# extref-lint evasion fixtures (V11IN-22)

`tests/extref_lint.rs` runs `cargo xtask extref-lint --root tests/extref_lint` on this tree.
It is a fake repo root: the lint scans `crates/*/src`, so the file names carry the file rules
(`frame.rs`, `external_ref.rs`, `layout.rs`, the `ep9_value` crate). The files are only
parsed, never built. A line that must be a finding ends in `// FLAG`; every other line must
give none.

| File | Form |
|---|---|
| `crates/ep9_vm/src/layout.rs` | rename: a const of another name takes its value from a field or the offset |
| `crates/ep9_value/src/encoding.rs` | rename: a const or static of another name takes the offset |
| `crates/ep9_vm/src/frame.rs` | rename: a function, macro or const of the encoding file wraps the encoding |
| `crates/ep9_vm/src/external_ref.rs` | rename: a function of the type file wraps the encoding |
| `crates/ep9_vm/src/kelvin.rs` | raw name: U+212A KELVIN SIGN, which rustc reads as `K` (NFC) |
| `crates/ep9_vm/src/macro_args.rs` | macro: the module and the name are two macro arguments (`$m::$f`) |
| `crates/ep9_vm/src/hiding.rs` | macro: `include!`, `concat_idents!`, `${concat(..)}` |
| `crates/ep9_vm/src/path_mod.rs` | macro: a `#[path]` module file outside the scanned `src/` |
| `crates/ep9_vm/src/clean.rs` | negative controls: forms that must give no finding |

`crates/ep9_vm/hidden/mod.rs` is the hidden module file: it holds a bare encoding call that
the lint never reads (its `src/` is not scanned), which is why the `#[path]` line is a finding.
`crates/ep9_vm/src/inner/ok.rs` is a `#[path]` module file that the lint scans.
