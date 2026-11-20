# ep9-typecheck fixture

A small tsconfig project for `tests/cli.rs` and the sp-tsrs-win smoke step (ci.yml).

| File | What it tests |
|---|---|
| `src/ok.ts` | a clean file with 2-, 3- and 4-byte UTF-8 characters |
| `src/bad.ts` | a type error (TS2322): `has_type_errors` = 1 for this file only |
| `src/bom.ts` | a UTF-8 BOM: the hash is of the text after the BOM strip (H37-Q3) |

`tests/cli.rs` adds a UTF-16 LE file in a copy of this project (the hash is of its UTF-8 form).
