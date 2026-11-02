#!/usr/bin/env node
// The M1 exit-1 check: do all 15 kernels (the 8 v8-v7 kernels and Octane-7) run on ep9, with
// node's output? Each kernel runs once (N = 1) on node (the oracle) and on ep9.
//
//   node tools/bench/cross/kernels.mjs --check target/release/ep9 [--out kernels.json]
//
//   --check PATH       the ep9 binary (required)
//   --node PATH        the oracle (default: the node that runs this script)
//   --harness H        the v8-v7 harness, stub or base (default DEFAULT_HARNESS of v8v7.mjs);
//                      Octane-7 always runs with Octane's base.js (octane.mjs)
//   --n N              run() calls per benchmark (default 1)
//   --not-yet LIST     kernel ids (v8v7/<k>, octane/<k>) that may fail: a role's named NOT-YET
//                      list. They are still run and reported
//   --commit SHA       the commit ep9 was built from (default: git HEAD of this checkout)
//   --timeout-s N      a run that takes longer fails (default 600)
//   --out FILE         the JSON (schema ep9-kernels/v1) for cargo xtask m1-exit --kernels;
//                      "-" prints it on stdout (the table then goes to stderr)
//
// Per kernel: "runs, output equal" when ep9 exits with 0 and prints what node prints, which must
// be the generator's "<name> <N>" lines (else node, or the pin, is wrong: exit 3). Otherwise
// "fails" (with the first error line, its place in base.js or the kernel file, and a named
// blocker when the error says which feature is missing) or "output differs".
//
// Result probes. The "<name> <N>" lines come from the harness, so they show only that run()
// returned. Most kernels check their own result and throw (richards, deltablue, crypto's
// decrypt, raytrace's checkNumber, earley-boyer's rewrite count, splay's tear-down). Two do not:
// v8-v7 regexp throws away every match, and navier-stokes checks nothing (v8-v7) or only at frame
// 15 (Octane), after the N = 1 of this check. For these kernels the checked file has a probe
// between the kernel and the driver (PROBES; timed runs have none): it wraps each benchmark's
// run() and prints "check <name> <i> <hash>" after the i-th call, a 32-bit hash of the results
// so far. node prints the same lines, so "output equal" then means equal results. The hash input
// names each value's type and keeps every value apart (a length before each piece), so a value
// of another type, NaN, an infinity or undefined never hashes as a number or string node gave.
//
// The JSON keeps what m1-exit needs for "15/15 on a fresh tree": the commit, the git tree ids
// of crates, tools/bench, the root Cargo.toml, Cargo.lock and rust-toolchain.toml at HEAD, and the paths among them
// with uncommitted changes (`dirty`; not fresh when it is not empty). `ep9.engine_build` is the
// build line of the checked binary's `--version` ("features <list>; preset <name>", M5-P0,
// MC-w17-5), or null for a binary that prints none.
//
// Exit status: 0 every kernel ran with node's output (kernels on --not-yet may fail); 1 a kernel
// failed; 2 usage error; 3 the oracle or the generator failed (no verdict).

import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { hostInfo } from "../lib/host.mjs";
import { generate as generateOctane, KERNELS as OCTANE_KERNELS } from "./octane.mjs";
import { DEFAULT_HARNESS, errorPlace, generate as generateV8v7, HARNESSES, KERNELS as V8V7_KERNELS, ROOT } from "./v8v7.mjs";

export const SCHEMA = "ep9-kernels/v1";
export const OK = "runs, output equal";

/** The 15 kernels of M1 exit 1, in report order. */
export const KERNEL_IDS = [
  ...Object.keys(V8V7_KERNELS).map((k) => `v8v7/${k}`),
  ...Object.keys(OCTANE_KERNELS).map((k) => `octane/${k}`),
];

/**
 * The paths whose change makes a kernels JSON stale (plan v9 section 9.4 item 1; the root
 * Cargo.toml since M4-P5: its [profile.release] sets lto and panic). m1-exit reads the same list.
 */
export const FRESH_PATHS = ["crates", "tools/bench", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"];

/** Rust target triples by node's platform-arch (the m1-exit column names). */
const TRIPLES = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
  "win32-arm64": "aarch64-pc-windows-msvc",
};

export class UsageError extends Error {}
export class OracleError extends Error {}

