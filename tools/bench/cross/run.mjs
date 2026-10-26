#!/usr/bin/env node
// Cross-engine runner: ep9 against the reference shells. Wall time in paired rounds (class M) and
// instructions retired (class R), by the PERF-NOISE rule (docs/plan/10-quality.md section 3) and
// decisions PERF-1..3 (week-4 plan section 11).
//
//   node tools/bench/cross/run.mjs --ep9 target/release/ep9 --host tools/bench/hosts/m5max-macos.toml \
//     --count target/release/ep9-bench --out run.json --md run.md
//
//   --ep9 PATH          the ep9 binary (release build; required)
//   --commit SHA        the commit ep9 was built from (default: git HEAD of this checkout)
//   --engines LIST      default ep9,qjs,node-jitless,d8-jitless,brimstone (ep9 is required)
//   --shells FILE       reference shells (default third_party/shells/versions.txt, written by
//                       scripts/sync-shells.sh)
//   --workloads LIST    default t0-loop,t0-branchy,t0-call,t0-prop,ci-v0,richards (workloads.mjs).
//                       Set names expand: v8v7-8, v8v7-7 (no regexp), octane-7, exit2 (v8v7-7 +
//                       octane-7, the M1 exit-2 run that exit2.mjs reads)
//   --harness H         the v8v7 harness: stub (runs 1 and 2) or base (the real base.js, run 3;
//                       v8v7.mjs); default DEFAULT_HARNESS of v8v7.mjs. The Octane kernels always
//                       run with Octane's base.js (octane.mjs). The JSON names it per workload
//   --rounds N          measured paired rounds, >= 10 (class M minimum; default 10)
//   --warmup N          dropped warm-up rounds (default 1)
//   --host FILE         tools/bench/hosts/<host>.toml. Without it the run is provisional
//   --allow-busy        if the load guard fails before the run, go on and label the run
//                       provisional (default: stop with "host busy", exit 3)
//   --settle-s N        seconds with no benchmark process before the first load check (default 60)
//   --wait-min N        minutes to wait for a quiet host before giving up (default 30)
//   --after-s N         seconds after the last round before the second load check (default 120)
//   --count PATH        an ep9-bench binary: also count instructions (class R) of every R workload
//                       on every engine (ep9-bench --runs 5 --warmup 1; median)
//   --count-runs N, --count-warmup N   (default 5 and 1)
//   --hash-seed S       EP9_HASH_SEED for ep9's counted runs (default 1; W4-CI-PERF-SEED). The
//                       string hash seed is random per process otherwise, and moves counts by up
//                       to 3.75%. Timed (class M) runs keep the random seed, as in run 1
//   --timeout-s N       a run that takes longer fails (default 600)
//   --work DIR          write the workload files there and keep them (default: a temporary dir)
//   --label TEXT        a name for the run, for the report
//   --out FILE          the JSON: schema ep9-cross/v1, with every sample
//   --md FILE           the Markdown tables (they also go to stdout)
//
// Order. A round runs every workload, plus the empty script, on every engine. The workload order
// rotates by one per round. For each workload, the engine order rotates by one per round too, from
// a start that differs per workload: over E rounds (E engines) each engine runs in each position
// once, so no engine always runs first, or right after a given workload.
//
// Statistics. Per round and workload, ratio = t(engine) / t(ep9): ep9's speed relative to that
// engine (> 1: ep9 is faster). Net: each time minus the same engine's empty-script time in the
// same round. The verdict uses net ratios; gross ratios are shown too. A net time <= 0 in any round
// makes the row's net ratio invalid: the row shows the gross verdict instead. Median, the
// order-statistic interval (n = 10: ranks 2 and 9), void when the IQR is > 5% of the median
// (lib/stats.mjs).
//
// Labels. The run is provisional when: there is no host file; the host is not a gate host; the
// host file has no idle baseline; or the load guard fails before (with --allow-busy) or after the
// run. A void row, or a row with an invalid net ratio, is provisional too. A provisional result
// never passes or fails a gate.
//
// Refusal. Every run's stdout must equal the workload's expected output and its exit status must
// be 0. Otherwise the runner stops with "REFUSED" and exit 1, and writes no report. Counted runs
// (class R) too: ep9-bench gets --expect-stdout, so a run that is right in the output check and
// wrong in a counted run gets no count (exit 4: REFUSED; same rule as perf-ci-v0, W4-PERF-REFUSE).
//
// Exit status: 0 done; 1 refused or another error; 2 usage error; 3 host busy.
// Tests (fake engines, no shells needed): node --test tools/bench/cross/run.test.mjs

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleepMs } from "node:timers/promises";

