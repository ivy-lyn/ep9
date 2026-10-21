// Host facts, host files (tools/bench/hosts/<host>.toml) and the load guard of the PERF-NOISE
// rule (docs/plan/10-quality.md section 3):
//
//   Load guard. Check before the run (after 60 s with no benchmark process) and 120 s after it:
//   1-minute load <= idle baseline + 0.5. If it fails, wait up to 30 min, then stop with
//   "host busy". The result records both values.
//
// A host file gives the CPU, the cores, the OS and the idle baseline (median 1-minute load over 10
// idle minutes; `node tools/bench/cross/idle.mjs` measures it).

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import os from "node:os";

/** Load guard margin above the idle baseline (10-quality section 3). */
export const LOAD_MARGIN = 0.5;

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

export function load1() {
  return Number(os.loadavg()[0].toFixed(2));
}

export function hostInfo() {
  return {
    cpu: cpuModel(),
    cpus: os.cpus().length,
    platform: `${process.platform}-${process.arch}`,
    os: `${os.type()} ${os.release()}`,
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
  return h;
}

/** One load check: pass when the 1-minute load is at most the idle baseline + 0.5. */
export function loadCheck(load, idleBaseline) {
  if (typeof idleBaseline !== "number") {
    return { load1: load, limit: null, pass: false, why: "no idle baseline in the host file" };
  }
  const limit = Number((idleBaseline + LOAD_MARGIN).toFixed(2));
  return { load1: load, limit, pass: load <= limit, why: load <= limit ? null : `load ${load} > ${limit}` };
}

/**
 * The check before a run: wait `settleS` with no benchmark process, then poll every `pollS`
 * until the load passes or `waitS` has gone by. Returns the last check plus how long it waited.
 * `deps` (loadavg, sleep) are injectable for tests.
 */
export async function guardBefore({ idleBaseline, settleS, waitS, pollS = 30 }, deps) {
  const { load = load1, sleep } = deps;
  await sleep(settleS);
  let waited = 0;
  let c = loadCheck(load(), idleBaseline);
  // Without a baseline no wait can make it pass.
  while (!c.pass && c.limit !== null && waited < waitS) {
    const step = Math.min(pollS, waitS - waited);
    await sleep(step);
    waited += step;
    c = loadCheck(load(), idleBaseline);
  }
  return { ...c, settle_s: settleS, waited_s: waited };
}

/** The check after a run: wait `afterS`, then one check. */
export async function guardAfter({ idleBaseline, afterS }, deps) {
  const { load = load1, sleep } = deps;
  await sleep(afterS);
  return { ...loadCheck(load(), idleBaseline), after_s: afterS };
}
