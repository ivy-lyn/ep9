# gc-design

## Summary
"Hybrid like V8" means a copying young generation plus a mark-sweep-compact old generation. It also means precise handles plus (still experimental) conservative stack scanning (CSS), and a separate C++ heap (Oilpan). As of V8 main on 2026-10-20, the young gen is still the parallel semi-space Scavenger. MinorMS and sticky mark bits are experimental and off. CSS/direct handles are off by default, and Chromium is still migrating Handle to DirectHandle with a clang rewriter. JSC (Riptide) shows another V8-class design: non-moving, CSS of all machine stacks (JIT frames too), generations via sticky mark bits, concurrent marking, and one header-byte barrier for both generational and concurrent work. For Rust that model removes the worst unsoundness risk: objects moving under live Rust pointers. So the recommendation is JSC/Immix-like rather than V8-like. Build a non-moving mark-region heap with conservative scanning of native Rust frames and precise tracing everywhere else. Evolve it to sticky-mark-bit generations, then concurrent SATB marking, then opportunistic Immix-style evacuation of unpinned objects. The object header and barrier are designed on day 1 so that none of these steps rewrites the object model. MMTk (v0.33.0, 2026-08-05) has the right algorithms (StickyImmix, ConcurrentImmix, pinning, internal-pointer lookup). It should not be the core for v1. It has no Windows support, its V8 binding only runs NoGC and was last pushed Jan 2025, and it has no single plan that is generational + concurrent apart from LXR (OpenJDK-only). Its Ruby and Julia ports are opt-in or experimental. Keep the object-model contract MMTk-compatible so it can be an A/B backend later. Existing Rust engines (Boa, Nova, Brimstone, Yavashark) all have simple stop-the-world or refcount GCs with no JIT. They mainly show how expensive the rooting API is: Nova has about 8k bind/unbind/scope calls, Brimstone about 2.2k Handle uses, and Boa has been redesigning its GC API in a separate repo (oscars) since 2025. My own probe (macOS aarch64, rustc 1.99, opt-level 3) confirmed CSS in Rust has hard requirements: spill callee-saved registers, resolve interior pointers, scan both 32-bit halves if pointers are compressed, untag NaN-boxed values, and root explicitly any GC refs held in Rust heap containers. The hardest decisions to change later are: rooting discipline, value/reference encoding, the 'every heap store goes through a barrier and every field access is relaxed-atomic' rule, header layout and heap parsability, and never exposing raw addresses or address-based hashes.

## Recommendation
PLAN (lowest risk first; each step reuses the same object model)

S0 (weeks 0-3, unblocks the interpreter): set the GC contract and build a trivial collector.
- Heap
  - 256 KiB aligned pages: page flags and side metadata found by `addr & !(256K-1)`.
  - Side bitmaps: mark bits, an object-start/VO bitmap for interior-pointer lookup, and Immix line marks for later.
  - A large-object space above about 8 KiB.
  - Free memory stays parsable (filler objects or the VO bitmap).
- Collector: stop-the-world, non-moving mark-sweep with lazy per-page sweeping. Bump allocation into empty lines/blocks, free lists after sweeping.
- Weak semantics from day 1, because test262 needs them: WeakRef with ClearKeptObjects, WeakMap ephemeron fixpoint, FinalizationRegistry, weak string table.
- Test tooling from day 1: GC-stress mode (collect at every allocation), heap verifier, zapping freed memory.

S1: generations without moving.
- Sticky mark bits, like JSC, MMTk StickyImmix and V8 sticky_mark_bits.
- Turn on the barrier slow path to remember old objects.
- JIT inline allocation from TLAB {cursor, limit} at a fixed offset from the isolate register.

S2: parallel and concurrent marking.
- Work-stealing parallel marking, concurrent marking on helper threads, concurrent sweeping.
- Short STW pauses for initial roots (including the conservative stack scan) and for the weak-semantics finish.
- Needs a safe protocol for shape transitions under concurrent reads (JSC 'nuke' protocol or V8 bailout worklist).
- Needs cooperative safepoint polls (loop back-edges, allocation slow path, long Rust builtins).