import { guardAfter, guardBefore, hostInfo, load1, LOAD_MARGIN, readHostFile } from "../lib/host.mjs";
import { geomean, median, verdict } from "../lib/stats.mjs";
import { DEFAULT_WORKLOADS, EMPTY, expandWorkloads, materialize, WORKLOADS } from "./workloads.mjs";
import { DEFAULT_HARNESS, HARNESSES, ROOT } from "./v8v7.mjs";

export const SCHEMA = "ep9-cross/v1";
/** Class M minimum (10-quality section 3). */
export const MIN_ROUNDS = 10;

/** Engines. `shell` names a line of third_party/shells/versions.txt. */
export const ENGINES = {
  ep9: { label: "ep9 T0", shell: null, args: [], role: "subject" },
  qjs: { label: "QuickJS", shell: "quickjs", args: [], role: "gate" },
  "node-jitless": { label: "node --jitless", shell: "node", args: ["--jitless"], role: "gate" },
  "d8-jitless": { label: "d8 --jitless", shell: "d8", args: ["--jitless"], role: "context" },
  brimstone: { label: "Brimstone", shell: "brimstone", args: [], role: "context" },
};
export const DEFAULT_ENGINES = ["ep9", "qjs", "node-jitless", "d8-jitless", "brimstone"];

/** M0 exit 4 (09-milestones): ep9 >= 0.6x QuickJS on richards and deltablue. */
export const CANARY = { engine: "qjs", threshold: 0.6, workloads: ["richards", "deltablue"] };

export class UsageError extends Error {}
export class Refusal extends Error {}
export class HostBusy extends Error {}

export function parseArgs(argv) {
  const o = {
    ep9: null,
    commit: null,
    engines: DEFAULT_ENGINES,
    shells: path.join(ROOT, "third_party/shells/versions.txt"),
    workloads: DEFAULT_WORKLOADS,
    harness: DEFAULT_HARNESS,
    rounds: MIN_ROUNDS,
    warmup: 1,
    host: null,
    allowBusy: false,
    settleS: 60,
    waitMin: 30,
    afterS: 120,
    pollS: 30,
    count: null,
    countRuns: 5,
    countWarmup: 1,
    hashSeed: "1",
    timeoutS: 600,
    work: null,
    label: null,
    out: null,
    md: null,
  };
  const num = (a, v) => {
    const n = Number(v);
    if (!Number.isFinite(n) || n < 0) throw new UsageError(`${a} needs a number >= 0, got ${v}`);
    return n;
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new UsageError(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case "--ep9": o.ep9 = path.resolve(val()); break;
      case "--commit": o.commit = val(); break;
      case "--engines": o.engines = val().split(","); break;
      case "--shells": o.shells = path.resolve(val()); break;
      case "--workloads": o.workloads = val().split(","); break; // sets expand below
      case "--harness": o.harness = val(); break;
      case "--rounds": o.rounds = num(a, val()); break;
      case "--warmup": o.warmup = num(a, val()); break;
      case "--host": o.host = path.resolve(val()); break;
      case "--allow-busy": o.allowBusy = true; break;
      case "--settle-s": o.settleS = num(a, val()); break;
      case "--wait-min": o.waitMin = num(a, val()); break;
      case "--after-s": o.afterS = num(a, val()); break;
      case "--poll-s": o.pollS = num(a, val()); break;
      case "--count": o.count = path.resolve(val()); break;
      case "--count-runs": o.countRuns = num(a, val()); break;
      case "--count-warmup": o.countWarmup = num(a, val()); break;
      case "--hash-seed": o.hashSeed = val(); break;
      case "--timeout-s": o.timeoutS = num(a, val()); break;
      case "--work": o.work = path.resolve(val()); break;
      case "--label": o.label = val(); break;
      case "--out": o.out = path.resolve(val()); break;
      case "--md": o.md = path.resolve(val()); break;
      default: throw new UsageError(`unknown option ${a}`);
    }
  }
  if (!o.ep9) throw new UsageError("--ep9 is required");
  if (!Number.isInteger(o.rounds) || o.rounds < MIN_ROUNDS) {
    throw new UsageError(`--rounds must be an integer >= ${MIN_ROUNDS} (PERF-NOISE class M)`);
  }
  if (!Number.isInteger(o.warmup)) throw new UsageError("--warmup must be an integer");
  if (!Number.isInteger(o.countRuns) || o.countRuns < 1) throw new UsageError("--count-runs must be >= 1");
  if (!o.engines.includes("ep9")) throw new UsageError("--engines must include ep9");
  if (new Set(o.engines).size !== o.engines.length) throw new UsageError("--engines has a duplicate");
  if (o.engines.length < 2) throw new UsageError("--engines needs ep9 and at least one other engine");
  for (const e of o.engines) if (!ENGINES[e]) throw new UsageError(`unknown engine ${e} (known: ${Object.keys(ENGINES).join(", ")})`);
  if (new Set(o.workloads).size !== o.workloads.length) throw new UsageError("--workloads has a duplicate");
  o.workloads = expandWorkloads(o.workloads);
  for (const w of o.workloads) if (!WORKLOADS[w]) throw new UsageError(`unknown workload ${w} (known: ${Object.keys(WORKLOADS).join(", ")})`);
  if (!HARNESSES.includes(o.harness)) throw new UsageError(`--harness must be one of ${HARNESSES.join(", ")}`);
  return o;
}

