# adr-lint test fixtures

Inputs for the unit tests of the full `cargo xtask adr-lint` in `tools/xtask/src/adr_lint.rs`
(plan v9 rule 24: the tests read only these files, never the live ADRs, the live README or an
expectation file). The tests copy them into a temporary git tree, commit that as the base, and
then add commits on top.

| File | Copied to | Used for |
|---|---|---|
| `adr-README.md` | `docs/adr/README.md` | the Counts line and the index (part 3) |
| `H16-accepted.md` | `docs/adr/H16-ics.md` | an Accepted ADR: its Crates row (part 1), its Change log and Review record (part 2), a `path:line` reference (part 4) |
| `H29-draft.md` | `docs/adr/H29-deopt.md` | a Draft ADR (part 2 skips it) |
| `vm-lib.rs.txt` | `crates/ep9_vm/src/lib.rs` | the code file that the reference names |
