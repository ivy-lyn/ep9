#!/usr/bin/env node
// The M2 gate reader (docs/plan/09-milestones.md M2 exit 1, 2, 3, 6 and 10; plan v12 GATES10
// task 2): from the ep9-cross/v1 runs of the jit preset build and the jitless build (run.mjs
// --workloads m2 --harness base), the speed readings of each build:
//
//   octane7-vs-qjs           Octane-7 geomean vs QuickJS             low end of the interval >= 1.33   item 1
//   v8v7-7-vs-node-jitless   v8-v7 7-kernel geomean vs node --jitless   low end >= 1.0             item 2
//                            (each kernel >= 0.8, else "parity debt", 09-milestones.md section 5)
//   ts                       TS workload (octane-typescript), V12-D5                              item 3
//   splay-vs-node-jitless    v8-v7 splay vs node --jitless           low end >= 1.0                item 6
//
//   node tools/bench/cross/m2gate.mjs run-jit.json run-jitless.json [--out m2gate.json]
//
// Item 10 (gates 1-3 with all profiling on) is the jit preset build: its preset's default
// feedback is "full". The D11 row is the jitless build (11-decisions.md D11). A cell passes only
// if both builds pass (V12-D6), so `pass` needs both runs. cargo xtask m2-exit reads the JSON.
//
// Build. Each run names its build from the ep9 build line (`ep9 --version` line 2, run.mjs
// `engine_build`): "jit" = preset jit with the feature jit; "jitless" = preset jitless without
// it. A run of another build, two runs of one build, or two runs from two hosts (host id, CPU or
// platform) are usage errors.
//
// Geomean readings: exit2.mjs `reading` (the per-round geomean of the net ratios, the
// order-statistic interval, the IQR void rule, the real base.js harness, >= 10 paired rounds).
// Gate: "pass" when the interval's low end is >= the threshold, "fail" when its high end is
// below it, else "marginal"; no gate verdict for a provisional run (no gate host, load guard).
//
// TS reading (V12-D5): ep9's process wall time of octane-typescript per paired round, gross and
// net (minus ep9's empty-script time in the same round), median and the order-statistic interval
// (n = 10: ranks 2 and 9, 97.9%), and the paired ratios to node --jitless and QuickJS (on every
// host both must have a valid net ratio, not a gross fallback, and not be void, else no gate
// verdict). The rule
// depends on the host: on m5max-macos the upper end of the gross interval must be <= 2.8 s
// ("fail" when its low end is above 2.8 s); on a Linux x64 host the net ratio to node --jitless
// must be >= 1.0 (the interval rule); on any other host there is no TS rule (no gate verdict,
// but the status is "valid" when the measurement is: the Windows x64 ratio to Linux x64 reads it).
// `value` is the gross median in seconds.
//
// Exit status: 0 every reading of both builds passes; 1 not; 2 usage error.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { verdict } from "../lib/stats.mjs";
import { reading } from "./exit2.mjs";
import { MIN_ROUNDS } from "./run.mjs";
import { EMPTY, OCTANE_7, TS_WORKLOAD, V8V7_7 } from "./workloads.mjs";

export const SCHEMA = "ep9-m2gate/v1";

/** The two builds of every M2 speed reading (V12-D6). */
export const BUILDS = ["jit", "jitless"];

/** The geomean readings (exit2.mjs `reading` specs) and the M2 exit item of each. */
export const READINGS = [
  { id: "octane7-vs-qjs", item: 1, label: "Octane-7 geomean vs QuickJS", workloads: OCTANE_7, comparator: "qjs", threshold: 1.33, harness: "octane-base" },
  { id: "v8v7-7-vs-node-jitless", item: 2, label: "v8-v7 7-kernel geomean vs node --jitless", workloads: V8V7_7, comparator: "node-jitless", threshold: 1.0, harness: "base", kernelFloor: 0.8 },
  { id: "splay-vs-node-jitless", item: 6, label: "splay vs node --jitless", workloads: ["splay"], comparator: "node-jitless", threshold: 1.0, harness: "base" },
];