/** The platform id that sync-shells.sh writes on the `# host:` line. */
export function platformId(platform = process.platform, arch = process.arch) {
  return `${{ darwin: "darwin", linux: "linux", win32: "windows" }[platform] ?? platform}_${arch}`;
}

/**
 * third_party/shells/versions.txt: a `# host: <id>` line, then `<shell> <version text> <path>`
 * lines (the path is the last field).
 */
export function parseVersions(text) {
  const shells = {};
  let host = null;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const h = line.match(/^#\s*host:\s*(\S+)/);
    if (h) host = h[1];
    if (line.startsWith("#")) continue;
    const parts = line.split(/\s+/);
    if (parts.length < 3) throw new Error(`versions file: bad line: ${raw}`);
    const file = parts[parts.length - 1];
    shells[parts[0]] = { version: line.slice(parts[0].length, line.lastIndexOf(file)).trim(), path: file };
  }
  return { host, shells };
}

/** Engine ids -> { id, label, role, bin, args, version }. */
export function resolveEngines(ids, { ep9, shellsFile }) {
  let shells = null;
  const need = ids.filter((id) => ENGINES[id].shell);
  if (need.length) {
    let text;
    try {
      text = readFileSync(shellsFile, "utf8");
    } catch (e) {
      throw new UsageError(`${shellsFile}: ${e.message} (run bash scripts/sync-shells.sh)`);
    }
    shells = parseVersions(text);
    if (shells.host !== platformId()) {
      throw new UsageError(`${shellsFile} is for host ${shells.host}, this host is ${platformId()}`);
    }
  }
  return ids.map((id) => {
    const d = ENGINES[id];
    if (!d.shell) {
      const v = spawnSync(ep9, ["--version"], { encoding: "utf8" });
      if (v.error) throw new UsageError(`--ep9 ${ep9}: ${v.error.message}`);
      return { id, label: d.label, role: d.role, bin: ep9, args: d.args, version: (v.stdout || "").trim() };
    }
    const s = shells.shells[d.shell];
    if (!s) throw new UsageError(`${shellsFile} has no ${d.shell} line (engine ${id})`);
    if (!existsSync(s.path)) throw new UsageError(`${s.path} (engine ${id}) does not exist (run bash scripts/sync-shells.sh)`);
    return { id, label: d.label, role: d.role, bin: s.path, args: d.args, version: s.version };
  });
}

/**
 * The run order: total rounds x (workloads + empty) x engines. See "Order" at the top.
 * `workloads` already includes the empty script.
 */
export function schedule(engines, workloads, rounds) {
  const out = [];
  const W = workloads.length;
  const E = engines.length;
  for (let r = 0; r < rounds; r++) {
    for (let i = 0; i < W; i++) {
      const wi = (i + r) % W;
      for (let j = 0; j < E; j++) out.push({ round: r, workload: workloads[wi], engine: engines[(j + r + wi) % E] });
    }
  }
  return out;
}

/** Runs one engine on one file; wall time in ms. */
export function spawnRun(engine, file, timeoutS) {
  const t0 = process.hrtime.bigint();
  const r = spawnSync(engine.bin, [...engine.args, file], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 1 << 26,
    timeout: timeoutS * 1000,
  });
  const ms = Number(process.hrtime.bigint() - t0) / 1e6;
  return { ms, status: r.status, signal: r.signal, error: r.error?.message ?? null, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
}

