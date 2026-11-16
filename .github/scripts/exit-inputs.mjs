#!/usr/bin/env node
// The exit-inputs mode of ci.yml (plan v14 section 4.1; V14-D12, V14-D59): the helpers of the
// `exit-inputs` job and of its reading. The job runs MEMREAD (tools/bench/cross/mem.mjs) and the
// JetStream M2 list (tools/bench/jetstream/run.mjs) per platform, and the Windows Path B M2 set
// (tools/bench/cross/run.mjs, both builds in each paired round) on 4 windows x64 runners.
//
//   node .github/scripts/exit-inputs.mjs host --out host.toml [--id ID] [--minutes N] [--every-s S]
//       Measures the idle baseline of this runner (tools/bench/cross/idle.mjs, its load source:
//       the 1-minute load, or typeperf on Windows) and writes a host file for the load guard:
//       id, cpu, cores, platform, os, gate_host = false (a GitHub runner is never a gate host,
//       10-quality.md section 3), idle_baseline_load1, load_source. Exit 3 when idle.mjs saw a
//       build or benchmark process (no baseline, no file).
//
//   node .github/scripts/exit-inputs.mjs pathb --leg DIR [--leg DIR]... --out compare.json
//       [--md compare.md]
//       The Path B compare of the runner legs (V14-D12, the box protocol of 10-quality.md
//       section 3): each leg DIR holds run-jit.json and run-jitless.json, the two views of one
//       both-builds run.mjs run (--out, --out-jitless). The runners are grouped by CPU model, in
//       the order given; the first 2 legs of the first model with 2 legs are box 1 and box 2.
//       The output is the ep9-pathb-compare/v1 file that `cargo xtask m2-exit --pathb` reads:
//       commit, platform (of the 2 boxes' runs; null when they differ), boxes, runners (every
//       leg and its CPU model), checks (box, build,
//       commit_ok, rounds, load) and cells (items 1, 2, 3, 6 per build: the 2 box intervals
//       against the M2 threshold, "AGREE above", "AGREE below", "OPPOSITE" or "NO AGREEMENT").
//       No 2 legs of one CPU model: the file says pair null and the exit status is 3. Then the
//       dispatch runs again once; a second miss is NOT-RUN (a DL line; plan v14 section 4.1 (6)).
//       The Markdown also gives the TS ratio to node --jitless per box and build (V14-D59).
//
// Exit status: 0 done; 2 usage error or an input that cannot be read; 3 no baseline (host) or
// no pair (pathb).
// Tests: node --test .github/scripts/exit-inputs.test.mjs

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { readM2Gate, TS } from "../../tools/bench/cross/m2gate.mjs";
import { cpuModel, loadSource } from "../../tools/bench/lib/host.mjs";

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

