// node --test tools/bench/cross/m2gate.test.mjs (run.test.mjs imports this file, so the CI step
// `node --test tools/bench/cross/run.test.mjs` runs these tests too).
// The M2 gate reader (m2gate.mjs) on synthetic ep9-cross/v1 runs made with run.mjs's own
// analyze() and label(), and the TS workload (octane.mjs kernel `typescript`). The Octane tests
// skip when third_party/bench-octane is not synced. The m2-exit fixtures
// (tools/xtask/tests/m2_exit/gate-*.json) are this reader's output, each number at 10
// significant digits (fixtureText): `EP9_BLESS=1 node --test tools/bench/cross/m2gate.test.mjs`
// writes them again.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { buildOf, readM2Gate, READINGS, renderMarkdown, TS, tsReading, UsageError } from "./m2gate.mjs";
import { EXTRA_KERNELS, generate as generateOctane, KERNELS as OCTANE_KERNELS, TS_KERNEL } from "./octane.mjs";
import { analyze, label, parseArgs, partPlace } from "./run.mjs";
import { EMPTY, expandWorkloads, materialize, OCTANE_7, SETS, TS_WORKLOAD, V8V7_7, WORKLOADS } from "./workloads.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(DIR, "../../..");
const M2GATE = path.join(DIR, "m2gate.mjs");
const SHA = "0123456789abcdef0123456789abcdef01234567";
const COMPARATORS = ["qjs", "node-jitless"];
const BUILD_LINE = { jit: "features jit; preset jit", jitless: "features none; preset jitless" };
const MAC = { id: "m5max-macos", gate_host: true, cpu: "Apple M5 Max", platform: "darwin-arm64" };
const WIN_GATE = { id: "win-gate", gate_host: true, cpu: "AMD EPYC 7763", platform: "win32-x64" };
const LIN_GATE = { id: "lin-gate", gate_host: true, cpu: "AMD EPYC 7763", platform: "linux-x64" };

/**
 * A synthetic m2 run: ep9 takes `ms` per workload (TS: `tsMs`), each comparator takes `ratio(w, c)`
 * times ep9's net time, plus a 10 ms empty script on every engine, with a small per-round jitter.
 */
export function fakeRun({ build = "jit", ratio = () => 1.4, tsMs = 2500, host = MAC, rounds = 10, warmup = 1, provisional = false } = {}) {
  const workloads = SETS.m2;
  const samples = [];
  let seq = 0;
  for (let r = 0; r < warmup + rounds; r++) {
    const j = 1 + 0.004 * ((r % 5) - 2);
    for (const w of [...workloads, EMPTY]) {
      const ep9 = w === EMPTY ? 10 : w === TS_WORKLOAD ? tsMs * j : 100 * j;
      samples.push({ seq: seq++, round: r, warmup: r < warmup, workload: w, engine: "ep9", ms: ep9 });
      for (const c of COMPARATORS) {
        const ms = w === EMPTY ? 10 : 10 + ratio(w, c) * (ep9 - 10) * (1 + 0.003 * (((r + 2) % 5) - 2));
        samples.push({ seq: seq++, round: r, warmup: r < warmup, workload: w, engine: c, ms });
      }
    }
  }
  const m = label(analyze(samples, { subject: "ep9", comparators: COMPARATORS, workloads, warmup }), provisional);
  return {
    schema: "ep9-cross/v1",
    label: "synthetic m2 run (m2gate.test.mjs)",
    started: "2026-11-06T00:00:00.000Z",
    finished: "2026-11-06T01:00:00.000Z",
    commit: { ep9: SHA, runner: SHA, runner_dirty: false, dirty: [] },
    host: {
      file: "tools/bench/hosts/test.toml",
      id: host.id,
      gate_host: host.gate_host,
      idle_baseline_load1: 1,
      measured: { cpu: host.cpu, cpus: 18, platform: host.platform, os: "test", node: "v24.22.0", transparent_hugepage: null },
    },
    load: { margin: 0.5, before: { load1: 1, limit: 1.5, pass: true }, after: { load1: 1.1, limit: 1.5, pass: true }, after_each_round: [] },
    provisional,
    provisional_reasons: provisional ? ["load guard failed before the run (test)"] : [],
    options: { rounds, warmup },
    engines: [
      { id: "ep9", label: "ep9 T0", role: "subject", version: "ep9 0.0.0", engine_build: BUILD_LINE[build], bin: "ep9", args: [] },
      { id: "qjs", label: "QuickJS", role: "gate", version: "2026-06-04", engine_build: null, bin: "qjs", args: [] },
      { id: "node-jitless", label: "node --jitless", role: "gate", version: "v24.22.0", engine_build: null, bin: "node", args: ["--jitless"] },
    ],
    workloads: [EMPTY, ...workloads].map((name) => ({ name, harness: name === EMPTY ? undefined : WORKLOADS[name].octane ? "octane-base" : "base" })),
    M: { ratio: "t(engine) / t(ep9) per round; > 1: ep9 is faster", samples, ...m },
    R: null,
  };
}

