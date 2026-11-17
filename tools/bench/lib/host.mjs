// Host facts, host files (tools/bench/hosts/<host>.toml) and the load guard of the PERF-NOISE
// rule (docs/plan/10-quality.md section 3):
//
//   Load guard. Check before the run (after 60 s with no benchmark process) and 120 s after it:
//   1-minute load <= idle baseline + 0.5. If it fails, wait up to 30 min, then stop with
//   "host busy". The result records both values.
//
// A host file gives the CPU, the cores, the OS and the idle baseline (median 1-minute load over 10
// idle minutes; `node tools/bench/cross/idle.mjs` measures it).
//
// Load source (plan v14 section 4.1 (5)). On Linux and macOS the load is the 1-minute load of
// os.loadavg() (source "loadavg"). On Windows os.loadavg() is always 0, so a guard on it always
// passes: the Windows source is "typeperf", the counters "\System\Processor Queue Length" and
// "\Processor(_Total)\% Processor Time" sampled once a second for 60 s; the load is the mean
// queue length plus the mean busy share times the logical CPUs (the threads that run or wait to
// run, the unit of the 1-minute load). The limit is the same: idle baseline + 0.5, with the idle
// baseline measured from the same source. A host file may name its source (`load_source`); it
// must be the source of this host. EP9_LOAD_SOURCE=loadavg on Windows keeps the old reading, and
// then every check fails with "no load guard" (a run is provisional, m2-exit reads it as OPEN).

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import os from "node:os";

/** Load guard margin above the idle baseline (10-quality section 3). */
export const LOAD_MARGIN = 0.5;

/** The load sources (see "Load source" above). */
export const LOAD_SOURCES = ["loadavg", "typeperf"];
/** The typeperf counters of the Windows load source, and its sample length in seconds. */
export const TYPEPERF_COUNTERS = ["\\System\\Processor Queue Length", "\\Processor(_Total)\\% Processor Time"];
export const TYPEPERF_SECONDS = 60;

/** The load source of a platform: "typeperf" on Windows, else "loadavg" (env EP9_LOAD_SOURCE wins). */
export function loadSource(platform = process.platform, env = process.env) {
  const forced = env.EP9_LOAD_SOURCE;
  if (forced) {
    if (!LOAD_SOURCES.includes(forced)) throw new Error(`EP9_LOAD_SOURCE=${forced}: not one of ${LOAD_SOURCES.join(", ")}`);
    return forced;
  }
  return platform === "win32" ? "typeperf" : "loadavg";
}

/**
 * The means of a typeperf CSV (`typeperf <queue> <busy> -si 1 -sc N`): a header line, then one
 * `"<time>","<queue>","<busy %>"` line per sample. Lines with an empty or non-numeric value (the
 * first busy sample can be blank) are skipped. null when no sample has both values.
 */
export function parseTypeperf(text) {
  const rows = [];
  for (const line of (text ?? "").split(/\r?\n/)) {
    const cells = [...line.matchAll(/"([^"]*)"/g)].map((m) => m[1].trim());
    if (cells.length !== 3) continue;
    const q = Number(cells[1]);
    const b = Number(cells[2]);
    if (cells[1] === "" || cells[2] === "" || !Number.isFinite(q) || !Number.isFinite(b)) continue;
    rows.push([q, b]);
  }
  if (!rows.length) return null;
  const mean = (i) => rows.reduce((a, r) => a + r[i], 0) / rows.length;
  return { samples: rows.length, queue: mean(0), busy_pct: mean(1) };
}

/** The Windows load of a typeperf reading: mean queue + mean busy share x logical CPUs. */
export function typeperfLoad(reading, cpus) {
  return Number((reading.queue + (reading.busy_pct / 100) * cpus).toFixed(2));
}

/** The CPU model: /proc/cpuinfo "model name" (x86-64 Linux), else lscpu "Model name" (arm64
 * Linux has no such line in /proc/cpuinfo), else what node reports (macOS, Windows). */
