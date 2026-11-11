// node --test .github/scripts/bench-set.test.mjs
// The bench mode script (bench-set.mjs) with a fake valgrind (tools/bench/cross/testdata/
// fake-valgrind.mjs) and fake engines (testdata/fake-engine.sh, one symlink per side): no
// valgrind, no ep9 build and no bench checkout needed. ci.yml runs it in the bench-set job.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { BENCH_SET, compare, groupOf, LIMIT_PCT, parseArgs, parseCgLog, parseCgOut, renderMarkdown, SCHEMA, T0, topFunctions } from "./bench-set.mjs";
import { materialize, OCTANE_7, TS_WORKLOAD, V8V7_7 } from "../../tools/bench/cross/workloads.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT = path.join(DIR, "bench-set.mjs");
const TESTDATA = path.resolve(DIR, "../../tools/bench/cross/testdata");
const ROOT = path.resolve(DIR, "../..");
const havePins = existsSync(path.join(ROOT, "third_party/bench-v8v7")) && existsSync(path.join(ROOT, "third_party/bench-octane"));
const BASE = "1".repeat(40);
const HEAD = "2".repeat(40);

// ---- the delta and the rule-13 verdict --------------------------------------------------------

test("the +0.5% floor: exactly +0.5% passes, one instruction more fails, faster always passes", () => {
  assert.equal(LIMIT_PCT, 0.5);
  assert.deepEqual(compare(1_000_000_000, 1_005_000_000), { delta_pct: 0.5, verdict: "pass" });
  assert.deepEqual(compare(1_000_000_000, 1_005_000_001), { delta_pct: 0.5, verdict: "FAIL" });
  assert.deepEqual(compare(1_000_000_000, 1_000_000_000), { delta_pct: 0, verdict: "pass" });
  assert.deepEqual(compare(1_000_000_000, 900_000_000), { delta_pct: -10, verdict: "pass" });
  assert.deepEqual(compare(6_003_588_538, 6_033_606_480), { delta_pct: 0.5, verdict: "pass" });
  assert.deepEqual(compare(6_003_588_538, 6_033_606_481), { delta_pct: 0.5, verdict: "FAIL" });
  // A float compare would get this one wrong: (1.005 - 1) * 100 is 0.49999999999998934.
  assert.equal(compare(200, 201).verdict, "pass");
  assert.equal(compare(200, 202).verdict, "FAIL");
  assert.deepEqual(compare(1000, 1012), { delta_pct: 1.2, verdict: "FAIL" });
  assert.deepEqual(compare(1000, 1012, 1.2), { delta_pct: 1.2, verdict: "pass" });
  assert.deepEqual(compare(1000, 1001, 0), { delta_pct: 0.1, verdict: "FAIL" });
  for (const [b, h] of [[null, 5], [5, null], [0, 5], [1.5, 2]]) assert.deepEqual(compare(b, h), { delta_pct: null, verdict: "no count" });
});

test("a head Ir of 0 or less is no count, as a base of 0 or less (JIT review D1)", () => {
  // A run that cachegrind did not trace must not read "pass" at -100%.
  for (const h of [0, -1]) assert.deepEqual(compare(1_000_000_000, h), { delta_pct: null, verdict: "no count" });
  assert.deepEqual(compare(1000, 1), { delta_pct: -99.9, verdict: "pass" });
});

test("the default set is t0-*, the 7 v8-v7 kernels and Octane-7 (rule 13); groups", () => {
  assert.deepEqual(BENCH_SET, [...T0, ...V8V7_7, ...OCTANE_7]);
  assert.equal(BENCH_SET.length, 19);
  assert.ok(!BENCH_SET.includes("regexp") && !BENCH_SET.includes("ci-v0"));
  assert.deepEqual(["t0-gen", "splay", "octane-splay"].map(groupOf), ["t0", "v8v7", "octane"]);
});

test("the default set: every workload file is written, with its expected output", { skip: !havePins && "third_party/bench-v8v7 or bench-octane not synced" }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-bench-set-files-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const files = materialize(BENCH_SET, dir);
  for (const w of BENCH_SET) {
    assert.ok(existsSync(files[w].file), w);
    assert.ok(files[w].expect.length > 0, w);
  }
  assert.equal(files["octane-splay"].harness, "octane-base");
  assert.equal(files.richards.harness, "base");
});

