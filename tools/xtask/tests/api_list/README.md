# api-list fixtures (H20-Q6)

`tests/api_list.rs` runs `cargo xtask api-list --json <file> --golden <file>` on these small
rustdoc JSON files (format 61, written by hand in the form of
`cargo +nightly-2026-10-04 rustdoc -p ep9_vm -- -Z unstable-options --output-format json`).
No rustdoc runs, so the tests need no nightly.

| File | What |
|---|---|
| `base.json` | a crate `k`: a struct `Cx<'s>` with one inherent method, and a free fn |
| `golden.txt` | the listing of `base.json` (`--bless` writes it) |
| `new-pub-item.json` | `base.json` plus one new `pub fn`: the diff fails with one `+` line |
| `format-62.json` | `base.json` with another `format_version`: exit 2 |
| `forms-base.json` | a crate `fx` (source: `forms-src/lib.rs.txt`, cargo features off): a trait `Downcast: Sealed` with a sealed supertrait, `pub use hidden::E::*` of a local enum, an `extern "C"` fn and fn pointer type; made by `cargo +nightly-2026-10-04 rustdoc --lib -- -Z unstable-options --output-format json --document-hidden-items` (aarch64-apple-darwin), then cut to the local items (no span, docs, target; no synthetic or blanket impls) |
| `forms-golden.txt` | the listing of `forms-base.json` |
| `forms-supertrait.json` | feature `t1`: `Sealed::test` loses its `Token` argument (M7-P5 Codex T1) |
| `forms-enum-glob.json` | feature `t3`: `E { A }` becomes `#[repr(u8)] E { A(u32), B { x: u64 } = 7 }` (Codex T3) |
| `forms-unwind.json` | feature `t4`: `extern "C"` becomes `extern "C-unwind"` on `f` and `Fp` (Codex T4) |
| `where-base.json` | a crate `fx` (source: `where-src/lib.rs.txt`, cargo features off): the where-clause form of a sealed supertrait, `trait Downcast where Self: sealed::Sealed`, and one level down `Sealed where Self: Inner`; made and cut as `forms-base.json` (M7-P5 Codex R3 finding 2) |
| `where-golden.txt` | the listing of `where-base.json` |
| `where-w64.json` | feature `w64`: `Sealed::test` returns `u64`, not `u32` |