export function parseArgs(argv) {
  const o = { check: null, node: process.execPath, harness: DEFAULT_HARNESS, n: 1, notYet: [], commit: null, timeoutS: 600, out: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new UsageError(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case "--check": o.check = path.resolve(val()); break;
      case "--node": o.node = path.resolve(val()); break;
      case "--harness": o.harness = val(); break;
      case "--n": o.n = Number(val()); break;
      case "--not-yet": o.notYet = val().split(",").filter(Boolean); break;
      case "--commit": o.commit = val(); break;
      case "--timeout-s": o.timeoutS = Number(val()); break;
      case "--out": {
        const v = val();
        o.out = v === "-" ? "-" : path.resolve(v);
        break;
      }
      default: throw new UsageError(`unknown option ${a}`);
    }
  }
  if (!o.check) throw new UsageError("--check ENGINE is required");
  if (!HARNESSES.includes(o.harness)) throw new UsageError(`--harness must be one of ${HARNESSES.join(", ")}`);
  if (!Number.isInteger(o.n) || o.n < 1) throw new UsageError("--n must be an integer >= 1");
  if (!(o.timeoutS > 0)) throw new UsageError("--timeout-s must be > 0");
  for (const k of o.notYet) if (!KERNEL_IDS.includes(k)) throw new UsageError(`--not-yet: unknown kernel ${k} (known: ${KERNEL_IDS.join(", ")})`);
  return o;
}

/** The generate() result of a kernel id. */
export function generateKernel(id, { n, harness, root = ROOT }) {
  const [suite, kernel] = id.split("/");
  if (suite === "v8v7") return { suite, kernel, ...generateV8v7(kernel, { n, harness, root }) };
  if (suite === "octane") return { suite, kernel, ...generateOctane(kernel, { n, root }) };
  throw new Error(`unknown kernel ${id}`);
}

/**
 * Result probes of the kernels that check no result of their own (see the top of this file), by
 * kernel name (v8-v7 and Octane alike). `install` runs once before the driver; `before` and
 * `after` run around each run() call. __ep9Mix(x) adds String(x) to the hash.
 */
export const PROBES = {
  // Every frame's density and velocity grids, through the solver's display callback (setup sets
  // a no-op one). A run() that skips solver.update() leaves no frame: "none". A cell is hashed
  // as its value * 1e9 cut to an integer (a 1e-9 grain; printing a whole number needs no
  // shortest-digits formatting), with no 32-bit wrap; a value that is not a finite number (NaN,
  // an infinity, undefined) is hashed by its type and name, never read as 0.
  "navier-stokes": {
    what: "density and velocity of the last frame",
    install: [
      "var __ep9Field = null;",
      "function __ep9Num(v) {",
      "  if (typeof v !== \"number\") return typeof v + \":\" + v;",
      "  var q = v * 1e9;",
      "  if (q - q !== 0) return \"nf:\" + v;",
      "  return String(q - q % 1);",
      "}",
    ].join("\n"),
    before: "__ep9Field = null; solver.setDisplayFunction(function (f) { __ep9Field = f; });",
    after: [
      "if (!__ep9Field) __ep9Mix(\"none\");",
      "else for (var y = 0; y < __ep9Field.height(); y++) for (var x = 0; x < __ep9Field.width(); x++)",
      "  __ep9Mix(__ep9Num(__ep9Field.getDensity(x, y)) + \",\" + __ep9Num(__ep9Field.getXVelocity(x, y)) + \",\" + __ep9Num(__ep9Field.getYVelocity(x, y)));",
    ].join("\n"),
  },
  // The result of every top-level exec, test, replace, match, split and search call (the methods
  // regexp.js calls). A call made inside another wrapped call (replace calls exec, in the spec) is
  // not hashed, so an engine that skips the inner property lookup still hashes the same. The
  // kernel replaces the global RegExp with its BenchmarkSuite, so %RegExp.prototype% comes from a
  // literal. Each element, length and index is hashed with its type ("s" + a string, "u" for
  // undefined, else "<typeof>:<value>"), so a number never hashes as the string of its digits.
  regexp: {
    what: "results of exec, test, replace, match, split, search",
    install: [
      "(function () {",
      "  var depth = 0;",
      "  function enc(x) { return x === undefined ? \"u\" : typeof x === \"string\" ? \"s\" + x : typeof x + \":\" + x; }",
      "  function val(r) {",
      "    if (r === null || typeof r !== \"object\") return __ep9Mix(typeof r + \":\" + r);",
      "    __ep9Mix(\"[\" + enc(r.length) + \"@\" + enc(r.index));",
      "    for (var i = 0; i < r.length; i++) __ep9Mix(enc(r[i]));",
      "  }",
      "  function wrap(o, k) {",
      "    var f = o[k];",
      "    o[k] = function (a, b) {",
      "      if (depth) return f.call(this, a, b);",
      "      depth++;",
      "      var r;",
      "      try { r = f.call(this, a, b); } finally { depth--; }",
      "      __ep9Mix(k);",
      "      val(r);",
      "      return r;",
      "    };",
      "  }",
      "  var R;",
      "  // An engine without RegExp literals fails on the kernel's own line (setup), not here.",
      "  try { R = Object.getPrototypeOf(/a/); } catch (e) { __ep9Mix(\"no RegExp: \" + e); return; }",
      "  wrap(R, \"exec\"); wrap(R, \"test\");",
      "  wrap(String.prototype, \"replace\"); wrap(String.prototype, \"match\");",
      "  wrap(String.prototype, \"split\"); wrap(String.prototype, \"search\");",
      "})();",
    ].join("\n"),
    before: "",
    after: "",
  },
};

