# gc-lint fixtures

Cases for `tools/xtask/tests/gc_lint.rs`. They are never compiled: `cargo xtask gc-lint` only
parses them. Each test runs `cargo xtask gc-lint --root tools/xtask/tests/gc_lint <case>` and
checks that the reported `file:line`s are exactly the lines that end in `// FLAG`, with exit 1
(or exit 0 when the case has none).

| Case | What it holds | gc-lint must |
|---|---|---|
| `vec_local.rs` | `Vec<Local>` in a struct field and a `let` | flag both |
| `hashmap_descriptor.rs` | `HashMap<_, PropertyDescriptor>` | flag it |
| `box_jsstr.rs` | `Box<[JsStr]>` | flag it |
| `holder_in_vec.rs` | a struct holding a `Local` (renamed by `use .. as`) inside a `Vec` | flag the `Vec` |
| `static_local.rs` | a `static` holding a `Local`, by value and through a reference | flag both |
| `thread_local_local.rs` | a `thread_local!` holding a `Local` | flag it |
| `boxed_closure.rs` | `Box::new(move \|\| first.bits())` with `first = args.get(0)` | flag the closure |
| `nested/deep/vec_in_mod.rs` | `Vec<Local>` in a nested directory | flag it; the path prints with `/` |
| `clean.rs` | references, `PhantomData`, fn pointers, trait objects, `HostFn` tables, root ranges, unboxed closures | exit 0 |
| `allowed.rs` | a `Vec<Local>` under `// gc-lint: allow(..)` | exit 0 and list the allow |
| `bad_allows.rs` | an allow that covers nothing, one before a blank line, two malformed | flag the 4 comment lines |
| `does_not_parse.txt` | a syntax error | exit 2 with `file:line` |
