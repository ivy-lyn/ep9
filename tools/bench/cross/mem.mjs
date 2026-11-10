#!/usr/bin/env node
// G7 memory reader (docs/plan/02-architecture.md G7; class Mem of docs/plan/10-quality.md section
// 3; M2 exit 4 of docs/plan/09-milestones.md): peak RSS of each engine on each workload, >= 5 runs,
// the median, and both G7 caps: ep9 <= 1.2x node --jitless and <= 1.8x a pointer-compressed d8
// --jitless (jsvu, third_party/shells.toml [d8] pointer_compression = "true").
//
//   node tools/bench/cross/mem.mjs --ep9 target/release/ep9 --ep9-jitless <jitless ep9> \
//     --host tools/bench/hosts/m5max-macos.toml --out mem.json --md mem.md
//
//   --ep9 PATH          the ep9 binary of the jit preset build (required). Its --version build line
//                       must say "preset jit"
//   --ep9-jitless PATH  the jitless ep9 build (`cargo build --release -p ep9_cli
//                       --no-default-features`); its build line must say "preset jitless". The
//                       two-build rule (plan v12 V12-D6) needs both: a cap holds only when it holds
//                       for each ep9 build in the run
//   --engines LIST      default ep9,ep9-jitless,node-jitless,d8-jitless (ep9-jitless only with
//                       --ep9-jitless). Also known: qjs (a report column, no cap)
//   --shells FILE       reference shells (default third_party/shells/versions.txt)
//   --workloads LIST    default mem (workloads.mjs: the 7 v8-v7 kernels, no regexp)
//   --harness H         the v8v7 harness (default DEFAULT_HARNESS of v8v7.mjs)
//   --runs N            measured runs per engine and workload, >= 5 (class Mem; default 5)
//   --host FILE         tools/bench/hosts/<host>.toml. Without it the run is provisional
//   --allow-busy        if the load guard fails before the run, go on and label the run provisional
//   --settle-s N, --wait-min N, --after-s N   the load guard (defaults 60, 30, 120)
//   --timeout-s N       a run that takes longer fails (default 600)
//   --label TEXT        a name for the run
//   --out FILE          the JSON (schema ep9-mem/v1, every sample); --md FILE the Markdown
//
// Commit. commit.ep9 is the commit that every ep9 binary of the run names in its `ep9 --version`
// build line (stderr, P0-BUILDID), and only when each was built clean; else null, and m2-exit
// reads the run as OPEN. commit.runner is the checkout the reader ran in.
//
// Peak RSS. The OS reports the high-water resident set of the engine process when it exits:
// macOS `/usr/bin/time -l` "maximum resident set size" (bytes); Linux GNU `/usr/bin/time -f %M`
// (KiB; the `time` package on Debian and Ubuntu). EP9_MEM_TIME names another program of the same
// form (tests use a fake). No other wrapper: valgrind or perf would add their own memory.
//
// Order. A round runs every workload on every engine once. The workload order rotates by one per
// round, and per workload the engine order rotates too (run.mjs `schedule`), so no engine always
// runs first. Every run must print the workload's expected output and exit 0, or the reader
// stops with "REFUSED" (exit 1) and writes no report.
//
// Statistics. Per engine and workload: the median of the runs (and min, max). Ratio per workload =
// median(ep9) / median(comparator); also the median, min and max of the per-round ratios. A cap
// holds when the ratio of every workload is at most the cap (the strict reading of "peak RSS on
// v8-v7"); the JSON gives the largest ratio and the geomean too. A provisional run (no host file,
// not a gate host, a failed load guard) is labelled so; class Mem may run on any host that passes
// the load guard (10-quality section 3), so a gate host is not needed for a valid reading.
//
// Exit status: 0 done; 1 refused or another error; 2 usage error; 3 host busy.
// Tests: node --test tools/bench/cross/mem.test.mjs

import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleepMs } from "node:timers/promises";

