# H36: FP semantics in every backend

| Field | Value |
|---|---|
| Status | Draft (2026-10-20). Decided 2026-10-20 (math decision run); Q1-Q4 closed; Q5 closes with H06-Q3. Accept needs the review record ([README §3.1](README.md#31-review-record-p-1)) |
| Fix by | M0 wk2: `clippy.toml` + `xtask fp-check` stub. M0 wk3: `ep9_math` skeleton, `ep9_value::to_int32`. M1: CR functions, Math builtins, FP differential CI |
| Freeze point | M1 exit |
| Origin | R16-D12; plan status N (new) |
| Crates | `ep9_math` (new), `ep9_vm` (T0 arithmetic), `ep9_builtins` (Math, Number; one crate until M1a, 08 §2), `ep9_value` (ToInt32, NaN purification), `ep9_numconv` (planned; today `ep9_vm` `numconv.rs`), `ep9_frontend` (folds), `ep9_masm`, `ep9_baseline`, `ep9_opt` (emitted FP code), `tools/xtask` (`fp-check`) |
| Owner (plan role) | AOT senior (rules, lint, `fp-check`, M0 wk2); VM lead (`ep9_math`, M0 wk3-M1) |
| Validated by | `clippy.toml` + `xtask fp-check` (M0 wk2); `ep9_math` golden and hash tests on 4 targets and x86-64 with/without FMA; nightly CR oracle; FP differential CI (M1). No SP-* spike |
| Plan sources | [02 §6 H36](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §10](../plan/04-execution.md#10-floating-point-rules-h36), [06 §4](../plan/06-object-model-and-spec.md#4-numbers-and-bigint), [10 §5](../plan/10-quality.md#5-other-ci-checks) |

## Context

- JS fixes FP results for `+ - * / %` and `sqrt`: IEEE-754 doubles, no fused multiply-add, exact ToInt32.
- ECMA-262 does not fix `sin`, `pow` and the other libm-class functions. It recommends fdlibm but does not require it [SPEC]. test262 checks only special values and exact cases, such as `Math.log2(8) === 3` [T262].
- ep9 has six code kinds and three modes. G12 needs the same test262 pass set and the same checksums in each.
- A C/LLVM backend silently changes JS results: Porffor's clang build contracted `a*b+c` into an FMA [R10][R15]. D9 rules out LLVM and C backends. Risk RK41.
- No engine gives one answer today [M]:
  - V8 15.8 (d8) gives CR results for 13 functions through LLVM libc. It calls the host `std::pow` and `std::tanh` [V8].
  - Node 24 (V8 13.6, fdlibm) gives 3 results in 3 official builds. macOS arm64 and Linux x64 differ on 0.99% of `tan` inputs, because the compilers contract to FMA differently. Its `pow` is the host libm.
  - Firefox: fdlibm, host `sin`/`cos`/`tan`, host `pow` [SM]. Safari: host libm [JSC]. QuickJS, Boa, Nova, Brimstone: host libm [ENG].
  - macOS libm on arm64 and on x86_64 differ on 20.4% of `sin` inputs.
- A CR result is unique. Every CR implementation gives the same bits on every platform.
- Programs compare Math results exactly. JetStream `3d-cube` fails when `Math.sin` or `Math.cos` of 1, 3 or 5 degrees moves by 1 ULP [M].

## Decision

No FMA contraction. Exact IEEE-754 and ToInt32. One Rust Math, crate `ep9_math`, for all tiers, both AOT code kinds and every compile-time fold. Its target is the correctly rounded result. No tier, compiler or dependency calls the platform libm for a JS-visible value. NaN purification. Differential tests.

The "bit-equal to d8" column comes from one host (macOS arm64) and about 15k inputs per function (2026-10-20 run [M]). It shows agreement, not proof of CR; the M1 worst-case tests cover proof.

| JS operation | `ep9_math` | Rule | Bit-equal to d8 15.8 [M] |
|---|---|---|---|
| `sin cos tan asin acos atan2 exp expm1 log log1p log2 log10 cbrt` | same names | CR | 100% |
| `atan` | `atan` | CR | 99.95% (V8 atan not CR yet) |
| `sinh cosh asinh acosh atanh` | same names | CR | 83-90% (V8: fdlibm formulas over CR exp/log) |
| `tanh` | `tanh` | CR | 99.9% on macOS (V8 calls host `std::tanh`) |
| `**`, `Math.pow` | `pow_js` | JS special cases, then CR | 99.8% (V8 calls host `std::pow`) |
| `Math.hypot` | `hypot_js` | Port of V8 `MathHypot` (math.tq); IEEE ops only | 100%, also node 24 |
| `Math.sqrt` | `f64::sqrt` | IEEE CR (hardware) | 100% |
| `%` | `fmod` | Exact; pure Rust; never C `fmod` | exact |
| `fround abs sign floor ceil trunc round min max clz32 imul` | `ep9_math` or core ops | Exact | exact |
| `f16round`, `sumPrecise` | `ep9_numconv`, xsum (06 §4-5) | Exact | exact |

- **Code.** `ep9_math` is pure Rust with no crates.io dependencies. The CR functions port CORE-MATH algorithms (MIT) [CM]. The seed is pxfm 0.1.30 (BSD-3-Clause OR Apache-2.0; ep9 takes the BSD-3-Clause side), normalized to one code path. FMA appears only as an exact operation (`exact::fma_exact`, `exact::two_prod`). Hardware/software dispatch is allowed only between exact implementations. `unsafe` only in the fma dispatch.
- **Provenance.** pxfm carries no upstream provenance notes (its file headers claim sole BSD-3 copyright). Before a function is vendored, it is diffed against the CORE-MATH C source: where it matches, CORE-MATH is named as the primary origin; else pxfm. Each ported file keeps its original lines and adds "Ported for ep9 from ...". `ep9_math` has no NOTICE file: the root `THIRD-PARTY-LICENSE` gets entries for pxfm (BSD-3), CORE-MATH (MIT), LLVM libc (Apache-2.0 WITH LLVM-exception), V8 `MathHypot` (BSD-3) and libm/musl `fmod` (MIT) in the PR that ports each ([12-governance §1](../plan/12-governance.md#1-licence)).
- **pow.** One function for `**` and `Math.pow`:
  1. `y` is NaN: NaN. 2. `y` is ±0: 1. 3. `x` is NaN: NaN. 4. `y` is ±∞ and `|x|` = 1: NaN.
  5. `y` = 2: `x*x`. `y` = 0.5: `x` = -∞ ? +∞ : `sqrt(x + 0)`. `y` = 1: `x`. `y` = -1: `1/x`. Each equals the CR result.
  6. Else: CR `pow` with the C99 Annex F special cases.
  - No `powi`, no repeated multiplication, no host `pow`.
- **fmod.** JS `%` uses `ep9_math::fmod`: a pure-Rust exact remainder (port of musl `src/math/fmod.c` at v1.2.6) with the JS special cases. Rust `%` on floats lowers to C `fmod` on every target, so engine code does not use it.
- **Emitters.** Inline only: `sqrt`, `abs`, `fround`, `floor`/`ceil`/`trunc` (x64 `roundsd` needs SSE4.1; arm64 `frint*`), `round` (the `ep9_math::round_js` algorithm), `sign`, `min`/`max`, `clz32`, `imul`, ToInt32 (arm64 FJCVTZS if JSCVT), and `**` with a constant exponent in {±0, 1, 2, -1, 0.5} as in step 5. All else calls `ep9_math` through the external-reference table. Folds call `ep9_math`. No FMA encoder. AOT units record SSE4.1 and JSCVT use in their target features (H35).
- **ToInt32.** `ep9_value::to_int32` (and `to_uint32`), exact by bit manipulation, never a saturating `as i32`.
- **FP environment.** Round-to-nearest-even, FTZ and DAZ off, exceptions masked. Debug builds assert MXCSR/FPCR at Isolate entry and after each Node-API callback.
- **Dependencies.** No dependency computes a JS-visible number with the host libm. `oxc_semantic` folds TS enum `**` and `%` with host `powf`/`fmod`; ep9 never uses those values. BytecodeGen folds TS enums with `ep9_math` (H3).

## Rules

| Rule | Check | From |
|---|---|---|
| No std libm methods, `powi`, `rem_euclid`, `div_euclid`, `mul_add` or `algebraic_*`; only `ep9_math::exact` may allow `mul_add` | Root `clippy.toml` `disallowed-methods`; clippy `-D warnings` on 4 targets. `crates/ep9_builtins/clippy.toml` repeats the list, because clippy reads the nearest file only (test `clippy_toml_repeats_the_root_bans`) | M0 wk2 |
| No reference to a banned libm symbol (transcendental math.h names, `fma`, `fmod`, `remainder`, `__powidf2`, f/l variants) outside `expectations/fp-check-allow.toml` | `cargo xtask fp-check`: the post-LTO object of `ep9` (`--emit=obj`, `-C linker=true`) on 5 targets plus x86_64 Linux with `x86-64-v3`, every PR | M0 wk4 |
| No FMA instruction outside `ep9_math::*` symbols; `ep9_math` public functions are `#[inline(never)]` | `xtask fp-check` disassembly scan | M0 wk4 |
| `libm`, `pxfm`, `core_maths`, `micromath` stay out of the engine graph unless allowlisted | `xtask fp-check` reads `cargo tree -e normal,build --target all` | M0 wk4 |
| `ep9_math` gives the same bits on every target | Golden table + 1M-input SHA-256 per function on 4 targets and x86-64 `x86-64`/`x86-64-v3` (not built yet: `ep9_math` has value tests only; H36-Q6) | M0 wk3 |
| CR functions are CR | Golden bits from mpmath (>= 160 bits) + CORE-MATH worst cases; nightly 200k fresh inputs per function vs mpmath 1.3.0 | M1 |
| ep9 vs V8 | Nightly vs pinned d8: bit-equal on the 100% rows; <= 1 ULP `atan tanh pow`; <= 2 ULP `sinh cosh asinh acosh atanh` (`expectations/math-ulp.toml`) | M1 |
| nbody = -0.169193895234998, raytracer = 576888 in every tier and code kind | FP differential checksums, every PR | M1 |
| Math results equal T0 in every tier and code kind; every inline sequence equals `ep9_math` on the golden inputs | FP differential suite; FP fuzz nightly | M1 |
| Generated FP programs give node's output, within `math-ulp.toml` for libm-class calls | FP differential fuzz | M1 |

## Consequences

- **Gains.** Same bits on every platform, compiler, tier and mode. Equal to V8 main on 13 libm functions plus `hypot` and `sqrt` today, and to every future CR engine. `10 ** k === 1ek` for k in -30..30; fdlibm `pow` fails 10 of these, Rust `libm` 8 [M].
- **Costs.** ep9 owns about 0.7 MB of Rust source and 1-1.25 PM of work (est.). On M5 Max the seed costs 1.2x macOS libm on `sin` (5.5 vs 4.6 ns), 2.2x on `pow` (10.8 vs 4.9 ns), 2.4x on `atan2` [M]. Results differ from V8 main by at most 1-2 ULP on 10-17% of `sinh cosh asinh acosh atanh` inputs and on about 0.1-0.2% of `tanh` and `pow` inputs, and from node 24 by 1-2 ULP on 0.03-21% of inputs per function.
- **Follow-ups.** `ep9_numconv` owns exact number-to-string (06 §4). BytecodeGen folds TS enums with `ep9_math` (H3). `xtask fp-check` and the root `clippy.toml` landed in M0 wk4 (Change log).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R10] | C/LLVM changes JS FP results unless FMA contraction is off | [10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R15] | Porffor nbody -0.16919383316763523 vs -0.169193895234998 | [15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [M] | 2026-10-20 run: 365,667 inputs; d8 15.8.24, node 24 (macOS, Linux arm64/x64), Rust std, libm 0.2.16, pxfm 0.1.30; mpmath CR reference on 84,613 inputs | [evidence_tables.txt](../research/artifacts/decide/math/evidence_tables.txt) (copy of the 2026-10-20 scratch file) |
| [V8] | `src/base/ieee754.cc` (LLVM libc; `std::tanh`), `src/numbers/ieee754.cc` (`std::pow`), `math.tq` (MathHypot); commits 55e89435d5f1, c1486295ae5b, 08be4445f60b, 43f04a52f24d | https://github.com/v8/v8 |
| [SM] | `js/src/builtin/Math.cpp`: fdlibm, native sin/cos/tan, std::pow + powi | https://github.com/mozilla-firefox/firefox |
| [JSC] | `MathCommon.cpp`/`.h`: std:: for all; integer pow loop 0..1000 | https://github.com/WebKit/WebKit |
| [ENG] | QuickJS `quickjs.c:14711`; Boa `math/mod.rs:772`; Nova `math_object.rs:1398`; Brimstone `numeric_operations.rs:83` | local clones |
| [CM] | CORE-MATH: MIT, CR binary64 functions incl. all above | https://core-math.gitlabpages.inria.fr/ |
| [SPEC] | Math functions are implementation-approximated; fdlibm recommended | tc39/ecma262 `spec.html` |
| [T262] | Math tests check special values and exact cases | test262 `test/built-ins/Math` |

**Local check (2026-10-20, rustc 1.99.1):**

| Rust source | aarch64 | x86_64 baseline | Note |
|---|---|---|---|
| `a * b + c` | `fmul` + `fadd` | `mulsd` + `addsd` | rustc does not contract on its own |
| `a.mul_add(b, c)` | `fmadd` | call `fma` | banned outside `ep9_math::exact` |
| `a.algebraic_mul(b).algebraic_add(c)` (stable) | `fmadd` | `vfmadd213sd` (haswell) | contracts; banned |
| `a.sin()` | `b _sin` | call `sin` | host libm |
| `a.powi(n)` | `b ___powidf2` | call `__powidf2` | compiler-rt; not CR |
| `a % b` | call `fmod` | call `fmod` | host libm; use `ep9_math::fmod` |
| `a.floor()` | `frintm` | call `floor` | exact; allowed |
| `3e9_f64 as i32` | 2147483647 | n/a | saturating; JS `3e9 \| 0` is -1294967296 |

## Validation

- No spike reopens H36. M0 wk2: `clippy.toml` + `xtask fp-check` stub (AOT senior). M0 wk3: `ep9_math` skeleton. Built in M0 wk4: the root `clippy.toml`, the full `fp-check` (a CI job on every PR) and `ep9_math` v0 (Change log). M1: CR functions, FP differential CI ([10-quality](../plan/10-quality.md#5-other-ci-checks)).

## Open questions

| ID | Question | Status |
|---|---|---|
| H36-Q1 | Ban std libm methods too? | Closed 2026-10-20: yes, plus `algebraic_*`, `rem_euclid`, `div_euclid`; symbols and FMA checked by `xtask fp-check` |
| H36-Q2 | Ban `powi`? | Closed: yes (clippy + `__powidf2` symbol); `**` and `Math.pow` use `pow_js` |
| H36-Q3 | Own port or pinned crate? | Closed: own vendored CR port in `ep9_math` (CORE-MATH algorithms, pxfm seed, no NOTICE file) |
| H36-Q4 | ToInt32 helper and FJCVTZS | Closed: `ep9_value::to_int32`; FJCVTZS allowed when JSCVT is present (JIT: CPU; AOT: unit target) |
| H36-Q5 | Canonical NaN | Owned by H06-Q3. H36 needs no particular pattern |
| H36-Q6 | Rules row 5 (golden table + 1M-input SHA-256 per function) is not built: `ep9_math` has value tests only (for example `fmod_edges`, `pow_js_spec_special_values`, `soft_vs_hw_quick`), and its test-only `ep9_bytecode` dependency (for SHA-256) has no user. When does it land, and does it cover the exact v0 functions? | Open (P2-ACCEPT-1, 2026-10-22). Needed before the first CR function (M1); owner VM lead |

## Reopens if

- An FP differential mismatch that the rule cannot explain (RK41).
- D9 changes to allow an LLVM AOT backend; then contraction must be off there.
- A libm or FMA path is found outside the checks, or `fp-check-allow.toml` grows past 5 entries.
- After tuning, a CR function costs > 1.5x V8 on a Math-heavy gate benchmark.
- V8 keeps a non-CR result for >= 4 releases and real code depends on its bits.
- TC39 or test262 adds precise expectations that differ from CR, or V8 changes its hypot algorithm (re-port to keep bit-equality).
- Normalizing the pxfm seed fails the CR oracle on CORE-MATH worst cases and cannot be fixed in 1 week per function, total effort passes 1.5 PM, or a provenance review rejects pxfm-derived code: then port CORE-MATH C directly to Rust (same outputs).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 04 §10, 06 §4; local rustc check added | ADR agent |
| 2026-10-20 | fill | Q1-Q4 closed: CR target, `ep9_math`, pow/hypot/fmod rules, inline list, `clippy.toml`, `xtask fp-check`, tests | math decision run |
| 2026-10-20 | rewrite (kind set 2026-10-22; was fill) | Critic changes: no NOTICE file in `ep9_math`; provenance through `THIRD-PARTY-LICENSE` (pxfm, CORE-MATH, LLVM libc, V8 MathHypot, libm/musl fmod) and a CORE-MATH diff before vendoring; the d8 column is from one host | apply agent |
| 2026-10-21 | clarify | Evidence [M] links the repo copy of the math tables in `docs/research/artifacts/decide/math/` instead of the session scratch path | docs-sync agent |
| 2026-10-21 | clarify | Clippy's `disallowed-methods` does not see float `%` or `Rem::rem` (no method call; both lower to the C `fmod`; probe on rustc 1.99.1), so the `fmod` symbol scan of `fp-check` is the guard for `%`. The root `clippy.toml` also reaches `spikes/*` through clippy's parent-directory lookup: `sp-jit-mem` (its gate runs clippy) has no banned call; `sp-dispatch` calls `ln`, `exp`, `powi` and `log2` (src/main.rs:367, 385, 660) but its gate runs no clippy. A spike that adds clippy adds its own allow | P1-MATH (M0 wk4) |
| 2026-10-21 | fill | M0 wk4 build. (1) Root `clippy.toml` = the prototype list; the one allow is on `HwFma` in `crates/ep9_math/src/exact.rs`. (2) `fp-check` is real: it builds the post-LTO object of `ep9` per target (`cargo rustc --release --target T -p ep9_cli --bin ep9 -- --emit=obj=F -C linker=true`; one Linux or macOS host covers all targets), reads `llvm-nm -u` and `llvm-objdump -dr -C` (caller names, FMA mnemonics), runs `cargo tree` for second math crates (a crate allow entry is not built yet; none is needed) and scans `crates/**` for `disallowed_methods` allow sites; exit 0/1/2. (3) Allow entry 1: {`fmod`, `pow`} in `oxc_semantic::ts_enum::eval::evaluate_expression`, all targets, reopen at H3. It is the only finding on all 6 builds today, and the objects have no FMA instruction. (4) FMA: `HwFma` (the only `mul_add`) and `SoftFma` (libm 0.2.16 generic fma, musl origin); x86-64 entry points are compiled twice (`#[target_feature(enable = "fma")]` copy and soft copy, picked by `is_x86_feature_detected!`), the crate's only `unsafe`. SoftFma equals the hardware on 10^7 random plus edge inputs. (5) `fmod` is a direct port of musl `src/math/fmod.c` (v1.2.6), not of the libm generic fmod (a different algorithm). (6) `pow_js` v0: steps 1-5, the special values of step 6, and the M0 exact integer path; it returns `None` (`**` throws "not implemented") where the CR core is needed. CR `exp`, `log` and `pow` move to month 2, inside M1 (Rules row 6) | P1-MATH (M0 wk4) |
| 2026-10-21 | rewrite (kind set 2026-10-22; was fill) | Correction to item (4) of the M0 wk4 row: `SoftFma` was not exact. The libm 0.2.16 generic `fma` (and so the first port) loses the sticky bit when `z` is 65 to 127 bits above `x * y`: it reads `rlo` after overwriting it, where musl `fma.c` uses one expression. On a rounding tie plus a tiny tail it then rounds to even: `fma(0x1.b4d42bf5242b8p+0, 0x1.2c0db0d0ff40dp-1, 0x1p53)` gave 2^53, not 2^53 + 2. Upstream: rust-lang/compiler-builtins#1353 (open, no fix on master). `SoftFma` is now libm 0.2.16 plus that one fix ("ep9 fix" in `exact.rs`). The 10^7 corpus reached that branch 2.17M times, with a wrong sticky bit 82,435 times, but never on a tie; it now also has near-tie inputs: 19,651 of 10,110,592 differed from the hardware before the fix, 0 after (macOS arm64; Rosetta x86-64 against the C `fma`: 0). New tests: `soft_fma_sticky_of_shifted_product`, `soft_vs_hw_near_ties` | P1 fix agent (M0 wk4) |
| 2026-10-22 | clarify | Crates line: `ep9_builtins` (one crate until M1a) and `ep9_numconv` (planned; today a module of `ep9_vm`). Rules row 1 names the `ep9_builtins` copy of the bans and its test. Rules row 5 says that its check is not built yet; new Open question H36-Q6. Follow-ups and Validation say that `clippy.toml` and `fp-check` landed in M0 wk4. No check changes | P2-ACCEPT-1 |
| 2026-10-22 | clarify | Kind cells set to rewrite (README §3.3) in two rows. 2026-10-20 "Critic changes": it replaced an option that the Q1-Q4 fill had chosen (a NOTICE file in `ep9_math`; decisions log H36-libm-src: "change taken"). 2026-10-21 "Correction to item (4)": it changed the result bits of the built `SoftFma` (code change in `exact.rs`; no Open question) | P2-ACCEPT-1 (critic row); P2 review fixer (Codex A2: SoftFma row, kind cells) |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P2-ACCEPT-1 (did not write the ADR) | Decision = the 02 §6 H36 row plus the closed Q1-Q4. Rules checked against the root and `ep9_builtins` `clippy.toml`, `tools/xtask/src/fpcheck.rs`, `expectations/fp-check-allow.toml` (1 entry) and `ep9_math` (every public fn `#[inline(never)]`; the one `disallowed_methods` allow is in `exact.rs`). The `pow` steps 1-5 and `3e9 \| 0` match node 24. R1: the Rules row 5 check is not built (H36-Q6 added). R2: Crates line names (fixed). R3: stale "fp-check lands later" text (fixed). R4: the 2026-10-20 critic row is a rewrite (kind cell set). R5 (Codex A2, fixed by the P2 review fixer): the 2026-10-21 `SoftFma` correction row is a rewrite, not a fill: it changes `SoftFma` result bits, changes code and fills no Open question (kind cell set). Other kinds: agree | ccaf5df |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | A2 (medium), H36 part: the 2026-10-21 SoftFma correction row (7ca6ef8 changed `SoftFma` result bits, 0x4340000000000000 -> 0x4340000000000001) was signed "fill" ("Other kinds: agree"), and the critic row kept its old kind behind an appended clarify row. Reproduced by the P2 verifier with Python `math.fma`: real, category (b). Fixed in 0c68c7d (M2-P2-A2, M2-P2-RF-A2-SOFTFMA): both kind cells rewrite, R5 in row 1 | decisions-log M2-P2-REVIEW, M2-P2-A1, M2-P2-A2; Codex text outside the repo: scratchpad m0m2/p2-codex-bcrhkdqte.md |
| 3 Verifier | P3-ACCEPT-2 (wrote none of the rows it checks) | Checked at e719ffa. A2 fix: the kind cells of H36:140 (critic) and H36:144 (SoftFma) read `rewrite (kind set 2026-10-22; was fill)`, and the clarify row H36:146 names both. `soft_fma_sticky_of_shifted_product` (`crates/ep9_math/src/exact.rs:745`) pins 0x4340000000000001 for fma(0x1.b4d42bf5242b8p+0, 0x1.2c0db0d0ff40dp-1, 2^53); Python 3 `math.fma` gives the same bits, so a result bit changed and the row is a rewrite: agree. R5 in row 1: true. The P2-ACCEPT-1 clarify row H36:145: `crates/ep9_builtins/clippy.toml` and `clippy_toml_repeats_the_root_bans` (`crates/ep9_builtins/tests/table.rs:165`) exist; `ep9_math` has no golden or hash test and its `ep9_bytecode` dev-dependency has no user (H36-Q6); `ep9_numconv` is `crates/ep9_vm/src/numconv.rs` today: true. Part 4 again at e719ffa: `cargo test -p ep9_math --locked` 14 pass, 1 ignored; `cargo xtask fp-check --target aarch64-apple-darwin` PASS (2 references, both allowed by entry 1; 0 FMA instructions outside `ep9_math`); `clippy_toml_repeats_the_root_bans` passes. Change-log kinds: agree | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/math.log, fpcheck.log, builtins.log |
| 4 Data | from the "Validated by" line | `cargo test -p ep9_math --locked`: 14 pass, 1 ignored (the 10^7 run). `cargo xtask fp-check --target aarch64-apple-darwin`: PASS (2 references, both allowed by entry 1; 0 FMA outside `ep9_math`). `clippy_toml_repeats_the_root_bans` passes. The 5-target fp-check is the CI fp-check job. Missing: the golden and 1M-input hash tests (H36-Q6); the CR oracle and FP differential CI (M1) | local run, mac a64, 383a317; logs outside the repo: scratchpad m0m2/accept1/h36-ep9_math-test.log, h36-fp-check-a64.log |