const runs = (...rs) => rs.map((res, i) => ({ res, source: `run-${i}.json` }));
const byId = (g, id, build) => g.readings.find((r) => r.id === id && r.build === build);

test("m2gate: the readings, the thresholds and the sets they read", () => {
  assert.deepEqual(READINGS.map((r) => [r.id, r.item, r.threshold, r.comparator]), [
    ["octane7-vs-qjs", 1, 1.33, "qjs"],
    ["v8v7-7-vs-node-jitless", 2, 1.0, "node-jitless"],
    ["splay-vs-node-jitless", 6, 1.0, "node-jitless"],
  ]);
  assert.deepEqual(READINGS[0].workloads, OCTANE_7);
  assert.deepEqual(READINGS[1].workloads, V8V7_7);
  assert.equal(READINGS[1].kernelFloor, 0.8);
  assert.deepEqual([TS.maxSeconds, TS.minRatio, TS.macHost, TS.linuxPlatform], [2.8, 1.0, "m5max-macos", "linux-x64"]);
  assert.deepEqual(SETS.m2, [...V8V7_7, ...OCTANE_7, TS_WORKLOAD]);
  assert.deepEqual(SETS.ts, ["octane-typescript"]);
  assert.deepEqual(expandWorkloads(["ts"]), ["octane-typescript"]);
  assert.deepEqual(parseArgs(["--ep9", "x", "--workloads", "m2"]).workloads, SETS.m2);
  // The TS workload is not in Octane-7.
  assert.ok(!OCTANE_7.includes(TS_WORKLOAD));
  assert.deepEqual(WORKLOADS[TS_WORKLOAD], { octane: TS_KERNEL, classes: ["M", "R"] });
});

test("m2gate: the build comes from the ep9 build line", () => {
  const b = (line) => buildOf({ engines: [{ id: "ep9", engine_build: line }] }).build;
  assert.equal(b("features jit; preset jit"), "jit");
  assert.equal(b("features jit,become; preset jit"), "jit");
  assert.equal(b("features none; preset jitless"), "jitless");
  // A jit preset without the jit feature, or jitless with it, is neither build.
  assert.equal(b("features none; preset jit"), null);
  assert.equal(b("features jit; preset jitless"), null);
  assert.equal(b(null), null);
});