export function cpuModel() {
  if (process.platform === "linux") {
    try {
      const m = readFileSync("/proc/cpuinfo", "utf8").match(/^model name\s*:\s*(.+)$/m);
      if (m) return m[1].trim();
    } catch {}
    const l = spawnSync("lscpu", [], { encoding: "utf8" });
    const m = (l.stdout || "").match(/^Model name:\s*(.+)$/m);
    if (m) return m[1].trim();
  }
  return os.cpus()[0]?.model?.trim() || "unknown";
}

/** The Linux file of the transparent huge page mode (MC-w18-O4: m1-exit item 2 records it). */
export const THP_FILE = "/sys/kernel/mm/transparent_hugepage/enabled";

/**
 * The transparent huge page mode of this host: the one bracketed word of `file` (the kernel
 * marks the mode in use: `always [madvise] never` -> "madvise"). null when the platform is not
 * Linux, the file is missing or unreadable, or it does not have exactly one bracketed word:
 * never a guess. `file` and `platform` are injectable for tests.
 */
export function thpMode({ file = THP_FILE, platform = process.platform } = {}) {
  if (platform !== "linux") return null;
  let text;
  try {
    text = readFileSync(file, "utf8");
  } catch {
    return null;
  }
  const m = [...text.matchAll(/\[([^\[\]\s]+)\]/g)];
  return m.length === 1 ? m[0][1] : null;
}

/**
 * The load of this host by its load source (see "Load source" above): the 1-minute load, or on
 * Windows a 60-s typeperf reading (this call then takes 60 s). null when typeperf gives no
 * reading. `deps` (platform, env, run, cpus, seconds) are injectable for tests.
 */
export function load1(deps = {}) {
  const { platform = process.platform, env = process.env, run = spawnSync, cpus = os.cpus().length, seconds = TYPEPERF_SECONDS } = deps;
  if (loadSource(platform, env) !== "typeperf") return Number(os.loadavg()[0].toFixed(2));
  const r = run("typeperf", [...TYPEPERF_COUNTERS, "-si", "1", "-sc", String(seconds)], {
    encoding: "utf8",
    timeout: (seconds + 60) * 1000,
    windowsHide: true,
  });
  const p = parseTypeperf(r.stdout);
  return p ? typeperfLoad(p, cpus) : null;
}

export function hostInfo() {
  return {
    cpu: cpuModel(),
    cpus: os.cpus().length,
    platform: `${process.platform}-${process.arch}`,
    os: `${os.type()} ${os.release()}`,
    load_source: loadSource(),
    loadavg: os.loadavg().map((x) => Number(x.toFixed(2))),
    node: process.version,
    date: new Date().toISOString(),
  };
}

/**
 * The TOML subset the host files use: `[table]` headers, `key = value` lines and `#` comments.
 * A value is a "string", a number, true/false, or a one-line array of numbers or strings.
 * Anything else is an error, so a typo cannot pass as a missing key.
 */
export function parseToml(text) {
  const out = {};
  let table = out;
  const scalar = (v, where) => {
    if (/^"([^"\\]*)"$/.test(v)) return v.slice(1, -1);
    if (v === "true" || v === "false") return v === "true";
    if (/^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/.test(v)) return Number(v);
    throw new Error(`${where}: unsupported TOML value ${v}`);
  };
  text.split("\n").forEach((raw, i) => {
    const where = `line ${i + 1}`;
    // Strip a comment that is not inside a string (host files keep '#' out of strings).
    const line = raw.replace(/\s+#.*$|^#.*$/, "").trim();
    if (!line) return;
    const t = line.match(/^\[([A-Za-z0-9_.-]+)\]$/);
    if (t) {
      table = out[t[1]] = {};
      return;
    }
    const kv = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.+)$/);
    if (!kv) throw new Error(`${where}: not a TOML key = value line: ${raw}`);
    const v = kv[2].trim();
    if (v.startsWith("[")) {
      if (!v.endsWith("]")) throw new Error(`${where}: arrays must fit on one line`);
      const body = v.slice(1, -1).trim();
      table[kv[1]] = body ? body.split(",").map((x) => x.trim()).filter(Boolean).map((x) => scalar(x, where)) : [];
    } else {
      table[kv[1]] = scalar(v, where);
    }
  });
  return out;
}

