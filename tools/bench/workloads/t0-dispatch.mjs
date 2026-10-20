#!/usr/bin/env node
// T0 dispatch comparison (H15 ADR): `ep9 --dispatch A` vs `ep9 --dispatch B` on the kernels of
// this directory, with paired, interleaved rounds and the PERF-NOISE statistics of
// docs/plan/10-quality.md section 3 (class S).
//
//   node tools/bench/workloads/t0-dispatch.mjs --ep9 target/release/ep9 [options]
//
//   --ep9 PATH          the ep9 binary (release build)
//   --ep9-b PATH        a second binary: config B runs it (compare two builds, for example
//                       `--configs sibling,sibling`); the labels become <dispatch>@a, <dispatch>@b
//   --rounds N          paired rounds after the warm-up (default 31, the hosted-runner minimum)
//   --warmup N          dropped warm-up rounds (default 1)
//   --kernels LIST      comma list of kernel files in this directory (default: all below)
//   --configs A,B       two --dispatch values; the ratio is time(A) / time(B) (default match,sibling)
//   --out FILE          write all samples and results as JSON
//   --summary FILE      append a Markdown table (for $GITHUB_STEP_SUMMARY)
//   --cachegrind        count instructions once per kernel and config under valgrind (class R,
//                       exact) instead of timing
//
// A round runs every kernel once with each config, back to back. The config order rotates per
// round, and the kernel order rotates too, so no config always runs first or after one kernel.
// Each run is one process; the time is its wall time (spawn included, the same for both configs).
// Every kernel checks its own result and throws on a wrong one; a run that does not exit with 0
// or prints another line than the first run stops the script.
//
// Verdict per kernel (ratio = A / B, > 1 means B is faster): median of the per-round ratios and
// the order-statistic interval (n = 31: ranks 10 and 22). "B faster" or "A faster" only when the
// whole interval is on one side of 1, else "tie". Void when the IQR of the ratios is > 5% of
// the median (the host was not quiet).

import { spawnSync } from "node:child_process";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const KERNELS = ["t0-loop.js", "t0-branchy.js", "t0-call.js", "t0-prop.js", "ci-v0.js"];

export function parseArgs(argv) {
  const o = {
    ep9: null,
    ep9b: null,
    rounds: 31,
    warmup: 1,
    kernels: KERNELS,
    configs: ["match", "sibling"],
    out: null,
    summary: null,
    cachegrind: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case "--ep9": o.ep9 = val(); break;
      case "--ep9-b": o.ep9b = val(); break;
      case "--rounds": o.rounds = Number(val()); break;
      case "--warmup": o.warmup = Number(val()); break;
      case "--kernels": o.kernels = val().split(","); break;
      case "--configs": o.configs = val().split(","); break;
      case "--out": o.out = val(); break;
      case "--summary": o.summary = val(); break;
      case "--cachegrind": o.cachegrind = true; break;
      default: throw new Error(`unknown option ${a}`);
    }
  }
  if (!o.ep9) throw new Error("--ep9 is required");
  if (o.configs.length !== 2) throw new Error("--configs needs exactly two values");
  if (!(o.rounds >= 1) || !(o.warmup >= 0)) throw new Error("bad --rounds or --warmup");
  // Each config: a label, a binary and a --dispatch value.
  o.runs = o.configs.map((d, i) => ({
    label: o.ep9b ? `${d}@${"ab"[i]}` : d,
    bin: i === 1 && o.ep9b ? o.ep9b : o.ep9,
    dispatch: d,
  }));
  if (o.runs[0].label === o.runs[1].label) throw new Error("the two configs are the same run");
  return o;
}

// ---- statistics (10-quality section 3) ----------------------------------------------------

function sorted(xs) {
  return [...xs].sort((a, b) => a - b);
}

