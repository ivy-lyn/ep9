# extref-lint evasion fixtures (V11IN-22)

`tests/extref_lint.rs` runs `cargo xtask extref-lint --root tests/extref_lint` on this tree.
It is a fake repo root: the lint scans `crates/*/src`, so the file names carry the file rules
(`frame.rs`, `external_ref.rs`, `layout.rs`, the `ep9_value` crate). The files are only
parsed, never built. A line that must be a finding ends in `// FLAG`; every other line must
give none.

| File | Form |
|---|---|
| `crates/ep9_vm/src/layout.rs` | rename: a const of another name takes its value from a field or the offset: no finding (H25:54 allows layout.rs); a field or the offset in an inline module: a finding (not an export; M7-P5 Codex T7) |
| `crates/ep9_value/src/encoding.rs` | rename: a const or static of another name takes the offset: no finding (H25:54 allows ep9_value) |
| `crates/ep9_vm/src/frame.rs` | rename: a function, macro or const of the encoding file wraps the encoding: no finding (H25:54 allows frame.rs) |
| `crates/ep9_vm/src/external_ref.rs` | rename: a function of the type file wraps the encoding: no finding (H25:54 allows external_ref.rs) |
| `crates/ep9_vm/src/kelvin.rs` | raw name: U+212A KELVIN SIGN, which rustc reads as `K` (NFC) |
| `crates/ep9_vm/src/macro_args.rs` | macro: the module and the name are two macro arguments (`$m::$f`) |
| `crates/ep9_vm/src/hiding.rs` | macro: `include!`, `concat_idents!`, `${concat(..)}`; import: a `use` that renames one of them (M7-P5 Codex T5) |
| `crates/ep9_vm/src/path_mod.rs` | macro: a `#[path]` module file outside the scanned `src/`, also the second `path` of two `cfg_attr` (M7-P5 Codex T6) |
| `crates/ep9_vm/src/clean.rs` | negative controls: forms that must give no finding |

`crates/ep9_vm/hidden/mod.rs` is the hidden module file: it holds a bare encoding call that
the lint never reads (its `src/` is not scanned), which is why the `#[path]` line is a finding.
`crates/ep9_vm/src/inner/ok.rs` is a `#[path]` module file that the lint scans.

The lint refuses only what H25 (Rules, lines 54-55) forbids (M7-P5 fixer r3, M6-ORCH-371
(2)): the rename rows of `layout.rs`, `encoding.rs`, `frame.rs` and `external_ref.rs` are
inputs that H25 allows, so they give no finding, as at 0c58e12f. The stricter V11IN-22 rules
for them wait for route H25-STRICT-V13.