/**
 * Reads a host file. Required keys: id, cpu, cores, platform, os, gate_host. idle_baseline_load1
 * may be missing (not measured yet): then the load guard cannot pass and every run is provisional.
 */
export function readHostFile(file) {
  const h = parseToml(readFileSync(file, "utf8"));
  for (const k of ["id", "cpu", "cores", "platform", "os", "gate_host"]) {
    if (h[k] === undefined) throw new Error(`${file}: missing key ${k}`);
  }
  const idle = h.idle_baseline_load1;
  if (idle !== undefined && typeof idle !== "number") throw new Error(`${file}: idle_baseline_load1 must be a number`);
  // The idle baseline is a reading of one load source: a host file that names another source
  // than this host's cannot set its limit.
  if (h.load_source !== undefined && !LOAD_SOURCES.includes(h.load_source)) {
    throw new Error(`${file}: load_source must be one of ${LOAD_SOURCES.join(", ")}`);
  }
  return h;
}

/**
 * One load check: pass when the load (of `source`, this host's by default) is at most the idle
 * baseline + 0.5. On Windows the source "loadavg" is no load guard (os.loadavg() is always 0
 * there): the check fails. A load that is not a number (no typeperf reading) fails too.
 */
export function loadCheck(load, idleBaseline, source = loadSource(), platform = process.platform) {
  if (typeof idleBaseline !== "number") {
    return { load1: load, limit: null, pass: false, why: "no idle baseline in the host file", source };
  }
  const limit = Number((idleBaseline + LOAD_MARGIN).toFixed(2));
  if (platform === "win32" && source === "loadavg") {
    return { load1: load, limit, pass: false, why: "no load guard: the load source loadavg is always 0 on Windows", source };
  }
  if (typeof load !== "number" || !Number.isFinite(load)) {
    return { load1: null, limit, pass: false, why: `no load reading (${source})`, source };
  }
  return { load1: load, limit, pass: load <= limit, why: load <= limit ? null : `load ${load} > ${limit}`, source };
}

/**
 * The check before a run: wait `settleS` with no benchmark process, then poll every `pollS`
 * until the load passes or `waitS` has gone by. Returns the last check plus how long it waited.
 * `deps` (load, sleep) are injectable for tests; `source` and `platform` too (this host's by
 * default).
 */
export async function guardBefore({ idleBaseline, settleS, waitS, pollS = 30, source = loadSource(), platform = process.platform }, deps) {
  const { load = load1, sleep } = deps;
  await sleep(settleS);
  let waited = 0;
  let c = loadCheck(load(), idleBaseline, source, platform);
  // Without a baseline, or with no load guard (loadavg on Windows), no wait can make it pass.
  const hopeless = (x) => x.limit === null || (platform === "win32" && source === "loadavg");
  while (!c.pass && !hopeless(c) && waited < waitS) {
    const step = Math.min(pollS, waitS - waited);
    await sleep(step);
    waited += step;
    c = loadCheck(load(), idleBaseline, source, platform);
  }
  return { ...c, settle_s: settleS, waited_s: waited };
}

/** The check after a run: wait `afterS`, then one check. */
export async function guardAfter({ idleBaseline, afterS, source = loadSource(), platform = process.platform }, deps) {
  const { load = load1, sleep } = deps;
  await sleep(afterS);
  return { ...loadCheck(load(), idleBaseline, source, platform), after_s: afterS };
}