// ---- the cachegrind files --------------------------------------------------------------------

test("cachegrind log totals: Ir alone, and the branch counts of --branch-sim=yes", () => {
  // The valgrind 3.24 form (the S0 box log of raytrace).
  const log = [
    "==4201== Cachegrind, a high-precision tracing profiler",
    "==4201== ",
    "==4201== I refs:        5,692,732,770",
    "==4201== ",
    "==4201== Branches:        928,142,840  (761,672,316 cond + 166,470,524 ind)",
    "==4201== Mispredicts:      94,750,118  ( 24,166,506 cond +  70,583,612 ind)",
    "==4201== Mispred rate:           10.2% (        3.2%     +        42.4%   )",
  ].join("\n");
  assert.deepEqual(parseCgLog(log), { ir: 5692732770, bc: 761672316, bcm: 24166506, bi: 166470524, bim: 70583612 });
  // The older "I   refs:" spacing; no branch-sim.
  assert.deepEqual(parseCgLog("==1== I   refs:      1,234,567\n"), { ir: 1234567, bc: null, bcm: null, bi: null, bim: null });
  assert.equal(parseCgLog("==1== error\n").ir, null);
});

test("cachegrind out file: summed per function name over source files; the summary check", () => {
  const out = [
    "cmd: ep9 k.js",
    "events: Ir Bc Bcm Bi Bim",
    "fl=a.rs",
    "fn=f",
    "1 10 2 1 0 0",
    "2 5 0 0 1 1",
    "fl=b.rs",
    "fn=f",
    "3 7 1 0 0 0",
    "fn=(1) g",
    "4 3 0 0 2 2",
    "fn=(1)",
    "5 1 0 0 0 0",
    "summary: 26 3 1 3 3",
  ].join("\n");
  const cg = parseCgOut(out);
  assert.deepEqual(cg.events, ["Ir", "Bc", "Bcm", "Bi", "Bim"]);
  assert.deepEqual([...cg.fns], [["f", [22, 3, 1, 1, 1]], ["g", [4, 0, 0, 2, 2]]]);
  assert.equal(cg.rowsMatch, true);
  assert.equal(parseCgOut(out.replace("summary: 26", "summary: 27")).rowsMatch, false);
  assert.throws(() => parseCgOut("fn=f\n1 2\n"), /no events line/);
});

test("function rows: the top by Ir and the top by Ir change, largest change first", () => {
  const cg = (rows) => ({ events: ["Ir"], fns: new Map(rows.map(([n, ir]) => [n, [ir]])) });
  const base = cg([["big", 1000], ["moved", 100], ["gone", 50], ["same", 10]]);
  const head = cg([["big", 1001], ["moved", 160], ["new", 40], ["same", 10]]);
  const rows = topFunctions(base, head, 2);
  assert.deepEqual(rows.map((r) => [r.fn, r.base.Ir, r.head.Ir, r.delta]), [
    ["moved", 100, 160, 60],
    ["gone", 50, 0, -50],
    ["big", 1000, 1001, 1],
  ]);
  assert.deepEqual(topFunctions(base, head, 0), []);
  assert.deepEqual(topFunctions(undefined, head, 5), []);
});