/** Throws a Refusal unless the run exited with 0 and printed exactly `expect`. */
export function checkRun(run, { engine, workload, expect }) {
  const out = run.stdout.replace(/\r\n/g, "\n").trim();
  const tail = (s) => s.trim().split("\n").slice(-5).join("\n    ");
  if (run.error || run.status !== 0) {
    const how = run.error ?? (run.signal ? `signal ${run.signal}` : `exit ${run.status}`);
    throw new Refusal(`${workload} on ${engine}: ${how}\n    ${tail(run.stderr)}`);
  }
  if (out !== expect) {
    throw new Refusal(`${workload} on ${engine}: printed ${JSON.stringify(out)}, want ${JSON.stringify(expect)}`);
  }
}

const sideText = { above: "ep9 faster", below: "ep9 slower", marginal: "marginal", "insufficient data": "insufficient data" };

/**
 * Class M statistics from the samples. `workloads` are the M workloads (no empty script).
 * Returns { rounds, times, ratios, geomean }.
 */
export function analyze(samples, { subject, comparators, workloads, warmup }) {
  const t = {};
  for (const s of samples) {
    if (s.round < warmup) continue;
    ((t[s.workload] ??= {})[s.engine] ??= [])[s.round - warmup] = s.ms;
  }
  const engines = [subject, ...comparators];
  const rounds = t[EMPTY]?.[subject]?.length ?? 0;
  for (const w of [...workloads, EMPTY]) {
    for (const e of engines) {
      // Array.from turns holes (a missing round) into undefined; some() would skip them.
      const xs = Array.from(t[w]?.[e] ?? []);
      const have = xs.filter((x) => typeof x === "number").length;
      if (xs.length !== rounds || have !== rounds) {
        throw new Error(`analyze: ${w} on ${e} has ${have} of ${rounds} samples`);
      }
    }
  }
  const net = (w, e) => t[w][e].map((x, r) => x - t[EMPTY][e][r]);
  const times = [];
  for (const w of [...workloads, EMPTY]) {
    for (const e of engines) {
      times.push({
        workload: w,
        engine: e,
        gross_ms: median(t[w][e]),
        net_ms: w === EMPTY ? null : median(net(w, e)),
      });
    }
  }
  const ratios = [];
  for (const w of workloads) {
    for (const c of comparators) {
      const gross = t[w][c].map((x, r) => x / t[w][subject][r]);
      const nc = net(w, c);
      const ns = net(w, subject);
      const bad = nc.flatMap((x, r) => (x > 0 && ns[r] > 0 ? [] : [r + warmup]));
      const netRatios = bad.length ? null : nc.map((x, r) => x / ns[r]);
      const row = {
        workload: w,
        comparator: c,
        n: rounds,
        net: netRatios && { ratios: netRatios, ...verdict(netRatios, 1) },
        net_invalid: bad.length ? `net time <= 0 in round(s) ${bad.join(", ")}` : null,
        gross: { ratios: gross, ...verdict(gross, 1) },
      };
      const main = row.net ?? row.gross;
      row.metric = row.net ? "net" : "gross";
      row.verdict = sideText[main.side];
      row.void = main.void;
      if (c === CANARY.engine && CANARY.workloads.includes(w)) {
        const v = verdict((row.net ?? row.gross).ratios, CANARY.threshold);
        row.canary = { threshold: CANARY.threshold, side: v.side };
      }
      ratios.push(row);
    }
  }
  const gm = {};
  for (const c of comparators) {
    const rows = ratios.filter((r) => r.comparator === c);
    gm[c] = {
      workloads: rows.map((r) => r.workload),
      net: rows.every((r) => r.net) ? geomean(rows.map((r) => r.net.median)) : null,
      gross: geomean(rows.map((r) => r.gross.median)),
    };
  }
  return { rounds, times, ratios, geomean: gm };
}

/** Adds `status` labels to every ratio row, and the canary's gate reading. */
export function label(m, provisional) {
  for (const row of m.ratios) {
    const s = [];
    if (provisional) s.push("provisional");
    if (row.void) s.push(`void (IQR ${(row.net ?? row.gross).iqrPct.toFixed(1)}% > 5%)`);
    // Gates read net ratios only: a gross fallback is shown, but it passes or fails nothing.
    if (row.net_invalid) s.push(`net invalid (${row.net_invalid})`);
    if ((row.void || row.net_invalid) && !provisional) s.push("provisional");
    row.status = s.length ? s : ["valid"];
    if (row.canary) {
      const side = row.canary.side;
      row.canary.gate =
        provisional || row.void || row.net_invalid
          ? `no gate verdict (provisional; the ${row.net ? "" : "gross "}interval is ${side === "marginal" ? "around" : side} ${CANARY.threshold})`
          : side === "above" ? "pass" : side === "below" ? "fail" : "marginal";
    }
  }
  return m;
}