S3: moving only where it pays.
- Immix-style opportunistic evacuation of sparse blocks in major GC. Objects pinned by conservative roots, FFI or embedder are skipped.
- Optional later: a copying nursery with pinned-page quarantine (V8 style). Only worth it if profiling shows sticky-nursery fragmentation or locality loss.
- Never build concurrent compaction.

S4 (optional): read-only/static-roots space, shared heap for workers, and a V8-style sandbox if wanted. Decide on the sandbox early, because it bans raw off-heap pointers inside heap objects.

ROOTING
- Native Rust frames: conservative scan using a naked trampoline that spills callee-saved registers (x19-x28, fp, lr, d8-d15 on aarch64), plus the matching set on x64. Resolve through the VO bitmap with interior pointers allowed. The scanner must decode the value encoding (untag NaN-boxes, scan both 32-bit halves if compressed). Objects found this way are pinned for that GC.
- Interpreter register file and runtime tables: precise.
- JIT frames: conservative while the heap is non-moving (S0-S2), like JSC. Switch to precise stack maps at call safepoints (taken from the deopt metadata the JIT needs anyway) only if a moving nursery is added.
- Rust API:
  - `Local<'s, T>`: Copy, a raw pointer branded with the lifetime of an `&'s Isolate`/scope borrow. It cannot be stored in 'static containers, which closes the 'ref hidden in a Vec' hole the probe found.
  - `Global<T>`: a registered precise root for the Rust heap and the embedder.
  - Traced containers for runtime-internal collections.
- Rules:
  - Never hold `&mut` to a GC object across a call that can run JS or GC.
  - All GC-visible fields are accessed through Relaxed atomics (free on aarch64).
  - No Rust Drop on heap objects.
- Do not adopt Nova-style lifetime rooting, Boa-style refcounted roots, or gc-arena: too much boilerplate, or they cannot GC while engine code is running.

HEADER / LAYOUT (8-byte word0)
- [u32 shape: compressed pointer or ShapeId]
- [u8 gc_state: sticky-mark/log/age state that the barrier reads]
- [u8 kind: lets the GC get the visitor and size without loading the shape]
- [u16 flags: hash-present, pinned, etc.]
- Every object's size is computable from the header (plus a length field).
- Minimum object 16 bytes, 8-byte granularity.
- word0 can be overwritten by a forwarding pointer (state kept in gc_state) for S3.
- Identity hash is never address-based.
- ArrayBuffer and other byte payloads live off-heap, with external-memory accounting.
- Code space is non-moving. JIT constants go through a traced constant pool, and embedded shapes are held weakly with deopt-on-death.

BARRIER
- One fast path, emitted BEFORE every heap reference store in Rust setters, interpreter handlers and every JIT tier: `if is_heap_ref(v) && host.gc_state <= threshold { slow(host) }`. The threshold is in a register or TLS.
- Slow path: object-remembering. For generations, log the host into the remembered set. During marking, SATB-snapshot the host's fields before its first mutation (MMTk SATBBarrier / LXR style). Hermes reports SATB gives bounded termination and no stack rescan, which suits conservative scanning.
- New objects are allocated in a state that skips the barrier. Large element arrays use card or field remembering.
- Also needed: bulk barriers for memmove-style element copies, and a read barrier on WeakRef deref during marking.
- The fast-path shape (pre-store, header byte) is the hard-to-change part. Slow-path semantics can be swapped.

MMTk
- Not the core for v1: no Windows support, no production JS binding, and no generational+concurrent plan outside OpenJDK.
- Keep the contract MMTk-compatible (header/side bits, size from header, slot visitors, pinning, VO lookup) so a binding can be used as an A/B benchmark later.

