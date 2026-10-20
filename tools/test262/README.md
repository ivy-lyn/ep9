# ep9-test262

The test262 runner. M0 week 1 has two commands: `index` and `parse`.

```
cargo run --release -p ep9_test262 -- parse          # exit 1 on a change vs the expectation file
cargo run --release -p ep9_test262 -- index --out /tmp/index.json
```

Default paths start at the repo root, so the commands work from any directory.

## `parse`

Each test goes through `ep9_frontend::check_early_errors` (oxc parse with regex validation, then
oxc_semantic syntax checks).

| Test set | Rule |
|---|---|
| `test/{language,built-ins,annexB,intl402}/**/*.js` | Run |
| `*_FIXTURE*` files | Skip. They are modules that tests import (INTERPRETING.md) |
| `test/staging/`, `test/harness/` | Skip. Not part of the parse gate |
| `harness/*.js` (includes) | Not tests. Each include that a selected test loads is parsed in every Script mode that loads it. Shown on its own line; a failure counts like a test failure |

| Flags | Runs |
|---|---|
| `module` | Module goal, source as-is |
| `raw` | Sloppy Script, source as-is |
| `onlyStrict` | Strict Script: `"use strict";\n` prefix |
| `noStrict` | Sloppy Script |
| none | Sloppy and strict Script |

- A test with `negative.phase: parse` passes if every run reports an early error. Every other test passes if every run parses.
- Counts are per test file (a file passes when all its runs pass), and per run.
- `--strict-by opts` makes strict runs strict through `SourceOpts` instead of the prefix. The results must be the same.
- Workers: one per CPU, 256 MiB stack each (oxc has no depth limit yet).

### Expectations

`expectations/test262-parse.<target-triple>.txt` lists the failing paths, sorted, relative to the
test262 root. `#` lines are comments.

| Result | Exit |
|---|---|
| Failures equal the file | 0 |
| A failure that is not in the file, or a listed path that passes now | 1 |
| Bad flag, missing file, I/O error | 2 |
| A test or include that cannot be read, is not UTF-8, has bad front matter, or makes the parser panic | 2 |

- Exit 2 for a file that the runner cannot check comes before the expectation file is read. So
  the expectation file cannot hide these errors, and `--update-expectations` does not write them.
- `--update-expectations` writes the file from the current failures. It needs a full run.
- Path prefixes as arguments (for example `test/annexB`) run a subset. Listed paths outside the subset are ignored.
- `--json <file>` writes a summary with counts and every failure.

## `index`

Writes the front matter of every test under `test/` as JSON: `path`, `flags`, `includes`,
`features`, `locale`, `negative`. The parser in `src/frontmatter.rs` reads only the YAML forms
that test262 uses, and gives an error for any other form. `scripts/verify_index.py` compares the
index with PyYAML on every file (usage in the script).

## Results at test262 d53ebc8b (M0 week 1)

| Count | aarch64-apple-darwin | x86_64-apple-darwin (Rosetta) |
|---|---|---|
| Negatives | 4,653 / 4,653 | 4,653 / 4,653 |
| Positives | 47,357 / 47,364 | 47,357 / 47,364 |
| Runs | 99,972 / 99,979 | 99,972 / 99,979 |
| Harness includes | 31 files, 62 runs, all parse | same |
| Index vs PyYAML | 53,616 files, 0 differences | n/a |

The 7 failures are `test/annexB/language/expressions/assignmenttargettype/*`. oxc 0.155 reports
`f() = 1` as an early error, but Annex B now makes it a runtime error (01-front-end §7).