/** V12-D5: the TS rule per host. */
export const TS = {
  id: "ts",
  item: 3,
  label: "TS workload (octane-typescript)",
  workload: TS_WORKLOAD,
  harness: "octane-base",
  // m5max-macos: the gross ep9 time, seconds (docs/research/06-execution-tiers.md: V8 jitless).
  macHost: "m5max-macos",
  maxSeconds: 2.8,
  // A Linux x64 gate host: >= 1.0x node --jitless.
  linuxPlatform: "linux-x64",
  minRatio: 1.0,
};

export class UsageError extends Error {}

/** The build of a run from its ep9 build line, or null. */
export function buildOf(res) {
  const line = res.engines?.find((e) => e.id === "ep9")?.engine_build ?? null;
  const m = line?.match(/^features (.*); preset (\S+)$/);
  if (!m) return { build: null, line };
  const features = m[1].split(",").map((f) => f.trim());
  if (m[2] === "jit" && features.includes("jit")) return { build: "jit", line };
  if (m[2] === "jitless" && !features.includes("jit")) return { build: "jitless", line };
  return { build: null, line };
}

const gateSide = (interval, threshold) => (!interval ? null : interval[0] >= threshold ? "pass" : interval[1] < threshold ? "fail" : "marginal");

/** The v8-v7 kernels below the floor: "debt" (whole interval below), "marginal" or "none". */
function kernelDebt(kernels, floor) {
  return kernels.map((k) => {
    const side = gateSide(k.interval, floor);
    return { ...k, floor, debt: side === "pass" ? "none" : side === "fail" ? "debt" : "marginal" };
  });
}

/** One geomean reading of one build. */
function geomeanReading(res, spec, build) {
  const r = reading(res, spec);
  const out = { ...r, item: spec.item, build, value: (r.net ?? r.gross)?.median ?? null, interval: (r.net ?? r.gross)?.interval ?? null };
  if (spec.kernelFloor) {
    out.kernels = kernelDebt(r.kernels, spec.kernelFloor);
    // A kernel not proven >= 0.8x (below or marginal) is debt until a reading clears it.
    out.parity_debt = out.kernels.filter((k) => k.debt !== "none").map((k) => k.workload);
  }
  return out;
}

/** The per-round series of `engine` on `workload` (no warm-up), in round order, or null. */
function series(res, workload, engine) {
  const xs = [];
  for (const s of res.M?.samples ?? []) {
    if (s.warmup || s.workload !== workload || s.engine !== engine) continue;
    xs[s.round - (res.options?.warmup ?? 0)] = s.ms;
  }
  const rounds = res.M?.rounds;
  return Array.from(xs).length === rounds && Array.from(xs).every((x) => typeof x === "number") ? xs : null;
}

