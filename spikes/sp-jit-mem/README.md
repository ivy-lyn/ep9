# SP-JIT-mem (M0 week 3, throwaway)

This spike tests H28 (and the `jit` feature of H27): generated code in the code range, W^X,
icache, unwind registration, backtraces and panics through generated frames.
Gate source: [docs/plan/09-milestones.md](../../docs/plan/09-milestones.md) section 4.
Results and numbers: [docs/measurements/2026-10-21-sp-jit-mem.md](../../docs/measurements/2026-10-21-sp-jit-mem.md).

The reusable parts live in `ep9_platform` (features `jit` and `unwind`). This spike adds the
hand-assembled code, the checks and the cost bench.

## What runs

```text
main -> run_mode -> call_generated -> [outer] -> [inner] -> callback (Rust)
                                      generated code in CodeMemory, unwind data registered

outer(x, cb, inner): save x19/rbx; x19/rbx := 0xbeef; inner(x, cb)
inner(x, cb):        save x19/rbx; x19/rbx := x;      cb(x + 1) + x
```

Per W^X mode of the host (`WxMode::available()`):

| Check | Pass rule |
|---|---|
| `call` | `outer(5)` and `inner(5)` return 65 |
| `icache` | 1000 rewrites of the `inner` immediate; every call sees the new value |
| `bt-std` | `std::backtrace` from the callback lists: callback, inner (exact return address), outer (exact), `call_generated`, ..., `main` |
| `bt-raw` | `ep9_platform::unwind::walk_native_stack` gives both exact return addresses in order and at least 3 more frames |
| `callee-saved` | the system unwinder restores x19/rbx = 0xbeef in the outer frame (inner changed it) |
| `panic` | a panic in the callback unwinds through both generated frames into `catch_unwind`; the code still runs after it |
| `control-no-unwind-info` | without registration the walk does not restore 0xbeef (unix; Windows: SKIP, an unregistered x64 frame looks like a leaf) |
| `wx-toggle` (measure) | ns per window open + close, per window with a 4-byte write + icache flush, per flush alone |

## Modes

| Target | Modes | Code memory |
|---|---|---|
| macOS arm64 | `map-jit` (default), `protect` | `map-jit`: a chunk of the process code range (`CodeRange::process`, one `MAP_JIT` RWX mapping). `protect`: a plain reservation outside it (MAP_JIT pages cannot be mprotected) |
| macOS x86-64 (Rosetta) | `protect` | a chunk of the process code range |
| Linux arm64, x86-64 | `protect` (default), `dual-map` | a chunk of the process code range |
| Windows x64 | `protect` | a chunk of the process code range |

Until month 2 (P1) the code went to a per-cage code range right after a heap cage; the
week-3 results in docs/measurements/2026-10-21-sp-jit-mem.md were measured that way.

## Files

| File | Content |
|---|---|
| `src/code.rs` | `inner` and `outer` for aarch64, x86-64 SysV and Win64; their CFI and UNWIND_INFO |
| `src/walk.rs` | stack walk that also reads x19/rbx per frame (`_Unwind_GetGR`, `RtlVirtualUnwind`) |
| `src/main.rs` | checks, `std::backtrace` parser, cost bench, report |
| `gate.sh` | host gate (below) |

## Gate

`bash spikes/sp-jit-mem/gate.sh` runs: ep9_platform tests and clippy (`--features jit`), the spike's
clippy and tests, and the binary. On macOS arm64 it repeats all of it for x86_64-apple-darwin under
Rosetta. On a non-Windows host it builds and lints both crates for x86_64-pc-windows-msvc.

| Gate id | Rule |
|---|---|
| `jitmem-<target>` | every step for the target passes (`macos-a64`, `x64-darwin`, `linux-a64`, `linux-x64`, `win64`) |
| `unwind-backtrace/<target>` | every `bt-std`, `bt-raw`, `callee-saved` and `panic` line passes, in every mode |
| `wx-toggle-cost/<target>` | one `MEASURE` line per mode |

Env: `CARGO_TARGET_DIR`, `SP_JIT_ITERS` (bench iterations per batch, default 100000),
`SP_JIT_EXTRAS=0` (skip Rosetta and the Windows build).