export function median(xs) {
  const s = sorted(xs);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

/** Quantile with linear interpolation between order statistics (type 7). */
export function quantile(xs, q) {
  const s = sorted(xs);
  const h = (s.length - 1) * q;
  const lo = Math.floor(h);
  return s[lo] + (h - lo) * ((s[Math.min(lo + 1, s.length - 1)] ?? s[lo]) - s[lo]);
}

/**
 * 1-based ranks (k, n + 1 - k) of the order-statistic interval for the median with coverage
 * >= 95%: the largest k with 1 - 2 P(Bin(n, 1/2) <= k - 1) >= 0.95. n = 31 gives (10, 22),
 * n = 15 (4, 12), n = 10 (2, 9), n = 7 (1, 7), as in the PERF-NOISE table.
 */
export function medianRanks(n) {
  // P(Bin(n, 1/2) <= j), summed in floating point (n is small).
  const cdf = (j) => {
    let c = 1;
    let sum = 0;
    for (let i = 0; i <= j; i++) {
      sum += c;
      c = (c * (n - i)) / (i + 1);
    }
    return sum / 2 ** n;
  };
  let k = 1;
  while (k + 1 <= n && 1 - 2 * cdf(k) >= 0.95) k++;
  return [k, n + 1 - k];
}

export function verdict(ratios) {
  const s = sorted(ratios);
  const [lo, hi] = medianRanks(s.length);
  const med = median(s);
  const iqr = quantile(s, 0.75) - quantile(s, 0.25);
  const interval = [s[lo - 1], s[hi - 1]];
  const side =
    interval[0] > 1 ? "B faster" : interval[1] < 1 ? "A faster" : "tie";
  return {
    median: med,
    interval,
    ranks: [lo, hi],
    iqrPct: (100 * iqr) / med,
    void: iqr / med > 0.05,
    side,
  };
}

export function geomean(xs) {
  return Math.exp(xs.reduce((a, x) => a + Math.log(x), 0) / xs.length);
}

// ---- host ---------------------------------------------------------------------------------

/** The CPU model: /proc/cpuinfo "model name" (x86-64 Linux), else lscpu "Model name" (arm64
 * Linux has no such line in /proc/cpuinfo), else what node reports (macOS, Windows). */
function cpuModel() {
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

function host() {
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

// ---- runs ---------------------------------------------------------------------------------

function runOnce(run, kernel, expect) {
  const file = path.join(DIR, kernel);
  const t0 = process.hrtime.bigint();
  const r = spawnSync(run.bin, ["--dispatch", run.dispatch, file], { encoding: "utf8" });
  const ms = Number(process.hrtime.bigint() - t0) / 1e6;
  if (r.error) throw r.error;
  const out = (r.stdout || "").trim();
  if (r.status !== 0) {
    throw new Error(`${kernel} ${run.label}: exit ${r.status}\n${r.stdout}\n${r.stderr}`);
  }
  if (expect !== undefined && out !== expect) {
    throw new Error(`${kernel} ${run.label}: printed ${out}, first run printed ${expect}`);
  }
  return { ms, out };
}

function timing(o) {
  const [A, B] = o.runs.map((r) => r.label);
  const samples = Object.fromEntries(o.kernels.map((k) => [k, { [A]: [], [B]: [] }]));
  const outputs = {};
  const total = o.warmup + o.rounds;
  for (let r = 0; r < total; r++) {
    const ks = o.kernels.map((_, i) => o.kernels[(i + r) % o.kernels.length]);
    for (const k of ks) {
      const order = r % 2 === 0 ? o.runs : [o.runs[1], o.runs[0]];
      for (const run of order) {
        const { ms, out } = runOnce(run, k, outputs[k]);
        outputs[k] ??= out;
        if (r >= o.warmup) samples[k][run.label].push(ms);
      }
    }
    process.stderr.write(`round ${r + 1}/${total}${r < o.warmup ? " (warm-up)" : ""}\n`);
  }
  const kernels = {};
  for (const k of o.kernels) {
    const a = samples[k][A];
    const b = samples[k][B];
    const ratios = a.map((x, i) => x / b[i]);
    kernels[k] = {
      output: outputs[k],
      [`${A}_ms`]: median(a),
      [`${B}_ms`]: median(b),
      ...verdict(ratios),
      samples: { [A]: a, [B]: b },
    };
  }
  const timed = o.kernels.filter((k) => k !== "ci-v0.js");
  return {
    kernels,
    geomean: geomean(timed.map((k) => kernels[k].median)),
    geomeanKernels: timed,
  };
}

function cachegrind(o) {
  const kernels = {};
  for (const k of o.kernels) {
    kernels[k] = {};
    for (const { label: c, bin, dispatch } of o.runs) {
      const r = spawnSync(
        "valgrind",
        [
          "--tool=cachegrind",
          "--cache-sim=no",
          `--cachegrind-out-file=${os.devNull}`,
          bin,
          "--dispatch",
          dispatch,
          path.join(DIR, k),
        ],
        { encoding: "utf8" },
      );
      if (r.error) throw r.error;
      if (r.status !== 0) throw new Error(`cachegrind ${k} ${c}: exit ${r.status}\n${r.stderr}`);
      const m = r.stderr.match(/I\s+refs:\s+([\d,]+)/);
      if (!m) throw new Error(`cachegrind ${k} ${c}: no "I refs" line\n${r.stderr}`);
      kernels[k][c] = Number(m[1].replaceAll(",", ""));
      process.stderr.write(`cachegrind ${k} ${c}: ${kernels[k][c]}\n`);
    }
    kernels[k].ratio = kernels[k][o.runs[0].label] / kernels[k][o.runs[1].label];
  }
  return { kernels };
}

function fmt(x, d = 3) {
  return x.toFixed(d);
}

function report(o, h, res) {
  const [A, B] = o.runs.map((r) => r.label);
  const lines = [];
  lines.push(`cpu: ${h.cpu} (${h.cpus} logical CPUs; ${h.platform}; ${h.os}; load ${h.loadavg.join(" ")})`);
  if (o.cachegrind) {
    lines.push("", `| kernel | ${A} instructions | ${B} instructions | ${A} / ${B} |`, "|---|---|---|---|");
    for (const [k, v] of Object.entries(res.kernels)) {
      lines.push(`| ${k} | ${v[A]} | ${v[B]} | ${fmt(v.ratio)} |`);
    }
  } else {
    lines.push(
      `rounds: ${o.rounds} paired (+${o.warmup} warm-up dropped); ratio = time(${A}) / time(${B}), > 1: ${B} faster`,
      "",
      `| kernel | ${A} ms | ${B} ms | ratio (median) | interval (ranks ${res.kernels[o.kernels[0]].ranks.join(", ")}) | IQR % | verdict |`,
      "|---|---|---|---|---|---|---|",
    );
    for (const [k, v] of Object.entries(res.kernels)) {
      const verdictText = v.side === "B faster" ? `${B} faster` : v.side === "A faster" ? `${A} faster` : "tie";
      lines.push(
        `| ${k} | ${fmt(v[`${A}_ms`], 1)} | ${fmt(v[`${B}_ms`], 1)} | ${fmt(v.median)} | [${fmt(v.interval[0])}, ${fmt(v.interval[1])}] | ${fmt(v.iqrPct, 1)} | ${verdictText}${v.void ? " (void: IQR > 5%)" : ""} |`,
      );
    }
    lines.push("", `geomean of the medians (${res.geomeanKernels.join(", ")}): ${fmt(res.geomean)}`);
  }
  return lines.join("\n");
}

function main() {
  const o = parseArgs(process.argv.slice(2));
  const h = host();
  process.stderr.write(`cpu: ${h.cpu}\n`);
  const res = o.cachegrind ? cachegrind(o) : timing(o);
  h.loadavgAfter = os.loadavg().map((x) => Number(x.toFixed(2)));
  const text = report(o, h, res);
  console.log(text);
  if (o.out) {
    writeFileSync(
      o.out,
      `${JSON.stringify({ host: h, options: { ...o, runs: o.runs.map((r) => ({ ...r, bin: path.basename(r.bin) })), ep9: path.basename(o.ep9), ep9b: o.ep9b && path.basename(o.ep9b) }, ...res }, null, 1)}\n`,
    );
  }
  if (o.summary) appendFileSync(o.summary, `${text}\n\n`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
