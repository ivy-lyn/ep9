# T0 workloads

Fixed JS programs for the T0 interpreter. Each one checks its own result and throws on a wrong one, so a wrong answer cannot pass as a fast run. The expected values come from node (`print` = `console.log`).

| File | Use | Content |
|---|---|---|
| `ci-v0.js` | perf CI v0 (instruction count, 10-quality §2) | fib, objects, strings, a loop; about 7 ms |
| `t0-loop.js` | H15 dispatch kernels | counted loops of small-integer arithmetic; predictable branches |
| `t0-branchy.js` | H15 dispatch kernels | xorshift32 stream; data-dependent branches, about half mispredict |
| `t0-call.js` | H15 dispatch kernels | JS-to-JS calls with 0-3 arguments, recursion, a closure |
| `t0-prop.js` | H15 dispatch kernels | named properties, class methods, array elements |
| `t0-gen.js` | generator kernel (P1-GEN-VM; not in `DEFAULT_WORKLOADS`) | generator resume and suspend: for-of over a generator, `next(v)` with sent values, `yield*` |
| `t0-dispatch.mjs` | runner | paired rounds of two configs, PERF-NOISE statistics (10-quality §3) |

```text
cargo build --release -p ep9_cli
node tools/bench/workloads/t0-dispatch.mjs --ep9 target/release/ep9                 # match vs sibling, 31 rounds
node tools/bench/workloads/t0-dispatch.mjs --ep9 old/ep9 --ep9-b new/ep9 --configs sibling,sibling
node tools/bench/workloads/t0-dispatch.mjs --ep9 target/release/ep9 --cachegrind    # Linux, valgrind
node --test tools/bench/workloads/t0-dispatch.test.mjs
```

Results: [docs/measurements/2026-10-21-t0-dispatch-x64.md](../../../docs/measurements/2026-10-21-t0-dispatch-x64.md).
