# ep9-test262

The test262 runner. Commands: `parse` (M0 week 1), `exec` (M0 week 3), `index`.

```
cargo run --release -p ep9_test262 -- parse          # exit 1 on a change vs the expectation file
cargo run --release -p ep9_test262 -- exec           # the engine end to end, same exit rules
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

## `exec`

Each test runs through the whole engine: `ep9_frontend::compile` -> `ep9_vm::Isolate::run_script`.

```text
parent (exec)                         worker processes (exec-worker), one per thread
  test/**/*.js, all 6 directories       one test path per stdin line
  "rel\n" ---------------------------->  front matter -> runs by flags (as for parse)
                                          per run: compile -> fresh Isolate
                                            -> $262 prelude, assert.js, sta.js,
                                               doneprintHandle.js (async), includes
                                            -> the test; a watchdog terminates it at --timeout
  <-------------- one result line ------  "rel  status  ok  runs  cause  detail"
  no line in 2 x timeout + 5 s: kill the worker, respawn, infrastructure error
  worker exits (crash, abort): infrastructure error with its stderr tail, respawn
```

| Item | Rule |
|---|---|
| Test set | Every `*.js` under `test/` (annexB, built-ins, harness, intl402, language, staging), except `*_FIXTURE*` |
| Runs | As for `parse`: `module`, `raw`, `onlyStrict` (`"use strict";\n` prefix), `noStrict`, else sloppy and strict |
| Harness | A `$262` prelude (`global`, `gc`), then `assert.js`, `sta.js`, `doneprintHandle.js` for `async`, then `includes`, each a separate sloppy script in the same isolate. `raw` tests get none of them |
| Isolate | A new one per run. `print` collects output for async tests |
| `negative.phase: parse` | Passes if compile gives an early SyntaxError of `negative.type`. A v0 "unsupported" error does not count |
| `negative.phase: runtime` | Passes if the uncaught exception is an object whose `constructor.name` is `negative.type` (INTERPRETING.md). The text does not count: a thrown string `"TypeError: x"` fails a `TypeError` test |
| `negative.phase: resolution` | Fails in v0 (no module linking) |
| `async` | Fails if `print` wrote a `Test262:AsyncTestFailure:` line, before or after a completion. Else passes if it wrote `Test262:AsyncTestComplete` exactly once (twice means `$DONE` ran twice: a fail) |
| Pass | Every run of the file passes. Counts are per file and per run |

Infrastructure errors are not test results. They are: a panic in the engine, a run that hits
the soft timeout (the watchdog calls `TerminateHandle::terminate`; the interrupt check stops
the run), a run that returns after the soft timeout (a native loop that does not poll the
interrupt check), a stuck worker (killed at the hard timeout), a worker crash (abort or
signal), an internal BytecodeGen error (`CodegenError::Internal`: a builder or verifier
rejection, in the test or a harness file), an unreadable file, bad front matter, or a
missing include. Any of them makes the run exit 2
before it compares or writes the expectation file. So no expectation file can hide them.

Failure causes: each failing file gets one cause, the first failing run's. The report lists
the top `--causes` (default 20). A cause is the unsupported feature, the "not implemented"
item, the uncaught error message (`ReferenceError: Symbol is not defined`), or
`TypeError: undefined is not a function (<callee>)` with the callee text from the source.

`expectations/test262.<target-triple>.jit.txt` (decision NAMING; mode `jit`, the only mode
before A3) lists the failing paths, as for `parse`. `--update-expectations` writes it from a
full run without infrastructure errors.

`--gc-stress <n>` gives every isolate GC-stress (a collection before every n-th allocation)
and the heap verifier after each collection; a verifier error panics, so it is an
infrastructure error.

Test hook: `EP9_TEST262_FAULT=<kind>:<substr>,...` makes a worker fail on matching paths:
`panic`, `abort`, `hang`, `late` (the run returns after the soft timeout) or `internal` (an
internal BytecodeGen error for a test path or `harness/<file>`). Only `tests/exec.rs` uses
it.

### Results at test262 d53ebc8b (M0 week 3, aarch64-apple-darwin, release, 18 workers)

| Directory | Files passed | Runs passed |
|---|---|---|
| language | 8,681 / 23,726 (36.59%) | 16,321 / 44,497 |
| built-ins | 614 / 23,823 | 1,179 / 47,341 |
| annexB | 319 / 1,086 | 344 / 1,377 |
| intl402 | 0 / 3,382 | 0 / 6,764 |
| staging | 128 / 1,483 | 232 / 2,783 |
| harness | 25 / 116 | 50 / 232 |
| All | 9,767 / 53,616 (18.22%) | 18,126 / 102,994 |

Wall time 4.7 s. 0 infrastructure errors.

With `--gc-stress 1 --timeout 60` (a collection before every allocation, verifier after each):
the same 9,767 files pass, 0 panics, 0 verifier errors, 0 crashes, wall time 155.7 s. 4 tests
time out. They also fail without GC-stress: 3 `language/expressions/call/tco-*` tests (no
tail calls, D7) and `staging/sm/String/string-upper-lower-mapping.js`.

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
