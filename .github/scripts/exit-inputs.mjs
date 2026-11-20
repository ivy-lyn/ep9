#!/usr/bin/env node
// The exit-inputs mode of ci.yml (plan v14 section 4.1; V14-D12, V14-D59): the helpers of the
// `exit-inputs` job and of its reading. The job runs MEMREAD (tools/bench/cross/mem.mjs) and the
// JetStream M2 list (tools/bench/jetstream/run.mjs) per platform, and the Path B M2 set
// (tools/bench/cross/run.mjs, both builds in each paired round) on 4 windows x64 runners and on
// 2 Linux x64 runners (plan v15 V15-D16: the same-CPU-model pairs of the win-x64 cell).
//
//   node .github/scripts/exit-inputs.mjs host --out host.toml [--id ID] [--minutes N] [--every-s S]
//       Measures the idle baseline of this runner (tools/bench/cross/idle.mjs, its load source:
//       the 1-minute load, or typeperf on Windows) and writes a host file for the load guard:
//       id, cpu, cores, platform, os, gate_host = false (a GitHub runner is never a gate host,
//       10-quality.md section 3), idle_baseline_load1, load_source. Exit 3 when idle.mjs saw a
//       build or benchmark process (no baseline, no file).
//
//   node .github/scripts/exit-inputs.mjs pathb --leg DIR [--leg DIR]... --out compare.json
//       [--md compare.md] [--platform P]
//       The Path B compare of the runner legs (V14-D12, the box protocol of 10-quality.md
//       section 3): each leg DIR holds run-jit.json and run-jitless.json, the two views of one
//       both-builds run.mjs run (--out, --out-jitless). The runners are grouped by platform and
//       CPU model, in the order given; the first 2 legs of the first group with 2 legs are box 1
//       and box 2 (2 legs of 2 platforms are never a pair). --platform P (win32-x64, linux-x64)
//       reads only the legs of P; the runners table names the others.
//       The output is the ep9-pathb-compare/v1 file that `cargo xtask m2-exit --pathb` reads:
//       commit, platform (of the 2 boxes' runs; null when they differ), boxes, runners (every
//       leg and its CPU model), checks (box, build,
//       commit_ok, rounds, load) and cells (items 1, 2, 3, 6 per build: the 2 box intervals
//       against the M2 threshold, "AGREE above", "AGREE below", "OPPOSITE" or "NO AGREEMENT").
//       A box reading that the gates refuse (void, gross only, a void or net invalid kernel row,
//       any other m2gate reason except the run labels of a runner that is not a gate host and,
//       for item 3, the QuickJS ratio) gives no interval (boxBlockers).
//       No 2 legs of one CPU model: the file says pair null and the exit status is 3. On
//       --platform win32-x64 the dispatch then runs again once; a second miss is NOT-RUN (a DL
//       line; plan v14 section 4.1 (6)). On --platform linux-x64 exit 3 is normal (2 legs, often of
//       2 CPU models): do not run the dispatch again; m2-exit reads the `legs` block of the file.
//       The Markdown also gives the TS ratio to node --jitless per box and build (V14-D59).
//       The file also has `legs` (V15-D16), with a pair or not: every leg that was read, its CPU
//       model, its own commit, a run check per build and the box value of each cell per build
//       (item 3 also `time_s`: the gross ep9 TS time, whose 1 / t is the cross-host TS speed).
//       Each leg also names the id, run_id and run_attempt of its host.toml when they are set (the
//       host step writes run_id and run_attempt from GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT; a run
//       off GitHub, such as a box run, has none).
//       `cargo xtask m2-exit --host x86_64-pc-windows-msvc --pathb <windows compare>
//       --linux-pathb <linux-x64 compare>` pairs each Windows leg with each Linux x64 leg of the
//       same CPU model from these legs (the 09:66 cross-host form).
//       A leg that was not read is in `runners` only (with its `error`): m2-exit then reads
//       NO AGREEMENT, as the same-model pairs of the dispatch are not known. It also reads NO
//       AGREEMENT when the legs name more than one run_id, or only some legs name one (EI2-R1);
//       when a compare does not hold each slot id once (gha-pathb-w1..w4, gha-pathb-l1 and -l2;
//       EI2-R3-LEG-SLOT); and when a windows compare with `legs` has no --linux-pathb (option B).
//       With no run_id on any leg (an older input), m2-exit gives a note only.
//
// Exit status: 0 done; 2 usage error or an input that cannot be read; 3 no baseline (host) or
// no pair (pathb).
// Tests: node --test .github/scripts/exit-inputs.test.mjs

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { readM2Gate, TS } from "../../tools/bench/cross/m2gate.mjs";
import { cpuModel, loadSource, parseToml } from "../../tools/bench/lib/host.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const SCHEMA = "ep9-pathb-compare/v1";
/** The box protocol: 31 paired rounds (10-quality.md section 3; m2_exit.rs PATHB_ROUNDS). */
export const PATHB_ROUNDS = 31;
/** The M2 builds of every speed cell (V12-D6). */
export const BUILDS = ["jit", "jitless"];

