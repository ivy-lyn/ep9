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