/** The TS reading of one build. */
export function tsReading(res, build) {
  const out = { id: TS.id, item: TS.item, label: TS.label, build, workload: TS.workload };
  const reasons = [];
  if (res.provisional) reasons.push(`provisional run (${(res.provisional_reasons ?? []).join("; ") || "no reason given"})`);
  if (!res.host?.gate_host) reasons.push("not a gate host");
  const w = (res.workloads ?? []).find((x) => x.name === TS.workload);
  if (!w) {
    return { ...out, status: "incomplete", gate: "no reading", reasons: [`no ${TS.workload} workload in the run`], value: null, interval: null };
  }
  if (w.harness !== TS.harness) reasons.push(`harness is not ${TS.harness} (${w.harness ?? "unknown"})`);
  const declared = res.M?.rounds;
  if (!(Number.isInteger(declared) && declared >= MIN_ROUNDS)) reasons.push(`the run has ${declared ?? "no"} paired rounds (M.rounds); class M needs >= ${MIN_ROUNDS}`);
  if (res.options?.rounds !== declared) reasons.push(`M.rounds (${declared}) differs from options.rounds (${res.options?.rounds})`);
  const ts = series(res, TS.workload, "ep9");
  const empty = series(res, EMPTY, "ep9");
  if (!ts || !empty) {
    return { ...out, status: "incomplete", gate: "no reading", reasons: [...reasons, `the ep9 ${TS.workload} or empty-script samples are not ${declared} paired rounds`], value: null, interval: null };
  }
  const gross = verdict(ts.map((x) => x / 1000), TS.maxSeconds);
  const netXs = ts.map((x, r) => (x - empty[r]) / 1000);
  const net = netXs.every((x) => x > 0) ? verdict(netXs, TS.maxSeconds) : null;
  const ratio = (c) => {
    const row = res.M?.ratios?.find((x) => x.workload === TS.workload && x.comparator === c);
    if (!row) return null;
    const pick = (v) => v && { median: v.median, interval: v.interval, iqrPct: v.iqrPct, void: v.void };
    return { comparator: c, metric: row.metric, net: pick(row.net), gross: pick(row.gross), status: row.status };
  };
  const vsNode = ratio("node-jitless");
  const vsQjs = ratio("qjs");
  // V12-D5: both comparators are part of the reading on every host (class M; the paired ratio
  // to node --jitless is always reported), so a missing or void ratio gives no gate verdict
  // (M7-P0 Codex r1 GATES10-2). Gates read net ratios only: a gross fallback (net time <= 0 in
  // a round) gives no gate verdict, and the row status must be "valid", as in exit2.mjs
  // reading() (M7-P0 Codex r2 C2-1).
  for (const [name, r] of [["node-jitless", vsNode], ["qjs", vsQjs]]) {
    const v = r?.net;
    if (!r) reasons.push(`no ${name} row`);
    else if (!v || !Number.isFinite(v.median)) reasons.push(`the net ratio to ${name} is invalid (${(r.status ?? []).join(", ") || "no status"})`);
    else if (v.void) reasons.push(`ratios to ${name} void (IQR ${v.iqrPct.toFixed(1)}% > 5%)`);
    else if (!res.provisional && !(r.status?.length === 1 && r.status[0] === "valid")) reasons.push(`the ${name} row is not valid: ${(r.status ?? ["no status"]).join(", ")}`);
  }
  let rule = null;
  let side = null;
  let noRule = null;
  if (res.host?.id === TS.macHost) {
    rule = `${TS.macHost}: the upper end of the 97.9% interval of the median gross ep9 time <= ${TS.maxSeconds} s`;
    if (gross.void) reasons.push(`gross times void (IQR ${gross.iqrPct.toFixed(1)}% > 5%)`);
    if (!gross.interval) reasons.push(`no interval (n = ${gross.n})`);
    side = !gross.interval ? null : gross.interval[1] <= TS.maxSeconds ? "pass" : gross.interval[0] > TS.maxSeconds ? "fail" : "marginal";
  } else if (res.host?.measured?.platform === TS.linuxPlatform) {
    rule = `${TS.linuxPlatform}: the net ratio to node --jitless >= ${TS.minRatio} (the whole interval)`;
    // A missing row, an invalid net ratio or a void ratio already has its reason (above).
    side = gateSide(vsNode?.net?.interval, TS.minRatio);
  } else {
    // No TS threshold on this host (V12-D5): no gate verdict, but the measurement can still be
    // valid, so that the Windows x64 ratio to Linux x64 (09:66) can read it (M7-P0 Codex r2
    // C2-3). "valid" here means: the gross times are not void and have an interval.
    noRule = `no TS rule for host ${res.host?.id ?? "(no host file)"} (${res.host?.measured?.platform ?? "?"}): V12-D5 names m5max-macos and Linux x64`;
    rule = noRule;
    if (gross.void) reasons.push(`gross times void (IQR ${gross.iqrPct.toFixed(1)}% > 5%)`);
    if (!gross.interval) reasons.push(`no interval (n = ${gross.n})`);
  }
  const iv = (v) => v && { median: v.median, interval: v.interval, iqrPct: v.iqrPct, void: v.void, n: v.n };
  return {
    ...out,
    status: reasons.length ? "provisional" : "valid",
    gate: reasons.length ? `no gate verdict (the interval reads ${side ?? "nothing"})` : noRule ? `no gate verdict (${noRule})` : side,
    reasons,
    rule,
    value: gross.median,
    interval: gross.interval,
    gross_s: iv(gross),
    net_s: iv(net),
    vs_node_jitless: vsNode,
    vs_qjs: vsQjs,
  };
}