test("function rows: a --functions name is always a row, beside the top cut (rule 81 _cold rows)", () => {
  const cg = (rows) => ({ events: ["Ir"], fns: new Map(rows.map(([n, ir]) => [n, [ir]])) });
  const base = cg([["big", 1000], ["moved", 100], ["ep9h__sib__tail__SetNamedProperty_cold", 30], ["ep9h__sib__tail__DefineNamedOwnProperty_cold", 5]]);
  const head = cg([["big", 1001], ["moved", 160], ["ep9h__sib__tail__SetNamedProperty_cold", 45], ["ep9h__sib__tail__DefineNamedOwnProperty_cold", 7]]);
  const named = ["SetNamedProperty_cold", "DefineNamedOwnProperty_cold", "not_there"];
  assert.deepEqual(topFunctions(base, head, 1).map((r) => r.fn), ["moved", "big"]);
  const rows = topFunctions(base, head, 1, named);
  assert.deepEqual(rows.map((r) => [r.fn, r.delta, r.named ?? false]), [
    ["moved", 60, false],
    ["ep9h__sib__tail__SetNamedProperty_cold", 15, true],
    ["ep9h__sib__tail__DefineNamedOwnProperty_cold", 2, true],
    ["big", 1, false],
  ]);
  // --top 0 with names: the named rows only.
  assert.deepEqual(topFunctions(base, head, 0, named).map((r) => r.fn), ["ep9h__sib__tail__SetNamedProperty_cold", "ep9h__sib__tail__DefineNamedOwnProperty_cold"]);
});

test("options: required paths, full commit ids, workloads and the limit", () => {
  const req = ["--base-ep9", "b", "--head-ep9", "h", "--out", "o.json"];
  const o = parseArgs(req);
  assert.deepEqual(o.workloads, BENCH_SET);
  assert.equal(o.limitPct, 0.5);
  assert.equal(o.branchSim, false);
  assert.equal(o.hashSeed, "1");
  assert.deepEqual(parseArgs([...req, "--workloads", "t0-loop,octane-7"]).workloads, ["t0-loop", ...OCTANE_7]);
  assert.throws(() => parseArgs(["--base-ep9", "b", "--out", "o"]), /required/);
  assert.throws(() => parseArgs([...req, "--base", "abc123"]), /40 hex digits/);
  assert.throws(() => parseArgs([...req, "--head", "main"]), /40 hex digits/);
  assert.throws(() => parseArgs([...req, "--workloads", "nope"]), /unknown workload nope/);
  assert.throws(() => parseArgs([...req, "--workloads", "t0-loop,t0-loop"]), /duplicate/);
  assert.throws(() => parseArgs([...req, "--limit-pct", "0.0005"]), /3 decimals/);
  assert.throws(() => parseArgs([...req, "--limit-pct", "-1"]), /3 decimals/);
  assert.throws(() => parseArgs([...req, "--frob"]), /unknown option/);
});

test("options: --workloads takes octane-typescript (the TS row, group octane); a bad list is refused", () => {
  const req = ["--base-ep9", "b", "--head-ep9", "h", "--out", "o.json"];
  assert.equal(TS_WORKLOAD, "octane-typescript");
  assert.deepEqual(parseArgs([...req, "--workloads", "octane-typescript,deltablue"]).workloads, ["octane-typescript", "deltablue"]);
  assert.equal(groupOf("octane-typescript"), "octane");
  // An empty name or a character outside a name is a usage error, not a silent skip.
  for (const bad of ["", "deltablue,", ",deltablue", "a,,b", "deltablue;id", "delta blue", "$(id)"]) {
    assert.throws(() => parseArgs([...req, "--workloads", bad]), /comma list of names/, bad);
  }
  assert.deepEqual(parseArgs([...req, "--functions", "SetNamedProperty_cold,DefineNamedOwnProperty_cold"]).functions, ["SetNamedProperty_cold", "DefineNamedOwnProperty_cold"]);
  assert.throws(() => parseArgs([...req, "--functions", "a,,b"]), /comma list of names/);
  // --list needs no binary and no --out.
  assert.deepEqual(parseArgs(["--list", "--workloads", "ts"]).workloads, ["octane-typescript"]);
});

test("CLI --list: the rows of --workloads, one per line; an unknown name exits 2 (the bench workloads step)", () => {
  const list = (...a) => spawnSync(process.execPath, [SCRIPT, "--list", ...a], { encoding: "utf8" });
  let r = list("--workloads", "octane-typescript,deltablue");
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout, "octane-typescript\ndeltablue\n");
  r = list();
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split("\n"), BENCH_SET);
  r = list("--workloads", "octane-typescript,nope");
  assert.equal(r.status, 2);
  assert.match(r.stderr, /unknown workload nope/);
});