HARD TO CHANGE (decide in month 1)
1. Rooting discipline.
2. Value encoding and pointer compression; agree this with the JIT/value-repr owner.
3. Every heap store goes through the barrier API, and every field access is atomic.
4. Header bits and heap parsability.
5. No exposed raw addresses or address hashes.
6. Page size and side-metadata scheme (the JIT bakes in the masks).

EASY TO CHANGE: collection algorithm per stage, pacing heuristics, slow-path barrier semantics, parallelism, evacuation policy.

## Findings
- [high] V8 (main, 2026-10-20) still uses the parallel Scavenger (semi-space copying) as its default young-gen collector. MinorMS (non-moving young-gen mark-sweep) is an experimental flag, off by default. Sticky mark bits are a build flag, off by default. Chromium's gin layer has no MinorMS feature switch (only kV8ScavengerHigherCapacity, disabled by default).
  EVIDENCE: v8/v8@8f6ac1ce src/flags/flag-definitions.h:3865 `DEFINE_EXPERIMENTAL_FEATURE(minor_ms, ...)` (macro at :309-311 sets default false); :3889 sticky_mark_bits = V8_ENABLE_STICKY_MARK_BITS_BOOL; BUILD.gn:507 `v8_enable_sticky_mark_bits = false`; chromium gin/gin_features.cc:106 only kV8ScavengerHigherCapacity; v8_initializer.cc:416-419. Local copies in scratchpad/work/gc/
- [medium] V8 conservative stack scanning (CSS) is still not on by default. It is forced on only by the v8_enable_direct_handle build flag, which defaults to false. Chromium keeps a clang tool to migrate Handle<T> to DirectHandle<T>, so changing the rooting model is a multi-year, tool-assisted migration even for V8. Scavenger pinning for conservative roots exists (whole pages are quarantined and then promoted) but is off by default.
  EVIDENCE: flag-definitions.h:505-524 (`#ifdef V8_ENABLE_DIRECT_HANDLE ... conservative_stack_scanning true` else experimental; 'Do not use direct handles without conservative stack scanning'); :526 scavenger_conservative_object_pinning false; :558 scavenger_promote_quarantined_pages true; gni/v8.gni:212 `v8_enable_direct_handle = false`; chromium tools/clang/v8_handle_migrate/README.md
- [high] In V8, CSS needs heap parsability and inner-pointer resolution: it finds the previous valid object via the marking bitmap, then walks forward using object sizes. With pointer compression it decompresses BOTH 32-bit halves of every stack word.
  EVIDENCE: v8 src/heap/conservative-stack-visitor-inl.h:66-130 (FindBasePtr, MarkingBitmap::FindPreviousValidObject, 'page is not iterable' DCHECK); :136-150; src/common/ptr-compr-inl.h:135-148 ('Extract, decompress and trace both halfwords')
- [high] The V8 old gen uses concurrent marking with a Dijkstra-style insertion barrier (source-colour check removed), relaxed-atomic field writes, and a main-thread bailout worklist. It also uses parallel compaction and concurrent sweeping. Concurrent marking cut main-thread marking time by 60-70%. The barrier fast path checks page flags: host page 'PointersFromHereAreInteresting', then value page 'PointersToHereAreInteresting'.
  EVIDENCE: https://v8.dev/blog/concurrent-marking ; https://v8.dev/blog/trash-talk ; v8 src/heap/heap-write-barrier-inl.h:52-79; src/objects/tagged-field-inl.h:118-140 (Relaxed_Load/Relaxed_Store)
- [high] V8 pointer compression uses a 4 GB cage and base+u32 decompression, and cuts Smis to 31 bits. It shrank the V8 heap by up to 43%. Retrofitting it started from a ~35% Octane regression that took many compiler fixes to recover, and it cost the 32-bit-Smi and double-field-unboxing optimizations. An 8 GB cage build option exists.
  EVIDENCE: https://v8.dev/blog/pointer-compression ; v8 BUILD.gn:684-689 (v8_enable_pointer_compression_8gb)
