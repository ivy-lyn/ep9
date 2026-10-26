#!/usr/bin/env node
// Generates a V8 benchmark suite v7 kernel with a stub harness (decision PERF-2), from the pinned
// checkout third_party/bench-v8v7 (scripts/sync-bench.sh; pin in third_party/bench-v8v7.commit).
//
//   node tools/bench/cross/v8v7.mjs --kernel richards [--n 200] [--harness stub|base] [--out FILE]
//                                                                  (stdout without --out)
//   node tools/bench/cross/v8v7.mjs --list                                     kernels and default N
//   node tools/bench/cross/v8v7.mjs --blockers target/release/ep9 [--harness base]   which kernels run
//
// --blockers runs every kernel once (N = 1) on the given engine and prints a Markdown table: "runs",
// or the first error line with its place in the kernel file (the generated line minus the stub).
//
// Why a stub: the official harness (base.js + run_harness.js) needs Date for its timer and score,
// and Math for its deterministic Math.random. ep9 T0 has neither yet. The stub keeps the kernel
// file as it is and replaces only the harness:
//   - BenchmarkSuite(name, reference, benchmarks) records the benchmarks; Benchmark(name, run,
//     setup, tearDown) is base.js's constructor (base.js:39-44);
//   - Math.random is base.js's deterministic version (copied from the pinned base.js), installed
//     only when Math exists;
//   - each benchmark runs setup, then `run` N times, then tear-down (as base.js RunStep), and
//     prints "<benchmark name> <N>". No timer and no score: the cross runner times the process.
// The kernels check their own results (richards.js:66-71 throws; deltablue calls `alert`, which
// no shell defines). `print` falls back to console.log, so the same text runs on every engine,
// node included.
//
// The base harness (--harness base; perf run 3, plan v8 M3-P2 PERF3 task 7): the real pinned
// base.js, unchanged, then the kernel, then a driver that calls base.js's own
// BenchmarkSuite.RunSuites. base.js runs as its own strict script, as d8 loads it (a separate
// file): an indirect eval of its text, which then copies Benchmark, BenchmarkResult and
// BenchmarkSuite to the global object for the kernel (sloppy, as in d8). The driver replaces one
// method: BenchmarkSuite.prototype.RunSingleBenchmark runs run() N times (timed with Date, as
// base.js's Measure does) instead of "until 1 s has passed", because the cross runner compares
// wall time per process (class M), which a time-bounded loop makes the same on every engine.
// Everything else is base.js: RunSuites, RunStep and its continuations, Setup and TearDown,
// NotifyStep, NotifyResult and GeometricMean (Math.log, Math.pow), BenchmarkResult. The runner's
// NotifyStep prints "<benchmark name> <N>" (the stub's output), NotifyError prints an "ERROR"
// line (so the cross runner refuses the run), and scores are not printed (they vary per run).
//
// The pin check: the checkout's HEAD must equal the pin, the text is read from the pinned commit
// (git cat-file), and the file on disk must equal it. The output names the pin and the blob, and
// holds no date, so two runs give identical files.

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

/** The first line of every cross workload: `print` for engines that have none (node). */
export const PRINT_PRELUDE = 'if (typeof print !== "function") globalThis.print = (...a) => console.log(...a);\n';

/**
 * Kernels of bench/bench-v8 and their default N. richards N = 200 is the plan's (week-4 plan
 * section 11): QuickJS 2026-06-04 takes about 0.36 s on it on the M5 Max. The other N give
 * QuickJS about the same time: N = round(0.35 s / (time per run() call)), where the time per call
 * is (t(N = 21) - t(N = 1)) / 20, best of 3, measured 2026-10-21 at load 2.4-3 (rough; it only
 * sizes the runs). splay's setup alone takes about 0.15 s. Change N only together with a new report.
 */
export const KERNELS = {
  richards: { file: "richards.js", n: 200 }, // 1.76 ms per run() on QuickJS
  deltablue: { file: "deltablue.js", n: 100 }, // 3.33 ms
  crypto: { file: "crypto.js", n: 8 }, // 42.7 ms
  raytrace: { file: "raytrace.js", n: 24 }, // 15.3 ms
  "earley-boyer": { file: "earley-boyer.js", n: 8 }, // 44.6 ms
  regexp: { file: "regexp.js", n: 5 }, // 77 ms
  splay: { file: "splay.js", n: 200 }, // 0.89 ms, plus about 0.15 s of setup
  "navier-stokes": { file: "navier-stokes.js", n: 12 }, // 28.4 ms
};