test("m2gate: both builds pass every reading", () => {
  const g = readM2Gate(runs(fakeRun({ build: "jit" }), fakeRun({ build: "jitless" })));
  assert.equal(g.schema, "ep9-m2gate/v1");
  assert.deepEqual(g.builds.map((b) => [b.build, b.engine_build, b.commit, b.dirty, b.host.platform]), [
    ["jit", BUILD_LINE.jit, SHA, false, "darwin-arm64"],
    ["jitless", BUILD_LINE.jitless, SHA, false, "darwin-arm64"],
  ]);
  assert.deepEqual(g.missing_builds, []);
  assert.equal(g.readings.length, 8);
  for (const r of g.readings) assert.equal(r.gate, "pass", `${r.id} ${r.build}: ${r.reasons}`);
  assert.equal(g.pass, true);
  const ts = byId(g, "ts", "jit");
  assert.ok(Math.abs(ts.value - 2.5) < 0.02, ts.value);
  assert.ok(ts.interval[1] <= 2.8);
  assert.ok(Math.abs(ts.net_s.median - 2.49) < 0.02);
  assert.ok(Math.abs(ts.vs_node_jitless.net.median - 1.4) < 0.01);
  assert.ok(ts.vs_qjs.gross.median > 1);
  // Both load-guard values are in each build block.
  assert.deepEqual(g.builds[0].load, { before: { load1: 1, limit: 1.5, pass: true }, after: { load1: 1.1, limit: 1.5, pass: true } });
  assert.match(renderMarkdown(g), /M2 speed gates: PASS/);
});

test("m2gate: one build alone does not pass (V12-D6)", () => {
  const g = readM2Gate(runs(fakeRun({ build: "jitless" })));
  assert.deepEqual(g.missing_builds, ["jit"]);
  assert.equal(g.pass, false);
  assert.match(renderMarkdown(g), /jit: no run/);
});

test("m2gate: Octane-7 at 1.2x QuickJS fails item 1; 1.33 inside the interval is marginal", () => {
  let g = readM2Gate(runs(fakeRun({ ratio: (w, c) => (c === "qjs" ? 1.2 : 1.4) })));
  assert.equal(byId(g, "octane7-vs-qjs", "jit").gate, "fail");
  assert.equal(byId(g, "v8v7-7-vs-node-jitless", "jit").gate, "pass");
  g = readM2Gate(runs(fakeRun({ ratio: (w, c) => (c === "qjs" ? 1.33 : 1.4) })));
  assert.equal(byId(g, "octane7-vs-qjs", "jit").gate, "marginal");
});

test("m2gate: a v8-v7 kernel below 0.8x is parity debt; the geomean still gates item 2", () => {
  const g = readM2Gate(runs(fakeRun({ ratio: (w, c) => (c === "qjs" ? 1.5 : w === "raytrace" ? 0.7 : w === "crypto" ? 0.8 : 1.2) })));
  const r = byId(g, "v8v7-7-vs-node-jitless", "jit");
  assert.equal(r.gate, "pass");
  // crypto at 0.8: the interval straddles the floor, so it is not proven: debt until a reading clears it.
  assert.deepEqual(r.parity_debt, ["crypto", "raytrace"]);
  assert.deepEqual(r.kernels.filter((k) => k.debt !== "none").map((k) => [k.workload, k.debt]), [["crypto", "marginal"], ["raytrace", "debt"]]);
  assert.match(renderMarkdown(g), /parity debt \(below or not proven >= 0\.8x\): crypto, raytrace/);
});

test("m2gate: splay below 1.0x fails item 6", () => {
  const g = readM2Gate(runs(fakeRun({ ratio: (w, c) => (w === "splay" && c === "node-jitless" ? 0.9 : 1.4) })));
  assert.equal(byId(g, "splay-vs-node-jitless", "jit").gate, "fail");
});

test("m2gate: TS on m5max-macos: the gross interval against 2.8 s", () => {
  const ts = (tsMs) => tsReading(fakeRun({ tsMs }), "jit");
  assert.equal(ts(2500).gate, "pass");
  assert.equal(ts(3560).gate, "fail");
  // 2.8 s inside the interval (the jitter is +-0.8%).
  assert.equal(ts(2800).gate, "marginal");
  assert.match(ts(2500).rule, /^m5max-macos: the upper end/);
  // Always reported: the net time and the paired ratios.
  assert.ok(ts(3560).net_s.median > 3.5);
  assert.ok(ts(3560).vs_node_jitless.net.median > 1);
});

