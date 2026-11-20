// node --test .github/scripts/exit-inputs.test.mjs
// The exit-inputs helpers (plan v14 section 4.1, plan v15 section 4.1): the runner host file and
// the Path B compare of the windows and Linux x64 legs. Synthetic inputs and fake engines
// (tools/bench/cross/testdata): no shells, no ep9 build, no GitHub runner. `EP9_BLESS=1` writes
// the m2-exit fixtures pathb-win.json and pathb-gha-linux*.json.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { cpuModel, parseToml, readHostFile } from "../../tools/bench/lib/host.mjs";
import { analyze, label, platformId } from "../../tools/bench/cross/run.mjs";
import { EMPTY, TS_WORKLOAD, V8V7_7 } from "../../tools/bench/cross/workloads.mjs";
import { agreement, BUILDS, CELLS, compare, host, hostToml, pairLegs, parseArgs, parseIdle, PATHB_ROUNDS, readLeg, renderMarkdown, SCHEMA, side } from "./exit-inputs.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(DIR, "../..");
const SCRIPT = path.join(DIR, "exit-inputs.mjs");
// The commit placeholder of the m2-exit fixtures (its tests put their scratch HEAD there).
const SHA = "0123456789abcdef0123456789abcdef01234567";
const FIXTURES = path.join(ROOT, "tools/xtask/tests/m2_exit");
const FIXTURE = path.join(FIXTURES, "pathb-win.json");
const WIN_CPU = "AMD EPYC 7763 64-Core Processor";

/** The bytes of an m2-exit fixture: written with EP9_BLESS=1, else compared. */
function fixture(name, c) {
  const file = path.join(FIXTURES, name);
  const text = `${JSON.stringify(c, null, 1)}\n`;
  if (process.env.EP9_BLESS === "1") writeFileSync(file, text);
  assert.equal(readFileSync(file, "utf8"), text, "run EP9_BLESS=1 node --test .github/scripts/exit-inputs.test.mjs");
}

/** A both-builds run view of a windows leg, cut to the fields that compare() reads. */
function runView(build, { cpu = WIN_CPU, platform = "win32-x64", dirty = false, rounds = PATHB_ROUNDS, commit = SHA } = {}) {
  const guard = (load1) => ({ load1, limit: 1.2, pass: true, why: null, source: "typeperf" });
  return {
    schema: "ep9-cross/v1",
    commit: { ep9: dirty ? null : commit, binary: { commit, dirty } },
    host: { id: "gha-win32-x64", gate_host: false, measured: { cpu, platform, load_source: "typeperf" } },
    load: { margin: 0.5, before: guard(0.41), after: guard(0.38) },
    M: { rounds },
    both_builds: { build },
  };
}

const leg = (name, opts = {}) => ({ name, cpu: opts.cpu ?? WIN_CPU, platform: opts.platform ?? "win32-x64", jit: runView("jit", opts), jitless: runView("jitless", opts), error: null });

/**
 * Made-up m2gate readings of one box: each cell's ratio is `base` plus a per-item step. With
 * `qjs` false the run has no QuickJS (the windows legs before V15-D17): item 1 has no reading.
 * The gross ep9 TS time is 4 / (`base` * `tsScale`) s (the speed 1 / t goes with `base`).
 */
function gateOf(base, { qjs = true, tsScale = 1 } = {}) {
  const r = (x) => Number(x.toFixed(4));
  const readings = [];
  for (const build of BUILDS) {
    const b = build === "jit" ? 0 : -0.02;
    const geo = (id, m, half) => ({ id, build, value: r(m), interval: [r(m - half), r(m + half)], net: { median: r(m) }, status: "provisional" });
    if (qjs) readings.push(geo("octane7-vs-qjs", base + 0.35 + b, 0.03));
    else readings.push({ id: "octane7-vs-qjs", build, value: null, interval: null, net: null, status: "incomplete" });
    readings.push(geo("v8v7-7-vs-node-jitless", base + 0.1 + b, 0.02));
    readings.push(geo("splay-vs-node-jitless", base + 0.2 + b, 0.03));
    const speed = (base + b) * tsScale;
    const gross = { median: r(4 / speed), interval: [r(4 / (speed + 0.02)), r(4 / (speed - 0.02))], void: false };
    readings.push({ id: "ts", build, status: "provisional", value: gross.median, interval: gross.interval, gross_s: gross, vs_node_jitless: { net: { median: r(base - 0.1 + b), interval: [r(base - 0.12 + b), r(base - 0.08 + b)], void: false } } });
  }
  return { builds: BUILDS.map((build) => ({ build })), readings };
}