/**
 * The Path B cells (m2_exit.rs PATHB_ITEMS): the m2gate reading of each M2 speed item and its
 * threshold. Item 3 is the TS ratio to node --jitless, the Linux x64 rule (V12-D5), which the
 * win-x64 cells use too (V13-D37).
 */
export const CELLS = [
  { item: 1, id: "octane7-vs-qjs", cell: "item 1: Octane-7 geomean vs QuickJS", threshold: 1.33 },
  { item: 2, id: "v8v7-7-vs-node-jitless", cell: "item 2: v8-v7 geomean vs node --jitless", threshold: 1.0 },
  { item: 3, id: TS.id, cell: "item 3: TS vs node --jitless (Linux x64 rule)", threshold: TS.minRatio },
  { item: 6, id: "splay-vs-node-jitless", cell: "item 6: splay vs node --jitless", threshold: 1.0 },
];

export class UsageError extends Error {}

// ---- host ----------------------------------------------------------------------------------

/** The `idle_baseline_load1 = <n>` line of idle.mjs's output, or null. */
export function parseIdle(stdout) {
  const m = (stdout ?? "").match(/^idle_baseline_load1 = ([0-9.]+)\s*$/m);
  return m ? Number(m[1]) : null;
}

const tomlString = (s) => `"${String(s).replace(/["\\#]/g, "_")}"`;

/**
 * The host file text (the TOML subset of lib/host.mjs: no '#', '"' or '\' in a string). `runId`
 * and `runAttempt` (GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT; EI2-R1) give the run_id and run_attempt
 * lines only when they are set: a run off GitHub (a box) has none.
 */
export function hostToml({ id, runId, runAttempt, cpu, cores, platform, osText, idle, source, measured }) {
  const run = (k, v) => (v === undefined || v === null || String(v) === "" ? [] : [`${k} = ${tomlString(v)}`]);
  return [
    "# Host file of a GitHub-hosted runner, written by .github/scripts/exit-inputs.mjs host",
    `# (${measured}). A GitHub runner is never a gate host (10-quality.md section 3).`,
    `id = ${tomlString(id)}`,
    ...run("run_id", runId),
    ...run("run_attempt", runAttempt),
    `cpu = ${tomlString(cpu)}`,
    `cores = ${cores}`,
    `platform = ${tomlString(platform)}`,
    `os = ${tomlString(osText)}`,
    "gate_host = false",
    `idle_baseline_load1 = ${idle}`,
    `load_source = ${tomlString(source)}`,
    "",
  ].join("\n");
}

/**
 * The host subcommand. `deps.idle(args)` runs idle.mjs: { status, stdout, stderr }. `deps.env`
 * (process.env by default) gives GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT (EI2-R1).
 */
