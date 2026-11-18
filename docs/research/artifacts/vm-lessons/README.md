# VM-lessons round: raw findings and verify verdicts (2026-11-18)

- These are the raw outputs of the 2026-11-18 VM-lessons round behind [R19](../../19-vm-aot-jit-lessons.md): 3 researchers (.NET with Roslyn; Kotlin with HotSpot and Leyden; Dart) and 3 adversarial verifiers, one per researcher.
- They were copied into the repo on 2026-11-18. Before the copy they existed only on the research host.
- `findings.json` is the round's `research-verify.json` with one change: the 3 `system` strings named the local clone folders on the research host. They now say "both local clones", "a sparse clone of JetBrains/kotlin" and "a sparse clone of dart-lang/sdk". The file also gained a final newline. Nothing else was changed.

## Layout of `findings.json`

A list of 6 objects.

| Index | Who | Keys |
|---|---|---|
| 0 | .NET researcher (Roslyn, CoreCLR, NativeAOT, Mono) | `system`, `findings` (39), `lessons` (15), `failures_and_reversals` (15) |
| 1 | Kotlin researcher (K2, Kotlin/Native, JS, Wasm, HotSpot, Leyden) | `system`, `findings` (37), `lessons` (17), `failures_and_reversals` (14) |
| 2 | Dart researcher | `system`, `findings` (39), `lessons` (15), `failures_and_reversals` (14) |
| 3 | .NET verifier | `checked` (39), `missed` (10) |
| 4 | Kotlin verifier | `checked` (37), `missed` (9) |
| 5 | Dart verifier | `checked` (39), `missed` (8) |

- A finding has `id` (F01...), `topic`, `claim`, `mechanism`, `numbers`, `sources` and `confidence`.
- A `checked` entry has the same `id`, a `verdict` (`confirmed` or `corrected`), the verifier's `source` cites and, when corrected, a `correction`. Totals: 88 confirmed, 27 corrected, 0 refuted.
- `missed` is a list of strings: items the verifier found that the researcher left out, each with its sources.
- `lessons` ids (L01...) are each researcher's own. They are not the lessons L1-L12 of R19, which come from the synthesis and the critic.

## Cite forms

- .NET: `dotnet/runtime@0968772:path` or `dotnet-runtime@0968772: path`, and `roslyn@14798cd` (dotnet/roslyn).
- Kotlin: `kotlin@c2edd73d:path` (JetBrains/kotlin).
- Dart: `sdk:path` and `dart-sdk@0f1a89e1 path` (dart-lang/sdk at 0f1a89e1).
- A verifier cite with a short path (for example `vm/tieredcompilation.cpp` or `runtime/vm/...`) is a path in the same upstream tree. R19 Appendix A prints every cite in the upstream form `owner/repo@rev path`.
- ep9 cites (`H33:25-35`, `D13:44`, `crates/...`) are at tag `m9-p5` (86e39c65).