test("side and agreement follow the m2-exit rule (pathb_side, pathb_form)", () => {
  assert.equal(side([1.0, 1.2], 1), "above");
  assert.equal(side([0.8, 0.99], 1), "below");
  assert.equal(side([0.9, 1.1], 1), "straddles");
  assert.equal(side(null, 1), null);
  assert.equal(side([1.2, 1.1], 1), null);
  assert.equal(agreement("above", "above"), "AGREE above");
  assert.equal(agreement("below", "below"), "AGREE below");
  assert.equal(agreement("above", "below"), "OPPOSITE");
  assert.equal(agreement("above", "straddles"), "NO AGREEMENT");
  assert.equal(agreement(null, null), "NO AGREEMENT");
  assert.deepEqual(CELLS.map((c) => [c.item, c.threshold]), [[1, 1.33], [2, 1], [3, 1], [6, 1]]);
});

test("pairing: the first 2 legs of the first platform and CPU model with 2, in leg order", () => {
  const l = (name, cpu, platform = "win32-x64") => ({ name, cpu, platform });
  assert.deepEqual(pairLegs([l("w1", "A"), l("w2", "B"), l("w3", "B"), l("w4", "A")]), { cpu: "A", boxes: [l("w1", "A"), l("w4", "A")] });
  assert.deepEqual(pairLegs([l("w1", "A"), l("w2", "B"), l("w3", "B"), l("w4", "C")]), { cpu: "B", boxes: [l("w2", "B"), l("w3", "B")] });
  assert.equal(pairLegs([l("w1", "A"), l("w2", "B"), l("w3", "C"), l("w4", "D")]), null);
  assert.equal(pairLegs([l("w1", null), l("w2", null)]), null);
  // EI2 Codex r1 F4: "unknown" is the cpuModel sentinel of a failed detection (host.mjs), not a model.
  assert.equal(pairLegs([l("w1", "unknown"), l("w2", "unknown")]), null);
  assert.equal(pairLegs([l("w1", " "), l("w2", " ")]), null);
  assert.deepEqual(pairLegs([l("w1", "unknown"), l("w2", "A"), l("w3", "unknown"), l("w4", "A")]).boxes.map((x) => x.name), ["w2", "w4"]);
  // V15-D16: the Linux x64 legs are in the same dispatch; one CPU model on 2 platforms is no pair.
  assert.equal(pairLegs([l("w1", "A"), l("l1", "A", "linux-x64")]), null);
  assert.deepEqual(pairLegs([l("w1", "A"), l("l1", "A", "linux-x64"), l("l2", "A", "linux-x64"), l("w2", "A")]), { cpu: "A", boxes: [l("w1", "A"), l("w2", "A")] });
  assert.deepEqual(pairLegs([l("l1", "A", "linux-x64"), l("w1", "A"), l("l2", "A", "linux-x64")]).boxes.map((x) => x.name), ["l1", "l2"]);
});

test("compare: no pair names every runner and no cell; the CLI exits 3", (t) => {
  const c = compare([leg("w1", { cpu: "A" }), leg("w2", { cpu: "B" }), leg("w3", { cpu: "C" }), { name: "w4", cpu: null, platform: null, error: "no run-jit.json", jit: null, jitless: null }]);
  assert.equal(c.schema, SCHEMA);
  assert.equal(c.pair, null);
  assert.match(c.result, /^NO PAIR: .*run the dispatch again once, then the reading is NOT-RUN/);
  assert.deepEqual(c.runners.map((r) => [r.leg, r.cpu]), [["w1", "A"], ["w2", "B"], ["w3", "C"], ["w4", null]]);
  assert.deepEqual([c.checks, c.cells, c.commit, c.platform], [[], [], null, null]);
  // The legs block names every leg that was read, with a pair or not (V15-D16).
  assert.deepEqual(c.legs.map((l) => [l.leg, l.cpu, l.commit]), [["w1", "A", SHA], ["w2", "B", SHA], ["w3", "C", SHA]]);
  assert.match(renderMarkdown(c), /\*\*NO PAIR/);
  assert.match(renderMarkdown(c), /^Per leg \(the input of the win-x64 same-CPU-model cell, V15-D16\):$/m);
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-exit-inputs-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const legs = ["A", "B"].map((cpu, i) => {
    const d = path.join(dir, `w${i + 1}`);
    mkdirSync(d);
    for (const b of BUILDS) writeFileSync(path.join(d, `run-${b}.json`), JSON.stringify(runView(b, { cpu })));
    return d;
  });
  const out = path.join(dir, "c.json");
  const r = spawnSync(process.execPath, [SCRIPT, "pathb", "--leg", legs[0], "--leg", legs[1], "--out", out], { encoding: "utf8" });
  assert.equal(r.status, 3, r.stderr);
  assert.equal(JSON.parse(readFileSync(out, "utf8")).pair, null);
  assert.deepEqual(readLeg(path.join(dir, "none")).error !== null, true);
});