- [high] Oilpan/cppgc (V8's C++ heap) relies only on conservative stack scanning, with Member/Persistent for heap and off-heap references, and is traced together with the JS heap as one 'unified heap'.
  EVIDENCE: https://v8.dev/blog/high-performance-cpp-gc
- [high] JSC Riptide: non-moving and generational via sticky mark bits. Marking is concurrent and parallel (up to 8 threads) and allocation uses 16 KB segregated blocks. Its retreating-wavefront barrier is one header-byte compare (`cellState <= blackThreshold`) shared by the generational and concurrent paths, with a conditional-fence trick. It scans all machine-thread stacks conservatively, including JIT frames. The header is 8 bytes: 32-bit StructureID plus indexingType, JSType, flags and cellState bytes. Reported results: ~5x better JetStream splay-latency and pauses kept under 3 ms.
  EVIDENCE: https://webkit.org/blog/7122/introducing-riptide-webkits-retreating-wavefront-concurrent-garbage-collector/ ; WebKit Source/JavaScriptCore/runtime/JSCell.h:293-300; heap/CellState.h:31-47; heap/Heap.cpp:774-779 (gatherConservativeRoots on machine threads)
- [high] SpiderMonkey uses precise rooting (Rooted/Handle wrappers checked by a static hazard analysis) with a moving nursery and compaction. Marking is incremental and, since 2024, parallel (20-30% less GC time), but not concurrent: the docs say concurrent marking is 'being investigated'. Compaction is not incremental.
  EVIDENCE: https://firefox-source-docs.mozilla.org/js/gc.html ; https://firefox-source-docs.mozilla.org/js/HazardAnalysis/index.html ; https://spidermonkey.dev/blog/2024/03/20/newletter-firefox-124-125.html
- [high] Hermes Hades has a copying young gen and a non-moving old gen with size-segregated free lists. It marks concurrently with a SATB barrier that logs into a 128-entry buffer (lock taken every 128 barriers); SATB was chosen for bounded termination and no root rescan. A short STW pause handles WeakMap and WeakRef work, and WeakRef reads go through a read barrier. It compacts one segment per cycle, piggybacked on the next YG GC, and runs in incremental mode on 32-bit. Its native rooting is moving from GCScope to PinnedValue Locals.
  EVIDENCE: facebook/hermes static_h doc/Hades.md lines 56-59, 63-91, 183-209, 213-259, 284-320, 323-344; doc/GCSafeCoding.md (roots: register stack, LocalsRAII PinnedValue, GCScope legacy)
- [medium] Pure refcounting is used only by small or embedded-first engines: QuickJS (RC plus a cycle-removal pass) and Yavashark (RC by default; cycle collection sits behind the off-by-default 'actual_gc' feature). Speed-focused engines avoid it: every pointer copy costs RC traffic (atomic if multi-threaded), cycles need a backup tracer, there is no compaction, and JIT code must emit RC ops. LXR shows RC can compete only as deferred/coalesced RC on an Immix heap with concurrent backup tracing (+4% throughput vs G1 on 17 workloads).
  EVIDENCE: https://bellard.org/quickjs/quickjs.html (GC sections); yavashark crates/yavashark_garbage/README.md, src/lib.rs:724-729, Cargo.toml (default = [], actual_gc = []); https://arxiv.org/pdf/2210.17175
- [high] MMTk status: mmtk-core v0.33.0 (2026-08-05), ~63k lines of Rust, MSRV 1.84. Plans: NoGC, SemiSpace, GenCopy, GenImmix, MarkSweep, Immix, MarkCompact (Lisp2, OVC), StickyImmix, LXR, ConcurrentImmix. ConcurrentImmix is non-generational (SATB, concurrent and non-moving, with defrag only in STW full GC). LXR is listed only for OpenJDK (WIP for Julia).
  EVIDENCE: repos/mmtk-core@ed897119 Cargo.toml (version 0.33.0, rust-version 1.84); src/util/options.rs PlanSelector; src/plan/concurrent/immix/global.rs:40,61-66; src/plan/barriers.rs:19-29; CHANGELOG.md 0.33.0; https://www.mmtk.io/status
- [high] MMTk has no JS engine in production. mmtk-v8 supports only NoGC, only on linux-x86_64, and was last pushed 2025-01-06. Julia's MMTk build is opt-in (USE_THIRD_PARTY_GC=mmtk, default plan StickyImmix). Ruby's is experimental: the 2024 merge proposal said it was slower than the default GC in many cases, and the 2026 ruby/mmtk README claims a ~6% geomean gain in one heap mode (self-reported). MMTk has no Windows support: os/imp only covers linux/macos/android, and issue #961 has been open since 2023.
  EVIDENCE: repos/mmtk-v8 last commit 7009cfb 2025-01-07, README 'only supports linux-x86_64'; https://www.mmtk.io/status (V8 column: NoGC only); JuliaLang/julia Make.inc:921-931; https://bugs.ruby-lang.org/issues/20860 ; mmtk-core src/util/os/imp/mod.rs; gh issue mmtk/mmtk-core#961 OPEN
- [high] The MMTk API can support our design. It offers pinning, internal-pointer lookup for conservative roots (VO bits), pre/post write barriers and memory_region_copy barriers, header-or-side metadata specs, a COMPRESSED_PTR_ENABLED flag, and weak-ref hooks. But fast inline allocation and barriers mean copying MMTk internals into the binding (BumpPointer in TLS, JIT-emitted sequences), and the API breaks between releases (e.g. 0.33 'Replace ProcessEdgesWork with Trace').
  EVIDENCE: mmtk-core src/memory_manager.rs:195,299,340-470,729,758,891,917; src/vm/object_model.rs:105-162,384-511; src/vm/scanning.rs:143-155; docs/userguide/src/portingguide/perf_tuning/alloc.md:13-14,50-82,126-138
- [high] Boa's boa_gc mallocs each object separately (Box per GcBox, tracked in a Vec). Roots are found at GC time by counting heap-internal references (`is_rooted = non_root_count < ref_count`). It is a stop-the-world, non-moving mark-sweep, thread-local, with no generations. Boa has been designing a new GC API and allocator in boa-dev/oscars since 2025; a bump-arena prototype allocates about 2x faster.
  EVIDENCE: repos/boa@9871775 core/gc/src/lib.rs:45-51,131-141,225-260,280-330; core/gc/src/internals/gc_header.rs:15-18,108-110; repos/oscars notes/gc_api_models.md, notes/arena2_vs_boa_gc.md
- [high] Nova stores each object type in its own Vec (SoA). Values are word-sized enums holding a u32 index (NonZeroU32). GC is a stop-the-world mark followed by compaction by index, with parallel sweep threads. Rooting is enforced at compile time with GcScope/NoGcScope reborrow lifetimes plus explicit `.scope()`. This costs about 924 `.scope(`, 2851 `.bind(` and 4238 `.unbind()` calls in 159k lines. Index-based heaps add an indirection that a JIT would have to carry.
  EVIDENCE: repos/nova@877ef16 nova_vm/src/heap.rs:69-99; heap/indexes.rs:24; ecmascript/types/language/value.rs:98,398; heap/heap_gc.rs:48,1233-1245,1386-1402; GARBAGE_COLLECTOR.md; grep counts run locally
- [high] Brimstone uses a Cheney semispace copying GC with V8-style HandleScope/HandleBlock handles, which are Copy and have no lifetime, so safety is by convention. Values are NaN-boxed (pointer tag 0), and the heap item header is just a shape pointer. It has about 2193 `Handle<` uses in 123k lines, reports >97% test262 language conformance, and describes itself as 'very unsafe Rust'.
  EVIDENCE: repos/brimstone@64d6698 src/js/runtime/gc/garbage_collector.rs:21-22; gc/handle.rs:16-56,124-159,254-281; gc/heap_item.rs:337-339; value.rs:24,93-104; README.md:5,14
- [high] Kiesel is written in Zig, not Rust, and uses bdwgc (Boehm, fully conservative). gc-arena uses 'mutation xor collection' with branded 'gc lifetimes: no GC while engine code is running, no moving, single-threaded. That does not fit an engine that must GC in the middle of builtins and JIT code.
  EVIDENCE: repos/kiesel@d3e8470 README.md:3,29, build.zig.zon:11-13; repos/gc-arena@5ea967a README.md:9-25,55-66,87-88
- [high] EXPERIMENT (macOS arm64, rustc 1.99, opt-level 3, LTO) with a conservative scanner over optimized Rust code: (1) If callee-saved registers are not spilled first, refs held in registers are missed (S1, S2, S3 all MISSING). (2) LLVM strength reduction left only an interior pointer live across the safepoint (`ldr x8, [x20], #0x8`). After the last iteration that pointer equals the next object's start, so interior resolution is required and one-past-end pointers can blame the wrong object. (3) Compressed u32 refs are only found by scanning 32-bit halves, and two refs can share one 64-bit word. (4) NaN-boxed or high-tagged pointers are invisible unless the scanner untags them. (5) A ref stored only in a Rust Vec (malloc heap) is invisible, so explicit Global roots or traced containers are needed. With all registers spilled, every plain/interior/compressed case was found.
  EVIDENCE: scratchpad/work/gc/css-probe/src/main.rs (naked spill_and_scan trampoline; scenarios S1-S7); run output: spilled: S1 Exact, S2 Exact, S3 Interior, S4 CompLow, S5 CompLow/CompHigh, S6 MISSING, S7 MISSING; not spilled: S1/S2/S3/S7 MISSING; objdump of s3_loop
- [high] Concurrent marking in Rust needs every GC-visible field read and write to be atomic (Relaxed); otherwise it is a data race and UB. On aarch64 this is free: Relaxed AtomicU64 load and store compile to plain ldr and str. A JSC/MMTk-style header-byte barrier fast path is str + ldrb(host) + ldrb(threshold) + cmp + b.ls. A V8-style page-flag barrier is str + and + ldrb + tbz, plus a second check on the value's page.
  EVIDENCE: scratchpad/work/gc/atomics-codegen/lib.rs and lib.s (rustc 1.99 -C opt-level=3)
- [high] Julia's experience retrofitting a moving GC (MMTk, ISMM'25): unreported 'dominated' roots, raw-pointer FFI and address-based hashing were the biggest obstacles. They used conservative pinning of stack and register roots, resolved through VO bits, and pinned objects that are hashed or exposed to FFI. 70-99% of objects stayed movable. Write barriers had to be lowered per GC plan in codegen.
  EVIDENCE: scratchpad/work/gc/pdf/julia-ismm25.txt lines ~540-700 (https://www.steveblackburn.org/pubs/papers/julia-ismm-2025.pdf)
- [medium] Running Rust Drop impls as GC finalizers causes soundness, performance and premature-finalization problems; Alloy (OOPSLA 2025) needed dedicated analyses to make it safe. A JS heap should therefore not hold Drop types and should run explicit, typed finalizers only.
  EVIDENCE: https://arxiv.org/abs/2504.01841 (Hughes & Tratt, 'Garbage Collection for Rust: The Finalizer Frontier'); read at abstract level only
- [high] Engines that use precise native rooting need extra tooling to stay correct: V8 has GCMole, SpiderMonkey has its hazard analysis, and Nova relies on borrow-checker lifetimes. A library cannot produce Rust stack maps (rustc does not expose LLVM statepoints), so the realistic choices for Rust frames are handles/shadow stacks or CSS.
  EVIDENCE: v8 tools/gcmole/README; https://firefox-source-docs.mozilla.org/js/HazardAnalysis/index.html ; https://without.boats/blog/shifgrethor-iii/ ; nova GARBAGE_COLLECTOR.md

## Risks
- The non-moving-first design (JSC-like) uses more memory and fragments more than V8's copying young gen plus compaction. V8 has had MinorMS for years without making it the default, so the non-moving-nursery trade-off is not a clear win on every workload.
- Conservative stack scanning is not guaranteed by LLVM/Rust semantics. Pointers can hide as derived or one-past-end values (seen in the probe), SIMD lanes, or XOR/packed forms. This is mitigated by call-only safepoints (the ABI limits what survives a call), interior resolution and stress testing, but it cannot be proven sound. JSC and Oilpan take the same risk with clang/LLVM C++.
- If pointer compression is adopted, every stack word means two 32-bit candidates, which raises false retention and pinning. If NaN-boxing is adopted, the scanner has to understand tags. Either way the GC is tied to the value-representation decision.
- Concurrent marking in Rust needs strict atomic-access discipline and a safe protocol for shape and elements reallocation under concurrent reads. Data-race bugs are UB and hard to find; TSan, loom-style tests and a heap verifier are needed from S2.
- MMTk is pre-1.0 and its API changes every release. Choosing it later could mean binding churn. Choosing to never use it means re-implementing Immix, StickyImmix and SATB ourselves (moderate effort, well-documented algorithms).
- Opportunistic evacuation (S3) needs precise tracing of every heap slot, including embedder and Rust-internal containers. Any slot that is not precise forces pinning, as Julia found, and erodes the benefit of moving.
- Every V8, SpiderMonkey, Hermes, JSC and MMTk claim above comes from current main branches or docs read on 2026-10-20. Shipping defaults can be changed by field trials or build configs outside the files checked (Chromium build_overrides/v8.gni and gin were checked).

## Open questions
- What is the value representation: NaN-boxed 64-bit everywhere (JSC/SpiderMonkey) or 32-bit compressed tagged fields with boxed doubles (V8)? This sets heap size, conservative-scan rules and barrier filters, and the value-representation/JIT owner has to decide it.
- Is Windows a required target from v1 (Node/Deno-like runtime)? If yes, MMTk is out unless Windows support lands upstream (issue #961 has been open since 2023).
- Will workers share a heap (shared structs, V8 shared space) or be isolate-per-thread with only SharedArrayBuffer? This affects the safepoint protocol and barriers.
- Will the embedder or FFI ever see raw object addresses or borrowed interior data (string chars, typed-array data)? If so, those objects must be permanently pinned or kept off-heap.
- Should a V8-style sandbox (no raw off-heap pointers in heap objects; pointer tables) be a goal? It constrains layout and is expensive to retrofit.
- Is a copying nursery with page quarantine (V8 style) worth adding after S2? This needs measurement on real workloads (Node-style server benchmarks, JetStream/Speedometer-like) against the sticky-mark-bit nursery.
- Not verified: up-to-date head-to-head numbers for MMTk StickyImmix/ConcurrentImmix against JSC or V8 collectors on JS-like workloads. None were found.

## Artifacts
- scratchpad/work/gc/css-probe/src/main.rs
- scratchpad/work/gc/css-probe/Cargo.toml
- scratchpad/work/gc/atomics-codegen/lib.rs
- scratchpad/work/gc/atomics-codegen/lib.s
- scratchpad/work/gc/flag-definitions.h
- scratchpad/work/gc/csv.h
- scratchpad/work/gc/hwb.h
- scratchpad/work/gc/hermes/Hades.md
- scratchpad/work/gc/hermes/GCSafeCoding.md
- scratchpad/work/gc/pdf/julia-ismm25.txt
- scratchpad/repos/mmtk-core
- scratchpad/repos/oscars
- scratchpad/repos/yavashark
- scratchpad/repos/kiesel
- scratchpad/repos/gc-arena
- scratchpad/repos/mmtk-v8