/** The build block of one run. */
function block(res, build, source) {
  return {
    build,
    source,
    commit: res.commit?.ep9 ?? null,
    dirty: Array.isArray(res.commit?.dirty) ? res.commit.dirty.length > 0 : null,
    dirty_paths: Array.isArray(res.commit?.dirty) ? res.commit.dirty : null,
    runner_commit: res.commit?.runner ?? null,
    engine_build: res.engines?.find((e) => e.id === "ep9")?.engine_build ?? null,
    started: res.started ?? null,
    host: {
      id: res.host?.id ?? null,
      gate_host: res.host?.gate_host ?? false,
      cpu: res.host?.measured?.cpu ?? null,
      platform: res.host?.measured?.platform ?? null,
      transparent_hugepage: res.host?.measured?.transparent_hugepage ?? null,
    },
    // Both load-guard values (10-quality.md section 3).
    load: { before: res.load?.before ?? null, after: res.load?.after ?? null },
    provisional: res.provisional ?? true,
    provisional_reasons: res.provisional_reasons ?? [],
  };
}

/** The ep9-m2gate/v1 result of runs [{ res, source }]. */
export function readM2Gate(runs) {
  if (!runs.length) throw new UsageError("give the run of the jit preset build and the run of the jitless build");
  const builds = [];
  const readings = [];
  for (const { res, source } of runs) {
    if (res?.schema !== "ep9-cross/v1") throw new UsageError(`${source}: not an ep9-cross/v1 run (schema ${JSON.stringify(res?.schema)})`);
    const { build, line } = buildOf(res);
    if (!build) throw new UsageError(`${source}: the ep9 build line ${JSON.stringify(line)} is neither the jit preset (preset jit, feature jit) nor the jitless build (preset jitless, no feature jit)`);
    if (builds.some((b) => b.build === build)) throw new UsageError(`${source}: a second run of the ${build} build`);
    const b = block(res, build, source);
    const first = builds[0];
    if (first && (first.host.id !== b.host.id || first.host.cpu !== b.host.cpu || first.host.platform !== b.host.platform)) {
      throw new UsageError(`${source}: measured on ${b.host.id ?? "no host file"} (${b.host.cpu}, ${b.host.platform}), the ${first.build} run on ${first.host.id ?? "no host file"} (${first.host.cpu}, ${first.host.platform}): one host per m2gate file`);
    }
    builds.push(b);
    for (const spec of READINGS) readings.push(geomeanReading(res, spec, build));
    readings.push(tsReading(res, build));
  }
  const missing = BUILDS.filter((x) => !builds.some((b) => b.build === x));
  return {
    schema: SCHEMA,
    builds,
    missing_builds: missing,
    readings,
    pass: missing.length === 0 && readings.every((r) => r.gate === "pass"),
  };
}

const f3 = (x) => (x == null ? "-" : x.toFixed(3));
const iv = (i) => (i ? `${f3(i[0])}-${f3(i[1])}` : "none");