test("compare: the pair's run checks and the 8 cells (the m2-exit fixture pathb-win.json)", () => {
  const gates = { w1: gateOf(1.0), w2: gateOf(1.05), w3: gateOf(1.06), w4: gateOf(1.01) };
  const readGate = (runs) => gates[runs[0].source.split("/")[0]];
  const legs = [leg("w1", { cpu: "Intel Xeon Platinum 8370C" }), leg("w2"), leg("w3"), leg("w4", { cpu: "Intel Xeon Platinum 8370C" })];
  const c = compare(legs, { readGate });
  assert.deepEqual(c.pair, { cpu: "Intel Xeon Platinum 8370C", legs: ["w1", "w4"] });
  // The legs of the first model with 2: w1 and w4 (not w2 and w3).
  const c2 = compare([legs[1], legs[0], legs[2], legs[3]], { readGate });
  assert.deepEqual(c2.boxes, ["w2", "w3"]);
  assert.equal(c2.platform, "win32-x64");
  assert.equal(c2.commit, SHA);
  assert.deepEqual(c2.checks.map((k) => [k.box, k.build, k.commit_ok, k.rounds, k.load.before.pass, k.load.after.pass]), [
    [1, "jit", true, 31, true, true],
    [1, "jitless", true, 31, true, true],
    [2, "jit", true, 31, true, true],
    [2, "jitless", true, 31, true, true],
  ]);
  assert.deepEqual(c2.cells.map((x) => [x.build, x.cell.split(":")[0], x.result]), [
    ["jit", "item 1", "AGREE above"],
    ["jit", "item 2", "AGREE above"],
    ["jit", "item 3", "AGREE below"],
    ["jit", "item 6", "AGREE above"],
    ["jitless", "item 1", "AGREE above"],
    ["jitless", "item 2", "AGREE above"],
    ["jitless", "item 3", "AGREE below"],
    ["jitless", "item 6", "AGREE above"],
  ]);
  const ts = c2.cells.find((x) => x.build === "jit" && x.cell.startsWith("item 3:"));
  assert.deepEqual(ts.box1, { median: 0.95, interval: [0.93, 0.97], note: "net, provisional" });
  assert.match(renderMarkdown(c2), /TS ratio to node --jitless \(V14-D59 input\): jit box 1 0\.950, box 2 0\.960; jitless box 1 0\.930, box 2 0\.940\./);
  // The legs block: all 4 legs in leg order, each with its own run checks and 8 values.
  assert.deepEqual(c2.legs.map((l) => [l.leg, l.cpu, l.platform, l.commit, l.checks.map((k) => k.commit_ok).join()]), [
    ["w2", WIN_CPU, "win32-x64", SHA, "true,true"],
    ["w1", "Intel Xeon Platinum 8370C", "win32-x64", SHA, "true,true"],
    ["w3", WIN_CPU, "win32-x64", SHA, "true,true"],
    ["w4", "Intel Xeon Platinum 8370C", "win32-x64", SHA, "true,true"],
  ]);
  assert.deepEqual(c2.legs[0].cells.find((x) => x.build === "jit" && x.cell.startsWith("item 2:")).value, { median: 1.15, interval: [1.13, 1.17], note: "net, provisional" });
  assert.equal(c2.legs[0].cells.length, 8);
  assert.match(renderMarkdown(c2), /^\| w2 \| AMD EPYC 7763 64-Core Processor \| jit \| yes \| 1\.400 \[1\.370-1\.430\] \| 1\.150 \[1\.130-1\.170\] \| 0\.950 \[0\.930-0\.970\] \| 1\.250 \[1\.220-1\.280\] \|$/m);
  // The fixture of `cargo xtask m2-exit --pathb` on x86_64-pc-windows-msvc is this output.
  fixture("pathb-win.json", c2);
  assert.ok(existsSync(FIXTURE));
  // A dirty binary, another commit or a gate that cannot read the pair: commit_ok false.
  const bad = compare([leg("w2", { dirty: true }), leg("w3")], { readGate });
  assert.deepEqual(bad.checks.map((k) => k.commit_ok), [false, false, true, true]);
  const other = compare([leg("w2"), leg("w3", { commit: "f".repeat(40) })], { readGate });
  assert.deepEqual(other.checks.map((k) => k.commit_ok), [true, true, false, false]);
  const thrown = compare([leg("w2"), leg("w3")], { readGate: () => { throw new Error("one host per m2gate file"); } });
  assert.ok(thrown.checks.every((k) => !k.commit_ok && k.error === "one host per m2gate file"));
  assert.ok(thrown.cells.every((x) => x.result === "NO AGREEMENT"));
  // One CPU model on 2 platforms is no pair (V15-D16: the Linux x64 legs are in the dispatch).
  const mixed = compare([leg("w2"), leg("w3", { platform: "linux-x64" })], { readGate });
  assert.equal(mixed.pair, null);
  assert.deepEqual(mixed.legs.map((l) => [l.leg, l.platform]), [["w2", "win32-x64"], ["w3", "linux-x64"]]);
});