test("m2gate: TS on Linux x64 needs >= 1.0x node --jitless; another host has no TS rule", () => {
  const linux = { id: "box", gate_host: true, cpu: "AMD EPYC 7763", platform: "linux-x64" };
  assert.equal(tsReading(fakeRun({ host: linux, tsMs: 9000, ratio: () => 1.1 }), "jit").gate, "pass");
  assert.equal(tsReading(fakeRun({ host: linux, tsMs: 1000, ratio: () => 0.9 }), "jit").gate, "fail");
  const other = tsReading(fakeRun({ host: { id: "other", gate_host: true, cpu: "x", platform: "linux-arm64" } }), "jit");
  assert.match(other.gate, /^no gate verdict \(no TS rule for host other/);
  assert.match(other.rule, /^no TS rule for host other/);
});

// M7-P0 Codex r2 C2-3: on a host with no TS rule (Windows x64), a valid measurement keeps the
// status "valid" (the Windows ratio to Linux x64 reads it), with no gate verdict; a void gross
// time or a provisional run still is not valid.
test("m2gate: a Windows TS reading is valid with no gate verdict (M7-P0 Codex r2 C2-3)", () => {
  const WIN = { id: "win-gate", gate_host: true, cpu: "AMD EPYC 7763", platform: "win32-x64" };
  const g = readM2Gate(runs(fakeRun({ build: "jit", host: WIN }), fakeRun({ build: "jitless", host: WIN })));
  for (const b of ["jit", "jitless"]) {
    const ts = byId(g, "ts", b);
    assert.equal(ts.status, "valid", `${b}: ${ts.reasons.join("; ")}`);
    assert.notEqual(ts.gate, "pass", b);
    assert.match(ts.gate, /^no gate verdict \(no TS rule for host win-gate \(win32-x64\)/, b);
    assert.deepEqual(ts.reasons, [], b);
  }
  assert.equal(g.pass, false);
  // A provisional run, no gate host, or void gross times: not valid.
  assert.equal(tsReading(fakeRun({ host: WIN, provisional: true }), "jit").status, "provisional");
  assert.match(tsReading(fakeRun({ host: { ...WIN, gate_host: false } }), "jit").reasons.join(), /not a gate host/);
  const noisy = fakeRun({ host: WIN });
  for (const x of noisy.M.samples) if (x.workload === TS_WORKLOAD && x.engine === "ep9" && x.round % 2) x.ms *= 1.5;
  const r = tsReading(noisy, "jit");
  assert.equal(r.status, "provisional");
  assert.match(r.reasons.join(), /gross times void/);
});

test("m2gate: no gate verdict for a provisional run or too few rounds; a missing TS workload is incomplete", () => {
  let g = readM2Gate(runs(fakeRun({ provisional: true })));
  for (const r of g.readings) assert.match(r.gate, /^no gate verdict/, r.id);
  assert.match(byId(g, "ts", "jit").gate, /interval reads pass/);
  const short = fakeRun();
  short.M.rounds = 9;
  short.options.rounds = 9;
  assert.match(tsReading(short, "jit").reasons.join(), /class M needs >= 10/);
  const no = fakeRun();
  no.workloads = no.workloads.filter((w) => w.name !== TS_WORKLOAD);
  assert.equal(tsReading(no, "jit").status, "incomplete");
  g = readM2Gate(runs(fakeRun({ host: { ...MAC, gate_host: false } })));
  assert.match(byId(g, "octane7-vs-qjs", "jit").reasons.join(), /not a gate host/);
});

test("m2gate: TS needs both comparator readings and no void ratio (M7-P0 Codex r1 GATES10-2)", () => {
  // (a) No TS rows of M.ratios in either run: no gate verdict (it read pass).
  const noRows = (res) => {
    res.M.ratios = res.M.ratios.filter((x) => x.workload !== TS_WORKLOAD);
    return res;
  };
  let g = readM2Gate(runs(noRows(fakeRun({ build: "jit" })), noRows(fakeRun({ build: "jitless" }))));
  for (const b of ["jit", "jitless"]) {
    const ts = byId(g, "ts", b);
    assert.match(ts.gate, /^no gate verdict/, b);
    assert.match(ts.reasons.join(), /no node-jitless row.*no qjs row/, b);
  }
  assert.equal(g.pass, false);
  // (b) A void net ratio to node --jitless: no gate verdict.
  const voidNode = (res) => {
    const row = res.M.ratios.find((x) => x.workload === TS_WORKLOAD && x.comparator === "node-jitless");
    row.net = { ...row.net, void: true, iqrPct: 66.8 };
    return res;
  };
  g = readM2Gate(runs(voidNode(fakeRun({ build: "jit" })), fakeRun({ build: "jitless" })));
  assert.match(byId(g, "ts", "jit").gate, /^no gate verdict/);
  assert.match(byId(g, "ts", "jit").reasons.join(), /ratios to node-jitless void \(IQR 66\.8% > 5%\)/);
  assert.equal(byId(g, "ts", "jitless").gate, "pass");
  assert.equal(g.pass, false);
});

// M7-P0 Codex r2 C2-1: a comparator whose TS net time is <= 0 in a round gets a gross fallback
// row (status "net invalid", metric "gross"). That row read pass on m5max-macos and, for
// QuickJS, on Linux x64. The TS reading needs a valid net ratio to both comparators.
test("m2gate: TS needs valid net ratios to both comparators (M7-P0 Codex r2 C2-1)", () => {
  const LINUX = { id: "linux-gate", gate_host: true, cpu: "AMD EPYC 7763", platform: "linux-x64" };
  const netInvalid = (res, engines) => {
    // In counted round 3, the comparator's TS time (5 ms) is below its empty-script time (10 ms).
    for (const x of res.M.samples) if (x.workload === TS_WORKLOAD && engines.includes(x.engine) && x.round === 3) x.ms = 5;
    Object.assign(res.M, label(analyze(res.M.samples, { subject: "ep9", comparators: COMPARATORS, workloads: SETS.m2, warmup: 1 }), false));
    return res;
  };
  for (const [name, engines, host] of [
    ["mac, qjs net invalid", ["qjs"], MAC],
    ["mac, both net invalid", ["qjs", "node-jitless"], MAC],
    ["linux, qjs net invalid", ["qjs"], LINUX],
  ]) {
    const mk = (build) => netInvalid(fakeRun({ build, host }), engines);
    const g = readM2Gate(runs(mk("jit"), mk("jitless")));
    for (const b of ["jit", "jitless"]) {
      const ts = byId(g, "ts", b);
      assert.equal(ts.vs_qjs.metric, "gross", `${name} ${b}`);
      assert.equal(ts.status, "provisional", `${name} ${b}`);
      assert.notEqual(ts.gate, "pass", `${name} ${b}`);
      assert.match(ts.gate, /^no gate verdict/, `${name} ${b}`);
      for (const e of engines) assert.match(ts.reasons.join("; "), new RegExp(`the net ratio to ${e} is invalid \\(net invalid`), `${name} ${b} ${e}`);
    }
    assert.equal(g.pass, false, name);
  }
  // A row that is not "valid" for another reason (a hand-edited status) in a run that is not
  // provisional: no gate verdict.
  const odd = fakeRun();
  odd.M.ratios.find((x) => x.workload === TS_WORKLOAD && x.comparator === "qjs").status = ["void (IQR 9.0% > 5%)", "provisional"];
  const r = tsReading(odd, "jit");
  assert.match(r.gate, /^no gate verdict/);
  assert.match(r.reasons.join(), /the qjs row is not valid: void/);
});

test("m2gate: usage errors: another build, the same build twice, two hosts, not a cross run", () => {
  const odd = fakeRun();
  odd.engines[0].engine_build = "features none; preset jit";
  assert.throws(() => readM2Gate(runs(odd)), UsageError);
  assert.throws(() => readM2Gate(runs(fakeRun(), fakeRun())), /a second run of the jit build/);
  const other = fakeRun({ build: "jitless", host: { ...MAC, cpu: "Apple M4" } });
  assert.throws(() => readM2Gate(runs(fakeRun(), other)), /one host per m2gate file/);
  assert.throws(() => readM2Gate(runs({ schema: "ep9-bench/v1" })), /not an ep9-cross\/v1 run/);
  assert.throws(() => readM2Gate([]), UsageError);
});

test("m2gate: CLI exit status and JSON", (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "m2gate-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const a = path.join(dir, "jit.json");
  const b = path.join(dir, "jitless.json");
  writeFileSync(a, JSON.stringify(fakeRun({ build: "jit" })));
  writeFileSync(b, JSON.stringify(fakeRun({ build: "jitless" })));
  const out = path.join(dir, "g.json");
  let r = spawnSync(process.execPath, [M2GATE, a, b, "--out", out], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.equal(JSON.parse(readFileSync(out, "utf8")).pass, true);
  r = spawnSync(process.execPath, [M2GATE, a], { encoding: "utf8" });
  assert.equal(r.status, 1);
  r = spawnSync(process.execPath, [M2GATE, a, a], { encoding: "utf8" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /a second run of the jit build/);
  r = spawnSync(process.execPath, [M2GATE], { encoding: "utf8" });
  assert.equal(r.status, 2);
});

// The fixture text of a reader result: JSON.stringify(g, null, 1), each number that is not an
// integer at 10 significant digits. Math.log is not the same in the last bit on every V8 build
// (node 24.22 on Linux x64 and arm64: Math.log(1.3004568674698793) is 0x3fd0d05541acb0c2; on
// macOS arm64 and in node 26: ...c3, the correctly rounded value), so the per-round geomeans
// (lib/stats.mjs geomean, exp of the mean log) and the IQR of them change in the last digits
// from host to host (CI run 42180919357). 10 digits are the same on every host; m2-exit reads
// `value` and `interval` and prints them to 3 decimals.
const fixtureText = (g) => `${JSON.stringify(g, (_, v) => (typeof v === "number" && !Number.isInteger(v) ? Number(v.toPrecision(10)) : v), 1)}\n`;

// The m2-exit fixtures: what this reader writes for 5 cases. m2-exit's tests replace the
// commit SHA with the HEAD of their scratch repository.
test("m2gate: the m2-exit fixtures are this reader's output", () => {
  const linux = { id: "linux-box", gate_host: false, cpu: "AMD EPYC 7763", platform: "linux-x64" };
  const cases = [
    ["gate-pass.json", runs(fakeRun({ build: "jit" }), fakeRun({ build: "jitless" }))],
    // jit passes; jitless: Octane-7 fails, TS is marginal, raytrace is parity debt.
    ["gate-mixed.json", runs(fakeRun({ build: "jit" }), fakeRun({ build: "jitless", tsMs: 2800, ratio: (w, c) => (c === "qjs" ? 1.2 : w === "raytrace" ? 0.7 : 1.2) }))],
    // Linux x64, not a gate host (the readings have no gate verdict).
    ["gate-linux.json", runs(fakeRun({ build: "jit", host: linux }), fakeRun({ build: "jitless", host: linux }))],
    // The Windows x64 ratio (M7-P0 Codex r2 C2-3): a Windows gate host and a Linux x64 gate host
    // with one CPU model. The TS readings on Windows have no TS rule but are valid; Octane-7 at
    // 1.33x QuickJS is marginal (valid) in the Windows jitless run and in the Linux jit run.
    ["gate-win.json", runs(fakeRun({ build: "jit", host: WIN_GATE }), fakeRun({ build: "jitless", host: WIN_GATE, ratio: (w, c) => (c === "qjs" ? 1.33 : 1.4) }))],
    ["gate-win-linux.json", runs(fakeRun({ build: "jit", host: LIN_GATE, ratio: (w, c) => (c === "qjs" ? 1.33 : 1.4) }), fakeRun({ build: "jitless", host: LIN_GATE }))],
  ];
  for (const [name, input] of cases) {
    const text = fixtureText(readM2Gate(input));
    const file = path.join(ROOT, "tools/xtask/tests/m2_exit", name);
    if (process.env.EP9_BLESS === "1") writeFileSync(file, text);
    assert.equal(readFileSync(file, "utf8"), text, `${name}: run EP9_BLESS=1 node --test tools/bench/cross/m2gate.test.mjs`);
  }
});

// ---- the TS workload (octane.mjs kernel `typescript`) ---------------------------------------

const haveOctane = existsSync(path.join(ROOT, "third_party/bench-octane/typescript-compiler.js"));

test("ts: 3 files in run.js order, N = 5, the pinned files unchanged", { skip: !haveOctane && "third_party/bench-octane not synced" }, () => {
  assert.deepEqual(EXTRA_KERNELS.typescript, { files: ["typescript.js", "typescript-input.js", "typescript-compiler.js"], n: 5 });
  assert.ok(!(TS_KERNEL in OCTANE_KERNELS));
  const runJs = readFileSync(path.join(ROOT, "third_party/bench-octane/run.js"), "utf8");
  const order = [...runJs.matchAll(/load\(base_dir \+ '(typescript[^']*)'\)/g)].map((m) => m[1]);
  assert.deepEqual(order, EXTRA_KERNELS.typescript.files);
  // The benchmark's own iteration count and warm-up flag.
  const ts = readFileSync(path.join(ROOT, "third_party/bench-octane/typescript.js"), "utf8");
  assert.match(ts, /new Benchmark\("Typescript",\s*false,\s*true,\s*5,/);
  const g = generateOctane("typescript");
  assert.equal(g.text, generateOctane("typescript").text);
  assert.equal(g.n, 5);
  assert.equal(g.expect, "Typescript 5");
  assert.deepEqual(g.parts.map((p) => p.file), ["base.js", ...EXTRA_KERNELS.typescript.files]);
  for (const p of g.parts) {
    const src = readFileSync(path.join(ROOT, "third_party/bench-octane", p.file), "utf8");
    assert.equal(g.text.split("\n").slice(p.offset, p.offset + p.lines).join("\n"), src.replace(/\n$/, ""), p.file);
  }
  assert.deepEqual(g.blobs.map((b) => b.file), EXTRA_KERNELS.typescript.files);
  assert.equal(g.source, "typescript.js, typescript-input.js, typescript-compiler.js");
  assert.match(g.text, /BenchmarkSuite\.config\.doWarmup = false;/);
  // A one-file kernel keeps its old header ("both unchanged") and gets one blob.
  assert.match(generateOctane("richards").text, /both unchanged, in d8's load order/);
  assert.equal(generateOctane("richards").blobs.length, 1);
  assert.throws(() => generateOctane("regexp"), /unknown Octane kernel regexp/);
});

test("ts: the workload file names every blob; an error line maps to its part file", { skip: !haveOctane && "third_party/bench-octane not synced" }, () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ts-wl-"));
  try {
    const m = materialize([TS_WORKLOAD, "octane-richards"], dir, {});
    const w = m[TS_WORKLOAD];
    assert.equal(w.expect, "Typescript 5");
    assert.equal(w.harness, "octane-base");
    assert.equal(w.blobs.length, 3);
    assert.equal(w.parts.length, 4);
    // A one-file kernel adds no field.
    assert.equal(m["octane-richards"].blobs, undefined);
    const p = w.parts[3];
    assert.equal(partPlace(w.parts, w.file, `${w.file}:${p.offset + 7}:3: Uncaught TypeError: x\n`), "typescript-compiler.js:7");
    assert.equal(partPlace(w.parts, w.file, "other.js:1:1: x\n"), null);
    assert.equal(partPlace(w.parts, w.file, `${w.file}:3:1: x\n`), "generated:3");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("ts: the generated file runs on node and prints the expected line", { skip: !haveOctane && "third_party/bench-octane not synced" }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ts-run-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  // N = 1 keeps the test short; the checksum and the parse-error count are checked inside run().
  const g = generateOctane("typescript", { n: 1 });
  const f = path.join(dir, "ts.js");
  writeFileSync(f, g.text);
  const r = spawnSync(process.execPath, [f], { encoding: "utf8", timeout: 120_000 });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout.trim(), "Typescript 1");
});
