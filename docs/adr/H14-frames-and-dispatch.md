# H14: Frames, calls, dispatch slots, pc→code registry

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). The T0 v0 interpreter implements it. Accept needs the review record (README §3.1) and Spike B |
| Fix by | M0 wk3; Spike B + SP-Unit (M0 month 2); Spike C (M1) |
| Freeze point | M1 exit |
| Origin | v1#10 + R16-D2; plan status E |
| Crates | `ep9_vm` (VM register stack, frame layout, calls, unwinder, dispatch slots, code registry) |
| Owner (plan role) | VM lead; the AOT senior also works on it in M0-M1 (09 §2) |
| Validated by | `ep9_vm` tests (listed in Rules); `cargo xtask disasm-check -p ep9_vm --expect 282`; Spike B, SP-Unit, Spike C later |
| Plan sources | [02 §6 H14](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §4](../plan/04-execution.md#4-tier-up-dispatch-slots-and-hybrid-re-tier), [03 §2 VM-stack rule](../plan/03-gc.md), [11 D7, D14, D21](../plan/11-decisions.md) |

## Context

- Every tier (T0-T3) and both AOT code kinds must share one frame, so deopt and OSR are frame copies and JIT and AOT code call each other through one path (02 §6).
- Depends on: D21 (frame home: VM register stack + native control stack), D14 (one isolate per thread), D7 (no proper tail calls; `TailCall` reserved), H6 (value encoding), H10 (roots), H15 (dispatch).
- Depended on by: H16 (feedback slot), H29 (deopt positions), H33 (hybrid re-tier), H35 (unit format: dispatch kind bytes).

## Decision

Canonical header (caller FP, return pc, callee, argc, SFI, feedback, context, offset), then registers. Args reversed, padded with undefined. One layout for T0-T3 and both AOT kinds. JS→JS calls never recurse in Rust. Dispatch-slot kinds {interp, baseline, opt, aot-baseline, aot-opt}. A pc→code registry over JIT code and unit ranges serves unwinding, Error.stack, profiler, debugger and deopt (02 §6).

**Frame.** The VM stack grows up. `fp` points at `r0`. The register operand `o` maps to `fp + o` for `o ≥ 0` and to `fp + o - 8` for `o < 0` (parameters).

```text
 low   a(n-1) .. a0  this | caller fp  ret pc  callee  argc  info  feedback  context  offset | r0 .. r(m-1)   high
 fp-   10+(n-1) .. 10  9  |    8         7       6      5     4       3         2       1    | 0  ..  m-1
       n = max(argc, parameter count); missing args = undefined          top = fp + register count
```

| Slot | Kind | Content |
|---|---|---|
| caller fp, return pc, function info | native word | `addr + 2^49` (a subnormal double; never a cell) |
| callee, feedback, context | Value | feedback is undefined until H16. v0: a `[[Construct]]` frame (argc bit 30) keeps new.target there (H14-Q1) |
| argc | int32 | actual count; bit 30 = the frame came from `[[Construct]]` |
| offset | int32 | bytecode offset, saved at runtime calls and Rust entries |

Every word in `[stack base, top)` is a valid Value. The GC scans that slice precisely (H10). Rust locals stay under the conservative native-stack scan.

**Calls.**

| Path | Mechanism | Native stack |
|---|---|---|
| JS → JS (`Call*`, `Construct`) | push frame, jump to the callee code | none |
| `[[Construct]]` from JS | push a stub frame (holds `this`), then the callee frame; callee return pc = `ConstructReturn` stub | none |
| JS → host function | push a host frame, call the Rust fn | one Rust call, checked |
| Rust → JS | push an entry frame with return pc = `Exit` stub, run the dispatch | one dispatch run, checked |

- `Return` pops the frame and continues at the return pc. The stub opcodes are internal: `Exit` (0xFF) leaves the dispatch, `ConstructReturn` (0xFE) applies §10.2.2 steps 10-12.
- The interpreter polls interrupts (termination) at `JumpLoop` and at function entry, once per budget of 10 000 polls.
- A frame that does not fit the VM stack throws a RangeError. A host call or Rust entry below the native limit (`stack low + clamp(size/4, 128 KiB, 1 MiB)`) also throws a RangeError. Both are catchable.

**Dispatch slots.** Each FunctionInfo holds one slot `{kind: u8, entry: usize}` (kind at offset 0, entry at offset 8). A call reads the slot. Tier-up, unit load, deopt-all and PIN write one slot. No code patches a call site. The kind bytes are `Interp = 0, Baseline = 1, Opt = 2, AotBaseline = 3, AotOpt = 4`; `0xFF` = no code (host functions, stubs). H35 uses the same bytes.

**pc→code registry.** Trait `CodeRegistry { register, unregister, lookup(pc) -> CodeEntry, len }`. `CodeEntry = {kind, start, len, function}`. Ranges never overlap. v0 registers each loaded bytecode array as `Interp`. The GC does not use the registry to find roots (03 §2).

**Positions and unwinding.** The offset of a frame is the return pc of the frame above it minus one. If that return pc is a VM stub, the offset is in the frame's own offset slot. The unwinder walks frames from the top. For each bytecode frame it takes the innermost handler-table range that holds the offset. At an entry frame it returns `Err` to Rust and keeps the exception pending. Error objects get `file:line:column` (UTF-16 column) and a `stack` string from the same walk.

## Rules

| Rule | Check | From |
|---|---|---|
| Header slots, register mapping and native words follow the layout above | `frame::tests::*`, `interp::dsl::tests::register_mapping_follows_the_frame_layout` | M0 wk3 |
| JS→JS calls and returns do not grow the native stack | `tests::interp::js_calls_do_not_recurse_in_rust` (same native sp at JS depth 0 and 100 000 on a 512 KiB thread, every dispatch in release) | M0 wk3 |
| VM-stack and native-stack overflow are catchable RangeErrors | `tests::exceptions::{vm,native}_stack_overflow_is_a_catchable_range_error` | M0 wk3 |
| Handler tables catch across frames; uncaught errors carry file:line:col (UTF-16) and a stack | `tests::exceptions::*` | M0 wk3 |
| The VM stack is a precise root; GC stress + verifier pass | `tests::gc::*`; `run_all` runs every program under stress | M0 wk3 |
| A tier change writes one slot; registry ranges do not overlap | `dispatch::tests::*`, `tests::interp::dispatch_slots_and_code_registry` | M0 wk3 |
| Every threaded handler ends in a tail jump, ≤ 4 register args | `cargo xtask disasm-check -p ep9_vm --all-targets --expect 282` (H15) | M0 wk3 |

## Consequences

- **Gains.** No adaptor frames: the callee frame pads arguments (+11.2% jitless, [R06]). Deep JS recursion is bounded by the VM stack (default 2^20 slots), not by the native stack. One walker serves the unwinder, Error.stack and later the profiler and deopt.
- **Costs.** Every argument is copied into the callee frame. Native words cost one add on store and one subtract on load. Calls into host functions and Rust entries still use the native stack and need a check.
- **Follow-ups.** Spike B decides where JIT frames live (D21). H16 fills the feedback slot. M5 adds frame walkers for JIT and AOT frames to the registry.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | V8 removed its arguments adaptor frame (args reversed, padded with undefined, argc in the header): +11.2% jitless | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R09] | Proper tail calls: 35 test262 tests, only JSC and XS pass; frames and JS→JS calls without Rust recursion keep the option | [../research/09-critic.md](../research/09-critic.md) |
| [R14] | Static Hermes runs AOT code, JIT code and the interpreter in one runtime on one frame model | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |

## Validation

- M0 wk3: the `ep9_vm` tests above on macOS arm64, x86_64-apple-darwin, Linux arm64 and amd64; a Windows x64 build.
- Spike B + SP-Unit (M0 month 2): JIT frame placement; a unit blob calls through the same slot. Spike C (M1): one deopt from a native frame to a VM-stack frame.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H14-Q1 | Where does `new.target` live? v0 keeps it in the feedback slot of a `[[Construct]]` frame (`call::push_frame`; `LdaNewTarget` reads it there) and passes it to host functions as an argument | M1 (classes, `Reflect.construct`) | A register filled at entry, not a header slot |
| H14-Q2 | Who wraps a primitive receiver of a sloppy function? | M1 | The callee prologue, from the FunctionInfo strict bit |
| H14-Q3 | Is the register mapping (`o < 0` → `fp + o - 8`) part of the H2 ABI? | H2 freeze (M1 exit) | Yes; the AOT emitter uses it |
| H14-Q4 | Default VM stack size and the RangeError depth versus V8 | M1 | Keep 2^20 slots; measure test262 and the benches |
| H14-Q5 | Spike B result: native-stack or VM-stack JIT frames | M0 month 2 | Per D21 rule (≤ 18 dispatches per round trip) |

## Reopens if

- Spike B, SP-Unit or Spike C fails its gate (09 §4).
- D21 or D7 changes.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from the T0 v0 interpreter in `ep9_vm` | VM lead (agent) |
| 2026-10-21 | clarify | Header table and H14-Q1 state where v0 keeps new.target (feedback slot of a construct frame), as the code does | docs-sync agent |