test("the TS workload file: 3 files, Octane's base.js, its expected output", { skip: !havePins && "third_party/bench-octane not synced" }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-bench-set-ts-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const files = materialize([TS_WORKLOAD], dir);
  const ts = files[TS_WORKLOAD];
  assert.ok(existsSync(ts.file));
  assert.equal(ts.harness, "octane-base");
  assert.equal(ts.blobs.length, 3);
  assert.ok(ts.expect.length > 0);
});

// ---- ci.yml: the bench_workloads dispatch input -------------------------------------------------

const CI = readFileSync(path.join(ROOT, ".github/workflows/ci.yml"), "utf8");

/** The `run: |` script of the step `name` of the bench-set job, as bash text. */
function stepScript(name) {
  const lines = CI.slice(CI.indexOf("\n  bench-set:\n")).split("\n");
  const at = lines.indexOf(`      - name: ${name}`);
  assert.ok(at > 0, `step ${name}`);
  let i = at + 1;
  while (!/^ {8}run: /.test(lines[i])) i++;
  if (lines[i] !== "        run: |") return lines[i].slice("        run: ".length);
  const body = [];
  for (i++; i < lines.length && (lines[i] === "" || lines[i].startsWith("          ")); i++) body.push(lines[i].slice(10));
  return body.join("\n");
}

function runStep(t, name, env) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-bench-set-step-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const out = path.join(dir, "out");
  const sum = path.join(dir, "summary");
  writeFileSync(out, "");
  // GitHub runs a `run:` script as `bash -e {file}`.
  const r = spawnSync("bash", ["-e", "-c", stepScript(name)], {
    cwd: ROOT,
    encoding: "utf8",
    env: { PATH: process.env.PATH, GITHUB_OUTPUT: out, GITHUB_STEP_SUMMARY: sum, ...env },
  });
  return { ...r, output: readFileSync(out, "utf8") };
}

test("ci.yml: bench_workloads is a dispatch input, in the concurrency group, and never in a run script", () => {
  assert.match(CI, /\n {6}bench_workloads:\n {8}description: "[^"\n]+"\n {8}type: string\n/);
  const group = CI.split("\n").find((l) => l.startsWith("  group: ci-"));
  assert.match(group, /format\('bench-\{0\}-\{1\}-\{2\}', inputs\.bench_base, inputs\.bench_head \|\| github\.sha, inputs\.bench_workloads\)/);
  // The input reaches the job through env only: no ${{ }} inside a run script of bench-set.
  for (const name of ["bench mode", "bench workloads", "count"]) assert.doesNotMatch(stepScript(name), /\$\{\{/, name);
  assert.match(CI, /\n {10}WORKLOADS: \$\{\{ inputs\.bench_workloads \}\}\n/);
  // The count step passes the list and the 2 store cold bodies (rule 81 rows).
  const count = stepScript("count");
  assert.match(count, /"\$\{wl\[@\]\}"/);
  assert.match(count, /--functions "\$FUNCTIONS"/);
  assert.match(CI, /\n {10}FUNCTIONS: SetNamedProperty_cold,DefineNamedOwnProperty_cold\n/);
});

test("ci.yml bench mode step: bench_workloads is checked before any build (exit 2), then passed on", (t) => {
  const on = { LEG: "linux-x64", BASE: BASE, HEAD: "", SHA: HEAD };
  let r = runStep(t, "bench mode", { ...on, WORKLOADS: "octane-typescript,deltablue" });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.output, /^on=true\nbase=1{40}\nhead=2{40}\nworkloads=octane-typescript,deltablue\n$/);
  r = runStep(t, "bench mode", { ...on, WORKLOADS: "" });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.output, /\nworkloads=\n$/);
  // Anything but a comma list of names: exit 2, and no on=true.
  for (const bad of ["deltablue,", "a,,b", "deltablue;id", "delta blue", "$(id)", "`id`", "deltablue\non=true", "Deltablue", "../x"]) {
    r = runStep(t, "bench mode", { ...on, WORKLOADS: bad });
    assert.equal(r.status, 2, `${JSON.stringify(bad)}: ${r.stdout}`);
    assert.equal(r.output, "", bad);
    assert.match(r.stdout, /::error::bench mode: bench_workloads must be a comma list/);
  }
  // The off leg (no bench_base): an input with no bench_base is refused; empty runs as before.
  r = runStep(t, "bench mode", { LEG: "off", BASE: "", HEAD: "", SHA: HEAD, WORKLOADS: "deltablue" });
  assert.equal(r.status, 2);
  assert.match(r.stdout, /bench_workloads needs bench_base/);
  r = runStep(t, "bench mode", { LEG: "off", BASE: "", HEAD: "", SHA: HEAD, WORKLOADS: "" });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.output, "on=false\n");
});

