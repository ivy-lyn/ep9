# SP-tsrs-win, M0 week 1 (2026-10-20): NOT RUN

- Spike: [09-milestones](../plan/09-milestones.md) section 4 row SP-tsrs-win; task: [08-repo](../plan/08-repo.md) section 5 item 7.
- Gate: `tsrs_cli` builds on x86_64-pc-windows-msvc and passes the smoke tests ([`.github/scripts/tsrs-smoke.sh`](../../.github/scripts/tsrs-smoke.sh)) with ≤ 2 local patches.
- **Result: none yet.** The week-1 check "SP-tsrs-win result recorded" is not met.

## Status

| Item | State |
|---|---|
| CI job | `sp-tsrs-win` in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) (windows-2025, `continue-on-error`). It writes the log to the job summary and to the artifact `sp-tsrs-win-log` |
| CI runs so far | 0. The repo has no commit and no remote. Pushing is a user decision |
| Pins | tsrs `f527e984d6e4d7141b387361bf74180acd6949d6` (`third_party/tsrs.commit`); Rust 1.99.1 (ep9 and tsrs `rust-toolchain.toml`) |
| Local patches | 0 so far |

## Local attempt (macOS arm64 host): inconclusive

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

## Next step

1. Run CI once (after the push).
2. Copy the job summary of `sp-tsrs-win` into a new section here: runner image, build result, smoke results, log tail, patches needed.
3. On a fail, D2 applies: `ep9 check` and `ep9c build --hints` print "not supported" on Windows ([09-milestones](../plan/09-milestones.md) section 4).