/** The probe line prefixes a probed kernel prints: "check <name> <i>" per benchmark and call, in order. */
export function probeKeys(g) {
  return g.expect.split("\n").flatMap((l) => {
    const name = l.slice(0, l.lastIndexOf(" "));
    return Array.from({ length: g.n }, (_, i) => `check ${name} ${i + 1}`);
  });
}

/**
 * The generate() result `g` with the kernel's probe (PROBES) between the kernel and the driver,
 * or `g` itself for a kernel with no probe. The base.js and kernel lines keep their offsets.
 */
export function withProbe(g) {
  const p = PROBES[g.kernel];
  if (!p) return { ...g, probe: null };
  const end = `// ---- end of ${g.source} ----\n`;
  const at = g.text.indexOf(end);
  if (at < 0 || g.text.indexOf(end, at + 1) >= 0) throw new Error(`${g.source}: no single "${end.trim()}" line to put the probe after`);
  const block = [
    "// ---- kernels.mjs result probe (check runs only) ----",
    "var __ep9Hash = 0;",
    "function __ep9Mix(x) {",
    "  var s = String(x), h = (__ep9Hash * 31 + s.length) | 0;",
    "  for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;",
    "  __ep9Hash = (h * 31 + 59) | 0;",
    "}",
    p.install,
    "(function () {",
    '  var list = typeof __ep9Benchmarks !== "undefined" ? __ep9Benchmarks : [];',
    "  if (!list.length) {",
    "    for (var i = 0; i < BenchmarkSuite.suites.length; i++)",
    "      for (var j = 0; j < BenchmarkSuite.suites[i].benchmarks.length; j++) list.push(BenchmarkSuite.suites[i].benchmarks[j]);",
    "  }",
    "  for (var k = 0; k < list.length; k++) (function (b) {",
    "    var run = b.run, calls = 0;",
    "    b.run = function () {",
    `      ${p.before}`,
    "      run.call(this);",
    `      ${p.after.split("\n").join("\n      ")}`,
    '      print("check " + b.name + " " + ++calls + " " + __ep9Hash);',
    "    };",
    "  })(list[k]);",
    "})();",
    "// ---- end of the result probe ----",
    "",
  ].join("\n");
  const cut = at + end.length;
  return { ...g, text: g.text.slice(0, cut) + block + g.text.slice(cut), probe: p.what };
}

const clean = (s) => (s ?? "").replace(/\r\n/g, "\n").trim();

function runOnce(bin, file, timeoutS) {
  const r = spawnSync(bin, [file], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 1 << 26, timeout: timeoutS * 1000 });
  return { status: r.status, signal: r.signal, error: r.error ?? null, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
}

/**
 * The feature an error names, or null. `mathNames` = the own property names of the engine's
 * Math (null when unknown): a "not a function" on a line that calls Math.<f> names the Math.<f>
 * calls of that line that the engine lacks.
 */
