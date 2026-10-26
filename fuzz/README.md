# ep9 fuzz targets

cargo-fuzz (libFuzzer) targets for the front end, in their own workspace on the nightly of
`rust-toolchain.toml` (the root `Cargo.toml` excludes `fuzz`). Plan: docs/plan/10-quality.md
section 4 ("Front end (source -> verified bytecode)": no panics, no aborts, the bytecode
verifier passes); budget 24 CPU-h at M1 (plan v9 V9-D24).

```text
input bytes ──> target (fuzz_targets/*.rs) ──> ep9_frontend compile entry ──> VerifiedUnit
                                                                                │
                      to_bytes ──> CompiledUnit::from_bytes (decode + verify) ──┘ same bytes?
crash = a panic or an abort anywhere (also on the front-end worker thread), an Internal
        BytecodeGen error, or a unit that does not reload
```

| Target | Entry | Input form |
|---|---|---|
| `frontend_compile` | `ep9_frontend::compile` | byte 0: goal (0 sloppy script, 1 strict script, 2 module; mod 3), then the source |
| `function_ctor` | `ep9_frontend::compile_function_ctor` | byte 0: kind (Function, Generator, Async, AsyncGenerator; mod 4), then `params NUL body` (no NUL: all body) |
| `eval_context` | `ep9_frontend::compile_eval` | bytes 0-1: EvalContext bits (see the target's doc), then the eval text |

- `dict/js.dict`: a token for every nesting form of `crates/ep9_frontend/src/forms.rs` (G6:
  the 13 M0 forms, the 7 M1 forms, the other recursive productions), then general JavaScript.
- `seeds/<target>/`: small committed seeds: every forms.rs form at 8 levels (sloppy and strict;
  the module form as a module), the PRE-1 forms, and Function / eval context cases.
- `scripts/seed-test262.sh <test262> <corpus root> [max]`: a test262 sample (test/language, below
  4 KiB, at most 1,500 files, front matter cut) in each target's input form.
- `scripts/run.sh`: the only way to run a target here and in CI (below).
- `scripts/self-test.sh`: proves run.sh's output rule on a fixture crash, and (with a stub
  `cargo`) that a libFuzzer `slow-unit-*` file is not counted as a crash.

## Rules

- **No input bytes leave the run** (the repo is public; plan v9 rule 14, V9-A17). libFuzzer and
  cargo-fuzz print a crash input as text, hex, Base64 and Debug bytes. `scripts/run.sh` sends all
  fuzzer output to `logs/<target>.log` and prints only the crash count, the SHA-256 of each
  crash file (and of each `slow-unit-*` file, which is listed but does not fail the run) and
  `stat::` numbers. CI uploads no log, no corpus and no crash file;
  `fuzz.yml` uploads a summary JSON of numbers and hashes. Crash files (`artifacts/`), corpora
  (`corpus/`) and logs (`logs/`) are git-ignored.
- **The Mac builds and replays only** (plan v9 rule 23, V9-A6): `run.sh build` and
  `run.sh <target> replay <dirs>` (`-runs=0`: each input once, no mutation). Mutation runs go to
  the CI `fuzz-smoke` job (60 s per target, every PR) and to `fuzz.yml` (dispatch, the
  orchestrator).
- AddressSanitizer is on (cargo-fuzz default; oxc has unsafe code). The front-end worker stack
  (crates/ep9_frontend/src/stack.rs) is sized from the debug-assertion frame costs, and a
  replay of every forms.rs form at 3,000 to 25,000 levels (up to the caps) passes under ASan.

## Commands

```sh
# from the repo root; cargo-fuzz 0.12.0 (cargo install cargo-fuzz --version 0.12.0 --locked)
bash fuzz/scripts/run.sh build
bash fuzz/scripts/seed-test262.sh third_party/test262 fuzz/corpus
bash fuzz/scripts/run.sh frontend_compile replay fuzz/corpus/frontend_compile fuzz/seeds/frontend_compile
bash fuzz/scripts/self-test.sh
# CI / campaign host only (mutation):
bash fuzz/scripts/run.sh frontend_compile 60 fuzz/corpus/frontend_compile fuzz/seeds/frontend_compile
FUZZ_FORK=4 FUZZ_SUMMARY=s.json bash fuzz/scripts/run.sh eval_context 7200 fuzz/corpus/eval_context fuzz/seeds/eval_context
```

A crash found in CI is reproduced off GitHub (the orchestrator's triage host, never the Mac):
the same commit, the same seed corpus and the libFuzzer seed that run.sh prints
(`INFO: Seed: <n>`) replay a single-process run; the SHA-256 tells when the same input is
found. The fix gets a regression test in the crate; the input goes into the repo only once the
fix is public.