// V15-D16: the Linux x64 GitHub legs of the same dispatch. --platform keeps one platform; the
// legs of the other one are named in the runners table only. The 3 m2-exit fixtures of the
// win-x64 same-CPU-model cell: pathb-gha-linux.json (2 legs of the Windows pair's model, each
// value a little above the Windows one: the cross-host intervals are above 0.9),
// pathb-gha-linux-fast.json (the same model, every value about 1.3x the Windows one: below 0.9)
// and pathb-gha-linux-other.json (another model: no same-model pair, the cell keeps today's form).
test("compare: the Linux x64 legs, --platform, and the m2-exit fixtures pathb-gha-linux*.json", () => {
  const lin = (name, cpu = WIN_CPU) => leg(name, { cpu, platform: "linux-x64" });
  const all = [leg("w1"), lin("l1"), leg("w2"), lin("l2")];
  const gates = { w1: gateOf(1.05), w2: gateOf(1.06), l1: gateOf(1.1), l2: gateOf(1.1) };
  const readGate = (runs) => gates[runs[0].source.split("/")[0]];
  const w = compare(all, { readGate, platform: "win32-x64" });
  assert.deepEqual([w.boxes, w.platform, w.legs.map((l) => l.leg)], [["w1", "w2"], "win32-x64", ["w1", "w2"]]);
  assert.deepEqual(w.runners.map((r) => [r.leg, r.error]), [
    ["w1", null],
    ["l1", "platform linux-x64, not win32-x64 (not read)"],
    ["w2", null],
    ["l2", "platform linux-x64, not win32-x64 (not read)"],
  ]);
  assert.match(renderMarkdown(w), /^\| l1 \| AMD EPYC 7763 64-Core Processor \| linux-x64 \| no \(platform linux-x64, not win32-x64 \(not read\)\) \|$/m);
  const l = compare(all, { readGate, platform: "linux-x64" });
  assert.deepEqual([l.boxes, l.platform, l.commit], [["l1", "l2"], "linux-x64", SHA]);
  // The Linux pair against the M2 thresholds (its TS 1.000 [0.980-1.020] straddles 1.0).
  assert.deepEqual(l.cells.map((x) => x.result), ["AGREE above", "AGREE above", "NO AGREEMENT", "AGREE above", "AGREE above", "AGREE above", "NO AGREEMENT", "AGREE above"]);
  // Without --platform the first group with 2 legs is the pair: here the windows one.
  assert.deepEqual(compare(all, { readGate }).boxes, ["w1", "w2"]);
  assert.equal(parseArgs(["pathb", "--leg", "a", "--out", "c", "--platform", "linux-x64"]).platform, "linux-x64");
  fixture("pathb-gha-linux.json", compare([lin("l1"), lin("l2")], { readGate }));
  const fast = { l1: gateOf(1.4), l2: gateOf(1.42) };
  fixture("pathb-gha-linux-fast.json", compare([lin("l1"), lin("l2")], { readGate: (runs) => fast[runs[0].source.split("/")[0]] }));
  const epyc4 = "AMD EPYC 9V74 80-Core Processor";
  const other = compare([lin("l1", epyc4), lin("l2", epyc4)], { readGate });
  assert.deepEqual(other.legs.map((x) => x.cpu), [epyc4, epyc4]);
  fixture("pathb-gha-linux-other.json", other);
  // EI2 Codex r1 F1: l2 was not read (a read error), so it is in `runners` only, not in `legs`;
  // m2-exit then knows no full set of same-model pairs (pathb-gha-linux-unread.json).
  const l2 = { name: "l2", cpu: null, platform: null, jit: null, jitless: null, error: "no run-jit.json" };
  const unread = compare([lin("l1"), l2], { readGate });
  assert.deepEqual(unread.legs.map((x) => x.leg), ["l1"]);
  assert.deepEqual(unread.runners.map((x) => [x.leg, x.error]), [["l1", null], ["l2", "no run-jit.json"]]);
  fixture("pathb-gha-linux-unread.json", unread);
  // EI2 Codex r1 F2: the TS ratios to node --jitless of l1 and l2 are those of the Windows legs,
  // but their gross ep9 TS time is 3/4 of the Windows one (pathb-gha-linux-ts.json): the
  // cross-host TS speed (1 / t, 10:157) is about 0.75, below 0.9.
  const slow = { l1: gateOf(1.05, { tsScale: 4 / 3 }), l2: gateOf(1.05, { tsScale: 4 / 3 }) };
  const tsc = compare([lin("l1"), lin("l2")], { readGate: (runs) => slow[runs[0].source.split("/")[0]] });
  const item3 = (c, l, build) => c.legs.find((x) => x.leg === l).cells.find((x) => x.build === build && x.cell.startsWith("item 3:"));
  assert.deepEqual(item3(tsc, "l1", "jit").value.interval, [0.93, 0.97]);
  assert.deepEqual(item3(tsc, "l1", "jit").time_s, { median: 2.8571, interval: [2.8169, 2.8986], note: "gross, provisional" });
  assert.equal(item3(w, "w1", "jit").time_s.median, 3.8095);
  assert.ok(tsc.legs.every((x) => x.cells.filter((y) => "time_s" in y).length === 2), "time_s on the 2 item 3 cells only");
  fixture("pathb-gha-linux-ts.json", tsc);
  // A void gross time, or a refused reading: time_s has no interval.
  const voidG = gateOf(1.05);
  voidG.readings.find((x) => x.id === "ts" && x.build === "jit").gross_s.void = true;
  const v = compare([lin("l1"), lin("l2")], { readGate: () => voidG });
  assert.deepEqual(item3(v, "l1", "jit").time_s, { median: 3.8095, interval: null, note: "gross, provisional; no interval: gross times void" });
  assert.deepEqual(item3(v, "l1", "jitless").time_s.interval, [3.8095, 3.9604]);
  const noNet = gateOf(1.05);
  delete noNet.readings.find((x) => x.id === "ts" && x.build === "jit").vs_node_jitless;
  const n = compare([lin("l1"), lin("l2")], { readGate: () => noNet });
  assert.match(item3(n, "l1", "jit").time_s.note, /no interval: no net ratio to node --jitless/);
});