export function renderMarkdown(g) {
  const L = [];
  for (const b of g.builds) {
    const dirty = b.dirty === null ? " (the run does not say whether the tree had uncommitted changes)" : b.dirty ? ` + uncommitted changes in ${b.dirty_paths.join(", ")}` : "";
    L.push(`- ${b.build}: ep9 commit \`${(b.commit ?? "unknown").slice(0, 12)}\`${dirty}, build "${b.engine_build}", host ${b.host.id ?? "no host file"}${b.host.gate_host ? " (gate host)" : ""}, ${b.host.platform}; ${b.source}.`);
  }
  for (const m of g.missing_builds) L.push(`- ${m}: no run (V12-D6: a cell passes only if both builds pass).`);
  L.push("");
  L.push("| item | reading | build | threshold | value | interval | gate |");
  L.push("|---|---|---|---|---|---|---|");
  for (const r of g.readings) {
    const th = r.id === TS.id ? (r.rule ?? "no rule") : r.threshold;
    const value = r.id === TS.id ? (r.value == null ? "-" : `${f3(r.value)} s gross`) : f3(r.value);
    L.push(`| ${r.item} | ${r.label} | ${r.build} | ${th} | ${value} | ${iv(r.interval)} | ${r.gate} |`);
  }
  for (const r of g.readings) {
    if (r.parity_debt?.length) L.push("", `${r.label} (${r.build}): parity debt (below or not proven >= 0.8x): ${r.parity_debt.join(", ")}.`);
    if (r.id === TS.id && r.gross_s) {
      const ratio = (x) => (x ? `${f3((x.net ?? x.gross)?.median)} (${x.net ? "net" : "gross"})` : "-");
      L.push("", `TS (${r.build}): gross ${f3(r.gross_s.median)} s, net ${f3(r.net_s?.median)} s; ratio to node --jitless ${ratio(r.vs_node_jitless)}, to QuickJS ${ratio(r.vs_qjs)}.`);
    }
    if (r.reasons?.length && r.gate !== "pass") L.push("", `${r.label} (${r.build}): ${r.reasons.join("; ")}.`);
  }
  L.push("");
  L.push(`M2 speed gates: ${g.pass ? "PASS (every reading of both builds; m2-exit checks the commit and the host)" : "not passed"}.`);
  return `${L.join("\n")}\n`;
}

export function parseArgs(argv) {
  const o = { inputs: [], out: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--out") {
      if (i + 1 >= argv.length) throw new UsageError("--out needs a value");
      o.out = path.resolve(argv[++i]);
    } else if (a.startsWith("-")) {
      throw new UsageError(`unknown option ${a}`);
    } else {
      o.inputs.push(path.resolve(a));
    }
  }
  if (!o.inputs.length) throw new UsageError("give the ep9-cross/v1 run of the jit preset build and the one of the jitless build");
  if (o.inputs.length > BUILDS.length) throw new UsageError(`at most ${BUILDS.length} runs (one per build)`);
  return o;
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) {
    // The usage is the comment at the top of this file.
    const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n");
    process.stdout.write(`${src.slice(1, src.findIndex((l, i) => i > 0 && !l.startsWith("//"))).map((l) => l.replace(/^\/\/ ?/, "")).join("\n")}\n`);
    return;
  }
  let g;
  try {
    const o = parseArgs(argv);
    const runs = o.inputs.map((p) => {
      try {
        return { res: JSON.parse(readFileSync(p, "utf8")), source: path.basename(p) };
      } catch (err) {
        throw new UsageError(`${p}: ${err.message}`);
      }
    });
    g = readM2Gate(runs);
    if (o.out) writeFileSync(o.out, `${JSON.stringify(g, null, 1)}\n`);
  } catch (err) {
    if (err instanceof UsageError) {
      console.error(`m2gate.mjs: ${err.message}`);
      process.exit(2);
    }
    throw err;
  }
  process.stdout.write(renderMarkdown(g));
  process.exit(g.pass ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
