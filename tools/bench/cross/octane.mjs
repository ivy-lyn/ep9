#!/usr/bin/env node
// Generates an Octane-7 kernel with the real Octane base.js and a fixed iteration count (plan v9
// V9-D14), from the pinned checkout third_party/bench-octane (scripts/sync-bench.sh; pin in
// third_party/bench-octane.commit).
//
//   node tools/bench/cross/octane.mjs --kernel richards [--n N] [--out FILE]   (stdout without --out)
//   node tools/bench/cross/octane.mjs --kernel typescript [--out FILE]         the TS workload (3 files)
//   node tools/bench/cross/octane.mjs --list                                   kernels and default N
//   node tools/bench/cross/octane.mjs --blockers target/release/ep9           which kernels run
//
// Octane-7 = richards, deltablue, crypto, raytrace, earley-boyer, splay, navier-stokes (the
// M1 exit-2 set, docs/research/06-execution-tiers.md; regexp is not in it).
//
// The TS workload (M2 exit 3; expectations/jitless-set.txt `octane/typescript`; plan v12
// V12-D5) is the kernel `typescript` (`--kernel typescript`; not in Octane-7). It has 3 files,
// in the load order of the pinned run.js (typescript.js, typescript-input.js,
// typescript-compiler.js), and N = 5: the benchmark's own deterministic iteration count
// (typescript.js `new Benchmark("Typescript", false, true, 5, ...)`), with no warm-up.
//
// The generated file is: the print prelude, the pinned base.js unchanged, the pinned kernel
// files unchanged, then a driver. d8 loads base.js and the kernel files as separate scripts
// (run.js); all are sloppy scripts with no top-level let, const or class, so one script with the
// texts in that order runs the same code (generate() refuses a file that starts with "use
// strict"). One difference: a function declaration of a later file exists from the start of the
// one script, not only from its own script; the pinned files read no such name before their own
// script runs. One script keeps every error line in a file: line L of a part is line L + offset.
//
// The driver changes data, not base.js code: BenchmarkSuite.config makes base.js run without
// warm-up in its deterministic mode, and each benchmark's deterministicIterations and
// minIterations are set to N. So base.js's own RunSuites, RunStep, RunSingleBenchmark (Measure,
// timed with Date), Setup and TearDown, NotifyResult (GeometricMeanTime: Math.log, Math.pow),
// the latency score (splay's rmsResult) and the final GeometricMean all run as in Octane, on
// exactly N run() calls per benchmark. The runner's NotifyStep prints "<benchmark name> <N>";
// NotifyError, a suite score or a total score that is not a number above 0 print an "ERROR" line,
// so the cross runner refuses the run. Scores are not printed (they vary per run).
//
// The pin check is v8v7.mjs's: the checkout's HEAD must equal the pin, the text is read from the
// pinned commit (git cat-file), and the file on disk must equal it. The output names the pin and
// the blobs, and holds no date, so two runs give identical files.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { benchmarkNames, checkPin, errorPlace, pinned, PRINT_PRELUDE, ROOT } from "./v8v7.mjs";

/** The harness name in generate() results and in the cross JSON. */
export const HARNESS = "octane-base";

/**
 * The Octane-7 kernels and their default N: the N of the same kernel in v8v7.mjs KERNELS. The
 * Octane files are the v8-v7 programs with small changes (the Benchmark arguments, a few fixes),
 * and on QuickJS 2026-06-04 (M5 Max, 2026-10-26, (t(N = 21) - t(N = 1)) / 20, best of 3, at load
 * 9-10: too busy to size runs anew) their time per run() call was 0.6-1.3x the v8-v7 one. So both
 * suites run each kernel the same number of times. Change N only together with a new report.
 */
export const KERNELS = {
  richards: { file: "richards.js", n: 200 },
  deltablue: { file: "deltablue.js", n: 100 },
  crypto: { file: "crypto.js", n: 8 },
  raytrace: { file: "raytrace.js", n: 24 },
  "earley-boyer": { file: "earley-boyer.js", n: 8 },
  splay: { file: "splay.js", n: 200 },
  "navier-stokes": { file: "navier-stokes.js", n: 12 },
};