/**
 * A both-builds run view of a windows leg with M2 data that the real reader reads (run.mjs
 * analyze and label, then m2gate.mjs readM2Gate in compare): the v8-v7 kernels and the TS
 * workload against node --jitless, 31 paired rounds, no QuickJS. ep9 takes 100 ms per kernel
 * and 10 ms for the empty script, node --jitless 20% more (net ratio 1.2). `noisy` lists the
 * workloads whose node --jitless time moves by up to 15% from round to round (a void row);
 * `netBad` is a workload whose ep9 time is below the empty-script time in round 3 (net invalid).
 */
function m2RunView(build, { noisy = [], netBad = null } = {}) {
  const workloads = [...V8V7_7, TS_WORKLOAD];
  const samples = [];
  for (let round = 0; round < PATHB_ROUNDS; round++) {
    const small = 1 + 0.001 * ((round * 7) % 5);
    const big = 1 + 0.15 * (((round % 4) - 1.5) / 1.5);
    for (const workload of [...workloads, EMPTY]) {
      const ep9 = workload === EMPTY ? 10 : workload === netBad && round === 3 ? 5 : 100;
      const node = (workload === EMPTY ? 12 : 120) * (noisy.includes(workload) ? big : small);
      samples.push({ round, warmup: false, workload, engine: "ep9", ms: ep9 }, { round, warmup: false, workload, engine: "node-jitless", ms: node });
    }
  }
  const m = label(analyze(samples, { subject: "ep9", comparators: ["node-jitless"], workloads, warmup: 0 }), true);
  return {
    ...runView(build),
    engines: [{ id: "ep9", engine_build: build === "jit" ? "features jit; preset jit" : "features gc; preset jitless" }],
    provisional: true,
    provisional_reasons: ["host gha-win32-x64 is not a gate host"],
    workloads: workloads.map((name) => ({ name, harness: name === TS_WORKLOAD ? "octane-base" : "base" })),
    options: { rounds: PATHB_ROUNDS, warmup: 0 },
    M: { ...m, rounds: PATHB_ROUNDS, samples },
  };
}

const m2Leg = (name, opts) => ({ name, cpu: WIN_CPU, platform: "win32-x64", jit: m2RunView("jit", opts), jitless: m2RunView("jitless", opts), error: null });

/** The jit cells of a compare of 2 legs with the same data, through readM2Gate. */
function jitCells(opts) {
  const c = compare([m2Leg("w1", opts), m2Leg("w2", opts)]);
  return Object.fromEntries(c.cells.filter((x) => x.build === "jit").map((x) => [x.cell.split(":")[0], x]));
}