/** Parses ep9-bench/v1 JSON (tools/bench/src/report.rs). */
export function parseBenchJson(text) {
  const j = JSON.parse(text);
  if (j.schema !== "ep9-bench/v1") throw new Error(`not ep9-bench/v1 JSON: ${j.schema}`);
  const pick = (xs) => {
    const ok = xs.filter((x) => typeof x === "number");
    return ok.length === xs.length && ok.length ? median(ok) : null;
  };
  return {
    counter: j.counter,
    counter_status: j.counter_status,
    counter_note: j.counter_note,
    median: j.summary?.instructions?.median ?? null,
    min: j.summary?.instructions?.min ?? null,
    max: j.summary?.instructions?.max ?? null,
    instructions: j.runs.map((r) => r.instructions),
    wall_ms: j.runs.map((r) => r.wall_ns / 1e6),
    max_rss_bytes: pick(j.runs.map((r) => r.max_rss_bytes)),
  };
}

/** ep9-bench exit status when a run printed something other than --expect-stdout (no JSON). */
export const BENCH_WRONG_OUTPUT = 4;

/**
 * Class R: ep9-bench for every R workload on every engine. Every counted run, warm-up included,
 * must print the workload's expected output (ep9-bench --expect-stdout): the output check before
 * the rounds cannot vouch for later runs. ep9 runs with EP9_HASH_SEED = o.hashSeed.
 */