export function host(o, deps = {}) {
  const env = deps.env ?? process.env;
  const idle =
    deps.idle ??
    ((args) => spawnSync(process.execPath, [path.join(ROOT, "tools/bench/cross/idle.mjs"), ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }));
  const r = idle(["--minutes", String(o.minutes), "--every-s", String(o.everyS)]);
  const base = r.status === 0 ? parseIdle(r.stdout) : null;
  if (base === null) return { status: 3, text: null, why: `idle.mjs gave no baseline (exit ${r.status}): ${(r.stdout ?? "").trim().split("\n").slice(-1)[0] ?? ""}` };
  const info = deps.info ?? {
    cpu: cpuModel(),
    cores: os.cpus().length,
    platform: `${process.platform}-${process.arch}`,
    osText: `${os.type()} ${os.release()}`,
    source: loadSource(),
  };
  const text = hostToml({
    id: o.id,
    runId: env.GITHUB_RUN_ID,
    runAttempt: env.GITHUB_RUN_ATTEMPT,
    ...info,
    idle: base,
    measured: `idle baseline: ${o.minutes} min, a sample every ${o.everyS} s, ${new Date().toISOString()}`,
  });
  return { status: 0, text, why: null };
}

// ---- pathb ---------------------------------------------------------------------------------

/** One side of a box interval against threshold t (m2_exit.rs pathb_side). */
export function side(interval, t) {
  if (!Array.isArray(interval) || interval.length !== 2 || !interval.every(Number.isFinite) || interval[0] > interval[1]) return null;
  return interval[0] >= t ? "above" : interval[1] < t ? "below" : "straddles";
}

/** The result word of two sides (m2_exit.rs pathb_form). */
export function agreement(s1, s2) {
  if (s1 === "above" && s2 === "above") return "AGREE above";
  if (s1 === "below" && s2 === "below") return "AGREE below";
  if ((s1 === "above" && s2 === "below") || (s1 === "below" && s2 === "above")) return "OPPOSITE";
  return "NO AGREEMENT";
}

/**
 * The m2gate reasons that a GitHub runner always has (the run is provisional because the
 * runner is not a gate host). Path B reads such runs (V14-D12): m2-exit checks the commit, the
 * rounds and the load guard of each run itself (pathb_form), so these reasons alone do not
 * stop a box interval. Any other reason does.
 */