// M9-P1 Codex r1 F1: m2-exit pathb_form reads only the box intervals, so a reading that the
// gates refuse must not give one. The runs are provisional (a runner is not a gate host): that
// alone does not stop an interval.
test("compare through readM2Gate: a not-gate-host run gives intervals; void, gross-only and void-kernel readings give none", () => {
  const clean = jitCells();
  for (const item of ["item 2", "item 3", "item 6"]) {
    assert.equal(clean[item].result, "AGREE above", item);
    assert.equal(clean[item].box1.note, "net, provisional", item);
  }
  assert.equal(clean["item 1"].result, "NO AGREEMENT", "no QuickJS in the run");
  // A void kernel row (richards): the v8-v7 geomean is not void, but the gates refuse it.
  const kernel = jitCells({ noisy: ["richards"] });
  assert.equal(kernel["item 2"].result, "NO AGREEMENT");
  assert.equal(kernel["item 2"].box1.interval, null);
  assert.match(kernel["item 2"].box1.note, /^net, provisional; no interval: kernel richards: void \(IQR /);
  assert.equal(kernel["item 6"].result, "AGREE above", "splay is not the noisy kernel");
  // A void reading (splay): its interval is above 1.0, but it is void.
  const splay = jitCells({ noisy: ["splay"] });
  assert.equal(splay["item 6"].result, "NO AGREEMENT");
  assert.match(splay["item 6"].box1.note, /no interval: the per-round geomeans are void/);
  assert.equal(splay["item 2"].result, "NO AGREEMENT", "splay is a v8-v7 kernel too");
  // Net invalid in one round (deltablue): the reading falls back to the gross ratios.
  const gross = jitCells({ netBad: "deltablue" });
  assert.equal(gross["item 2"].result, "NO AGREEMENT");
  assert.match(gross["item 2"].box1.note, /^gross, provisional; no interval: no net ratio \(gross only\)/);
  assert.equal(gross["item 6"].result, "AGREE above");
  // A void TS ratio to node --jitless.
  const ts = jitCells({ noisy: [TS_WORKLOAD] });
  assert.equal(ts["item 3"].result, "NO AGREEMENT");
  assert.match(ts["item 3"].box1.note, /^net, provisional, void; no interval: the net ratio to node --jitless is void/);
  assert.equal(ts["item 2"].result, "AGREE above");
});

test("host: the runner host file from idle.mjs; a busy runner gives none", () => {
  assert.equal(parseIdle("# samples\nidle_baseline_load1 = 0.42\nload_source = \"typeperf\"\n"), 0.42);
  assert.equal(parseIdle("# NOT IDLE\n"), null);
  const info = { cpu: 'Odd "CPU" #1', cores: 4, platform: "win32-x64", osText: "Windows_NT 10.0.26100", source: "typeperf" };
  const seen = [];
  const r = host({ id: "gha-w1", minutes: 10, everyS: 120 }, { idle: (args) => (seen.push(args), { status: 0, stdout: "idle_baseline_load1 = 0.42\n" }), info });
  assert.deepEqual(seen, [["--minutes", "10", "--every-s", "120"]]);
  assert.equal(r.status, 0);
  const h = parseToml(r.text);
  assert.deepEqual(
    { id: h.id, cpu: h.cpu, cores: h.cores, platform: h.platform, gate_host: h.gate_host, idle: h.idle_baseline_load1, source: h.load_source },
    { id: "gha-w1", cpu: "Odd _CPU_ _1", cores: 4, platform: "win32-x64", gate_host: false, idle: 0.42, source: "typeperf" },
  );
  const busy = host({ id: "x", minutes: 10, everyS: 10 }, { idle: () => ({ status: 1, stdout: "# NOT IDLE: 2 sample(s) saw build or benchmark processes; no baseline.\n" }), info });
  assert.equal(busy.status, 3);
  assert.equal(busy.text, null);
  assert.match(busy.why, /NOT IDLE/);
  assert.match(hostToml({ ...info, id: "a", idle: 1, measured: "m" }), /^gate_host = false$/m);
});

test("options", () => {
  assert.throws(() => parseArgs([]), /host or pathb/);
  assert.throws(() => parseArgs(["host"]), /--out is required/);
  assert.throws(() => parseArgs(["pathb", "--out", "x"]), /at least one --leg/);
  assert.throws(() => parseArgs(["host", "--out", "x", "--leg", "d"]), /unknown option --leg for host/);
  assert.throws(() => parseArgs(["host", "--out", "x", "--minutes", "0"]), /needs a number > 0/);
  assert.deepEqual(parseArgs(["pathb", "--leg", "a", "--leg", "b", "--out", "c"]).legs, ["a", "b"]);
  assert.equal(parseArgs(["host", "--out", "h", "--every-s", "120"]).everyS, 120);
});

const skipFakes = process.platform === "win32" && "shell-script fake engines";

test("CLI: 2 both-builds legs of run.mjs (fake engines, 31 rounds) give a paired compare", { skip: skipFakes }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-exit-inputs-cli-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fake = path.join(ROOT, "tools/bench/cross/testdata/fake-engine.sh");
  for (const n of ["ep9", "ep9-jitless", "node"]) symlinkSync(fake, path.join(dir, n));
  writeFileSync(path.join(dir, "versions.txt"), `# host: ${platformId()}\nnode      fake 2  ${dir}/node\n`);
  const hostFile = path.join(dir, "host.toml");
  writeFileSync(hostFile, hostToml({ id: "gha-test", cpu: cpuModel(), cores: os.cpus().length, platform: `${process.platform}-${process.arch}`, osText: "test", idle: 1000, source: "loadavg", measured: "test" }));
  assert.equal(readHostFile(hostFile).gate_host, false);
  const legs = ["w1", "w2"].map((name) => {
    const d = path.join(dir, name);
    mkdirSync(d);
    const r = spawnSync(
      process.execPath,
      [
        path.join(ROOT, "tools/bench/cross/run.mjs"),
        "--ep9", path.join(dir, "ep9"), "--ep9-jitless", path.join(dir, "ep9-jitless"),
        "--engines", "ep9,ep9-jitless,node-jitless", "--shells", path.join(dir, "versions.txt"),
        "--workloads", "t0-loop", "--rounds", String(PATHB_ROUNDS), "--warmup", "0",
        "--settle-s", "0", "--after-s", "0", "--wait-min", "0", "--host", hostFile,
        "--out", path.join(d, "run-jit.json"), "--out-jitless", path.join(d, "run-jitless.json"),
      ],
      { encoding: "utf8", env: { ...process.env, FAKE_ENGINE_BUILD_ep9: "features jit; preset jit", FAKE_BUILD_ep9: `commit ${SHA} clean`, FAKE_BUILD_ep9_jitless: `commit ${SHA} clean` } },
    );
    assert.equal(r.status, 0, r.stderr);
    return d;
  });
  const out = path.join(dir, "compare.json");
  const md = path.join(dir, "compare.md");
  const r = spawnSync(process.execPath, [SCRIPT, "pathb", "--leg", legs[0], "--leg", legs[1], "--out", out, "--md", md], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  const c = JSON.parse(readFileSync(out, "utf8"));
  assert.deepEqual(c.boxes, ["w1", "w2"]);
  assert.equal(c.commit, SHA);
  assert.equal(c.platform, `${process.platform}-${process.arch}`);
  assert.deepEqual(c.checks.map((k) => [k.box, k.build, k.commit_ok, k.rounds, k.load.before.pass, k.load.after.pass]), [
    [1, "jit", true, 31, true, true],
    [1, "jitless", true, 31, true, true],
    [2, "jit", true, 31, true, true],
    [2, "jitless", true, 31, true, true],
  ]);
  // The fake run has no M2 workload: every cell has no interval.
  assert.equal(c.cells.length, 8);
  assert.ok(c.cells.every((x) => x.result === "NO AGREEMENT"));
  assert.ok(existsSync(md));
});

// Option B (EI2-R3-LINUX-PATHB-REQUIRED): m2-exit reads a windows compare with a `legs` block
// and no --linux-pathb as NO AGREEMENT; only an older compare (no `legs`: 2 boxes, no Linux x64
// legs in its run) keeps the Path B form. pathb-run-nolegs.json is pathb-win.json with the
// `legs` block cut, the input of that fallback.
test("compare: the m2-exit fixture pathb-run-nolegs.json (an older compare, no legs block)", () => {
  const gates = { w1: gateOf(1.0), w2: gateOf(1.05), w3: gateOf(1.06), w4: gateOf(1.01) };
  const readGate = (runs) => gates[runs[0].source.split("/")[0]];
  const c = compare([leg("w2"), leg("w1", { cpu: "Intel Xeon Platinum 8370C" }), leg("w3"), leg("w4", { cpu: "Intel Xeon Platinum 8370C" })], { readGate });
  assert.equal(c.legs.length, 4);
  delete c.legs;
  assert.deepEqual(c.boxes, ["w2", "w3"]);
  fixture("pathb-run-nolegs.json", c);
});

// EI2-R1: the host step writes the GitHub run id and attempt of the dispatch; off GitHub (a box
// run, a local run) the env has none, and the file has no such line (it is still a valid host file).
test("host: run_id and run_attempt from GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT, none off GitHub (EI2-R1)", (t) => {
  const info = { cpu: WIN_CPU, cores: 4, platform: "win32-x64", osText: "Windows_NT 10.0.26100", source: "typeperf" };
  const idle = () => ({ status: 0, stdout: "idle_baseline_load1 = 0.1\n" });
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-exit-inputs-host-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const read = (text) => {
    const f = path.join(dir, "host.toml");
    writeFileSync(f, text);
    return readHostFile(f);
  };
  const gh = host({ id: "gha-pathb-w1", minutes: 10, everyS: 120 }, { idle, info, env: { GITHUB_RUN_ID: "44568926404", GITHUB_RUN_ATTEMPT: "2" } });
  assert.equal(gh.status, 0);
  const h = read(gh.text);
  assert.deepEqual([h.id, h.run_id, h.run_attempt, h.gate_host], ["gha-pathb-w1", "44568926404", "2", false]);
  assert.match(gh.text, /^id = "gha-pathb-w1"\nrun_id = "44568926404"\nrun_attempt = "2"\ncpu = /m);
  for (const env of [{}, { GITHUB_RUN_ID: "", GITHUB_RUN_ATTEMPT: "" }]) {
    const box = host({ id: "box-1", minutes: 10, everyS: 10 }, { idle, info, env });
    const b = read(box.text);
    assert.equal(b.id, "box-1");
    assert.ok(!("run_id" in b) && !("run_attempt" in b), box.text);
    assert.doesNotMatch(box.text, /run_/);
  }
});

// EI2-R1, EI2-R3-LEG-SLOT: readLeg reads the id, run_id and run_attempt of the leg's host.toml,
// and the legs block copies them. A leg with no host.toml (an older leg) has none; a host.toml
// that cannot be read is a read error of the leg.
test("readLeg and the legs block: the host.toml id, run_id and run_attempt of each leg", (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-exit-inputs-ids-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const legDir = (name, hostText, runHost = "gha-win32-x64") => {
    const d = path.join(dir, name);
    mkdirSync(d);
    for (const b of BUILDS) writeFileSync(path.join(d, `run-${b}.json`), JSON.stringify({ ...runView(b), host: { ...runView(b).host, id: runHost } }));
    if (hostText !== null) writeFileSync(path.join(d, "host.toml"), hostText);
    return d;
  };
  const toml = (id, runId, runAttempt) => hostToml({ id, runId, runAttempt, cpu: WIN_CPU, cores: 4, platform: "win32-x64", osText: "t", idle: 0.1, source: "typeperf", measured: "t" });
  const w1 = readLeg(legDir("w1", toml("gha-pathb-w1", "1001", "1"), "gha-pathb-w1"));
  assert.deepEqual([w1.id, w1.runId, w1.runAttempt, w1.error], ["gha-pathb-w1", "1001", "1", null]);
  const box = readLeg(legDir("w2", toml("gha-pathb-w2"), "gha-pathb-w2"));
  assert.deepEqual([box.id, box.runId, box.runAttempt, box.error], ["gha-pathb-w2", null, null, null]);
  const old = readLeg(legDir("w3", null));
  assert.deepEqual([old.id, old.runId, old.runAttempt, old.error], [null, null, null, null]);
  const bad = readLeg(legDir("w4", "id = gha\n"));
  assert.match(bad.error, /^host\.toml: line 1: unsupported TOML value gha/);
  // Codex ei3-r1 F2: the run files of slot w1 copied under the host.toml of slot w2 are not that leg.
  const copy = readLeg(legDir("copy", toml("gha-pathb-w2", "1001", "1"), "gha-pathb-w1"));
  assert.match(copy.error, /^host\.toml id gha-pathb-w2: run-jit\.json names host gha-pathb-w1/);
  assert.equal(copy.id, null);
  const c = compare([w1, box, old], { readGate: () => gateOf(1.05) });
  assert.deepEqual(c.legs.map((l) => [l.leg, l.id, l.run_id, l.run_attempt]), [["w1", "gha-pathb-w1", "1001", "1"], ["w2", "gha-pathb-w2", undefined, undefined], ["w3", undefined, undefined, undefined]]);
  assert.deepEqual(Object.keys(c.legs[2]).slice(0, 2), ["leg", "cpu"], "no id keys on an older leg");
  assert.deepEqual(Object.keys(c.legs[0]).slice(0, 5), ["leg", "id", "run_id", "run_attempt", "cpu"]);
});

// The m2-exit fixtures of the run id and slot checks (pathb-run-*.json): the legs of
// pathb-win.json and pathb-gha-linux.json with the host.toml id of their matrix slot and the run
// id 1001 (attempt 1). Variants of the Linux x64 compare: run id 1002 (2 runs), l2 with no run
// id (a mixed input), l2 with the id of l1 (a doubled slot), l2 with the id gha-pathb-l3 (slot
// gha-pathb-l2 missing, an id that is not a slot).
test("compare: the m2-exit fixtures pathb-run-*.json (run ids and slot ids in the legs block)", () => {
  const gates = { w1: gateOf(1.0), w2: gateOf(1.05), w3: gateOf(1.06), w4: gateOf(1.01), l1: gateOf(1.1), l2: gateOf(1.1) };
  const readGate = (runs) => gates[runs[0].source.split("/")[0]];
  const ids = (l, id, runId = "1001", runAttempt = "1") => ({ ...l, id, runId, runAttempt });
  const intel = "Intel Xeon Platinum 8370C";
  const w = compare([ids(leg("w2"), "gha-pathb-w2"), ids(leg("w1", { cpu: intel }), "gha-pathb-w1"), ids(leg("w3"), "gha-pathb-w3"), ids(leg("w4", { cpu: intel }), "gha-pathb-w4")], { readGate });
  assert.deepEqual(w.legs.map((l) => [l.leg, l.id, l.run_id, l.run_attempt]), [
    ["w2", "gha-pathb-w2", "1001", "1"],
    ["w1", "gha-pathb-w1", "1001", "1"],
    ["w3", "gha-pathb-w3", "1001", "1"],
    ["w4", "gha-pathb-w4", "1001", "1"],
  ]);
  fixture("pathb-run-win.json", w);
  const lin = (name) => leg(name, { platform: "linux-x64" });
  const linux = (l1, l2) => compare([l1, l2], { readGate });
  fixture("pathb-run-linux.json", linux(ids(lin("l1"), "gha-pathb-l1"), ids(lin("l2"), "gha-pathb-l2")));
  fixture("pathb-run-linux-2.json", linux(ids(lin("l1"), "gha-pathb-l1", "1002"), ids(lin("l2"), "gha-pathb-l2", "1002")));
  const mixed = linux(ids(lin("l1"), "gha-pathb-l1"), ids(lin("l2"), "gha-pathb-l2", null, null));
  assert.deepEqual(mixed.legs.map((l) => "run_id" in l), [true, false]);
  fixture("pathb-run-linux-mixed.json", mixed);
  fixture("pathb-run-linux-doubled.json", linux(ids(lin("l1"), "gha-pathb-l1"), ids(lin("l2"), "gha-pathb-l1")));
  fixture("pathb-run-linux-missing.json", linux(ids(lin("l1"), "gha-pathb-l1"), ids(lin("l2"), "gha-pathb-l3")));
});