test("ci.yml bench workloads step: an unknown name exits 2; octane-typescript is a row", (t) => {
  let r = runStep(t, "bench workloads", { WORKLOADS: "octane-typescript,deltablue" });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout, "octane-typescript\ndeltablue\n");
  r = runStep(t, "bench workloads", { WORKLOADS: "" });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout.trim().split("\n").length, 19);
  r = runStep(t, "bench workloads", { WORKLOADS: "octane-typescrpt" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /unknown workload octane-typescrpt/);
});

// ---- the CLI with the fake valgrind ------------------------------------------------------------

function fakeSetup(t) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-bench-set-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const n of ["ep9-base", "ep9-head"]) symlinkSync(path.join(TESTDATA, "fake-engine.sh"), path.join(dir, n));
  const out = path.join(dir, "out.json");
  const md = path.join(dir, "out.md");
  const keep = path.join(dir, "cg");
  // Each fake engine prints the build line of its commit, built clean (the CI form), unless the
  // test sets FAKE_BUILD_ep9_base or FAKE_BUILD_ep9_head ("" for no build line); shas: false
  // runs with no --base and --head.
  const run = (args = [], env = {}, { shas = true } = {}) => {
    const argv = [
      SCRIPT,
      "--base-ep9", path.join(dir, "ep9-base"),
      "--head-ep9", path.join(dir, "ep9-head"),
      ...(shas ? ["--base", BASE, "--head", HEAD] : []),
      "--target", "test-x64",
      "--valgrind", path.join(TESTDATA, "fake-valgrind.mjs"),
      "--workloads", "t0-loop,t0-call",
      "--jobs", "2",
      "--keep", keep,
      "--out", out,
      "--md", md,
      ...args,
    ];
    const builds = { FAKE_BUILD_ep9_base: `commit ${BASE} clean`, FAKE_BUILD_ep9_head: `commit ${HEAD} clean` };
    const r = spawnSync(process.execPath, argv, { encoding: "utf8", env: { ...process.env, ...builds, ...env } });
    const res = existsSync(out) ? JSON.parse(readFileSync(out, "utf8")) : null;
    return { ...r, res, md: existsSync(md) ? readFileSync(md, "utf8") : null };
  };
  return { dir, run, keep };
}

const counts = (rows) => JSON.stringify(Object.fromEntries(rows.map(([k, ir, bim]) => [k, { ir, ...(bim ? { bim } : {}) }])));

