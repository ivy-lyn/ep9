# expectations/

Committed expectations and pinned benchmark lists ([08-repo](../docs/plan/08-repo.md) section 1;
[10-quality](../docs/plan/10-quality.md) sections 1-3). CI reports only differences against them.

| File | What | Written by |
|---|---|---|
| `test262-parse.<target>.txt` | Parse-only test262 results per target | `cargo run --release -p ep9_test262 -- parse` |
| `jetstream-m7.txt` | JetStream 3 M7 list (G4, M7 exit 3) | `node tools/bench/jetstream/lists.mjs` |
| `jetstream-m2.txt` | JetStream 3 M2 list (M2 exit 7) | `node tools/bench/jetstream/lists.mjs` |
| `jitless-set.txt` | JITLESS-SET: Octane-7, v8-v7 kernels, TS workload | by hand |
| `startup-set.txt` | STARTUP-SET | by hand |
| `short-run.txt` | SHORT-RUN | by hand |
| `hint-set.txt` | HINT-SET | by hand |
| `typed-kernels.txt` | TYPED-KERNELS | by hand |

Benchmark set files (`*-set.txt`, `short-run.txt`, `typed-kernels.txt`): one entry per line.

```
<id>  <status>  <source>
  status  pinned = every input is pinned in this repo today
          TBD    = an input or its pin does not exist yet (the comment above the entry says what)
  source  <pin>:<path>[,<path>...]   <pin> = the checkout of third_party/<pin>.commit; "-" when TBD
```

Lines that start with `#` are comments. The header of each file names its content, comparators and the gates it feeds.
