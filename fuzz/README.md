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
- `scripts/record.sh`: writes one `ep9-fuzz/v1` record (below) from a run.sh summary or from a
  Fuzzilli box session.
- `scripts/self-test.sh`: proves run.sh's output rule on a fixture crash, that (with a stub
  `cargo`) a libFuzzer `slow-unit-*` file is not counted as a crash, and the record rules for
  both tools (with a stub FuzzilliCli for `infra/fuzz/run.sh`).

## Rules

- **No input bytes leave the run** (the repo is public; plan v9 rule 14, V9-A17). libFuzzer and
  cargo-fuzz print a crash input as text, hex, Base64 and Debug bytes. `scripts/run.sh` sends all
  fuzzer output to `logs/<target>.log` and prints only the crash count, the SHA-256 of each
  crash file (and of each `slow-unit-*` file, which is listed but does not fail the run) and
  `stat::` numbers. CI uploads no log, no corpus and no crash file;
  `fuzz.yml` uploads one `ep9-fuzz/v1` record per target (names, numbers and hashes). Crash
  files (`artifacts/`), corpora (`corpus/`) and logs (`logs/`) are git-ignored.

| Place | May go out | Never goes out |
|---|---|---|
| GitHub job log, step summary, artifact | counts, SHA-256, `stat::` numbers, the `ep9-fuzz/v1` record | crash files, corpus, fuzzer log |
| Cloudflare fuzz box | everything stays on the box | - |
| Repo | counted records (hashes only, at M1 exit); a regression test only in the fix commit, once the fix is public | raw crash inputs before the fix; Fuzzilli logs |
| PR text, commit message, decisions log, review focus text | hashes and triage group ids | program text of a crash |
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

A crash found in CI is reproduced off GitHub (on a fuzz box, never the Mac; the input is never
uploaded, not even encrypted; plan v10 V10-D26): the same commit, the same seed corpus and the
libFuzzer seed that run.sh prints (`INFO: Seed: <n>`) replay a single-process run; the SHA-256
tells when the same input is found. The fix gets a regression test in the crate; the input goes
into the repo only once the fix is public.

The triage box runs the `infra/fuzz` image (built from a `git archive` of the crash's commit,
so `/src` is that tree): it holds the fuzz nightly of `rust-toolchain.toml` here and
cargo-fuzz 0.12.0. On the box (an orchestrator run, plan v9 rule 17):

```sh
cd /src
bash scripts/sync-test262.sh                                   # the seed corpus source (network)
bash fuzz/scripts/seed-test262.sh third_party/test262 fuzz/corpus
bash fuzz/scripts/run.sh build <target>
# single process, the CI seed, until the crash file's SHA-256 shows up (or the time ends)
bash fuzz/scripts/run.sh <target> <seconds> fuzz/corpus/<target> fuzz/seeds/<target> -- -seed=<n>
```

The crash file stays on the box (`fuzz/artifacts/<target>/`); only its SHA-256 and the triage
notes leave it. Fetch the file to the private scratchpad (never the repo or a public log) only
to debug the fix, then destroy the box.

## Records (`ep9-fuzz/v1`)

`cargo xtask m1-exit --fuzz <record>...` reads them (M1 exit item 5). One record per
cargo-fuzz target run (`fuzz.yml`) or per Fuzzilli box session (`infra/fuzz/run.sh`), one JSON
line, written by `scripts/record.sh`:

| Field | cargo-fuzz (`fuzz.yml`) | Fuzzilli (`infra/fuzz/run.sh`) |
|---|---|---|
| `schema`, `tool` | `ep9-fuzz/v1`, `cargo-fuzz` | `ep9-fuzz/v1`, `fuzzilli` |
| `campaign` | `gh<run id>-<attempt>-<target>` | `fz-<UTC date>-<box>-<session>` (`FUZZ_BOX`, `FUZZ_SESSION`) |
| `target` | the cargo-fuzz target | `reprl`, or `reprl-jitless` for the jitless ep9 |
| `commit` | the checked-out HEAD | `/opt/ep9/SOURCE` (the image build arg `EP9_SOURCE_COMMIT`), else `unknown` |
| `dirty` | `git status --porcelain --untracked-files=no` before the build or after the run is not empty | the build arg `EP9_SOURCE_DIRTY`; **no field** when unknown |
| `cpu_hours` | the measured run seconds x `-fork` workers / 3600 | the measured FuzzilliCli seconds x `JOBS` / 3600 |
| `counted` | the dispatch input `counted` (default false) | `COUNTED` (default false) |
| `crashes` | `[{id, state: "open"}]`, id = the crash file SHA-256 | id = the SHA-256 of the triage key (`infra/fuzz/triage.sh --ids`) |
| `binary_sha256` | the target binary | the ep9 that ran (`/opt/ep9/bin/ep9` or `ep9-jitless`) |

