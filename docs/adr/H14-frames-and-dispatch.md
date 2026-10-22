# H14: Frames, calls, dispatch slots, pc→code registry

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; H14-Q1 filled 2026-10-21: FRAME 2, fp-3 = new.target). The T0 v0 interpreter implements it. Accept needs the review record (README §3.1) and Spike B |
| Fix by | M0 wk3; Spike B + SP-Unit (M0 month 2); Spike C (M1) |
| Freeze point | M1 exit |
| Origin | v1#10 + R16-D2; plan status E |
| Crates | `ep9_vm` (VM register stack, frame layout, calls, unwinder, dispatch slots, code registry) |
| Owner (plan role) | VM lead; the AOT senior also works on it in M0-M1 (09 §2) |
| Validated by | `ep9_vm` tests (listed in Rules); `cargo xtask disasm-check -p ep9_vm --expect 284`; Spike B, SP-Unit, Spike C later |
| Plan sources | [02 §6 H14](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §4](../plan/04-execution.md#4-tier-up-dispatch-slots-and-hybrid-re-tier), [03 §2 VM-stack rule](../plan/03-gc.md), [11 D7, D14, D21](../plan/11-decisions.md) |

## Context

- Every tier (T0-T3) and both AOT code kinds must share one frame, so deopt and OSR are frame copies and JIT and AOT code call each other through one path (02 §6).
- Depends on: D21 (frame home: VM register stack + native control stack), D14 (one isolate per thread), D7 (no proper tail calls; `TailCall` reserved), H6 (value encoding), H10 (roots), H15 (dispatch).
- Depended on by: H16 (feedback slot), H29 (deopt positions), H33 (hybrid re-tier), H35 (unit format: dispatch kind bytes).

## Decision

Canonical header (caller FP, return pc, callee, argc, SFI, new.target, context, offset), then registers. Args reversed, padded with undefined. One layout for T0-T3 and both AOT kinds. JS→JS calls never recurse in Rust. Dispatch-slot kinds {interp, baseline, opt, aot-baseline, aot-opt}. A pc→code registry over JIT code and unit ranges serves unwinding, Error.stack, profiler, debugger and deopt (02 §6).

**Frame.** The VM stack grows up. `fp` points at `r0`. The register operand `o` maps to `fp + o` for `o ≥ 0` and to `fp + o - 8` for `o < 0` (parameters).

```text
 low   a(n-1) .. a0  this | caller fp  ret pc  callee  argc  info  new.target  context  offset | r0 .. r(m-1)   high
 fp-   10+(n-1) .. 10  9  |    8         7       6      5     4        3          2       1    | 0  ..  m-1
       n = max(argc, parameter count); missing args = undefined          top = fp + register count
```

| Slot | Kind | Content |
|---|---|---|
| caller fp, return pc, function info | native word | `addr + 2^49` (a subnormal double; never a cell) |
| callee, context | Value | the function object; the current context |
| new.target (fp-3, `slot::NEW_TARGET`; FRAME 2) | Value | the incoming new.target: the constructor of a `[[Construct]]` call (argc bit 30), undefined for a plain call. A resumed generator frame (`call::push_resume_frame`, construct bit clear) holds the generator object. `LdaNewTarget` reads the slot with no construct-bit check; host functions get new.target as an argument (`abi::new_target` keeps its construct-bit check). No tier keeps the feedback vector here (H14-Q1) |
| argc | int32 | actual count; bit 30 = the frame came from `[[Construct]]` |
| offset | int32 | bytecode offset, saved at runtime calls and Rust entries |

Every word in `[stack base, top)` is a valid Value. The GC scans that slice precisely (H10). Rust locals stay under the conservative native-stack scan.

**Feedback vector (H16).** T0 reaches a function's vector through the function info slot (fp-4): `FunctionInfo.feedback` points at the function's slot in its unit's strong root range, and that slot holds the FEEDBACK cell. So a frame that is already running sees a new vector at once, and the frame layout does not change. The vector is lazy: a call counts while the function has none, and the call that reaches `IsolateConfig::feedback_threshold` (default 8; 0 = first call; `u32::MAX` = never) makes it before the frame push. A `JumpLoop` budget poll makes one for a running function that has none (`crates/ep9_vm/src/feedback.rs`). JIT and AOT frames reach the vector the same way, through fp-4 (H14-Q1).

**Calls.**

| Path | Mechanism | Native stack |
|---|---|---|
| JS → JS (`Call*`, `Construct`) | push frame, jump to the callee code | none |
| `[[Construct]]` from JS | push a stub frame (holds `this`), then the callee frame; callee return pc = `ConstructReturn` stub | none |
| JS → host function | push a host frame, call the Rust fn | one Rust call, checked |
| Rust → JS | push an entry frame with return pc = `Exit` stub, run the dispatch | one dispatch run, checked |
| generator resume (`next`/`return`/`throw`) | push a resumed frame (`call::push_resume_frame`: fp-3 = the generator, return pc = `Exit` stub) with the saved parameters, `this` and context, run the dispatch from the first instruction | one dispatch run, checked (`call::run_resumed`) |
| T0 → JIT code (slot kind Baseline or AotBaseline; feature `spike-native`, Spike B) | T0's `call_fast` sends the call cold; `slow::call` enters at once through its decode-free native entry (`native_entry`, M2-P3-D21-ENTRY: an unprefixed `CallAnyReceiver`, `CallProperty*` or `CallUndefinedReceiver*`, operands read in place, no heap list), or for a prefixed call through `decode_call` and `call_native`. Both push the canonical frame on the VM stack, then call the native entry `(cx, fp)`; the JIT code runs on the native stack | one native call per JIT level, checked (the prologue compares sp with `Cx::native_limit`) |
| JIT → any callee (`Call*` in JIT code) | `RuntimeTable::call_any(cx, fp, offset)` decodes the call instruction at the offset: a native callee is a nested native entry; a T0, host or bound callee runs through `call::call_with` (one dispatch run) | one table call plus the callee's level, checked |
| Rust → JIT code | `native::call_from_rust`: the frame above the VM stack top with return pc = `Exit` stub, then the native entry. On every exit (Ok, Err, termination, stack overflow) it restores the VM stack top, the scratch top and `Cx::out_fp`/`out_pc` (D20) | one native call, checked |

- `Return` pops the frame and continues at the return pc. The stub opcodes are internal: `Exit` (0xFF) leaves the dispatch, `ConstructReturn` (0xFE) applies §10.2.2 steps 10-12.
- The interpreter polls interrupts (termination) at `JumpLoop` and at function entry, once per budget of 10 000 polls.
- A frame that does not fit the VM stack throws a RangeError. A host call or Rust entry below the native limit (`stack low + clamp(size/4, 128 KiB, 1 MiB)`) also throws a RangeError. Both are catchable.

**Generators (V8 model, M2-P1).** A generator function runs like any function until its prologue makes the generator object (`CallRuntime CreateGeneratorObject`) and suspends. `SuspendGenerator` copies the frame's parameters and its register list into the generator's REGS cell (an ELEMENTS cell: `[sent value, parameters, registers]`), saves the current context and the suspend id, and returns like `Return`. A resume pushes a new frame, never the old one: `SwitchOnGeneratorState` (the first instruction after `LdaNewTarget`) jumps to the suspend point's table entry, and `ResumeGenerator` copies the registers back and gives the sent value in the accumulator. A generator frame never stays on the VM stack while suspended; deep `yield*` chains cost one checked native level per level, so they end in a catchable RangeError (`crates/ep9_vm/src/generator.rs`, `interp/slow/generator.rs`).

**JIT frames (Spike B, D21 placement (a); feature `spike-native` only, M2-P2).** A JIT frame is a canonical VM frame: the caller pushes the same header, registers and argument slots as for T0. The JIT code runs on the native stack with a frame record (x29/x30 on AArch64, rbp on x86-64). It keeps the isolate (`Cx`) in x28/r15, the VM fp in x26/r13 and the accumulator in x0/rax; x27/r14 stay free for the unit-data register (H35). It writes no VmStack field: every runtime call passes (cx, its own fp, the bytecode offset of the instruction), and the table function publishes fp and writes the offset slot (`Runtime::enter`). So the unwinder, Error.stack and the precise VM-stack scan see a JIT frame as a T0 frame. A Value that JIT code keeps only in a register is found by the conservative native-stack scan (the GC entry trampolines spill the callee-saved registers). CFI (unix) or UNWIND_INFO (Windows) is registered before the code runs, so native stack walks pass through JIT frames. A table result of 0 means an exception is pending: JIT code has no handlers and returns 0 at once, and its caller unwinds. A generator function never gets native code (`InstallError::Generator`): a resume pushes a new frame that T0 enters at the suspend point.

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
| Every threaded handler ends in a tail jump, ≤ 4 register args | `cargo xtask disasm-check -p ep9_vm --all-targets --expect 284` (H15) | M0 wk3 |
| T0 finds the vector through fp-4; it appears on the threshold call (from JS or from Rust) or at a loop poll, never at `u32::MAX`; the unit keeps it | `tests::feedback::*` | M0 wk4 |
| A suspended generator keeps its frame state in its REGS cell (traced, never a Rust buffer); a resumed frame is a new frame that `SwitchOnGeneratorState` enters at the suspend point; resume is one checked native level | `tests::generator::*` (hand-built V8-shape bytecode, every dispatch, GC stress: `a_suspended_generator_is_the_only_holder_of_an_object`, `a_deep_resume_chain_is_a_range_error`, the opcode argument checks) | M2-P1 |
| fp-3 holds new.target, or the generator object in a resumed frame; `LdaNewTarget` returns the slot | `tests::calls::lda_new_target_reads_the_resumed_frame_slot` (a call gives undefined; a frame pushed by `push_resume_frame` with the construct bit clear and fp-3 = an object gives that object; `[[Construct]]` gives new.target); `host_info_reads_the_contract_constants` (`crates/ep9_unit/tests/check.rs`) reads `FRAME_VERSION` = 2 | M2-P0 |
| JIT code gives the T0 result for the 11 Spike B opcodes (default, `--feedback-threshold 0`, GC stress + verifier + zapping); a function with any other reachable opcode, a prefix or a handler stays in T0 | `bash spikes/spike-b/gate.sh structural` check (a) and the opcode lists; `spike_b::jit::tests::*` | M2-P2 |
| T0→JIT, JIT→T0 and JIT→JIT calls run; an exception from a T0 callee crosses a JIT frame to a T0 catch, and Error.stack names the JIT function at its call line | gate checks (b), (c); `tests::native::exceptions_cross_native_frames` | M2-P2 |
| A native stack walk from a runtime fn passes through JIT frames (registered CFI or UNWIND_INFO) | gate check (d) | M2-P2 |
| `JumpLoop` in JIT code takes one budget unit and polls: `TerminateHandle` stops a JIT loop; deep T0↔JIT recursion is a catchable RangeError, and the next call works | gate checks (e), (f); `tests::native::{termination_stops_at_a_native_poll, deep_t0_native_recursion_is_a_catchable_range_error}` | M2-P2 |
| A Value held only in a JIT callee-saved register survives a GC (a masked copy does not keep it); a panic in a runtime fn under JIT code aborts, also in a panic=unwind build | gate checks (g), (h); `tests::native::a_panic_under_native_code_aborts` | M2-P2 |
| T0→JIT: an unprefixed call of a native-slotted callee (9 call shapes) enters with no decode and no heap list, with the receiver, argument count, exception and GC behaviour of the decode path; every other call (prefix, spread, construct, bound, T0, host, not callable) takes the decode path | `tests::native::{the_decode_free_entry_takes_every_unprefixed_call_shape, every_other_call_falls_through_to_the_decode_path, exceptions_from_the_entry_unwind_from_the_call}` (every dispatch, with and without GC stress + verifier) | M2-P3 |
| A generator function gets no native code; `call_from_rust` restores the entry state on every exit | `tests::native::{a_generator_function_gets_no_native_code_and_its_generators_still_run, call_from_rust_enters_native_code_and_restores_the_entry_state, call_from_rust_deep_recursion_is_a_range_error_and_the_next_call_works}` | M2-P2 |

## Consequences

- **Gains.** No adaptor frames: the callee frame pads arguments (+11.2% jitless, [R06]). Deep JS recursion is bounded by the VM stack (default 2^20 slots), not by the native stack. One walker serves the unwinder, Error.stack and later the profiler and deopt.
- **Costs.** Every argument is copied into the callee frame. Native words cost one add on store and one subtract on load. Calls into host functions and Rust entries still use the native stack and need a check.
- **Follow-ups.** Spike B decides where JIT frames live (D21). H14-Q1 is filled (FRAME 2): fp-3 keeps new.target, and every tier reaches the vector through fp-4. M5 adds frame walkers for JIT and AOT frames to the registry.
- **Follow-up: the real T0→JIT entry must be decode-free (M2-P3-D21-ENTRY).** The D21 round trip of placement (a) is measured through `slow::call`'s decode-free native entry (`cfg(feature = "spike-native")`). As built before it, 78% of config A's time was the cold path's second decode and its heap argument list. Provisional runs ([2026-10-22-spike-b.md](../measurements/2026-10-22-spike-b.md), no verdict): RT about 46 as built and about 16-19 with a one-opcode draft entry under load; in one quieter session, 47.9 as built and 20.1 with this entry; cachegrind ratio 100.9 as built, 33.4 with it. So a placement (a) pass in gate window 1 holds only for a T0→JIT entry that, like this one, reads the call operands in place and the arguments from the caller's registers, with no decode and no heap allocation. The production JIT (M1, Spike C) must ship such an entry (or a T0 handler path that is cheaper still), or measure its own round trip again with the [protocol](../measurements/2026-10-22-spike-b-protocol.md).
- **Plan row fixed (2026-10-21).** The plan row (02 §6 H14) and the Decision text above now name fp-3 "new.target" in the header list (it said "feedback" before FRAME 2).

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
| H14-Q1 | Where does `new.target` live? v0 keeps it in the feedback slot of a `[[Construct]]` frame (`call::push_frame`; `LdaNewTarget` reads it there) and passes it to host functions as an argument | M1 (classes, `Reflect.construct`) | **Filled (2026-10-21, FRAME 2).** fp-3 is the new.target slot (`slot::NEW_TARGET`): the incoming new.target, or the generator object in a resumed frame. JIT and AOT frames reach the feedback vector through fp-4, as T0 does. The proposed default (a register filled at entry) was not taken: one header slot serves `[[Construct]]` and generator resume (M2-GEN-MODEL), and the frame size does not change |
| H14-Q2 | Who wraps a primitive receiver of a sloppy function? | M1 | The callee prologue, from the FunctionInfo strict bit |
| H14-Q3 | Is the register mapping (`o < 0` → `fp + o - 8`) part of the H2 ABI? | H2 freeze (M1 exit) | Yes; the AOT emitter uses it |
| H14-Q4 | Default VM stack size and the RangeError depth versus V8 | M1 | Keep 2^20 slots; measure test262 and the benches |
| H14-Q5 | Spike B result: native-stack or VM-stack JIT frames | M0 month 2 | Per D21 rule (≤ 18 dispatches per round trip). Placement (a) passes the structural gate (M2-P2, gate checks (a)-(h)); the D21 speed rule is P3-SPIKEB-BENCH, measured through the decode-free native entry (Follow-ups); the verdict is gate window 1's |

## Reopens if

- Spike B, SP-Unit or Spike C fails its gate (09 §4).
- D21 or D7 changes.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from the T0 v0 interpreter in `ep9_vm` | VM lead (agent) |
| 2026-10-21 | clarify | Header table and H14-Q1 state where v0 keeps new.target (feedback slot of a construct frame), as the code does | docs-sync agent |
| 2026-10-21 | fill | T0 reaches the lazy FeedbackVector (H16) through the function info slot (fp-4, `FunctionInfo.feedback`); fp-3 keeps new.target (H14-Q1 unchanged). New rule `tests::feedback::*` | VM lead (agent, P2-FEED) |
| 2026-10-21 | fill | H14-Q1 filled: fp-3 holds the incoming new.target, or the generator object in a resumed frame; JIT and AOT frames reach the feedback vector through fp-4. Header table, frame diagram and a new rule row updated | P0-DOCS agent (M2-P0) |
| 2026-10-21 | rewrite (version) | FRAME_VERSION 1 -> 2 (`crates/ep9_vm/src/versions.rs`, P0-VM): `slot::FEEDBACK` becomes `slot::NEW_TARGET`, a resumed frame (`push_resume_frame`, construct bit clear) carries the generator object there, and `LdaNewTarget` reads fp-3 without the construct-bit check. A unit built for FRAME 1 falls back to its bytecode (H35 S3). Pre-freeze churn (README §3.3) | P0-DOCS agent (M2-P0) |
| 2026-10-21 | clarify | Follows the FRAME 2 row above, no new rule: the Decision header list and the plan row (02 §6 H14) name fp-3 new.target; the FRAME 2 rule row names its test `tests::calls::lda_new_target_reads_the_resumed_frame_slot`; the `disasm-check` checks say `--expect 284` (H15, ABI 3) | M2-P0 integration fixer |
| 2026-10-21 | fill | Generators (M2-P1): the resume path in the Calls table, a Generators paragraph (V8 model: copy-out at `SuspendGenerator`, a new frame per resume, `push_resume_frame` + `run_resumed`) and a Rules row `tests::generator::*`. No existing rule or check changes | P1-GEN-VM agent (M2-P1) |
| 2026-10-22 | fill | JIT frames (Spike B placement (a), M2-P2): three Calls rows (T0→JIT, JIT→any callee, Rust→JIT through `call_from_rust`), a JIT frames paragraph, six Rules rows (`spikes/spike-b/gate.sh structural` checks (a)-(h), `tests::native`), and an H14-Q5 note. H14-Q5 stays open until P3-SPIKEB-BENCH. No existing rule or check changes | P2-SPIKE-B agent (M2-P2) |
| 2026-10-22 | fill | M2-P3-D21-ENTRY: the T0→JIT Calls row names `slow::call`'s decode-free native entry (`spike-native` only; the default build is unchanged), a new Rules row with its three `tests::native` tests, a Follow-up (the real T0→JIT entry must be decode-free, since a placement (a) pass in gate window 1 depends on it), and an H14-Q5 note. Kind fill, not clarify: the change ships code and adds a rule for a part the plan defers (the production JIT entry, M1); every existing Rules check keeps its result (DISASM 284 and the default handler stats unchanged, `tests::native` and `gate.sh structural` pass) | P3 integration fixer |