test("CLI: a head +0.6% on one row fails; base and head Ir and the delta per row (the JSON)", (t) => {
  const f = fakeSetup(t);
  const cgLog = path.join(f.dir, "cg.log");
  const r = f.run([], { FAKE_CG: counts([["ep9-base:t0-call", 2_000_000_000], ["ep9-head:t0-call", 2_012_000_000]]), FAKE_CG_LOG: cgLog });
  assert.equal(r.status, 1, r.stderr);
  const res = r.res;
  assert.equal(res.schema, SCHEMA);
  assert.equal(res.verdict, "FAIL");
  assert.equal(res.target, "test-x64");
  assert.equal(res.counter, "cachegrind-i-refs");
  assert.deepEqual([res.base.commit, res.head.commit], [BASE, HEAD]);
  assert.equal(res.valgrind, "valgrind-fake");
  assert.deepEqual(res.rows.map((x) => x.name), ["t0-loop", "t0-call"]);
  const [loop, call] = res.rows;
  // The fake's default count: 1e9 + 1000 x the workload name length, the same on both sides.
  assert.deepEqual([loop.base.ir, loop.head.ir, loop.delta_pct, loop.verdict], [1_000_007_000, 1_000_007_000, 0, "pass"]);
  assert.deepEqual([call.base.ir, call.head.ir, call.delta_pct, call.verdict], [2_000_000_000, 2_012_000_000, 0.6, "FAIL"]);
  for (const row of res.rows) {
    assert.equal(row.group, "t0");
    assert.equal(row.base.status, "ok");
    assert.equal(row.head.status, "ok");
    assert.match(row.sha256, /^[0-9a-f]{64}$/);
    assert.equal(row.out_rows_match, true);
    assert.equal(row.base.bim, null, "no branch counts without --branch-sim");
  }
  assert.deepEqual(res.worst, { t0: { name: "t0-call", delta_pct: 0.6 } });
  // Function rows: the fake's hot function is in 2 source files, summed into one name.
  assert.deepEqual(call.functions.map((x) => [x.fn, x.base.Ir, x.head.Ir, x.delta]), [
    ["ep9_vm::interp::ops::hot", 1_400_000_000, 1_408_400_000, 8_400_000],
    ["t0-call::cold", 600_000_000, 603_600_000, 3_600_000],
  ]);
  // Every run: valgrind with the seed, no GC helpers and no branch-sim; one per side and workload.
  const runs = readFileSync(cgLog, "utf8").trim().split("\n").sort();
  assert.deepEqual(runs, [
    "ep9-base t0-call seed=1 helpers=0 branch=no",
    "ep9-base t0-loop seed=1 helpers=0 branch=no",
    "ep9-head t0-call seed=1 helpers=0 branch=no",
    "ep9-head t0-loop seed=1 helpers=0 branch=no",
  ]);
  for (const s of ["base", "head"]) for (const w of ["t0-loop", "t0-call"]) for (const e of ["out", "log"]) assert.ok(existsSync(path.join(f.keep, `${s}.${w}.${e}`)), `${s}.${w}.${e}`);
  // The table: one line per row, the FAIL in bold.
  assert.match(r.md, /\*\*Verdict: FAIL\*\* \(2 of 2 rows counted; worst t0: t0-call \+0\.600%\)/);
  assert.match(r.md, /\| t0-call \| 2,000,000,000 \| 2,012,000,000 \| \+0\.600% \| \*\*FAIL\*\* \|/);
  assert.match(r.md, /\| t0-loop \| 1,000,007,000 \| 1,000,007,000 \| 0\.000% \| pass \|/);
  assert.equal(r.stdout, r.md);
});

test("CLI: --functions keeps a named function beside --top; the table lists it", (t) => {
  const f = fakeSetup(t);
  let r = f.run(["--workloads", "t0-call", "--top", "1"]);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.res.rows[0].functions.map((x) => x.fn), ["ep9_vm::interp::ops::hot"]);
  assert.deepEqual(r.res.functions_named, []);
  r = f.run(["--workloads", "t0-call", "--top", "1", "--functions", "::cold"]);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.res.functions_named, ["::cold"]);
  assert.deepEqual(r.res.rows[0].functions.map((x) => [x.fn, x.named ?? false]), [["ep9_vm::interp::ops::hot", false], ["t0-call::cold", true]]);
  assert.match(r.md, /\| `t0-call::cold` \|/);
});

test("markdown: a --functions row below the 10 largest changes is still in the table", () => {
  const side = (ir) => ({ status: "ok", error: null, ir, bc: null, bcm: null, bi: null, bim: null });
  const fns = Array.from({ length: 12 }, (_, i) => ({ fn: `f${i}`, base: { Ir: 100 }, head: { Ir: 200 - i }, delta: 100 - i }));
  fns.push({ fn: "ep9h__sib__tail__DefineNamedOwnProperty_cold", base: { Ir: 5 }, head: { Ir: 7 }, delta: 2, named: true });
  const res = {
    target: "linux-x64", branch_sim: false, hash_seed: "1", gc_helpers: "0", limit_pct: 0.5, valgrind: "v", host: { cpu: "c" },
    base: { commit: BASE }, head: { commit: HEAD }, notes: [], verdict: "PASS", worst: {},
    rows: [{ name: "deltablue", base: side(1000), head: side(1000), delta_pct: 0, verdict: "pass", functions: fns }],
  };
  const md = renderMarkdown(res);
  assert.match(md, /\| `f9` \|/);
  assert.doesNotMatch(md, /\| `f10` \|/);
  assert.match(md, /\| `ep9h__sib__tail__DefineNamedOwnProperty_cold` \| 5 \| 7 \| \+2 \|/);
});