Extra fields: `seconds`, `workers`, `runner`, `run` (run.sh's summary) and, when a run that
was asked to count ended abnormally (libFuzzer exit not 0 with no crash file, FuzzilliCli exit
not 0, no binary hash), `counted: false` with `uncounted_reason`. m1-exit counts a record only
when it is `counted`, has a campaign id, says `"dirty": false` and is fresh at HEAD; a counted
run is pre-registered by a decisions-log row before it starts (plan v10 rule 41), and a
bug-finding run keeps `counted: false`. record.sh refuses (exit 2) any string that is not a
name, a hash or a fixed text, so no input text can reach a record.

## Fuzzilli (REPRL)

Fuzzilli (Google Project Zero's JavaScript fuzzer) drives `ep9 --reprl`
(`crates/ep9_cli/src/reprl.rs`, plan v9 §5.5): the REPRL loop on fds 100-103, a fresh isolate
per script, the `fuzzilli('FUZZILLI_CRASH', n)` / `fuzzilli('FUZZILLI_PRINT', s)` builtin and
the SanitizerCoverage callbacks. The status word is 0 (ok) or 0x100 (JS error); a panic, a
wild store or a trap kills the child by a signal (a crash for Fuzzilli); a full GC heap exits
71 with no status word (a failed run, not a crash).

```text
third_party/fuzzilli.commit + patches/fuzzilli/0001-ep9-profile.patch
   │ scripts/sync-fuzzilli.sh (Swift >= 6.3)                infra/fuzz/build-ep9.sh (stable rustc)
   ▼                                                         ▼
FuzzilliCli --profile=ep9, REPRLRun ──── fds 100-103 ───▶ target/sancov/<triple>/release/ep9 --reprl
                                                           (--jitless: target/sancov-jitless/...)
```

Two builds of the child, both with SanitizerCoverage (plan v10 §3.6, MC-w15-13): the default
one (`jit` feature; image `/opt/ep9/bin/ep9`, record target `reprl`) and the jitless one
(`build-ep9.sh --jitless` = `-p ep9_cli --no-default-features`; image
`/opt/ep9/bin/ep9-jitless`, record target `reprl-jitless`). `infra/fuzz/run.sh` names the
target from line 2 of `ep9 --version`. The child's host functions are `print`, `fuzzilli` and
`gc(kind)`: `gc('minor')` is a minor GC (`Cx::collect_garbage_minor`; with the S1 heap only the
young cells), `gc()` and `gc('major')` a full GC, and any other argument a full GC too (ep9 does
not convert it). The ep9 profile calls all three forms.

```sh
# Mac: build and check only (no fuzz run, plan v9 rule 23)
bash scripts/sync-fuzzilli.sh                # prints the FuzzilliCli and REPRLRun paths
bash infra/fuzz/build-ep9.sh                 # prints the ep9 path
bash infra/fuzz/build-ep9.sh --jitless        # the jitless ep9
printf '42\nthrow 1\n' | <REPRLRun> <ep9> --reprl     # REPRLRun's own tests, then status 0 and 256
cargo test --locked -p ep9_cli reprl         # the fake-parent test (200 scripts, crashes, OOM,
                                             # gc kinds under each preset, the settings)
# infra/fuzz image only (Cloudflare, the orchestrator; never the Mac). Build the image with the
# EP9_SOURCE_COMMIT and EP9_SOURCE_DIRTY build args (infra/fuzz/Dockerfile header):
FUZZ_BOX=b1 FUZZ_SESSION=s1 infra/fuzz/run.sh 10 /work/fuzz-out   # exit 0: Fuzzilli exit 0,
                                             # coverage grew; hashes only; /work/fuzz-out/record.json
EP9="$EP9_JITLESS" FUZZ_BOX=b1 FUZZ_SESSION=s2 infra/fuzz/run.sh 10 /work/fuzz-out-jitless
infra/fuzz/triage.sh /work/fuzz-out/storage/crashes        # groups, each with its id
```

Settings of the child (environment): `EP9_REPRL_HEAP_MB` (512), `EP9_REPRL_STACK_MB` (64),
`EP9_REPRL_GC_STRESS` (0), `EP9_REPRL_PRESET` (`jit` or `jitless`; default the build's own;
`jit` in the jitless build is a usage error, exit 2); the ep9 profile forwards each one that
FuzzilliCli's environment sets (the child gets only the profile's environment).
`EP9_REPRL_GENERATIONAL` (1: S1, the generational heap, so `gc('minor')` is a minor GC; 0: S0)
defaults to 1; the profile does not forward it, so a Fuzzilli run is S1. The record of a
Fuzzilli session (`infra/fuzz/run.sh`) names the effective preset as `preset`: the
`EP9_REPRL_PRESET` setting, else the build's own; a setting the child would refuse stops
`run.sh` with exit 2. Fuzz hooks of
later features (fuse pop, tier-up, deopt) go in the per-isolate seat of `ep9_vm`'s
`ExecOptions`, never in a static (MODES-CHECK-w16 O7, V10-D27). A mutation run is an orchestrator run on Cloudflare `infra/fuzz` (at
most 4 boxes), asked for by a request file (plan v9 rule 17, V9-A6, V9-D24). Crash files and the
Fuzzilli log hold program text: they stay on the box or in the scratchpad, never in the repo
or a public log (rule 14).