export function countAll(o, engines, files, commit) {
  const rows = [];
  const tmp = mkdtempSync(path.join(os.tmpdir(), "ep9-cross-count-"));
  try {
    for (const w of o.workloads.filter((x) => files[x].classes.includes("R"))) {
      const expect = path.join(tmp, `${w}.expect.txt`);
      writeFileSync(expect, files[w].expect);
      for (const e of engines) {
        const out = path.join(tmp, `${w}-${e.id}.json`);
        const args = ["--runs", String(o.countRuns), "--warmup", String(o.countWarmup), "--name", `${w}@${e.id}`, "--out", out, "--expect-stdout", expect, "--", e.bin, ...e.args, files[w].file];
        const env = { ...process.env, EP9_COMMIT: e.id === "ep9" ? commit : "" };
        if (e.id === "ep9") env.EP9_HASH_SEED = o.hashSeed;
        else delete env.EP9_HASH_SEED;
        const r = spawnSync(o.count, args, {
          encoding: "utf8",
          env,
          timeout: o.timeoutS * 1000 * (o.countRuns + o.countWarmup),
        });
        if (r.error) throw new Error(`${o.count}: ${r.error.message}`);
        const tail = () => (r.stderr || "").trim().split("\n").slice(-5).join("\n    ");
        if (r.status === BENCH_WRONG_OUTPUT) {
          throw new Refusal(`count ${w} on ${e.id}: a counted run printed the wrong output (ep9-bench exit 4)\n    ${tail()}`);
        }
        // 3: no counter on this host; the JSON is still written, with null counts.
        if (r.status !== 0 && r.status !== 3) {
          throw new Refusal(`count ${w} on ${e.id}: ep9-bench exit ${r.status}\n    ${tail()}`);
        }
        const p = parseBenchJson(readFileSync(out, "utf8"));
        rows.push({ workload: w, engine: e.id, ...p });
        process.stderr.write(`count ${w} ${e.id}: ${p.median}\n`);
      }
    }
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
  return {
    tool: "ep9-bench",
    runs: o.countRuns,
    warmup: o.countWarmup,
    counter: rows[0]?.counter ?? null,
    expect_stdout: true,
    ep9_hash_seed: o.hashSeed,
    rows,
  };
}

function git(args) {
  try {
    return execFileSync("git", ["-C", ROOT, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return null;
  }
}

/**
 * The whole run. `deps` = { sleep(s), load(), log(text) } (injectable for tests).
 * Returns the ep9-cross/v1 result object.
 */
export async function run(o, deps = {}) {
  const sleep = deps.sleep ?? ((s) => sleepMs(s * 1000));
  const load = deps.load ?? load1;
  const log = deps.log ?? ((s) => process.stderr.write(`${s}\n`));
  const started = new Date().toISOString();
  const measured = hostInfo();
  const reasons = [];

  let hostFile = null;
  if (o.host) {
    hostFile = readHostFile(o.host);
    if (hostFile.cpu !== measured.cpu || hostFile.platform !== measured.platform) {
      throw new UsageError(`${o.host} is for "${hostFile.cpu}" (${hostFile.platform}); this host is "${measured.cpu}" (${measured.platform})`);
    }
    if (!hostFile.gate_host) reasons.push(`host ${hostFile.id} is not a gate host`);
    if (typeof hostFile.idle_baseline_load1 !== "number") reasons.push(`host file ${path.basename(o.host)} has no idle baseline`);
  } else {
    reasons.push("no host file (not a gate host)");
  }

  const engines = resolveEngines(o.engines, { ep9: o.ep9, shellsFile: o.shells });
  const commit = o.commit ?? git(["rev-parse", "HEAD"]);
  const runnerCommit = git(["rev-parse", "HEAD"]);
  const runnerDirty = git(["status", "--porcelain", "--untracked-files=no"]);

  const work = o.work ?? mkdtempSync(path.join(os.tmpdir(), "ep9-cross-"));
  mkdirSync(work, { recursive: true });
  try {
    const files = materialize(o.workloads, work, { harness: o.harness });
    const mWorkloads = o.workloads.filter((w) => files[w].classes.includes("M"));
    const exec = deps.exec ?? ((e, f) => spawnRun(e, f, o.timeoutS));
    const byId = Object.fromEntries(engines.map((e) => [e.id, e]));
    const once = (e, w) => {
      const r = exec(byId[e], files[w].file);
      checkRun(r, { engine: e, workload: w, expect: files[w].expect });
      return r.ms;
    };

    // Output check: every workload once on every engine, before any timing.
    for (const w of [EMPTY, ...o.workloads]) for (const e of o.engines) once(e, w);
    log(`output check: ${o.workloads.length + 1} files x ${o.engines.length} engines ok`);

    // Load guard, before.
    let before = null;
    if (hostFile) {
      log(`load guard: ${o.settleS} s with no benchmark process, then load <= idle + ${LOAD_MARGIN}`);
      before = await guardBefore(
        { idleBaseline: hostFile.idle_baseline_load1, settleS: o.settleS, waitS: o.waitMin * 60, pollS: o.pollS },
        { load, sleep },
      );
      log(`load guard before: load ${before.load1}, limit ${before.limit}, ${before.pass ? "pass" : `FAIL (${before.why})`}`);
      if (!before.pass) {
        if (!o.allowBusy) {
          throw new HostBusy(
            before.limit === null
              ? `cannot run the load guard: ${before.why}`
              : `host busy: ${before.why} after waiting ${before.waited_s} s`,
          );
        }
        if (before.limit !== null) reasons.push(`load guard failed before the run (${before.why})`);
      }
    }

    // Class M rounds.
    const total = o.warmup + o.rounds;
    const plan = schedule(o.engines, [...mWorkloads, EMPTY], total);
    const samples = [];
    const roundLoad = [];
    plan.forEach((p, seq) => {
      const ms = once(p.engine, p.workload);
      samples.push({ seq, round: p.round, warmup: p.round < o.warmup, workload: p.workload, engine: p.engine, ms: Number(ms.toFixed(3)) });
      if (seq + 1 === plan.length || plan[seq + 1].round !== p.round) {
        roundLoad.push(load());
        log(`round ${p.round + 1}/${total}${p.round < o.warmup ? " (warm-up)" : ""}: load ${roundLoad.at(-1)}`);
      }
    });

    // Load guard, after.
    let after = null;
    if (hostFile) {
      after = await guardAfter({ idleBaseline: hostFile.idle_baseline_load1, afterS: o.afterS }, { load, sleep });
      log(`load guard after: load ${after.load1}, limit ${after.limit}, ${after.pass ? "pass" : `FAIL (${after.why})`}`);
      if (!after.pass && after.limit !== null) reasons.push(`load guard failed after the run (${after.why})`);
    }

    const provisional = reasons.length > 0;
    const m = label(
      analyze(samples, { subject: "ep9", comparators: o.engines.filter((e) => e !== "ep9"), workloads: mWorkloads, warmup: o.warmup }),
      provisional,
    );

    // Class R counts (after the guard: counting is load too).
    const r = o.count ? countAll(o, engines, files, commit) : null;

    return {
      schema: SCHEMA,
      label: o.label,
      started,
      finished: new Date().toISOString(),
      commit: { ep9: commit, runner: runnerCommit, runner_dirty: runnerDirty === null ? null : runnerDirty !== "" },
      host: {
        file: o.host && path.relative(ROOT, o.host),
        id: hostFile?.id ?? null,
        gate_host: hostFile?.gate_host ?? false,
        idle_baseline_load1: hostFile?.idle_baseline_load1 ?? null,
        measured,
      },
      load: { margin: LOAD_MARGIN, before, after, after_each_round: roundLoad },
      provisional,
      provisional_reasons: reasons,
      options: { rounds: o.rounds, warmup: o.warmup, settle_s: o.settleS, wait_min: o.waitMin, after_s: o.afterS, allow_busy: o.allowBusy },
      // The binary by file name only: the version (ep9: the commit) names the build.
      engines: engines.map((e) => ({ id: e.id, label: e.label, role: e.role, version: e.version, bin: path.basename(e.bin), args: e.args })),
      workloads: Object.values(files).map(({ file, ...w }) => ({ ...w, file: path.basename(file) })),
      M: { ratio: "t(engine) / t(ep9) per round; > 1: ep9 is faster", samples, ...m },
      R: r,
    };
  } finally {
    if (!o.work) rmSync(work, { recursive: true, force: true });
  }
}

// ---- Markdown -------------------------------------------------------------------------------

const f3 = (x) => (x == null ? "-" : x.toFixed(3));
const f1 = (x) => (x == null ? "-" : x.toFixed(1));
const gi = (x) => (x == null ? "-" : (x / 1e9).toFixed(3));

export function renderMarkdown(res) {
  const L = [];
  const eng = Object.fromEntries(res.engines.map((e) => [e.id, e]));
  const h = res.host;
  const ld = res.load;
  const sha = (res.commit.ep9 ?? "unknown").slice(0, 12);
  const roundLoads = ld.after_each_round;
  L.push(`Run ${res.label ? `"${res.label}" ` : ""}${res.started} to ${res.finished}; ep9 commit \`${sha}\`.`);
  L.push("");
  L.push(`- Host: ${h.id ?? "no host file"}: ${h.measured.cpu}, ${h.measured.cpus} logical CPUs, ${h.measured.os} (${h.measured.platform}); runner on node ${h.measured.node}.`);
  const g = (c) =>
    !c ? "not checked" : c.limit === null ? `${c.load1} (no idle baseline: the guard cannot pass)` : `${c.load1} (limit ${c.limit}: ${c.pass ? "pass" : "FAIL"})`;
  L.push(`- 1-minute load: before ${g(ld.before)}; after ${g(ld.after)}; after each round ${Math.min(...roundLoads)}-${Math.max(...roundLoads)}. Idle baseline ${h.idle_baseline_load1 ?? "not measured"}.`);
  L.push(`- Engines: ${res.engines.map((e) => `${e.label} (${e.role}; ${e.version})`).join("; ")}.`);
  L.push(`- ${res.M.rounds} paired rounds (+${res.options.warmup} warm-up dropped), rotated order. Ratio = t(engine) / t(ep9) per round: ep9's speed relative to that engine (> 1: ep9 faster). Net = minus the same engine's empty-script time in the same round. Interval: order statistics, ranks ${res.M.ratios[0]?.net?.ranks?.join(" and ") ?? res.M.ratios[0]?.gross.ranks?.join(" and ") ?? "-"}.`);
  L.push(res.provisional ? `- **PROVISIONAL**: ${res.provisional_reasons.join("; ")}. No row passes or fails a gate.` : "- Gate run: the load guard passed before and after.");
  L.push("");
  L.push("| workload | vs | net ratio | interval | IQR % | gross ratio | gross interval | verdict | status |");
  L.push("|---|---|---|---|---|---|---|---|---|");
  const iv = (v) => (v?.interval ? `${f3(v.interval[0])}-${f3(v.interval[1])}` : "none");
  for (const r of res.M.ratios) {
    const n = r.net;
    L.push(`| ${r.workload} | ${eng[r.comparator].label} | ${n ? f3(n.median) : "invalid"} | ${iv(n)} | ${f1((n ?? r.gross).iqrPct)} | ${f3(r.gross.median)} | ${iv(r.gross)} | ${r.verdict}${n ? "" : " (gross)"} | ${r.status.join(", ")} |`);
  }
  L.push("");
  L.push(`Geomean of the median ratios over ${res.M.ratios.length ? [...new Set(res.M.ratios.map((r) => r.workload))].join(", ") : "-"}: ${Object.entries(res.M.geomean).map(([c, v]) => `${eng[c].label} net ${f3(v.net)} / gross ${f3(v.gross)}`).join("; ")}.`);
  if (res.engines.some((e) => e.id === CANARY.engine)) {
    L.push("");
    L.push(`M0 exit 4 canary (ep9 >= ${CANARY.threshold}x QuickJS on ${CANARY.workloads.join(" and ")}):`);
    L.push("");
    L.push("| workload | net ratio vs QuickJS | interval | gate reading |");
    L.push("|---|---|---|---|");
    for (const w of CANARY.workloads) {
      const r = res.M.ratios.find((x) => x.canary && x.workload === w);
      // Net only, as the heading says; the gross ratio of an invalid row is in the table above.
      L.push(r ? `| ${w} | ${r.net ? f3(r.net.median) : "invalid"} | ${iv(r.net)} | ${r.canary.gate} |` : `| ${w} | not in this run | - | no reading |`);
    }
  }
  L.push("");
  L.push("Median wall time per process, ms (net = minus the empty script):");
  L.push("");
  const ws = [...new Set(res.M.times.map((t) => t.workload))];
  L.push(`| workload | ${res.engines.map((e) => `${e.label} gross | net`).join(" | ")} |`);
  L.push(`|---|${res.engines.map(() => "---|---").join("|")}|`);
  for (const w of ws) {
    const cells = res.engines.map((e) => {
      const t = res.M.times.find((x) => x.workload === w && x.engine === e.id);
      return `${f1(t.gross_ms)} | ${w === EMPTY ? "-" : f1(t.net_ms)}`;
    });
    L.push(`| ${w} | ${cells.join(" | ")} |`);
  }
  if (res.R) {
    L.push("");
    const how = [`${res.R.tool}, counter ${res.R.counter ?? "none"}`];
    if (res.R.expect_stdout) how.push("every run's output checked");
    if (res.R.ep9_hash_seed != null) how.push(`ep9 with EP9_HASH_SEED=${res.R.ep9_hash_seed}`);
    L.push(`Class R: instructions retired, median of ${res.R.runs} runs after ${res.R.warmup} warm-up (${how.join("; ")}), in billions (G). Compare within one counter only.`);
    L.push("");
    const rw = [...new Set(res.R.rows.map((x) => x.workload))];
    const re = [...new Set(res.R.rows.map((x) => x.engine))];
    L.push(`| workload | ${re.map((e) => eng[e].label).join(" | ")} |`);
    L.push(`|---|${re.map(() => "---").join("|")}|`);
    for (const w of rw) L.push(`| ${w} | ${re.map((e) => gi(res.R.rows.find((x) => x.workload === w && x.engine === e)?.median)).join(" | ")} |`);
  }
  return `${L.join("\n")}\n`;
}

// ---- main -----------------------------------------------------------------------------------

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) {
    // The usage is the comment at the top of this file.
    const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n");
    process.stdout.write(`${src.slice(1, src.findIndex((l, i) => i > 0 && !l.startsWith("//"))).map((l) => l.replace(/^\/\/ ?/, "")).join("\n")}\n`);
    return;
  }
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`run.mjs: ${e.message}`);
    process.exit(2);
  }
  let res;
  try {
    res = await run(o);
  } catch (e) {
    if (e instanceof UsageError) {
      console.error(`run.mjs: ${e.message}`);
      process.exit(2);
    }
    if (e instanceof HostBusy) {
      console.error(`run.mjs: ${e.message}. Wait for a quiet window, or pass --allow-busy for a provisional run.`);
      process.exit(3);
    }
    if (e instanceof Refusal) {
      console.error(`REFUSED: ${e.message}\nrun.mjs: a run gave a wrong result; no report was written.`);
      process.exit(1);
    }
    throw e;
  }
  const md = renderMarkdown(res);
  if (o.out) writeFileSync(o.out, `${JSON.stringify(res, null, 1)}\n`);
  if (o.md) writeFileSync(o.md, md);
  process.stdout.write(md);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main().catch((e) => {
    console.error(e.stack ?? String(e));
    process.exit(1);
  });
}