import { guardAfter, guardBefore, hostInfo, load1, LOAD_MARGIN, readHostFile } from "../lib/host.mjs";
import { geomean, median } from "../lib/stats.mjs";
import { splitVersion } from "./kernels.mjs";
import { buildNote, cleanCommit, combineBuilds, ENGINES as CROSS_ENGINES, HostBusy, parseBuild, Refusal, resolveEngines, schedule, UsageError } from "./run.mjs";
import { DEFAULT_HARNESS, HARNESSES, ROOT } from "./v8v7.mjs";
import { expandWorkloads, materialize, V8V7_7, WORKLOADS } from "./workloads.mjs";

export const SCHEMA = "ep9-mem/v1";
/** Class Mem minimum (10-quality section 3). */
export const MIN_RUNS = 5;

/** The G7 caps (02-architecture G7): the comparator engine and the largest ep9 / comparator ratio. */
export const CAPS = [
  { comparator: "node-jitless", cap: 1.2, text: "<= 1.2x node 24 --jitless" },
  { comparator: "d8-jitless", cap: 1.8, text: "<= 1.8x a pointer-compressed d8 --jitless" },
];

/** Engines of this reader. An ep9 engine names its option and the preset its build line must say. */
export const ENGINES = {
  ep9: { label: "ep9 (jit preset)", ep9: "ep9", preset: "jit", role: "subject" },
  "ep9-jitless": { label: "ep9 (jitless build)", ep9: "ep9Jitless", preset: "jitless", role: "subject" },
  "node-jitless": { ...CROSS_ENGINES["node-jitless"], role: "cap" },
  "d8-jitless": { ...CROSS_ENGINES["d8-jitless"], label: "d8 --jitless (pointer-compressed)", role: "cap" },
  qjs: { ...CROSS_ENGINES.qjs, role: "context" },
};

export function parseArgs(argv) {
  const o = {
    ep9: null,
    ep9Jitless: null,
    engines: null,
    shells: path.join(ROOT, "third_party/shells/versions.txt"),
    workloads: ["mem"],
    harness: DEFAULT_HARNESS,
    runs: MIN_RUNS,
    host: null,
    allowBusy: false,
    settleS: 60,
    waitMin: 30,
    afterS: 120,
    pollS: 30,
    timeoutS: 600,
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
      case "--ep9-jitless": o.ep9Jitless = path.resolve(val()); break;
      case "--engines": o.engines = val().split(","); break;
      case "--shells": o.shells = path.resolve(val()); break;
      case "--workloads": o.workloads = val().split(","); break;
      case "--harness": o.harness = val(); break;
      case "--runs": o.runs = num(a, val()); break;
      case "--host": o.host = path.resolve(val()); break;
      case "--allow-busy": o.allowBusy = true; break;
      case "--settle-s": o.settleS = num(a, val()); break;
      case "--wait-min": o.waitMin = num(a, val()); break;
      case "--after-s": o.afterS = num(a, val()); break;
      case "--poll-s": o.pollS = num(a, val()); break;
      case "--timeout-s": o.timeoutS = num(a, val()); break;
      case "--label": o.label = val(); break;
      case "--out": o.out = path.resolve(val()); break;
      case "--md": o.md = path.resolve(val()); break;
      default: throw new UsageError(`unknown option ${a}`);
    }
  }
  if (!o.ep9) throw new UsageError("--ep9 is required");
  o.engines ??= ["ep9", ...(o.ep9Jitless ? ["ep9-jitless"] : []), "node-jitless", "d8-jitless"];
  for (const e of o.engines) if (!ENGINES[e]) throw new UsageError(`unknown engine ${e} (known: ${Object.keys(ENGINES).join(", ")})`);
  if (new Set(o.engines).size !== o.engines.length) throw new UsageError("--engines has a duplicate");
  if (!o.engines.includes("ep9")) throw new UsageError("--engines must include ep9");
  if (o.engines.includes("ep9-jitless") && !o.ep9Jitless) throw new UsageError("engine ep9-jitless needs --ep9-jitless");
  if (!Number.isInteger(o.runs) || o.runs < MIN_RUNS) throw new UsageError(`--runs must be an integer >= ${MIN_RUNS} (class Mem)`);
  if (new Set(o.workloads).size !== o.workloads.length) throw new UsageError("--workloads has a duplicate");
  o.workloads = expandWorkloads(o.workloads);
  for (const w of o.workloads) if (!WORKLOADS[w]) throw new UsageError(`unknown workload ${w}`);
  if (!HARNESSES.includes(o.harness)) throw new UsageError(`--harness must be one of ${HARNESSES.join(", ")}`);
  return o;
}