const RUN_REASONS = [/^not a gate host$/, /^provisional run \(/];
/** The TS reasons of the QuickJS ratio: not part of the item 3 cell (its comparator is node). */
const QJS_REASONS = [/^no qjs row$/, /^the net ratio to qjs /, /^ratios to qjs void /, /^the qjs row is not valid: /];

/**
 * Why a box reading must not give the cell an interval (empty: it can). Gates read valid net
 * readings only (m2gate.mjs, exit2.mjs `reading`): a void reading (IQR > 5%), a void or net
 * invalid kernel row, a gross fallback (no net ratio, or net ratios not paired round by round),
 * and every other m2gate reason except those of RUN_REASONS (a missing interval gives no side
 * anyway). A run label of a kernel row is "provisional" or "valid"; any other label (void, net
 * invalid) stops it.
 */
export function boxBlockers(r, spec) {
  const why = [];
  if (r.status === "incomplete") why.push("no reading");
  if (spec.id === TS.id) {
    const v = r.vs_node_jitless?.net;
    if (!v || !Number.isFinite(v.median)) why.push("no net ratio to node --jitless");
    else if (v.void) why.push("the net ratio to node --jitless is void");
  } else {
    if (!r.net) why.push("no net ratio (gross only)");
    else if (r.net.void) why.push("the per-round geomeans are void");
    for (const k of r.kernels ?? []) {
      const bad = (k.status ?? []).filter((x) => x !== "provisional" && x !== "valid");
      if (bad.length) why.push(`kernel ${k.workload}: ${bad.join(", ")}`);
    }
  }
  const allowed = spec.id === TS.id ? [...RUN_REASONS, ...QJS_REASONS] : RUN_REASONS;
  for (const x of r.reasons ?? []) if (!allowed.some((re) => re.test(x)) && !why.includes(x)) why.push(x);
  return why;
}

/**
 * The box value of one cell from an m2gate result: { median, interval, note }. The interval is
 * null when boxBlockers gives a reason (the note names them): m2-exit pathb_form reads only the
 * interval, so the producer must not give one for a reading that the gates refuse (M9-P1 Codex
 * r1 F1).
 */
export function boxValue(gate, spec, build) {
  const r = gate?.readings?.find((x) => x.id === spec.id && x.build === build);
  if (!r) return { median: null, interval: null, note: "no reading" };
  const why = boxBlockers(r, spec);
  // An interval that a reason stops is dropped, and the note says why.
  const box = (median, interval, note) => (interval && why.length ? { median, interval: null, note: `${note}; no interval: ${why.join("; ")}` } : { median, interval, note });
  if (spec.id === TS.id) {
    // The TS cell is the net ratio to node --jitless (the Linux x64 rule; gates read net only).
    const v = r.vs_node_jitless?.net;
    if (!v) return { median: null, interval: null, note: "no net ratio to node --jitless" };
    return box(v.median, v.interval ?? null, ["net", r.status, v.void ? "void" : null].filter(Boolean).join(", "));
  }
  const metric = r.net ? "net" : "gross";
  return box(r.value ?? null, r.interval ?? null, [metric, r.status].filter(Boolean).join(", "));
}

/** A CPU model that names a model: not null, not empty, not the cpuModel sentinel "unknown". */
export const knownCpu = (cpu) => typeof cpu === "string" && cpu.trim() !== "" && cpu.trim() !== "unknown";

/**
 * Pairs the legs: the first 2 of the first (platform, CPU model) group that has 2, in the order
 * given. A leg with no CPU model, or with the cpuModel sentinel "unknown" (host.mjs), is never
 * in a pair.
 */
export function pairLegs(legs) {
  const by = new Map();
  for (const l of legs) {
    if (!knownCpu(l.cpu)) continue;
    const k = JSON.stringify([l.platform ?? null, l.cpu]);
    if (!by.has(k)) by.set(k, []);
    by.get(k).push(l);
  }
  for (const ls of by.values()) if (ls.length >= 2) return { cpu: ls[0].cpu, boxes: ls.slice(0, 2) };
  return null;
}

/**
 * Reads a leg dir: { name, id, runId, runAttempt, cpu, platform, jit, jitless, error }. `id`,
 * `runId` and `runAttempt` are the id, run_id and run_attempt of the leg's host.toml (the host
 * step writes it in the leg's out/; EI2-R1, EI2-R3-LEG-SLOT), null when the file or the key is
 * not there (an older leg). A host.toml that cannot be read is a read error of the leg. A run
 * file whose host.id is not the host.toml id is a read error too (not the run files of this leg).
 */
export function readLeg(dir) {
  const name = path.basename(path.resolve(dir));
  const read = (f) => JSON.parse(readFileSync(path.join(dir, f), "utf8"));
  const none = { id: null, runId: null, runAttempt: null };
  try {
    const jit = read("run-jit.json");
    const jitless = read("run-jitless.json");
    const file = path.join(dir, "host.toml");
    let ids = none;
    if (existsSync(file)) {
      let h;
      try {
        h = parseToml(readFileSync(file, "utf8"));
      } catch (e) {
        throw new Error(`host.toml: ${e.message}`);
      }
      ids = { id: h.id ?? null, runId: h.run_id ?? null, runAttempt: h.run_attempt ?? null };
      for (const [b, r] of [["jit", jit], ["jitless", jitless]])
        if (ids.id !== null && r.host?.id !== ids.id) throw new Error(`host.toml id ${ids.id}: run-${b}.json names host ${r.host?.id ?? "none"} (not the run files of this leg)`);
    }
    return { name, ...ids, cpu: jit.host?.measured?.cpu ?? null, platform: jit.host?.measured?.platform ?? null, jit, jitless, error: null };
  } catch (e) {
    return { name, ...none, cpu: null, platform: null, jit: null, jitless: null, error: e.message };
  }
}

/**
 * The ep9-pathb-compare/v1 object of the legs (see the header). `deps.readGate` (m2gate.mjs
 * readM2Gate by default) is injectable for tests.
 */
/** The commit that a leg's jit binary names (P0-BUILDID), or null. */
const legCommit = (l) => l.jit.commit?.binary?.commit ?? l.jit.commit?.ep9 ?? null;

/** The m2gate result of a leg's 2 views: { gate, why } (`why`: readGate refused them). */
function legGate(leg, readGate) {
  try {
    return {
      gate: readGate([
        { res: leg.jit, source: `${leg.name}/run-jit.json` },
        { res: leg.jitless, source: `${leg.name}/run-jitless.json` },
      ]),
      why: null,
    };
  } catch (e) {
    return { gate: null, why: e.message };
  }
}

/**
 * The run check of one build of a leg: the run is of `commit`, its binary was built clean
 * (P0-BUILDID) and the gate read the build.
 */
function runCheck(res, build, commit, g) {
  const own = g.gate?.builds?.find((b) => b.build === build);
  return {
    build,
    commit_ok: !!commit && res.commit?.ep9 === commit && res.commit?.binary?.dirty === false && !!own,
    rounds: res.M?.rounds ?? null,
    platform: res.host?.measured?.platform ?? null,
    cpu: res.host?.measured?.cpu ?? null,
    load: { before: res.load?.before ?? null, after: res.load?.after ?? null },
    ...(g.why ? { error: g.why } : {}),
  };
}

/**
 * The gross ep9 TS time (s) of a build of a leg: { median, interval, note }. The win-x64
 * same-CPU-model form reads the TS speed as 1 / this time at each end (10-quality.md section 3,
 * P0-WININT), not the ratio to node --jitless (EI2 Codex r1 F2). The interval is null when
 * boxBlockers gives a reason or the gross times are void (the note names them).
 */
export function tsTime(gate, build) {
  const r = gate?.readings?.find((x) => x.id === TS.id && x.build === build);
  if (!r) return { median: null, interval: null, note: "no reading" };
  const why = boxBlockers(r, CELLS.find((s) => s.id === TS.id));
  if (r.gross_s?.void) why.push("gross times void");
  const note = ["gross", r.status].filter(Boolean).join(", ");
  return { median: r.value ?? null, interval: why.length ? null : (r.interval ?? null), note: why.length ? `${note}; no interval: ${why.join("; ")}` : note };
}

/**
 * The `legs` block (V15-D16): every leg that was read, with its own commit, a run check per
 * build against that commit, and the box value of each cell per build (for item 3 also
 * `time_s`, the gross ep9 TS time: see tsTime). The leg's host.toml id, run_id and run_attempt
 * (readLeg) are copied when they are set; m2-exit checks them (EI2-R1, EI2-R3-LEG-SLOT).
 */
function legBlocks(usable, gateOf) {
  const set = (k, v) => (v === undefined || v === null ? {} : { [k]: v });
  return usable.map((l) => {
    const commit = legCommit(l);
    const g = gateOf(l);
    return {
      leg: l.name,
      ...set("id", l.id),
      ...set("run_id", l.runId),
      ...set("run_attempt", l.runAttempt),
      cpu: l.cpu,
      platform: l.platform,
      commit,
      checks: BUILDS.map((build) => runCheck(l[build], build, commit, g)),
      cells: BUILDS.flatMap((build) => CELLS.map((spec) => ({ build, cell: spec.cell, value: boxValue(g.gate, spec, build), ...(spec.id === TS.id ? { time_s: tsTime(g.gate, build) } : {}) }))),
    };
  });
}

export function compare(legs, deps = {}) {
  const readGate = deps.readGate ?? readM2Gate;
  const only = deps.platform ?? null;
  const other = (l) => only !== null && !l.error && l.platform !== only;
  const runners = legs.map((l) => ({ leg: l.name, cpu: l.cpu, platform: l.platform, error: other(l) ? `platform ${l.platform ?? "unknown"}, not ${only} (not read)` : l.error }));
  const usable = legs.filter((l) => !l.error && !other(l));
  const pair = pairLegs(usable);
  // One m2gate read per leg, for the pair and for the legs block.
  const seen = new Map();
  const gateOf = (l) => {
    if (!seen.has(l)) seen.set(l, legGate(l, readGate));
    return seen.get(l);
  };
  if (!pair) {
    return {
      schema: SCHEMA,
      commit: null,
      platform: null,
      pair: null,
      result: "NO PAIR: no 2 runners of one CPU model; run the dispatch again once, then the reading is NOT-RUN (a DL line)",
      runners,
      boxes: [],
      checks: [],
      cells: [],
      legs: legBlocks(usable, gateOf),
    };
  }
  const [b1, b2] = pair.boxes;
  // The commit that box 1's jit binary names (P0-BUILDID); each run must be a clean build of it.
  const commit = legCommit(b1);
  // The platform of the compare: both boxes' runs name it (m2-exit checks it against --host).
  const plat = b1.platform && b1.platform === b2.platform ? b1.platform : null;
  const checks = [];
  const gates = [];
  for (const [n, leg] of [[1, b1], [2, b2]]) {
    const g = gateOf(leg);
    gates.push(g.gate);
    for (const build of BUILDS) checks.push({ box: n, leg: leg.name, ...runCheck(leg[build], build, commit, g) });
  }
  const cells = [];
  for (const build of BUILDS) {
    for (const spec of CELLS) {
      const box1 = boxValue(gates[0], spec, build);
      const box2 = boxValue(gates[1], spec, build);
      const sides = [side(box1.interval, spec.threshold), side(box2.interval, spec.threshold)];
      cells.push({
        group: "A. M2 gate items (m2gate.mjs)",
        build,
        cell: spec.cell,
        threshold: spec.threshold,
        box1,
        box2,
        sides,
        result: agreement(...sides),
      });
    }
  }
  return {
    schema: SCHEMA,
    commit,
    platform: plat,
    pair: { cpu: pair.cpu, legs: [b1.name, b2.name] },
    result: "paired",
    runners,
    boxes: [b1.name, b2.name],
    checks,
    cells,
    legs: legBlocks(usable, gateOf),
  };
}

const f3 = (x) => (Number.isFinite(x) ? x.toFixed(3) : "-");
const iv = (i) => (Array.isArray(i) ? `${f3(i[0])}-${f3(i[1])}` : "none");

export function renderMarkdown(c) {
  const L = [];
  L.push(`Path B compare (${SCHEMA}), platform ${c.platform ?? "unknown"}, commit \`${(c.commit ?? "unknown").slice(0, 12)}\`.`);
  L.push("");
  L.push("| runner leg | CPU model | platform | read |");
  L.push("|---|---|---|---|");
  for (const r of c.runners) L.push(`| ${r.leg} | ${r.cpu ?? "-"} | ${r.platform ?? "-"} | ${r.error ? `no (${r.error.replace(/\|/g, "\\|")})` : "yes"} |`);
  L.push("");
  if (!c.pair) {
    L.push(`**${c.result}.**`);
    L.push("");
    legTable(c, L);
    return `${L.join("\n")}\n`;
  }
  L.push(`Pair: ${c.pair.legs.join(" and ")} (${c.pair.cpu}).`);
  L.push("");
  L.push("| box | build | commit ok | rounds | load before | load after |");
  L.push("|---|---|---|---|---|---|");
  const g = (x) => (!x ? "-" : `${x.load1 ?? "-"} / ${x.limit ?? "-"} ${x.pass ? "pass" : "FAIL"}`);
  for (const k of c.checks) L.push(`| ${k.box} | ${k.build} | ${k.commit_ok ? "yes" : "no"} | ${k.rounds ?? "-"} | ${g(k.load.before)} | ${g(k.load.after)} |`);
  L.push("");
  L.push("| build | cell | threshold | box 1 | box 2 | result |");
  L.push("|---|---|---|---|---|---|");
  for (const x of c.cells) L.push(`| ${x.build} | ${x.cell} | ${x.threshold} | ${f3(x.box1.median)} [${iv(x.box1.interval)}] | ${f3(x.box2.median)} [${iv(x.box2.interval)}] | ${x.result} |`);
  L.push("");
  const ts = c.cells.filter((x) => x.cell.startsWith("item 3:"));
  L.push(`TS ratio to node --jitless (V14-D59 input): ${ts.map((x) => `${x.build} box 1 ${f3(x.box1.median)}, box 2 ${f3(x.box2.median)}`).join("; ")}.`);
  L.push("");
  legTable(c, L);
  return `${L.join("\n")}\n`;
}

/** The legs table (V15-D16 input): the value of each cell per leg and build. */
function legTable(c, L) {
  const legs = c.legs ?? [];
  if (!legs.length) return;
  L.push("Per leg (the input of the win-x64 same-CPU-model cell, V15-D16):");
  L.push("");
  L.push(`| leg | CPU model | build | commit ok | ${CELLS.map((s) => `item ${s.item}`).join(" | ")} |`);
  L.push(`|---|---|---|---|${CELLS.map(() => "---|").join("")}`);
  for (const l of legs) {
    for (const k of l.checks) {
      const v = CELLS.map((s) => l.cells.find((x) => x.build === k.build && x.cell === s.cell)?.value).map((x) => `${f3(x?.median)} [${iv(x?.interval)}]`);
      L.push(`| ${l.leg} | ${l.cpu ?? "-"} | ${k.build} | ${k.commit_ok ? "yes" : "no"} | ${v.join(" | ")} |`);
    }
  }
}

// ---- main ----------------------------------------------------------------------------------

export function parseArgs(argv) {
  const [cmd, ...rest] = argv;
  if (cmd !== "host" && cmd !== "pathb") throw new UsageError("the first argument is host or pathb");
  const o = { cmd, out: null, md: null, id: `gha-${process.platform}-${process.arch}`, minutes: 10, everyS: 10, legs: [], platform: null };
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    const val = () => {
      if (i + 1 >= rest.length) throw new UsageError(`${a} needs a value`);
      return rest[++i];
    };
    const num = () => {
      const n = Number(val());
      if (!(Number.isFinite(n) && n > 0)) throw new UsageError(`${a} needs a number > 0`);
      return n;
    };
    if (a === "--out") o.out = val();
    else if (a === "--md") o.md = val();
    else if (cmd === "host" && a === "--id") o.id = val();
    else if (cmd === "host" && a === "--minutes") o.minutes = num();
    else if (cmd === "host" && a === "--every-s") o.everyS = num();
    else if (cmd === "pathb" && a === "--leg") o.legs.push(val());
    else if (cmd === "pathb" && a === "--platform") o.platform = val();
    else throw new UsageError(`unknown option ${a} for ${cmd}`);
  }
  if (!o.out) throw new UsageError("--out is required");
  if (cmd === "pathb" && !o.legs.length) throw new UsageError("pathb needs at least one --leg");
  return o;
}

function main() {
  let o;
  try {
    o = parseArgs(process.argv.slice(2));
  } catch (e) {
    console.error(`exit-inputs.mjs: ${e.message}`);
    process.exit(2);
  }
  if (o.cmd === "host") {
    const r = host(o);
    if (r.status !== 0) {
      console.error(`exit-inputs.mjs host: ${r.why}`);
      process.exit(r.status);
    }
    writeFileSync(o.out, r.text);
    process.stdout.write(r.text);
    return;
  }
  const c = compare(o.legs.map(readLeg), { platform: o.platform });
  writeFileSync(o.out, `${JSON.stringify(c, null, 1)}\n`);
  const md = renderMarkdown(c);
  if (o.md) writeFileSync(o.md, md);
  process.stdout.write(md);
  process.exit(c.pair ? 0 : 3);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