test("CLI: +0.5% exactly passes; a faster head passes; exit 0", (t) => {
  const f = fakeSetup(t);
  const r = f.run([], { FAKE_CG: counts([["ep9-base:t0-call", 2_000_000_000], ["ep9-head:t0-call", 2_010_000_000], ["ep9-base:t0-loop", 1_000_000_000], ["ep9-head:t0-loop", 800_000_000]]) });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.res.verdict, "PASS");
  assert.deepEqual(r.res.rows.map((x) => [x.name, x.delta_pct, x.verdict]), [["t0-loop", -20, "pass"], ["t0-call", 0.5, "pass"]]);
  assert.match(r.md, /\*\*Verdict: PASS\*\*/);
});

test("CLI: --branch-sim gives the Bim columns (X86IR)", (t) => {
  const f = fakeSetup(t);
  const cgLog = path.join(f.dir, "cg.log");
  const r = f.run(["--branch-sim", "--workloads", "t0-loop"], { FAKE_CG: counts([["ep9-base:t0-loop", 1_000_000_000, 2_000_000], ["ep9-head:t0-loop", 1_000_000_000, 1_000_000]]), FAKE_CG_LOG: cgLog });
  assert.equal(r.status, 0, r.stderr);
  const [row] = r.res.rows;
  assert.equal(r.res.branch_sim, true);
  assert.deepEqual([row.base.bim, row.head.bim, row.bim_delta_pct], [2_000_000, 1_000_000, -50]);
  assert.deepEqual([row.base.bc, row.base.bi], [100_000_000, 20_000_000]);
  assert.deepEqual(row.functions[0].base, { Ir: 600_000_000 + 100_000_000, Bi: 12_000_000 + 2_000_000, Bim: 1_200_000 + 200_000 });
  assert.equal(row.out_rows_match, true);
  assert.match(readFileSync(cgLog, "utf8"), /branch=yes/);
  assert.match(r.md, /\| workload \| base Ir \| head Ir \| delta \| verdict \| base Bim \| head Bim \| Bim delta \|/);
  assert.match(r.md, /\| t0-loop \| 1,000,000,000 \| 1,000,000,000 \| 0\.000% \| pass \| 2,000,000 \| 1,000,000 \| -50\.000% \|/);
});

