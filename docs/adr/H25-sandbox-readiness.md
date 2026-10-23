# H25: Sandbox readiness

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P5-ACCEPT; review record complete). Was: Draft (2026-10-21): the rule and an inventory of today's off-heap pointers. `ExternalRef<T>` is not built yet (H25-Q1) |
| Fix by | M0 |
| Freeze point | M1 exit. D6 names M1a (object layout freeze) as the last cheap point for a sandbox |
| Origin | v1#23; plan status U |
| Crates | `ep9_vm` (heap cell layouts), `ep9_heap`, `ep9_unit` (unit trust), `ep9_bytecode` (verifier) |
| Owner (plan role) | GC lead (S4, D6); VM lead (cell layouts) |
| Validated by | The native-word tests and the inventory check (Rules); the verifier; the H35 load checks |
| Plan sources | [02 §6 H25](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [11 D6](../plan/11-decisions.md), [03 §4 S4](../plan/03-gc.md), [H12](H12-no-address-identity.md), [H35](H35-aot-unit-format.md) |

## Context

- D6 (decided 2026-10-20): no untrusted-code sandbox in v1. A V8-style sandbox (pointer tables, cage-relative ArrayBuffer pointers, CFI) would cost 6-12 PM and must come before the M1a layout freeze. The `ExternalRef` rule applies from M0 so that the option stays open (11 D6).
- A V8-style sandbox bans raw off-heap pointers inside heap objects. Retrofitting that means touching every layout [R05].
- Depends on: D6, H8 and H17 (layouts), H12 (no address escapes), H35 (unit trust). Depended on by: H13 (snapshot rewrites each off-heap word), H22 (SAB backing stores).

## Decision

Copied from the plan row: off-heap pointers only in `ExternalRef<T>`. Units are trusted code (D6) (02 §6).

| Part | Rule |
|---|---|
| Off-heap pointers | A heap cell holds an off-heap address only in a field of type `ExternalRef<T>`: one typed wrapper, one encoding, one place to change when a sandbox comes |
| Encoding today | The native word of H14: `addr + 2^49`, a subnormal double that is never a cell, so the GC skips it (`crates/ep9_vm/src/frame.rs`) |
| Units | Native unit code is trusted, like a native addon (D6). The loader checks the header, versions and content hash (H35 S4), never the machine code. With a sandbox, only signed or bytecode-only units may load (11 D6) |
| Bytecode | The VM runs only verified bytecode, from any source (compiler, code cache or unit) |

**Inventory of off-heap pointers in heap cells (55c15d0; checked again at 0c4c70e).** `native_of` makes every native word; these are its stores into heap cells.

| Cell | Field | Points at | Store |
|---|---|---|---|
| FUNCTION | +24 | `FunctionInfo` (in its unit, or a host or stub info) | `crates/ep9_vm/src/object.rs:919` |
| FEEDBACK | info word (`fv::INFO`) | `FunctionInfo` | `crates/ep9_vm/src/feedback.rs:169` |
| CONTEXT | scope word (OBJECT_LAYOUT 2, C5) | `ContextScope { unit, scope }` | `crates/ep9_vm/src/object.rs:1140` (P0-VM, M2-P0) |

The other `native_of` stores write VM stack frame slots (caller fp, return pc, function info; `crates/ep9_vm/src/call.rs`). The VM stack is not a heap cell.

## Rules

| Rule | Check | From |
|---|---|---|
| A native word is a valid non-cell Value, so the GC never traces an off-heap pointer | `frame::tests::native_words_are_valid_non_cell_values` | M0 wk3 |
| Every store of an off-heap pointer into a heap cell is in the inventory above | `grep -rn 'native_of(' crates/*/src` lists only the inventory rows, the VM stack slots of `call.rs` and the `frame.rs` test (review, until the H25-Q2 lint) | M0 |
| From M1a, every inventory field has the type `ExternalRef<T>` | a lint that rejects `native_of` outside `ExternalRef` (H25-Q2) | M1a |
| No raw address escapes to JS, hashes or embedders | H12 rules | M0 wk4 |
| The VM loads only verified bytecode | `LoadedUnit::load` takes a `VerifiedUnit` (type check); `verifier::*` tests | M0 wk3 |
| A unit's native code loads only after the H35 header, version and hash checks; the machine code is not inspected (trusted, D6) | `ep9_unit` check and mutation tests (`crates/ep9_unit/tests/check.rs`) | M0 wk4 |
| Off-heap data of later kinds (ArrayBuffer and SAB backing stores, external strings, kinds 6-7) uses `ExternalRef<T>` from its first commit | review, then the H25-Q2 lint | M1 |

## Consequences

- **Gains.** A sandbox, if D6 ever changes, edits one type and its loads, not every layout. A snapshot (H13) finds every off-heap word through one type.
- **Costs.** One more wrapper in the VM. Until the lint exists, the inventory relies on review.
- **Follow-ups.** H25-Q1 and H25-Q2 before M1a. The ArrayBuffer and external-string layouts (M1) follow the rule.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | S4 options include a V8-style sandbox; decide early, because it bans raw off-heap pointers inside heap objects | [../research/05-gc-design.md](../research/05-gc-design.md) |
| [R09] | The threat model was never decided; untrusted code in an embedding implies a sandbox, W^X and CFI on JIT entries; about 45% of V8 CVEs were JIT-related | [../research/09-critic.md](../research/09-critic.md) |
| inventory | `grep -rn 'native_of(' crates/*/src` at 55c15d0: object.rs:608, feedback.rs:169, call.rs:92-101 (3 frame slots), frame.rs:220 (test). At 0c4c70e: the FUNCTION, FEEDBACK and CONTEXT stores of the table, the 3 frame slots of `call.rs` and the `frame.rs` test, nothing else (GENERATOR and slot objects hold Values only) | this ADR |

## Validation

- M0: the native-word test and the inventory grep. M1a: the lint passes and every inventory field is an `ExternalRef<T>`.
- If D6 changes: a sandbox build swaps the encoding to a table index and runs test262 and the GC-stress matrix.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H25-Q1 | The shape of `ExternalRef<T>` | M1a | A `#[repr(transparent)]` wrapper of the native-word encoding with typed `new` and `get`; a sandbox build would make it an index into a per-isolate pointer table |
| H25-Q2 | How is the rule checked? | M1a | An `xtask` lint (like gc-lint) that rejects `native_of` and `native_ptr` outside `ExternalRef` and the VM stack code |
| H25-Q3 | Do host function pointers in a `FunctionInfo` need the wrapper? | M1a | No: a `FunctionInfo` is off-heap Rust memory, not a heap cell. Only the cell field that points at it does |

## Reopens if

- D6 changes to a sandbox (an owner request), or an embedder needs to run untrusted code.
- A unit signature scheme is required (H35-Q2).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from 02 §6 row H25 and 11 D6, with the inventory of off-heap pointers in heap cells at 55c15d0 | P0-DOCS agent (M2-P0) |
| 2026-10-21 | clarify | 1 `path:line` reference moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-22 | clarify | Inventory: the CONTEXT row names its store (`object.rs`, the scope word of `new_context`), and the inventory and its Evidence row record the grep at 0c4c70e: the same 3 heap-cell stores, the 3 frame slots and the test. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | 4 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | Review record row 2 names the P5 Codex run (slice accept4): 2 findings, A1 on H23 and A2 on H04, both fixed (clarify); no finding on this ADR. Text only; no check changes | orchestrator |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P5-ACCEPT). The §3.1 review record is complete: row 1 P4-ACCEPT-3, row 2 Codex (m2p4-docs run; P5 accept4 run), row 3 P5-ACCEPT-4 (with the P5 round-2 notes), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-23 | clarify (proposed; Accepted path) | 4 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). Text only; no check changes. A ref in a dated history row keeps the line that the named commit gave it (round-2 review finding X-ADR1: this run had moved the 375d953 destinations :703 and :916 in review row 3 to :918 and :1139; restored, and a history pin in the drift script keeps it). Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P0 integration fixer |
| 2026-10-23 | clarify (proposed; Accepted path) | 6 `path:line` references moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P1 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P4-ACCEPT-3 (did not write the ADR) | Decision = the 02 §6 H25 row, word for word ("Off-heap pointers only in `ExternalRef<T>`. Units are trusted code (D6)"). Checked at 0c4c70e: `grep -rn 'native_of(' crates/*/src` gives `crates/ep9_vm/src/object.rs:919` (FUNCTION info), `crates/ep9_vm/src/feedback.rs:169` (FEEDBACK info), `crates/ep9_vm/src/object.rs:1140` (CONTEXT scope), `crates/ep9_vm/src/call.rs:100-109` (3 frame slots) and the `frame.rs` test, so Rules row 2 holds; the GENERATOR fields and its REGS cell hold Values only (`object::genf`), and slot objects hold Values only (M2-P3-SLOTS-VALUES); a native word is the address + 2^49 (`native`, `DOUBLE_ENCODE_OFFSET`); `LoadedUnit::load` takes a `VerifiedUnit`; `crates/ep9_unit/tests/check.rs` exists. R1: the CONTEXT row had no store ref and the inventory was dated 55c15d0 only (fixed, clarify). `ExternalRef<T>` is not built (H25-Q1, M1a), as the Status line says. Open questions due in M0: none (Q1-Q3 M1a). Change-log kinds: agree | branch m0m2/p4-accept-3 |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | The P4 docs run (slice m2p4-docs: the P4 edits of 12 ADRs, among them every B3 record and review fix in H04, H23, H24, H25 and H26; base 2e6940a = the P4 head 575569f without the slice): needs-attention, 1 finding, D1 on H09 and H10; no finding on H25. The ADRLINE ref moves of 375d953 came after the slice and were not in it. P5 run (slice accept4: the P5-ACCEPT-4 commits dffd611 and e4767c1, base bfd5cd8; Codex gpt-6.1-sol): needs-attention, 2 findings: A1 on H23 (host-function calls take no budget unit; fixed 6684988, clarify) and A2 on H04 (the spike-native match-loop fallback; fixed c4ca7de, clarify). No finding on this ADR. | Codex text outside the repo: scratchpad review/m2p4-docs.codex.txt (file list review/m2p4-docs.list); Codex text outside the repo: scratchpad m0m2/p5/codex-accept4.out |
| 3 Verifier | P5-ACCEPT-4 (wrote none of the rows it checks) | Checked at bfd5cd8. Rows 1 and 2 stay as written (row 2: no H25 finding in the m2p4-docs Codex text: true). Rows written by others after the Codex slice: the 375d953 clarify row (4 refs): the FUNCTION and CONTEXT stores `crates/ep9_vm/src/object.rs:703` and `crates/ep9_vm/src/object.rs:916`, in the inventory and in row 1 (705 and 919 at 0c4c70e, so row 1 now cites c39f782 lines under its 0c4c70e date; same stores): true; clarify: agree. The inventory grep at bfd5cd8 (`grep -rn 'native_of(' crates/*/src`) gives the same 7 sites as at 0c4c70e: `crates/ep9_vm/src/feedback.rs:169`, the 2 object.rs stores above, `crates/ep9_vm/src/call.rs:100-109` (the 3 frame slots, lines 93, 94 and 102) and the `frame.rs` test; the `spike-native` file native.rs has none. Rules row 2 holds; no new check (plan). Open questions due in M0: none (Q1-Q3 M1a; the table is unchanged since 90db5ee). Part 4 again at bfd5cd8: `frame::tests::native_words_are_valid_non_cell_values` passes; `cargo test -p ep9_bytecode --locked` 75 pass, 2 ignored (the verifier tests); `cargo test -p ep9_unit --locked` 28 pass. ADRLINE at bfd5cd8: 0 FIX, 0 UNFIXABLE; the call.rs refs (here and in row 1) are in a file P5-VM-SMALL edits, so the P5 fixer's ADRLINE run re-checks them. Change-log kinds: agree | branch m0m2/p5-accept-4; local run, mac a64, bfd5cd8; logs outside the repo: scratchpad m0m2/p5/accept4/logs/vm.log, bytecode.log, unit.log |
| 4 Data | from the "Validated by" line | `frame::tests::native_words_are_valid_non_cell_values` passes; the inventory grep (above); `cargo test -p ep9_bytecode --locked` 75 pass (the verifier tests); `cargo test -p ep9_unit --locked` 28 pass (check and mutation tests). The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3). Missing: the H25-Q2 lint (M1a) | local run, mac a64, 0c4c70e; logs outside the repo: scratchpad m0m2/accept3/logs/vm.log, bytecode.log, unit.log |
| Approval (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 approver: a second reviewer agent (README §3.2); it wrote none of the M3-P0 rows | **Not approved**: the review record is not complete. Checked at 70d79e5 (m0m3/p0-fix-docs). Content: true. The 6 refs are the only text change in this ADR besides the Change-log row (`git diff --word-diff 6b652e1..70d79e5`), and each new ref names the item its text names: the FUNCTION info store `init(f, fun::INFO, native_of(info))` (3) and the CONTEXT scope store `init(c, ctxf::SCOPE, native_of(scope))` (3). ADRLINE: 0 FIX, 0 UNFIXABLE. Kind clarify: agree. Missing: rows 1-3, which the Change-log row names (README §3.1); none is written. No content change is needed. Approve again once the rows are written, or once the orchestrator logs that a ref-only move needs only one reviewer row (as the 2026-10-22 H28 approvals had) | local read, 70d79e5; scratchpad approve-p0/refpairs.py; ADRLINE (scratchpad p4docs/path_line_drift.py) |
| 1 Reviewer (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 approver (5c271b5; a second reviewer agent, wrote none of the M3-P0 rows) and the M3-P0 round-1 fixer reviewer (Claude, diff 6b652e1..70d79e5, wrote none of it); orchestrator decision M3-P0-ADRLINE-ACCEPTED names both as row 1 | Checked at 70d79e5. The approver compared the 6 moved refs with the code and found each names the FUNCTION info store or the CONTEXT scope store: true for the inventory and row 1. It did not see that 2 of the 6 (review row 3, 'Checked at bfd5cd8') record what the 375d953 clarify row wrote, so moving them made that row false (X-ADR1, rows 2 and 3). Kind clarify: agree. The fixer reviewer (0 findings) spot-checked refs in H25 among others: they point at the right code. | scratchpad m0m3/p0/fixer-result.json (keys "approve" and "review"); scratchpad approve-p0/refpairs.py; row written by the M3-P0 round-2 docs fixer from the reports named here |
| 2 Codex (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | orchestrator (Codex gpt-6.1-sol) | Round 1 (Codex ADR slice, base a04dd70; it ran before the ADRLINE run): no finding on H25. Round 2 (Codex ADR slice, base 6b652e1: the round-1 fixer docs diff, which holds this ADRLINE row): needs-attention, 2 findings: X-ADR1 (history rows whose refs the ADRLINE run moved: H36 and H25) and X-ADR2 (H20, the nullish receiver). | Codex text outside the repo: scratchpad m0m3/p0/codex-adr.out (round 1), m0m3/p0/codex2-adr.out (round 2); row written by the M3-P0 round-2 docs fixer from the reports named here |
| 3 Verifier (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 round-2 verifier (wrote neither the ADRLINE rows, nor the Codex text, nor this fix) | Checked at 5c271b5 with a word diff of every ADR file from 6b652e1. X-ADR1: real, (b), made by the round-1 ADRLINE --fix (d6c8620, eb2ad2c): 3 dated history rows got HEAD lines (H36 077d1fe row, H25 review row 3, H20 P0-VM3 step 2 row). X-ADR2 is on H20 (Draft): (c) for this record. On H25: review row 3 says the 375d953 clarify row put the stores at lines that were :703 and :916 at 375d953 and bfd5cd8 (`grep -n 'native_of(' object.rs`: 0c4c70e 705/919, c39f782 and 375d953 and bfd5cd8 703/916, 6b652e1 918/1139); the ADRLINE run wrote :918 and :1139 there: false. The inventory and row 1 name current items: true. Fixed in 1757580 (round-2 docs fixer): review row 3 cites :703 and :916 again, a history pin in the drift script checks them against 375d953, and the 2026-10-23 row counts 4 refs (was 6). The approval row above still says 6 refs, (3) and (3); with this fix it is 4, (2) and (2). The fix is not yet checked by an agent that did not write it. | verifier output (X-ADR1, X-ADR2, sorted; word diff of every ADR file 6b652e1 -> 5c271b5), held by the orchestrator for round 2; row written by the M3-P0 round-2 docs fixer from the reports named here |
| 4 Data (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 round-2 docs fixer (runs only) | ADRLINE (scratchpad p4docs/path_line_drift.py, with 5 history pins added in round 2) at 0bde51c: checked=360 ok=360 fixed=0 unfixable=0, exit 0 (log: scratchpad m0m3/p0/r2docs/drift3.txt); it is run again on the commit that adds these rows; the 2 restored refs of review row 3 are reported as history OK (375d953) | local run, mac a64; logs outside the repo: scratchpad m0m3/p0/r2docs/ |
| Approval (M3-P0 amendments: the 2026-10-23 ADRLINE clarify row, with the X-ADR1 fix 1757580) | M3-P0 approver round 2: a reviewer agent that wrote none of the M3-P0 rows, none of rows 1-4 and none of the round-2 fixes | **Approved** (orchestrator approval under README §3.2, checked by the M3-P0 approver round 2). Checked at 5d6faa3 (m0m3/p0-fix-docs). Rows 1-4 of the amendment are present and the files they cite exist (scratchpad m0m3/p0/fixer-result.json keys "approve" and "review", approve-p0/refpairs.py, m0m3/p0/codex-adr.out, m0m3/p0/codex2-adr.out, m0m3/p0/r2docs/drift3.txt). Row 3 cites the round-2 verifier output, which the orchestrator holds (no file); its X-ADR1 facts were checked again here with `git show`. Each of the 4 moved refs (word diff 6b652e1 -> 5d6faa3) names at 5d6faa3 the same code text that the old ref named at 6361b9d (the M3-P0 base): the FUNCTION info store and the CONTEXT scope store (lines 35 and 37, review row 1 twice). X-ADR1 fix 1757580, checked here by an agent that did not write it: `git show <c>:crates/ep9_vm/src/object.rs | grep -n 'native_of('` gives 703/916 at c39f782, 375d953 and bfd5cd8, 705/919 at 0c4c70e and 918/1139 at 6b652e1, so review row 3 (checked at bfd5cd8) is true again with :703 and :916, and the 2026-10-23 row's count of 4 and its X-ADR1 sentence are true. ADRLINE reports both as history OK (375d953). The round-1 approval row above says 6 refs; that was true at 70d79e5. Kind clarify: agree (README §3.3: text only, no check changes). ADRLINE (scratchpad p4docs/path_line_drift.py) at 5d6faa3: checked=360 ok=360 fixed=0 unfixable=0, exit 0 | local run, mac a64, 5d6faa3; logs outside the repo: scratchpad approve-p0r2/tests.log, adrline-pre.txt, pairs.py |