/** The harnesses: "stub" (decision PERF-2, runs 1 and 2) and "base" (the real base.js, run 3). */
export const HARNESSES = ["stub", "base"];

/**
 * The default harness of v8v7.mjs, run.mjs, workloads.mjs and kernels.mjs (and so of perf CI v0).
 * "base" since MATH1 (plan v9 M4-P1, V9-D14): ep9 has Math.log and the correctly rounded pow,
 * which base.js's GeometricMean needs. "--harness stub" keeps the runs 1 and 2 form.
 */
export const DEFAULT_HARNESS = "base";

function git(dir, args) {
  return execFileSync("git", ["-C", dir, ...args], { encoding: "utf8", maxBuffer: 1 << 26 });
}

/** The pinned bench checkouts (scripts/sync-bench.sh) and the variable that overrides each dir. */
export const PIN_DIR_ENV = { "bench-v8v7": "EP9_V8V7_DIR", "bench-octane": "EP9_OCTANE_DIR" };

/**
 * The checkout third_party/<repo> and its pin (third_party/<repo>.commit); throws when HEAD is not
 * the pin. EP9_V8V7_DIR (EP9_OCTANE_DIR for bench-octane) overrides the dir.
 */
export function checkPin(root = ROOT, repo = "bench-v8v7") {
  if (!PIN_DIR_ENV[repo]) throw new Error(`unknown bench checkout ${repo}`);
  const dir = process.env[PIN_DIR_ENV[repo]] || path.join(root, "third_party", repo);
  const pin = readFileSync(path.join(root, "third_party", `${repo}.commit`), "utf8").trim();
  if (!/^[0-9a-f]{40}$/.test(pin)) throw new Error(`third_party/${repo}.commit: not a commit id: ${pin}`);
  let head;
  try {
    head = git(dir, ["rev-parse", "HEAD"]).trim();
  } catch (e) {
    throw new Error(`${dir}: not a git checkout (run bash scripts/sync-bench.sh): ${e.message.split("\n")[0]}`);
  }
  if (head !== pin) throw new Error(`${dir} is at ${head}, the pin is ${pin}: run bash scripts/sync-bench.sh ${repo}`);
  return { dir, pin };
}

/** A file of the checkout (`rel` inside it) at the pin, and its blob id. The file on disk must be the same. */
export function pinned({ dir, pin }, rel) {
  const blob = git(dir, ["rev-parse", `${pin}:${rel}`]).trim();
  const text = git(dir, ["cat-file", "blob", blob]);
  const disk = readFileSync(path.join(dir, rel), "utf8");
  if (disk !== text) throw new Error(`${path.join(dir, rel)} differs from the pinned blob ${blob}`);
  return { rel, blob, text };
}

/** base.js's deterministic Math.random, as text (base.js:85-98 at the pin). */
function mathRandom(pin) {
  const m = pin.text.match(/^Math\.random = \(function\(\) \{\n[\s\S]*?^\}\)\(\);$/m);
  if (!m) throw new Error(`${pin.rel}: no "Math.random = (function() {" block`);
  return m[0];
}