test("CLI: a wrong output, a failed run or a log with no count gives no count: INCOMPLETE, exit 1", (t) => {
  const f = fakeSetup(t);
  let r = f.run([], { FAKE_WRONG: "ep9-head:t0-loop" });
  assert.equal(r.status, 1, r.stderr);
  assert.equal(r.res.verdict, "INCOMPLETE");
  const loop = r.res.rows[0];
  assert.deepEqual([loop.head.status, loop.head.ir, loop.verdict, loop.delta_pct], ["refused", null, "no count", null]);
  assert.match(loop.head.error, /printed "12345", want "604229"/);
  assert.equal(r.res.rows[1].verdict, "pass");
  assert.match(r.md, /\| t0-loop \| 1,000,007,000 \| - \| - \| no count \(head refused: printed/);
  r = f.run([], { FAKE_FAIL: "ep9-base:t0-call" });
  assert.equal(r.status, 1);
  assert.deepEqual([r.res.rows[1].base.status, r.res.rows[1].verdict], ["error", "no count"]);
  assert.match(r.res.rows[1].base.error, /exit 3: fake failure/);
  r = f.run([], { FAKE_CG_NO_IR: "ep9-head:t0-call" });
  assert.equal(r.status, 1);
  assert.match(r.res.rows[1].head.error, /no I refs line/);
  // A head Ir of 0 (a run that cachegrind did not trace) is no count, not a pass at -100% (D1).
  r = f.run([], { FAKE_CG: counts([["ep9-head:t0-call", 0]]) });
  assert.equal(r.status, 1, r.stderr);
  assert.equal(r.res.verdict, "INCOMPLETE");
  assert.deepEqual([r.res.rows[1].head.status, r.res.rows[1].head.ir, r.res.rows[1].delta_pct, r.res.rows[1].verdict], ["ok", 0, null, "no count"]);
  // A FAIL row beside a row with no count: the run reads FAIL.
  r = f.run([], { FAKE_WRONG: "ep9-head:t0-loop", FAKE_CG: counts([["ep9-head:t0-call", 3_000_000_000]]) });
  assert.equal(r.res.verdict, "FAIL");
});

test("CLI: each binary must name its own commit, built clean; usage errors exit 2", (t) => {
  const f = fakeSetup(t);
  // No --base and --head: a binary with no build line (an old ep9) or "commit unknown" runs,
  // and a note says so.
  let r = f.run([], { FAKE_BUILD_ep9_base: "", FAKE_BUILD_ep9_head: "commit unknown" }, { shas: false });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.res.notes, ["the base binary prints no build line", "the head binary names no commit"]);
  assert.deepEqual([r.res.base.ep9.version, r.res.base.ep9.engine_build, r.res.base.ep9.path], ["fake ep9-base 0.0", "features none; preset jitless", "ep9-base"]);
  // With --base and --head, each binary must name its commit (JIT review D2): no build line or
  // "commit unknown" is a usage error, exit 2.
  r = f.run([], { FAKE_BUILD_ep9_base: "" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /--base-ep9 names no commit; want 1{40}/);
  r = f.run([], { FAKE_BUILD_ep9_head: "commit unknown" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /--head-ep9 names no commit; want 2{40}/);
  r = f.run();
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.res.notes, []);
  assert.deepEqual(r.res.head.ep9.build, { commit: HEAD, dirty: false });
  r = f.run([], { FAKE_BUILD_ep9_base: `commit ${BASE} clean`, FAKE_BUILD_ep9_head: `commit ${BASE} clean` });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /--head-ep9 names commit 1{40}, not --head 2{40}/);
  r = f.run([], { FAKE_BUILD_ep9_base: `commit ${BASE} dirty` });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /--base-ep9 was built with uncommitted engine changes/);
  r = f.run(["--base", "main"]);
  assert.equal(r.status, 2);
  assert.match(r.stderr, /40 hex digits/);
});

test("markdown: notes, the worst row per group and the function table", () => {
  const side = (ir) => ({ status: "ok", error: null, ir, bc: null, bcm: null, bi: null, bim: null });
  const res = {
    target: "linux-x64",
    branch_sim: false,
    hash_seed: "1",
    gc_helpers: "0",
    limit_pct: 0.5,
    valgrind: "valgrind-3.22.0",
    host: { cpu: "fake cpu" },
    base: { commit: BASE },
    head: { commit: HEAD },
    notes: ["the base binary names no commit"],
    verdict: "PASS",
    worst: { v8v7: { name: "splay", delta_pct: 0.1234 }, octane: { name: "octane-splay", delta_pct: -0.2 } },
    rows: [
      { name: "splay", base: side(1000), head: side(1001), delta_pct: 0.1, verdict: "pass", functions: [{ fn: "a|b`c", base: { Ir: 10 }, head: { Ir: 11 }, delta: 1 }] },
    ],
  };
  const md = renderMarkdown(res);
  assert.match(md, /^### bench set \(linux-x64\): cachegrind Ir, head vs base\n/);
  assert.match(md, /base `111111111111`, head `222222222222`; EP9_HASH_SEED=1, EP9_GC_HELPERS=0; valgrind-3\.22\.0 --tool=cachegrind --cache-sim=no --branch-sim=no/);
  assert.match(md, /\*\*Verdict: PASS\*\* \(1 of 1 rows counted; worst v8v7: splay \+0\.123%; worst octane: octane-splay -0\.200%\)/);
  assert.match(md, /Note: the base binary names no commit\./);
  assert.match(md, /\| `a\\\|b'c` \| 10 \| 11 \| \+1 \|/);
});