/**
 * Kernels outside Octane-7: the TS workload (M2 exit 3), with its files in run.js order and the
 * benchmark's own N (typescript.js:28-38 at the pin).
 */
export const EXTRA_KERNELS = {
  typescript: { files: ["typescript.js", "typescript-input.js", "typescript-compiler.js"], n: 5 },
};

/** The kernel name of the TS workload. */
export const TS_KERNEL = "typescript";

/** The file list of a kernel of KERNELS or EXTRA_KERNELS, or null. */
function kernelFiles(kernel) {
  const k = KERNELS[kernel] ?? EXTRA_KERNELS[kernel];
  if (!k) return null;
  return { files: k.files ?? [k.file], n: k.n };
}

const STRICT = /^\s*(\/\/[^\n]*\n\s*)*["']use strict["']/;

/**
 * The generated text of a kernel and what it prints. `n` defaults to the KERNELS (or
 * EXTRA_KERNELS) table. Returns { text, expect, kernel, n, pin, blob, source, harness,
 * harness_blob, offset, parts, blobs }. `offset`: line L of the (first) kernel file is line
 * L + offset of the generated file. `parts`: the same for base.js and each kernel file
 * ({ file, offset, lines }), for error lines. `blobs`: each kernel file and its blob, in load
 * order; `blob` and `source` name the first file (one file for an Octane-7 kernel), `source`
 * then lists all of them.
 */
export function generate(kernel, { n, root = ROOT } = {}) {
  const k = kernelFiles(kernel);
  if (!k) throw new Error(`unknown Octane kernel ${kernel} (known: ${[...Object.keys(KERNELS), ...Object.keys(EXTRA_KERNELS)].join(", ")})`);
  const iterations = n ?? k.n;
  if (!Number.isInteger(iterations) || iterations < 1) throw new Error(`bad --n ${n}`);
  const at = checkPin(root, "bench-octane");
  const srcs = k.files.map((f) => pinned(at, f));
  const src = srcs[0];
  const base = pinned(at, "base.js");
  for (const f of [base, ...srcs]) {
    if (STRICT.test(f.text)) throw new Error(`${f.rel}: a strict file cannot share a script with the others`);
  }
  const names = srcs.flatMap((f) => benchmarkNames(f.text));
  if (!names.length) throw new Error(`${srcs.map((f) => f.rel).join(", ")}: no "new Benchmark(" call`);
  const header = [
    "// Generated by tools/bench/cross/octane.mjs. Do not edit; generate it again.",
    ...srcs.map((f, i) => `// ${i ? "and" : "Source: chromium/octane at " + at.pin + ","} ${f.rel} (blob ${f.blob}),`),
    `// with its harness ${base.rel} (blob ${base.blob}); ${srcs.length > 1 ? "all" : "both"} unchanged, in d8's load order.`,
    `// The driver below sets base.js's deterministic mode with N = ${iterations} run() calls per benchmark`,
    `// (no warm-up), then runs base.js's RunSuites. Prints "<name> ${iterations}" per benchmark.`,
  ].join("\n");
  const end = (t) => (t.endsWith("\n") ? "" : "\n");
  const driver = [
    "BenchmarkSuite.config.doWarmup = false;",
    "BenchmarkSuite.config.doDeterministic = true;",
    "(function () {",
    "  var suites = BenchmarkSuite.suites;",
    "  for (var i = 0; i < suites.length; i++) {",
    "    for (var j = 0; j < suites[i].benchmarks.length; j++) {",
    `      suites[i].benchmarks[j].deterministicIterations = ${iterations};`,
    `      suites[i].benchmarks[j].minIterations = ${iterations};`,
    "    }",
    "  }",
    "})();",
    "BenchmarkSuite.RunSuites({",
    `  NotifyStep: function (name) { print(name + " ${iterations}"); },`,
    '  NotifyError: function (name, error) { print("ERROR " + name + " " + error); },',
    '  NotifyResult: function (name, score) { if (!(Number(score) > 0)) print("ERROR " + name + " score " + score); },',
    '  NotifyScore: function (score) { if (!(Number(score) > 0)) print("ERROR score " + score); }',
    "});",
  ].join("\n");
  const lines = (t) => t.split("\n").length - (t.endsWith("\n") ? 1 : 0);
  const offsetOf = (t) => t.split("\n").length - 1;
  let text = `${header}\n${PRINT_PRELUDE}// ---- ${base.rel} ----\n`;
  const parts = [{ file: base.rel, offset: offsetOf(text), lines: lines(base.text) }];
  text += `${base.text}${end(base.text)}// ---- end of ${base.rel} ----\n`;
  for (const f of srcs) {
    text += `// ---- ${f.rel} ----\n`;
    parts.push({ file: f.rel, offset: offsetOf(text), lines: lines(f.text) });
    text += `${f.text}${end(f.text)}// ---- end of ${f.rel} ----\n`;
  }
  text += `${driver}\n`;
  return {
    text,
    expect: names.map((x) => `${x} ${iterations}`).join("\n"),
    kernel,
    n: iterations,
    pin: at.pin,
    blob: src.blob,
    source: srcs.map((f) => f.rel).join(", "),
    harness: HARNESS,
    harness_blob: base.blob,
    offset: parts[1].offset,
    parts,
    blobs: srcs.map((f) => ({ file: f.rel, blob: f.blob })),
  };
}

/**
 * Runs every kernel once on `engine` (N = 1) and returns a Markdown table, as v8v7.mjs --blockers.
 * kernels.mjs --check does the same for all 15 kernels, against node's output.
 */
export function blockers(engine, n = 1) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-octane-"));
  const rows = [`| kernel | result on ${path.basename(engine)} (N = ${n}, Octane base.js) | source line |`, "|---|---|---|"];
  try {
    for (const k of Object.keys(KERNELS)) {
      const g = generate(k, { n });
      const file = path.join(dir, `${k}.js`);
      writeFileSync(file, g.text);
      const r = spawnSync(engine, [file], { encoding: "utf8", timeout: 600_000 });
      const out = (r.stdout || "").trim();
      if (r.status === 0 && out === g.expect) {
        rows.push(`| ${k} | runs (prints ${JSON.stringify(out)}) | |`);
        continue;
      }
      const e = errorPlace(g, file, r);
      const cell = (x) => x.replaceAll("|", "\\|").replaceAll("`", "'");
      rows.push(`| ${k} | ${e.place ? `${e.place}: ` : ""}${cell(e.message)} | ${e.line ? `\`${cell(e.line.slice(0, 80))}\`` : ""} |`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return rows.join("\n");
}

export function parseArgs(argv) {
  const o = { kernel: null, n: undefined, out: null, list: false, blockers: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case "--kernel": o.kernel = val(); break;
      case "--n": o.n = Number(val()); break;
      case "--out": o.out = val(); break;
      case "--list": o.list = true; break;
      case "--blockers": o.blockers = val(); break;
      default: throw new Error(`unknown option ${a}`);
    }
  }
  if (!o.list && !o.kernel && !o.blockers) throw new Error("--kernel NAME, --list or --blockers ENGINE is required");
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
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`octane.mjs: ${e.message}`);
    process.exit(2);
  }
  if (o.list) {
    for (const [k, v] of Object.entries(KERNELS)) console.log(`${k}\tN = ${v.n}\t${v.file}`);
    for (const [k, v] of Object.entries(EXTRA_KERNELS)) console.log(`${k}\tN = ${v.n}\t${v.files.join(", ")} (not in Octane-7)`);
    return;
  }
  try {
    if (o.blockers) {
      console.log(blockers(o.blockers, o.n ?? 1));
      return;
    }
    const g = generate(o.kernel, { n: o.n });
    if (o.out) {
      writeFileSync(o.out, g.text);
      console.error(`octane.mjs: wrote ${o.out} (${g.source} at ${g.pin}, N = ${g.n}; prints ${JSON.stringify(g.expect)})`);
    } else {
      process.stdout.write(g.text);
    }
  } catch (e) {
    console.error(`octane.mjs: ${e.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