export function blockerOf(message, line, mathNames) {
  if (/\*\* with a result that is not an exact integer|pow core/.test(message)) return "pow core (Math.pow and ** with a non-integer result; H36)";
  if (/RegExp/.test(message)) return "RegExp";
  if (/is not a function/.test(message) && line) {
    const calls = [...new Set([...line.matchAll(/\bMath\.([A-Za-z_$][\w$]*)\s*\(/g)].map((m) => m[1]))];
    const missing = mathNames ? calls.filter((f) => !mathNames.includes(f)) : calls;
    if (missing.length) return missing.map((f) => `Math.${f}`).join(", ");
  }
  return null;
}

/** Own property names of the engine's Math, or null if the probe fails. */
function mathNamesOf(bin, dir, timeoutS) {
  const f = path.join(dir, "math-names.js");
  writeFileSync(f, 'if (typeof print !== "function") globalThis.print = (...a) => console.log(...a);\nprint(Object.getOwnPropertyNames(Math).join(" "));\n');
  const r = runOnce(bin, f, timeoutS);
  return r.status === 0 ? clean(r.stdout).split(/\s+/) : null;
}

/** git output (not trimmed: a porcelain line starts with a space), or null on an error. */
function git(args) {
  try {
    return execFileSync("git", ["-C", ROOT, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return null;
  }
}

/** HEAD, the tree ids of FRESH_PATHS at HEAD, and the FRESH_PATHS entries with uncommitted changes. */
export function treeState() {
  const head = git(["rev-parse", "HEAD"])?.trim() ?? null;
  const tree = Object.fromEntries(FRESH_PATHS.map((p) => [p, head ? (git(["rev-parse", `HEAD:${p}`])?.trim() ?? null) : null]));
  const status = git(["status", "--porcelain", "--untracked-files=all", "--", ...FRESH_PATHS]);
  const dirty = status === null ? null : status.split("\n").filter(Boolean).map((l) => l.slice(3));
  return { head, tree, dirty };
}

/**
 * Runs the 15 kernels. Returns the ep9-kernels/v1 object. Throws OracleError when node does not
 * print the generator's expected output (the check would mean nothing).
 */
export function check(o, deps = {}) {
  const log = deps.log ?? ((s) => process.stderr.write(`${s}\n`));
  const started = new Date().toISOString();
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-kernels-"));
  try {
    const version = (bin) => clean(runVersion(bin));
    const mathNames = mathNamesOf(o.check, dir, o.timeoutS);
    const rows = [];
    for (const id of KERNEL_IDS) {
      let g;
      try {
        g = withProbe(generateKernel(id, { n: o.n, harness: o.harness }));
      } catch (e) {
        throw new OracleError(`${id}: ${e.message}`);
      }
      const file = path.join(dir, `${id.replace("/", "-")}.js`);
      writeFileSync(file, g.text);
      const node = runOnce(o.node, file, o.timeoutS);
      const nodeOut = clean(node.stdout);
      // node must print the generator's lines and, for a probed kernel, one probe line per run() call.
      const nodeLines = nodeOut.split("\n");
      const nodeChecks = nodeLines.filter((l) => l.startsWith("check ")).map((l) => l.replace(/ -?\d+$/, ""));
      const wantChecks = g.probe ? probeKeys(g) : [];
      const probeOk = nodeChecks.length === wantChecks.length && nodeChecks.every((l, i) => l === wantChecks[i]);
      if (node.status !== 0 || nodeLines.filter((l) => !l.startsWith("check ")).join("\n") !== g.expect || !probeOk) {
        const why = node.error ? node.error.message : node.status !== 0 ? `exit ${node.status}: ${clean(node.stderr).split("\n")[0]}` : `printed ${JSON.stringify(nodeOut)}`;
        throw new OracleError(`${id} on the oracle ${o.node}: ${why}; want ${JSON.stringify(g.expect)}${g.probe ? ` and the probe lines ${JSON.stringify(wantChecks.join("\n"))}` : ""}`);
      }
      const r = runOnce(o.check, file, o.timeoutS);
      const out = clean(r.stdout);
      const row = {
        id,
        suite: g.suite,
        kernel: g.kernel,
        n: g.n,
        harness: g.harness,
        source: g.source,
        pin: g.pin,
        blob: g.blob,
        expect: g.expect,
        probe: g.probe,
        oracle_output: nodeOut,
        output: out,
        exit: r.status,
        signal: r.signal,
        not_yet: o.notYet.includes(id),
      };
      if (r.status === 0 && !r.error && out === nodeOut) {
        Object.assign(row, { status: OK, ok: true, error: null, place: null, line: null, blocker: null });
      } else {
        const e = errorPlace(g, file, r);
        const differs = r.status === 0 && !r.error && !out.split("\n").some((l) => l.startsWith("ERROR "));
        Object.assign(row, {
          status: differs ? "output differs" : "fails",
          ok: false,
          error: differs ? `printed ${JSON.stringify(out)}, node printed ${JSON.stringify(nodeOut)}` : e.message,
          place: differs ? null : e.place,
          line: differs ? null : e.line,
          blocker: differs ? null : blockerOf(e.message, e.line, mathNames),
        });
      }
      log(`${id}: ${row.status}${row.blocker ? ` (blocker: ${row.blocker})` : ""}`);
      rows.push(row);
    }
    const st = treeState();
    const failing = rows.filter((r) => !r.ok).map((r) => r.id);
    const host = hostInfo();
    return {
      schema: SCHEMA,
      tool: "tools/bench/cross/kernels.mjs",
      started,
      finished: new Date().toISOString(),
      commit: o.commit ?? st.head,
      head: st.head,
      tree: st.tree,
      dirty: st.dirty,
      host: { ...host, target: TRIPLES[host.platform] ?? null },
      ep9: { bin: path.basename(o.check), ...splitVersion(runVersion(o.check)), sha256: sha256File(o.check) },
      oracle: { engine: "node", bin: path.basename(o.node), version: version(o.node) },
      n: o.n,
      harness: { v8v7: o.harness, octane: rows.find((r) => r.suite === "octane")?.harness ?? null },
      pins: {
        "bench-v8v7": rows.find((r) => r.suite === "v8v7")?.pin ?? null,
        "bench-octane": rows.find((r) => r.suite === "octane")?.pin ?? null,
      },
      math: mathNames,
      not_yet: o.notYet,
      kernels: rows,
      summary: {
        total: rows.length,
        ok: rows.length - failing.length,
        failing,
        failing_not_named: failing.filter((id) => !o.notYet.includes(id)),
        all_ok: failing.length === 0,
      },
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/**
 * An ep9 `--version` text: its first line is the version, and its line that starts with
 * "features " names the build ("features <list>; preset <name>", M5-P0, MC-w17-5). A binary
 * from before M5-P0 prints no such line: `engine_build` is null.
 */
export function splitVersion(text) {
  const lines = clean(text).split("\n").map((l) => l.trim());
  return { version: lines[0], engine_build: lines.find((l) => l.startsWith("features ")) ?? null };
}

function runVersion(bin) {
  const r = spawnSync(bin, ["--version"], { encoding: "utf8" });
  return r.status === 0 ? r.stdout : `unknown (--version: ${r.error?.message ?? `exit ${r.status}`})`;
}

function sha256File(f) {
  try {
    return createHash("sha256").update(readFileSync(f)).digest("hex");
  } catch {
    return null;
  }
}

/** The Markdown table of a check result. */
export function renderTable(res) {
  const cell = (x) => (x ?? "").replaceAll("|", "\\|").replaceAll("`", "'").replaceAll("\n", " / ");
  const L = [];
  L.push(`Kernels on ${res.ep9.bin} (${res.ep9.version}${res.ep9.engine_build ? `; ${res.ep9.engine_build}` : ""}), N = ${res.n}, v8-v7 harness ${res.harness.v8v7}; oracle node ${res.oracle.version}; commit ${(res.commit ?? "unknown").slice(0, 12)}${res.dirty?.length ? ` + uncommitted changes in ${res.dirty.join(", ")}` : ""}.`);
  L.push("");
  L.push("| kernel | result | blocker | place | source line |");
  L.push("|---|---|---|---|---|");
  for (const r of res.kernels) {
    const result = r.ok ? `${r.status} (prints ${JSON.stringify(r.output)})` : `${r.status}${r.not_yet ? " (named NOT-YET)" : ""}: ${r.error}`;
    L.push(`| ${r.id} | ${cell(result)} | ${cell(r.blocker ?? (r.ok ? "" : "-"))} | ${r.place ?? ""} | ${r.line ? `\`${cell(r.line.slice(0, 80))}\`` : ""} |`);
  }
  L.push("");
  const s = res.summary;
  const bySuite = (suite) => `${res.kernels.filter((r) => r.suite === suite && r.ok).length}/${res.kernels.filter((r) => r.suite === suite).length}`;
  L.push(`${s.ok}/${s.total} kernels run with node's output (v8-v7 ${bySuite("v8v7")}, Octane-7 ${bySuite("octane")}).${s.failing.length ? ` Failing: ${s.failing.join(", ")}${s.failing_not_named.length < s.failing.length ? ` (named NOT-YET: ${s.failing.filter((x) => !s.failing_not_named.includes(x)).join(", ")})` : ""}.` : ""}`);
  return `${L.join("\n")}\n`;
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
    console.error(`kernels.mjs: ${e.message}`);
    process.exit(2);
  }
  let res;
  try {
    res = check(o);
  } catch (e) {
    if (e instanceof OracleError) {
      console.error(`kernels.mjs: no verdict: ${e.message}`);
      process.exit(3);
    }
    throw e;
  }
  const table = renderTable(res);
  const json = `${JSON.stringify(res, null, 1)}\n`;
  if (o.out === "-") {
    process.stderr.write(table);
    process.stdout.write(json);
  } else {
    if (o.out) writeFileSync(o.out, json);
    process.stdout.write(table);
  }
  process.exit(res.summary.failing_not_named.length ? 1 : 0);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