/** The peak RSS tool of this platform: { program, args(reportFile), parse(text) -> bytes, name }. */
export function rssTool(platform = process.platform, env = process.env) {
  const program = env.EP9_MEM_TIME || "/usr/bin/time";
  if (platform === "darwin") {
    return { name: "macos-time-l-maximum-resident-set-size", unit: "bytes", program, args: (f) => ["-l", "-o", f], parse: parseTimeL };
  }
  if (platform === "linux") {
    return { name: "gnu-time-f-M", unit: "KiB", program, args: (f) => ["-f", "%M", "-o", f], parse: parseGnuM };
  }
  return null;
}

/** macOS `time -l`: the "maximum resident set size" line, in bytes. */
export function parseTimeL(text) {
  const m = text.match(/^\s*(\d+)\s+maximum resident set size\s*$/m);
  return m ? Number(m[1]) : null;
}

/** GNU `time -f %M`: the last line that is one number, in KiB. */
export function parseGnuM(text) {
  const lines = text.trim().split("\n").map((l) => l.trim()).filter((l) => /^\d+$/.test(l));
  return lines.length ? Number(lines[lines.length - 1]) * 1024 : null;
}

// The process groups of runGroup that are still live. A group is not in the terminal's
// foreground group, so Ctrl-C (or a SIGTERM or SIGHUP to the runner) does not reach it: while a
// group is live, a handler kills every live group, then raises the signal again (M7-P0 Codex r2
// C2-4). The handlers go when no group is live, so importers keep the default behaviour.
const liveGroups = new Set();
const GROUP_SIGNALS = ["SIGINT", "SIGTERM", "SIGHUP"];
function killLiveGroups() {
  for (const pid of liveGroups) {
    try {
      process.kill(-pid, "SIGKILL");
    } catch {}
  }
  liveGroups.clear();
}
const onGroupSignal = Object.fromEntries(
  GROUP_SIGNALS.map((sig) => [
    sig,
    () => {
      killLiveGroups();
      unhookGroups();
      process.kill(process.pid, sig);
    },
  ]),
);
function hookGroups() {
  for (const sig of GROUP_SIGNALS) process.on(sig, onGroupSignal[sig]);
  process.on("exit", killLiveGroups);
}
function unhookGroups() {
  for (const sig of GROUP_SIGNALS) process.off(sig, onGroupSignal[sig]);
  process.off("exit", killLiveGroups);
}

/**
 * Runs `program args` with its own process group (POSIX): after `timeoutS` the whole group gets
 * SIGKILL, so the engine under the RSS tool stops too, not only the tool (M7-P0 Codex r1
 * BENCH10-2); a SIGINT, SIGTERM or SIGHUP of the runner kills the group too (M7-P0 Codex r2
 * C2-4). Resolves to { status, signal, error, stdout, stderr } when the process closes.
 */
export function runGroup(program, args, timeoutS) {
  return new Promise((resolve) => {
    const group = process.platform !== "win32";
    // Hook before spawn: the child exists before spawn returns, and a signal handler (a JS
    // callback) runs only after this synchronous part, so it sees the pid (M7-P0 Codex r3 C3-2).
    if (group && !liveGroups.size) hookGroups();
    let child;
    try {
      child = spawn(program, args, { stdio: ["ignore", "pipe", "pipe"], detached: group });
    } catch (e) {
      if (group && !liveGroups.size) unhookGroups();
      throw e;
    }
    if (group) {
      if (child.pid !== undefined) liveGroups.add(child.pid);
      else if (!liveGroups.size) unhookGroups();
    }
    let stdout = "";
    let stderr = "";
    let error = null;
    let timedOut = false;
    let done = false;
    const finish = (status, signal) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (liveGroups.delete(child.pid) && !liveGroups.size) unhookGroups();
      resolve({ status, signal, error: timedOut ? `timed out after ${timeoutS} s (the process group was killed)` : error, stdout, stderr });
    };
    const timer = setTimeout(() => {
      timedOut = true;
      try {
        if (group) process.kill(-child.pid, "SIGKILL");
        else child.kill("SIGKILL");
      } catch {}
    }, timeoutS * 1000);
    child.stdout.setEncoding("utf8").on("data", (d) => (stdout += d));
    child.stderr.setEncoding("utf8").on("data", (d) => (stderr += d));
    child.on("error", (e) => {
      error = e.message;
      if (child.pid === undefined) finish(null, null);
    });
    child.on("close", (status, signal) => finish(status, signal));
  });
}