/** The benchmark names a kernel registers (`new Benchmark("Name", ...)`), in order. */
export function benchmarkNames(text) {
  return [...text.matchAll(/new Benchmark\(\s*(['"])([^'"]+)\1/g)].map((m) => m[2]);
}

/**
 * The generated text of a kernel and what it prints. `n` defaults to the KERNELS table.
 * Returns { text, expect, kernel, n, pin, blob, source, offset }.
 */
export function generate(kernel, { n, root = ROOT, harness = DEFAULT_HARNESS } = {}) {
  if (!HARNESSES.includes(harness)) throw new Error(`unknown harness ${harness} (known: ${HARNESSES.join(", ")})`);
  const k = KERNELS[kernel];
  if (!k) throw new Error(`unknown kernel ${kernel} (known: ${Object.keys(KERNELS).join(", ")})`);
  const iterations = n ?? k.n;
  if (!Number.isInteger(iterations) || iterations < 1) throw new Error(`bad --n ${n}`);
  const at = checkPin(root);
  const src = pinned(at, `bench/bench-v8/${k.file}`);
  const base = pinned(at, "bench/bench-v8/base.js");
  const names = benchmarkNames(src.text);
  if (!names.length) throw new Error(`${src.rel}: no "new Benchmark(" call`);
  if (harness === "base") return generateBase({ at, src, base, names, iterations, kernel });
  const header = [
    "// Generated by tools/bench/cross/v8v7.mjs. Do not edit; generate it again.",
    `// Source: boa-dev/data at ${at.pin}, ${src.rel} (blob ${src.blob}),`,
    "// the V8 benchmark suite v7. Stub harness (decision PERF-2): no timer and no score; each",
    `// benchmark runs setup, then run() ${iterations} times, then tear-down, and prints "<name> ${iterations}".`,
    `// Math.random: base.js (blob ${base.blob}), only where Math exists.`,
  ].join("\n");
  const stub = [
    "var __ep9Benchmarks = [];",
    "function BenchmarkSuite(name, reference, benchmarks) {",
    "  for (var i = 0; i < benchmarks.length; i++) __ep9Benchmarks.push(benchmarks[i]);",
    "}",
    "function Benchmark(name, run, setup, tearDown) {",
    "  this.name = name;",
    "  this.run = run;",
    "  this.Setup = setup ? setup : function() { };",
    "  this.TearDown = tearDown ? tearDown : function() { };",
    "}",
    'if (typeof Math === "object") {',
    mathRandom(base),
    "}",
  ].join("\n");
  const loop = [
    "(function () {",
    `  var n = ${iterations};`,
    "  for (var j = 0; j < __ep9Benchmarks.length; j++) {",
    "    var b = __ep9Benchmarks[j];",
    "    b.Setup();",
    "    for (var i = 0; i < n; i++) b.run();",
    "    b.TearDown();",
    '    print(b.name + " " + n);',
    "  }",
    "})();",
  ].join("\n");
  const before = `${header}\n${PRINT_PRELUDE}${stub}\n// ---- ${src.rel} ----\n`;
  const text = `${before}${src.text}${src.text.endsWith("\n") ? "" : "\n"}// ---- end of ${src.rel} ----\n${loop}\n`;
  return {
    text,
    expect: names.map((x) => `${x} ${iterations}`).join("\n"),
    kernel,
    n: iterations,
    pin: at.pin,
    blob: src.blob,
    source: src.rel,
    harness,
    // Line L of the kernel file is line L + offset of the generated file.
    offset: before.split("\n").length - 1,
    parts: [{ file: src.rel, offset: before.split("\n").length - 1, lines: lineCount(src.text) }],
  };
}

/** The number of lines of a text (a last line without "\n" counts). */
const lineCount = (t) => t.split("\n").length - (t.endsWith("\n") ? 1 : 0);


/** generate() with the real base.js harness (see the comment at the top of this file). */
function generateBase({ at, src, base, names, iterations, kernel }) {
  const header = [
    "// Generated by tools/bench/cross/v8v7.mjs --harness base. Do not edit; generate it again.",
    `// Source: boa-dev/data at ${at.pin}, ${src.rel} (blob ${src.blob}),`,
    `// the V8 benchmark suite v7, with its harness ${base.rel} (blob ${base.blob}), unchanged, as a`,
    "// strict script of its own (an indirect eval). The driver below replaces only",
    `// BenchmarkSuite.prototype.RunSingleBenchmark: run() ${iterations} times per benchmark (not "for 1 s"),`,
    `// then base.js's RunSuites goes on as it is. Prints "<name> ${iterations}" per benchmark.`,
  ].join("\n");
  const exportsText = "\nglobalThis.Benchmark = Benchmark;\nglobalThis.BenchmarkResult = BenchmarkResult;\nglobalThis.BenchmarkSuite = BenchmarkSuite;\n";
  const load = `(0, eval)(${JSON.stringify(base.text + exportsText)});`;
  const driver = [
    "BenchmarkSuite.prototype.RunSingleBenchmark = function (benchmark, data) {",
    "  var start = new Date();",
    `  for (var i = 0; i < ${iterations}; i++) benchmark.run();`,
    "  var elapsed = new Date() - start;",
    `  this.NotifyStep(new BenchmarkResult(benchmark, (elapsed * 1000) / ${iterations}));`,
    "  return null;",
    "};",
    "BenchmarkSuite.RunSuites({",
    `  NotifyStep: function (name) { print(name + " ${iterations}"); },`,
    '  NotifyError: function (name, error) { print("ERROR " + name + " " + error); }',
    "});",
  ].join("\n");
  const before = `${header}\n${PRINT_PRELUDE}${load}\n// ---- ${src.rel} ----\n`;
  const text = `${before}${src.text}${src.text.endsWith("\n") ? "" : "\n"}// ---- end of ${src.rel} ----\n${driver}\n`;
  return {
    text,
    expect: names.map((x) => `${x} ${iterations}`).join("\n"),
    kernel,
    n: iterations,
    pin: at.pin,
    blob: src.blob,
    source: src.rel,
    harness: "base",
    harness_blob: base.blob,
    offset: before.split("\n").length - 1,
    parts: [{ file: src.rel, offset: before.split("\n").length - 1, lines: lineCount(src.text) }],
    // An error inside base.js is reported at "<eval>:<line>": line L of base.js.
    eval_source: base.rel,
  };
}

/**
 * Runs every kernel once on `engine` (N = 1) and returns a Markdown table: "runs (prints ...)", or
 * the first error line, with the generated file's line mapped back to the kernel file.
 */
export function blockers(engine, n = 1, harness = DEFAULT_HARNESS) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-v8v7-"));
  const rows = [`| kernel | result on ${path.basename(engine)} (N = ${n}, harness ${harness}) | source line |`, "|---|---|---|"];
  try {
    for (const k of Object.keys(KERNELS)) {
      const g = generate(k, { n, harness });
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

/**
 * The first error of a failed run of the generated file `file` (a generate() result `g` of this
 * file or of octane.mjs): { message, place ("richards.js:66" or "base.js:152"), line (that source
 * line, trimmed) }. ep9 prints "<file>:<line>:<col>: <message>" first, and "<eval>:<line>:..." for
 * an error in eval code (the v8v7 base harness evals base.js); other engines' forms are kept as
 * they are. With no stderr, a driver "ERROR ..." line on stdout is the error.
 */
export function errorPlace(g, file, r) {
  const out = (r.stdout || "").replace(/\r\n/g, "\n").trim();
  const err = (
    (r.stderr || "").split("\n").find((l) => l.trim()) ??
    out.split("\n").find((l) => l.startsWith("ERROR ")) ??
    `${r.error ? r.error.message : `exit ${r.status}${r.signal ? ` (signal ${r.signal})` : ""}`}, printed ${JSON.stringify(out)}`
  ).trim();
  const m = err.match(/^(.*?):(\d+):(\d+): (.*)$/);
  if (!m) return { message: err, place: null, line: null };
  const at = Number(m[2]);
  if (m[1] === "<eval>" && g.eval_source) {
    // Only the base harness evals, and it evals base.js alone (plus three export lines).
    return { message: m[4], place: `${path.basename(g.eval_source)}:${at}`, line: evalLine(g, at) };
  }
  if (m[1] !== file) return { message: err, place: null, line: null };
  const text = g.text.split("\n")[at - 1]?.trim() ?? null;
  for (const p of g.parts ?? []) {
    if (at > p.offset && at <= p.offset + p.lines) return { message: m[4], place: `${path.basename(p.file)}:${at - p.offset}`, line: text };
  }
  return { message: m[4], place: `generated:${at}`, line: text };
}

/** Line `at` of the text that a base-harness file evals (base.js), from its eval string. */
function evalLine(g, at) {
  const m = g.text.match(/^\(0, eval\)\((".*")\);$/m);
  if (!m) return null;
  return JSON.parse(m[1]).split("\n")[at - 1]?.trim() ?? null;
}

export function parseArgs(argv) {
  const o = { kernel: null, n: undefined, out: null, list: false, blockers: null, harness: DEFAULT_HARNESS };
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
      case "--harness": o.harness = val(); break;
      default: throw new Error(`unknown option ${a}`);
    }
  }
  if (!o.list && !o.kernel && !o.blockers) throw new Error("--kernel NAME, --list or --blockers ENGINE is required");
  if (!HARNESSES.includes(o.harness)) throw new Error(`--harness must be one of ${HARNESSES.join(", ")}`);
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
    console.error(`v8v7.mjs: ${e.message}`);
    process.exit(2);
  }
  if (o.blockers) {
    console.log(blockers(o.blockers, o.n ?? 1, o.harness));
    return;
  }
  if (o.list) {
    for (const [k, v] of Object.entries(KERNELS)) console.log(`${k}\tN = ${v.n}\tbench/bench-v8/${v.file}`);
    return;
  }
  let g;
  try {
    g = generate(o.kernel, { n: o.n, harness: o.harness });
  } catch (e) {
    console.error(`v8v7.mjs: ${e.message}`);
    process.exit(1);
  }
  if (o.out) {
    writeFileSync(o.out, g.text);
    console.error(`v8v7.mjs: wrote ${o.out} (${g.source} at ${g.pin}, N = ${g.n}, harness ${g.harness}; prints ${JSON.stringify(g.expect)})`);
  } else {
    process.stdout.write(g.text);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