/** The host file text (the TOML subset of lib/host.mjs: no '#', '"' or '\' in a string). */
export function hostToml({ id, cpu, cores, platform, osText, idle, source, measured }) {
  return [
    "# Host file of a GitHub-hosted runner, written by .github/scripts/exit-inputs.mjs host",
    `# (${measured}). A GitHub runner is never a gate host (10-quality.md section 3).`,
    `id = ${tomlString(id)}`,
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

/** The host subcommand. `deps.idle(args)` runs idle.mjs: { status, stdout, stderr }. */
export function host(o, deps = {}) {
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

/** The box value of one cell from an m2gate result: { median, interval, note }. */
export function boxValue(gate, spec, build) {
  const r = gate?.readings?.find((x) => x.id === spec.id && x.build === build);
  if (!r) return { median: null, interval: null, note: "no reading" };
  if (spec.id === TS.id) {
    // The TS cell is the net ratio to node --jitless (the Linux x64 rule; gates read net only).
    const v = r.vs_node_jitless?.net;
    if (!v) return { median: null, interval: null, note: "no net ratio to node --jitless" };
    return { median: v.median, interval: v.interval ?? null, note: ["net", r.status, v.void ? "void" : null].filter(Boolean).join(", ") };
  }
  const metric = r.net ? "net" : "gross";
  return { median: r.value ?? null, interval: r.interval ?? null, note: [metric, r.status].filter(Boolean).join(", ") };
}

/** Pairs the legs: the first 2 of the first CPU model that has 2, in the order given. */
export function pairLegs(legs) {
  const by = new Map();
  for (const l of legs) {
    const k = l.cpu ?? "unknown";
    if (!by.has(k)) by.set(k, []);
    by.get(k).push(l);
  }
  for (const [cpu, ls] of by) if (cpu !== "unknown" && ls.length >= 2) return { cpu, boxes: ls.slice(0, 2) };
  return null;
}

/** Reads a leg dir: { name, cpu, platform, jit, jitless, error }. */
export function readLeg(dir) {
  const name = path.basename(path.resolve(dir));
  const read = (f) => JSON.parse(readFileSync(path.join(dir, f), "utf8"));
  try {
    const jit = read("run-jit.json");
    const jitless = read("run-jitless.json");
    return { name, cpu: jit.host?.measured?.cpu ?? null, platform: jit.host?.measured?.platform ?? null, jit, jitless, error: null };
  } catch (e) {
    return { name, cpu: null, platform: null, jit: null, jitless: null, error: e.message };
  }
}

/**
 * The ep9-pathb-compare/v1 object of the legs (see the header). `deps.readGate` (m2gate.mjs
 * readM2Gate by default) is injectable for tests.
 */
export function compare(legs, deps = {}) {
  const readGate = deps.readGate ?? readM2Gate;
  const runners = legs.map((l) => ({ leg: l.name, cpu: l.cpu, platform: l.platform, error: l.error }));
  const usable = legs.filter((l) => !l.error);
  const pair = pairLegs(usable);
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
    };
  }
  const [b1, b2] = pair.boxes;
  // The commit that box 1's jit binary names (P0-BUILDID); each run must be a clean build of it.
  const commit = b1.jit.commit?.binary?.commit ?? b1.jit.commit?.ep9 ?? null;
  // The platform of the compare: both boxes' runs name it (m2-exit checks it against --host).
  const plat = b1.platform && b1.platform === b2.platform ? b1.platform : null;
  const checks = [];
  const gates = [];
  for (const [n, leg] of [[1, b1], [2, b2]]) {
    let gate = null;
    let why = null;
    try {
      gate = readGate([
        { res: leg.jit, source: `${leg.name}/run-jit.json` },
        { res: leg.jitless, source: `${leg.name}/run-jitless.json` },
      ]);
    } catch (e) {
      why = e.message;
    }
    gates.push(gate);
    for (const [build, res] of [["jit", leg.jit], ["jitless", leg.jitless]]) {
      const own = gate?.builds?.find((b) => b.build === build);
      checks.push({
        box: n,
        leg: leg.name,
        build,
        // The run is of the compare commit and its binary was built clean (P0-BUILDID).
        commit_ok: !!commit && res.commit?.ep9 === commit && res.commit?.binary?.dirty === false && !!own,
        rounds: res.M?.rounds ?? null,
        platform: res.host?.measured?.platform ?? null,
        cpu: res.host?.measured?.cpu ?? null,
        load: { before: res.load?.before ?? null, after: res.load?.after ?? null },
        ...(why ? { error: why } : {}),
      });
    }
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
  return `${L.join("\n")}\n`;
}

// ---- main ----------------------------------------------------------------------------------

export function parseArgs(argv) {
  const [cmd, ...rest] = argv;
  if (cmd !== "host" && cmd !== "pathb") throw new UsageError("the first argument is host or pathb");
  const o = { cmd, out: null, md: null, id: `gha-${process.platform}-${process.arch}`, minutes: 10, everyS: 10, legs: [] };
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
  const c = compare(o.legs.map(readLeg));
  writeFileSync(o.out, `${JSON.stringify(c, null, 1)}\n`);
  const md = renderMarkdown(c);
  if (o.md) writeFileSync(o.md, md);
  process.stdout.write(md);
  process.exit(c.pair ? 0 : 3);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