/** Runs one engine on one file under the RSS tool: { rss, ms, status, signal, error, stdout, stderr }. */
export async function measureRss(tool, engine, file, timeoutS) {
  const tmp = mkdtempSync(path.join(os.tmpdir(), "ep9-mem-"));
  try {
    const report = path.join(tmp, "time.txt");
    const t0 = process.hrtime.bigint();
    const r = await runGroup(tool.program, [...tool.args(report), engine.bin, ...engine.args, file], timeoutS);
    const ms = Number(process.hrtime.bigint() - t0) / 1e6;
    let text = "";
    try {
      text = readFileSync(report, "utf8");
    } catch {}
    return { rss: tool.parse(text), ms, status: r.status, signal: r.signal, error: r.error, stdout: r.stdout, stderr: r.stderr };
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

/** Ep9 engines: --version, and the build line must name the preset of the engine. */
function resolveEp9(id, bin) {
  const v = spawnSync(bin, ["--version"], { encoding: "utf8" });
  if (v.error) throw new UsageError(`${id} ${bin}: ${v.error.message}`);
  const sv = splitVersion(v.stdout || "");
  const want = ENGINES[id].preset;
  if (!new RegExp(`preset ${want}\\b`).test(sv.engine_build ?? "")) {
    throw new UsageError(`${id} ${bin}: the build line is "${sv.engine_build}", want "preset ${want}"`);
  }
  // The freshness line of P0-BUILDID (`build`, null for an older binary).
  return { id, label: ENGINES[id].label, role: "subject", bin, args: [], ...sv, build: parseBuild(v.stderr) };
}

export function resolveAll(o) {
  const shellIds = o.engines.filter((e) => !ENGINES[e].ep9);
  const shells = shellIds.length ? resolveEngines(shellIds, { ep9: o.ep9, shellsFile: o.shells }) : [];
  return o.engines.map((id) => {
    if (ENGINES[id].ep9) return resolveEp9(id, o[ENGINES[id].ep9]);
    const s = shells.find((x) => x.id === id);
    return { ...s, label: ENGINES[id].label, role: ENGINES[id].role };
  });
}

/** Per engine and workload medians, per workload ratios, and the cap readings. */
export function analyze(samples, { engines, workloads }) {
  const by = {};
  for (const s of samples) ((by[s.workload] ??= {})[s.engine] ??= []).push(s);
  const table = [];
  for (const w of workloads) {
    for (const e of engines) {
      const xs = (by[w]?.[e] ?? []).map((s) => s.rss_bytes);
      if (!xs.length || xs.some((x) => typeof x !== "number")) throw new Error(`analyze: ${w} on ${e} has no full set of RSS samples`);
      table.push({ workload: w, engine: e, n: xs.length, median_bytes: median(xs), min_bytes: Math.min(...xs), max_bytes: Math.max(...xs) });
    }
  }
  const med = (w, e) => table.find((t) => t.workload === w && t.engine === e).median_bytes;
  const subjects = engines.filter((e) => ENGINES[e].role === "subject");
  const comparators = engines.filter((e) => ENGINES[e].role !== "subject");
  const ratios = [];
  for (const s of subjects) {
    for (const c of comparators) {
      for (const w of workloads) {
        const rounds = by[w][s].map((x) => {
          const y = by[w][c].find((z) => z.round === x.round);
          return y ? x.rss_bytes / y.rss_bytes : null;
        }).filter((x) => x !== null);
        ratios.push({
          subject: s,
          comparator: c,
          workload: w,
          ratio: med(w, s) / med(w, c),
          round_ratios: { n: rounds.length, median: rounds.length ? median(rounds) : null, min: Math.min(...rounds), max: Math.max(...rounds) },
        });
      }
    }
  }
  const caps = [];
  for (const cap of CAPS.filter((c) => engines.includes(c.comparator))) {
    for (const s of subjects) {
      const rows = ratios.filter((r) => r.subject === s && r.comparator === cap.comparator);
      const worst = rows.reduce((a, r) => (r.ratio > a.ratio ? r : a));
      caps.push({
        subject: s,
        comparator: cap.comparator,
        cap: cap.cap,
        text: cap.text,
        rule: "every workload",
        ratios: Object.fromEntries(rows.map((r) => [r.workload, r.ratio])),
        max_ratio: worst.ratio,
        max_workload: worst.workload,
        geomean_ratio: geomean(rows.map((r) => r.ratio)),
        holds: rows.every((r) => r.ratio <= cap.cap),
        over: rows.filter((r) => r.ratio > cap.cap).map((r) => r.workload),
      });
    }
  }
  // G7 has a reading only for the whole matrix (plan v12 section 3.4; M7-P0 Codex r1 BENCH10-1):
  // both caps, both ep9 builds (V12-D6) and the 7 v8-v7 kernels; then it holds when every cap
  // holds for both builds.
  const missing = [
    ...CAPS.filter((c) => !engines.includes(c.comparator)).map((c) => c.comparator),
    ...["ep9", "ep9-jitless"].filter((b) => !subjects.includes(b)),
    ...V8V7_7.filter((w) => !workloads.includes(w)),
  ];
  const g7 = {
    caps_in_run: CAPS.filter((c) => engines.includes(c.comparator)).map((c) => c.comparator),
    builds: subjects,
    missing,
    holds: missing.length ? null : caps.every((c) => c.holds),
  };
  return { table, ratios, caps, g7 };
}

function git(args) {
  const r = spawnSync("git", ["-C", ROOT, ...args], { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
}

/** Throws a Refusal unless the run exited 0, printed `expect` and gave a peak RSS. */
export function checkMemRun(r, { engine, workload, expect }) {
  const tail = (s) => s.trim().split("\n").slice(-5).join("\n    ");
  if (r.error || r.status !== 0) {
    const how = r.error ?? (r.signal ? `signal ${r.signal}` : `exit ${r.status}`);
    throw new Refusal(`${workload} on ${engine}: ${how}\n    ${tail(r.stderr)}`);
  }
  const out = r.stdout.replace(/\r\n/g, "\n").trim();
  if (out !== expect) throw new Refusal(`${workload} on ${engine}: printed ${JSON.stringify(out.slice(0, 200))}, want ${JSON.stringify(expect)}`);
  if (typeof r.rss !== "number" || !(r.rss > 0)) throw new Refusal(`${workload} on ${engine}: the RSS tool gave no peak RSS`);
}

/**
 * The whole reading. `deps` = { sleep(s), load(), log(text), exec(engine, file), tool } (injectable
 * for tests). Returns the ep9-mem/v1 result object.
 */
export async function run(o, deps = {}) {
  const sleep = deps.sleep ?? ((s) => sleepMs(s * 1000));
  const load = deps.load ?? load1;
  const log = deps.log ?? ((s) => process.stderr.write(`${s}\n`));
  const tool = deps.tool ?? rssTool();
  if (!tool) throw new UsageError(`no peak RSS tool for ${process.platform}`);
  const started = new Date().toISOString();
  const measured = hostInfo();
  const reasons = [];
  let hostFile = null;
  if (o.host) {
    hostFile = readHostFile(o.host);
    if (hostFile.cpu !== measured.cpu || hostFile.platform !== measured.platform) {
      throw new UsageError(`${o.host} is for "${hostFile.cpu}" (${hostFile.platform}); this host is "${measured.cpu}" (${measured.platform})`);
    }
    if (typeof hostFile.idle_baseline_load1 !== "number") reasons.push(`host file ${path.basename(o.host)} has no idle baseline`);
  } else {
    reasons.push("no host file (the load guard cannot run)");
  }
  const engines = deps.engines ?? resolveAll(o);
  const byId = Object.fromEntries(engines.map((e) => [e.id, e]));
  const exec = deps.exec ?? ((e, f) => measureRss(tool, e, f, o.timeoutS));

  const work = mkdtempSync(path.join(os.tmpdir(), "ep9-mem-work-"));
  try {
    const files = materialize(o.workloads, work, { harness: o.harness });
    const once = async (e, w) => {
      const r = await exec(byId[e], files[w].file);
      checkMemRun(r, { engine: e, workload: w, expect: files[w].expect });
      return r;
    };
    for (const w of o.workloads) for (const e of o.engines) await once(e, w);
    log(`output check: ${o.workloads.length} workloads x ${o.engines.length} engines ok`);

    let before = null;
    if (hostFile) {
      before = await guardBefore(
        { idleBaseline: hostFile.idle_baseline_load1, settleS: o.settleS, waitS: o.waitMin * 60, pollS: o.pollS },
        { load, sleep },
      );
      log(`load guard before: load ${before.load1}, limit ${before.limit}, ${before.pass ? "pass" : `FAIL (${before.why})`}`);
      if (!before.pass) {
        if (!o.allowBusy) {
          throw new HostBusy(before.limit === null ? `cannot run the load guard: ${before.why}` : `host busy: ${before.why} after waiting ${before.waited_s} s`);
        }
        if (before.limit !== null) reasons.push(`load guard failed before the run (${before.why})`);
      }
    }

    const plan = schedule(o.engines, o.workloads, o.runs);
    const samples = [];
    const roundLoad = [];
    for (const [seq, p] of plan.entries()) {
      const r = await once(p.engine, p.workload);
      samples.push({ seq, round: p.round, workload: p.workload, engine: p.engine, rss_bytes: r.rss, ms: Number(r.ms.toFixed(3)) });
      if (seq + 1 === plan.length || plan[seq + 1].round !== p.round) {
        roundLoad.push(load());
        log(`round ${p.round + 1}/${o.runs}: load ${roundLoad.at(-1)}`);
      }
    }

    let after = null;
    if (hostFile) {
      after = await guardAfter({ idleBaseline: hostFile.idle_baseline_load1, afterS: o.afterS }, { load, sleep });
      log(`load guard after: load ${after.load1}, limit ${after.limit}, ${after.pass ? "pass" : `FAIL (${after.why})`}`);
      if (!after.pass && after.limit !== null) reasons.push(`load guard failed after the run (${after.why})`);
    }

    const a = analyze(samples, { engines: o.engines, workloads: o.workloads });
    const binary = combineBuilds(engines.filter((e) => ENGINES[e.id]?.ep9).map((e) => e.build ?? null));
    return {
      schema: SCHEMA,
      label: o.label,
      started,
      finished: new Date().toISOString(),
      // The commit of the measured ep9 binaries (P0-BUILDID), not of this checkout: null when one
      // has no build line, was built dirty or names another commit (m2-exit: OPEN).
      commit: { ep9: cleanCommit(binary), binary, runner: git(["rev-parse", "HEAD"]), runner_dirty: (git(["status", "--porcelain", "--untracked-files=no"]) ?? "") !== "" },
      host: {
        file: o.host && path.relative(ROOT, o.host),
        id: hostFile?.id ?? null,
        gate_host: hostFile?.gate_host ?? false,
        idle_baseline_load1: hostFile?.idle_baseline_load1 ?? null,
        measured,
      },
      load: { margin: LOAD_MARGIN, before, after, after_each_round: roundLoad },
      provisional: reasons.length > 0,
      provisional_reasons: reasons,
      reader: { tool: tool.name, program: path.basename(tool.program), unit_of_tool: tool.unit, unit: "bytes" },
      runs: o.runs,
      harness: o.harness,
      engines: engines.map((e) => ({ id: e.id, label: e.label, role: e.role, version: e.version, engine_build: e.engine_build ?? null, build: e.build ?? null, bin: path.basename(e.bin), args: e.args })),
      workloads: Object.values(files).filter((f) => o.workloads.includes(f.name)).map(({ file, ...w }) => ({ ...w, file: path.basename(file) })),
      samples,
      ...a,
    };
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

// ---- Markdown -------------------------------------------------------------------------------

const mib = (b) => (b == null ? "-" : (b / (1 << 20)).toFixed(1));
const f3 = (x) => (x == null ? "-" : x.toFixed(3));

export function renderMarkdown(res) {
  const L = [];
  const eng = Object.fromEntries(res.engines.map((e) => [e.id, e]));
  const g = (c) => (!c ? "not checked" : c.limit === null ? `${c.load1} (no idle baseline)` : `${c.load1} (limit ${c.limit}: ${c.pass ? "pass" : "FAIL"})`);
  L.push(`Peak RSS ${res.label ? `"${res.label}" ` : ""}${res.started} to ${res.finished}; commit \`${(res.commit.ep9 ?? "unknown").slice(0, 12)}\`${buildNote(res.commit.binary)}.`);
  L.push("");
  L.push(`- Host: ${res.host.id ?? "no host file"}: ${res.host.measured.cpu}, ${res.host.measured.os} (${res.host.measured.platform}).`);
  L.push(`- Load: before ${g(res.load.before)}; after ${g(res.load.after)}; after each round ${Math.min(...res.load.after_each_round)}-${Math.max(...res.load.after_each_round)}.`);
  L.push(`- Engines: ${res.engines.map((e) => `${e.label} (${e.version}${e.engine_build ? `, ${e.engine_build}` : ""})`).join("; ")}.`);
  L.push(`- Reader: ${res.reader.tool}; ${res.runs} runs per engine and workload, rotated order; median. Ratio = median(ep9) / median(comparator).`);
  L.push(res.provisional ? `- **PROVISIONAL**: ${res.provisional_reasons.join("; ")}.` : "- Class Mem reading: the load guard passed before and after.");
  L.push("");
  L.push(`| workload | ${res.engines.map((e) => `${e.label} MiB`).join(" | ")} |`);
  L.push(`|---|${res.engines.map(() => "---").join("|")}|`);
  for (const w of [...new Set(res.table.map((t) => t.workload))]) {
    L.push(`| ${w} | ${res.engines.map((e) => {
      const t = res.table.find((x) => x.workload === w && x.engine === e.id);
      return `${mib(t.median_bytes)} [${mib(t.min_bytes)}-${mib(t.max_bytes)}]`;
    }).join(" | ")} |`);
  }
  L.push("");
  L.push("| cap | ep9 build | " + [...new Set(res.ratios.map((r) => r.workload))].join(" | ") + " | max | geomean | holds |");
  L.push(`|---|---|${[...new Set(res.ratios.map((r) => r.workload))].map(() => "---").join("|")}|---|---|---|`);
  for (const c of res.caps) {
    L.push(`| ${c.text} | ${eng[c.subject].label} | ${Object.values(c.ratios).map(f3).join(" | ")} | ${f3(c.max_ratio)} (${c.max_workload}) | ${f3(c.geomean_ratio)} | ${c.holds ? "yes" : `no (${c.over.join(", ")})`} |`);
  }
  L.push("");
  L.push(`G7 (both caps, both ep9 builds, the 7 v8-v7 kernels): ${res.g7.holds === null ? `no reading (missing: ${(res.g7.missing ?? []).join(", ")})` : res.g7.holds ? "holds" : "does not hold"}${res.provisional ? " (provisional)" : ""}.`);
  return `${L.join("\n")}\n`;
}

// ---- main -----------------------------------------------------------------------------------

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) {
    const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n");
    process.stdout.write(`${src.slice(1, src.findIndex((l, i) => i > 0 && !l.startsWith("//"))).map((l) => l.replace(/^\/\/ ?/, "")).join("\n")}\n`);
    return;
  }
  let o;
  let res;
  try {
    o = parseArgs(argv);
    res = await run(o);
  } catch (e) {
    if (e instanceof UsageError) {
      console.error(`mem.mjs: ${e.message}`);
      process.exit(2);
    }
    if (e instanceof HostBusy) {
      console.error(`mem.mjs: ${e.message}. Wait for a quiet window, or pass --allow-busy for a provisional run.`);
      process.exit(3);
    }
    if (e instanceof Refusal) {
      console.error(`REFUSED: ${e.message}\nmem.mjs: a run gave a wrong result; no report was written.`);
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
