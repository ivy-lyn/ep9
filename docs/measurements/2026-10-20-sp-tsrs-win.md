# SP-tsrs-win, M0 week 1: PASS (first CI run 2026-10-20; recorded 2026-10-21)

- Spike: [09-milestones](../plan/09-milestones.md) section 4 row SP-tsrs-win; task: [08-repo](../plan/08-repo.md) section 5 item 7.
- Gate: `tsrs_cli` builds on x86_64-pc-windows-msvc and passes the smoke tests ([`.github/scripts/tsrs-smoke.sh`](../../.github/scripts/tsrs-smoke.sh)) with ≤ 2 local patches.
- **Result: PASS.** `tsrs_cli` builds on the Windows x64 runner and passes all 3 smoke tests with 0 local patches, in every CI run that had the job (12 of 12). The week-1 check "SP-tsrs-win result recorded" is met as of 2026-10-21.
- Review: [M0 spike go/no-go review](2026-10-21-m0-spike-review.md).

## 1. Latest run (the artifact)

| Item | Value |
|---|---|
| Run | CI [39518270586](https://github.com/Brooooooklyn/ep9/actions/runs/39518270586), job 118955576889 (`SP-tsrs-win (recorded, not blocking)`), conclusion success |
| Commit | `9b6cd6e` (PR #2 head). Its tree `18fcb648` is the tree of `main` at `dabc38a`, the week-4 base |
| Runner | label `windows-2025`; image `windows-2025-vs2026` version 20261018.275.1; Windows Server 2025 Datacenter 10.0.26100 |
| Toolchain | Rust 1.99.1 (x86_64-pc-windows-msvc), the channel of tsrs's `rust-toolchain.toml`; `scripts/sync-tsrs.sh` runs `cargo +1.99.1 build --release --locked -p tsrs_cli` |
| tsrs pin | `f527e984d6e4d7141b387361bf74180acd6949d6` (`third_party/tsrs.commit`, unchanged since `ea31dcb`) |
| Local patches | 0 (`third_party/patches/tsrs/` does not exist) |
| Build | 33 `tsrs_*` crates compiled; `Finished` in 8m 41s (rust-cache restored the dependency builds) |
| Smoke | 3 of 3 ok: `--version`, a correct file, TS2322 on a wrong file |
| Job time | 2026-10-20 23:57:07Z to 2026-10-21 00:06:33Z |

Artifact `sp-tsrs-win-log`, file `sp-tsrs-win.log` (SHA-256 `68701f26…86051863`), with the ANSI colour codes removed:

```text
    Updating crates.io index
   Compiling tsrs_core v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_core)
   Compiling tsrs_diagnostics v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_diagnostics)
   Compiling tsrs_execute v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_execute)
   Compiling tsrs_spanmap v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_spanmap)
   Compiling tsrs_ast v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_ast)
   Compiling tsrs_vfs v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_vfs)
   Compiling tsrs_scanner v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_scanner)
   Compiling tsrs_parser v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_parser)
   Compiling tsrs_module v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_module)
   Compiling tsrs_sourcemap v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_sourcemap)
   Compiling tsrs_tsoptions v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_tsoptions)
   Compiling tsrs_modulespecifiers v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_modulespecifiers)
   Compiling tsrs_printer v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_printer)
   Compiling tsrs_binder v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_binder)
   Compiling tsrs_astnav v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_astnav)
   Compiling tsrs_pseudochecker v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_pseudochecker)
   Compiling tsrs_checker v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_checker)
   Compiling tsrs_ipc v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_ipc)
   Compiling tsrs_contentmapper v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_contentmapper)
   Compiling tsrs_lsproto v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_lsproto)
   Compiling tsrs_transformers v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_transformers)
   Compiling tsrs_declarations v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_declarations)
   Compiling tsrs_compiler v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_compiler)
   Compiling tsrs_projectutil v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_projectutil)
   Compiling tsrs_ls v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_ls)
   Compiling tsrs_project v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_project)
   Compiling tsrs_fswatch v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_fswatch)
   Compiling tsrs_incremental v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_incremental)
   Compiling tsrs_api_codec v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_api_codec)
   Compiling tsrs_api_transport v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_api_transport)
   Compiling tsrs_api v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_api)
   Compiling tsrs_lsp v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_lsp)
   Compiling tsrs_cli v0.18.0 (D:\a\ep9\ep9\third_party\tsrs\crates\tsrs_cli)
    Finished `release` profile [optimized] target(s) in 8m 41s
tsrs:          /d/a/ep9/ep9/third_party/tsrs/target/release/tsrs
ep9-typecheck: skipped (/d/a/ep9/ep9/tools/ep9-typecheck does not exist before stage A1h)
ok:   --version: Version 7.1.0-dev (tsrs 0.18.0-ts7.1.0-dev.20260929, microsoft/TypeScript@b85298b6a81f)
ok:   ok.ts has no errors
ok:   bad.ts gave exit 2 and TS2322
```

## 2. Every run

| Runs | Commits | Build step | Smoke step |
|---|---|---|---|
| 12: 39458801246, 39476143311, 39498876200, 39500446793, 39501063919, 39502316082, 39505241524, 39513636457, 39514911809, 39515106992, 39516980035, 39518270586 | `ea31dcb` (2026-10-20 16:35Z) to `9b6cd6e` (23:57Z) | success in all 12 | success in all 12 |

- The push run on `main` at `dabc38a` (39519345205) started no job: "The job was not started because recent account payments have failed or your spending limit needs to be increased". It is not a test result.
- The job is `continue-on-error`, so a failure would not block CI. The step results above show that no step failed.

## 3. What the result means (D2)

- D2: "`ep9 check` and `ep9c build --hints` are unix-only until SP-tsrs-win passes". The `ep9 check` part (tsrs on Windows) passed. So `ep9 check` may support Windows when it is built.
- `ep9c build --hints` needs ep9-typecheck. The same gate checks it again at A1h (09 §4); until then it stays unix-only.
- The gate reopens no ADR (09 §4: "none").

## 4. Local attempt (macOS arm64 host, 2026-10-20): inconclusive

```
cd third_party/tsrs
cargo +1.99.1 check --locked -p tsrs_cli --target x86_64-pc-windows-msvc --target-dir <own dir>
```

| Output | Value |
|---|---|
| Exit | 101 |
| First error | `failed to run custom build command for libmimalloc-sys2 v0.1.63` |
| Cause | the build script compiles C with clang: `mimalloc.h:100:10: fatal error: 'assert.h' file not found`. This host has no MSVC C headers or Windows SDK |

This says nothing about the Windows runner. It only shows that a macOS host cannot cross-check tsrs for MSVC without a Windows SDK.

## 5. History

| Date | Change |
|---|---|
| 2026-10-20 | File created: "NOT RUN". No CI run yet (the repo had no commit and no remote) |
| 2026-10-21 | Result recorded from the CI artifact (§1, §2); D2 note (§3) |
