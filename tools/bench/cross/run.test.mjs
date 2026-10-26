// node --test tools/bench/cross/run.test.mjs
// Fake engines (testdata/fake-engine.sh, one symlink per engine; testdata/fake-kernels-ep9.sh),
// synthetic samples and fixtures in testdata/: no shells and no ep9 build needed. The v8v7 and
// Octane tests skip when third_party/bench-v8v7 or third_party/bench-octane is not synced.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { cpuModel, guardAfter, guardBefore, loadCheck, parseToml, readHostFile } from "../lib/host.mjs";
import { medianRanks, verdict } from "../lib/stats.mjs";
import {
  analyze,
  checkRun,
  label,
  parseArgs,
  parseBenchJson,
  parseVersions,
  platformId,
  Refusal,
  renderMarkdown,
  schedule,
} from "./run.mjs";
import { readExit2, READINGS, renderMarkdown as renderExit2 } from "./exit2.mjs";
import { blockerOf, generateKernel, KERNEL_IDS, OK, parseArgs as kernelsArgs, PROBES, probeKeys, withProbe } from "./kernels.mjs";
import { generate as generateOctane, HARNESS as OCTANE_HARNESS, KERNELS as OCTANE_KERNELS } from "./octane.mjs";
import { benchmarkNames, DEFAULT_HARNESS, errorPlace, generate, HARNESSES, PRINT_PRELUDE } from "./v8v7.mjs";
import { EMPTY, expandWorkloads, materialize, OCTANE_7, SETS, V8V7_7, V8V7_8, WORKLOADS } from "./workloads.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(DIR, "../../..");
const RUN = path.join(DIR, "run.mjs");

// ---- statistics ---------------------------------------------------------------------------

test("n = 10: the interval is the 2nd and 9th smallest ratio", () => {
  assert.deepEqual(medianRanks(10), [2, 9]);
  // 1.0 .. 1.9, shuffled. Rank 1 (1.0) and rank 10 (1.9) are outside the interval.
  const xs = [1.9, 1.1, 1.5, 1.3, 1.7, 1.2, 1.6, 1.4, 1.8, 1.0];
  const v = verdict(xs, 1);
  assert.equal(v.n, 10);
  assert.deepEqual(v.interval, [1.1, 1.8]);
  assert.deepEqual(v.ranks, [2, 9]);
  assert.ok(Math.abs(v.median - 1.45) < 1e-12);
  // The threshold 1 equals the minimum, but the minimum is rank 1: still "above".
  assert.equal(v.side, "above");
  assert.equal(verdict(xs, 1.15).side, "marginal");
  assert.equal(verdict(xs, 1.85).side, "below");
});

test("void when the IQR is more than 5% of the median", () => {
  // n = 10, type-7 quartiles: Q1 = s[2] + 0.25 (s[3] - s[2]), Q3 = s[6] + 0.75 (s[7] - s[6]).
  const quiet = [0.9, 0.95, 0.975, 0.975, 1, 1, 1.0245, 1.0245, 1.05, 1.1]; // IQR 4.95%
  const noisy = [0.9, 0.95, 0.975, 0.975, 1, 1, 1.0255, 1.0255, 1.05, 1.1]; // IQR 5.05%
  const q = verdict(quiet);
  const n = verdict(noisy);
  assert.ok(Math.abs(q.iqrPct - 4.95) < 1e-9, `${q.iqrPct}`);
  assert.equal(q.void, false);
  assert.ok(Math.abs(n.iqrPct - 5.05) < 1e-9, `${n.iqrPct}`);
  assert.equal(n.void, true);
});

// ---- schedule (rotation) ------------------------------------------------------------------

test("rotation: workload order rotates, and each engine takes each position once per E rounds", () => {
  const engines = ["ep9", "qjs", "node-jitless"];
  const workloads = ["w1", "w2", EMPTY];
  const plan = schedule(engines, workloads, 6);
  assert.equal(plan.length, 6 * 3 * 3);
  for (let r = 0; r < 6; r++) {
    const round = plan.filter((p) => p.round === r);
    const order = [...new Set(round.map((p) => p.workload))];
    assert.deepEqual(order, workloads.map((_, i) => workloads[(i + r) % 3]), `round ${r}`);
    // Every workload runs on every engine once per round.
    for (const w of workloads) assert.deepEqual(round.filter((p) => p.workload === w).map((p) => p.engine).sort(), [...engines].sort());
  }
  for (const w of workloads) {
    for (let r0 = 0; r0 + 3 <= 6; r0++) {
      for (const e of engines) {
        const pos = [r0, r0 + 1, r0 + 2].map((r) => plan.filter((p) => p.round === r && p.workload === w).map((p) => p.engine).indexOf(e));
        assert.deepEqual(pos.sort(), [0, 1, 2], `${w} ${e} from round ${r0}`);
      }
    }
  }
  // The first engine differs between workloads of one round.
  const first = (r, w) => plan.find((p) => p.round === r && p.workload === w).engine;
  assert.notEqual(first(0, "w1"), first(0, "w2"));
});

// ---- analysis: net time, pairing, labels --------------------------------------------------

/** Synthetic samples: per round r, the empty script of engine e takes empty(e, r) and the
 * workload takes empty(e, r) + work(e). Round 0 is a warm-up with absurd times. */
function synth({ rounds = 10, emptyMs, workMs, workloads = ["k"], engines = ["ep9", "qjs"] }) {
  const s = [];
  for (let r = 0; r <= rounds; r++) {
    for (const e of engines) {
      const em = r === 0 ? 1e6 : emptyMs(e, r);
      s.push({ round: r, workload: EMPTY, engine: e, ms: em });
      for (const w of workloads) s.push({ round: r, workload: w, engine: e, ms: r === 0 ? 1 : em + workMs(e, r, w) });
    }
  }
  return s;
}

test("net time: each run minus the same engine's empty script in the same round", () => {
  // The empty-script time drifts by round and differs per engine; the work does not.
  const emptyMs = (e, r) => (e === "ep9" ? 5 + 10 * r : 3 + 7 * r);
  const workMs = (e) => (e === "ep9" ? 100 : 50);
  const m = analyze(synth({ emptyMs, workMs }), { subject: "ep9", comparators: ["qjs"], workloads: ["k"], warmup: 1 });
  assert.equal(m.rounds, 10);
  const row = m.ratios[0];
  assert.equal(row.n, 10);
  // Net: 50 / 100 in every round (a median empty time would not give exactly 0.5 each round).
  assert.deepEqual(row.net.ratios, Array(10).fill(0.5));
  assert.equal(row.net.median, 0.5);
  assert.equal(row.net.void, false);
  assert.equal(row.metric, "net");
  // ratio = t(qjs) / t(ep9) = 0.5: QuickJS takes half of ep9's time, so ep9 is slower.
  assert.equal(row.verdict, "ep9 slower");
  // Gross: (empty + work) per round, paired.
  assert.deepEqual(
    row.gross.ratios,
    Array.from({ length: 10 }, (_, i) => (3 + 7 * (i + 1) + 50) / (5 + 10 * (i + 1) + 100)),
  );
  const time = (w, e) => m.times.find((t) => t.workload === w && t.engine === e);
  assert.equal(time("k", "ep9").net_ms, 100);
  assert.equal(time("k", "qjs").net_ms, 50);
  assert.equal(time(EMPTY, "ep9").net_ms, null);
  assert.equal(m.geomean.qjs.net, 0.5);
});

test("net time <= 0 makes the net ratio invalid; the row falls back to gross", () => {
  const m = analyze(
    synth({ emptyMs: () => 10, workMs: (e, r) => (e === "ep9" && r === 4 ? 0 : 20) }),
    { subject: "ep9", comparators: ["qjs"], workloads: ["k"], warmup: 1 },
  );
  const row = m.ratios[0];
  assert.equal(row.net, null);
  assert.equal(row.net_invalid, "net time <= 0 in round(s) 4");
  assert.equal(row.metric, "gross");
  assert.equal(m.geomean.qjs.net, null);
});

test("a row with an invalid net time is provisional: the gross ratio passes or fails no gate", () => {
  // ep9: richards 99 ms, empty script 100 ms (net -1 ms in every round); QuickJS: 100 and 10 ms.
  const m = label(
    analyze(
      synth({ emptyMs: (e) => (e === "ep9" ? 100 : 10), workMs: (e) => (e === "ep9" ? -1 : 90), workloads: ["richards"] }),
      { subject: "ep9", comparators: ["qjs"], workloads: ["richards"], warmup: 1 },
    ),
    false,
  );
  const row = m.ratios[0];
  assert.equal(row.net, null);
  assert.equal(row.metric, "gross");
  // Gross 100 / 99: quiet, and its interval is above 0.6. It stays as a shown value only.
  assert.equal(row.void, false);
  assert.equal(row.verdict, "ep9 faster");
  assert.deepEqual(row.status, ["net invalid (net time <= 0 in round(s) 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)", "provisional"]);
  assert.equal(row.canary.side, "above");
  assert.equal(row.canary.gate, "no gate verdict (provisional; the gross interval is above 0.6)");

  // Markdown: no gross value under a "net ratio" heading.
  const md = renderMarkdown({
    label: null,
    started: "t0",
    finished: "t1",
    commit: { ep9: "0123456789abcdef" },
    host: { id: "test", idle_baseline_load1: 1, measured: { cpu: "c", cpus: 1, os: "o", platform: "p", node: "n" } },
    load: { before: null, after: null, after_each_round: [1] },
    provisional: false,
    provisional_reasons: [],
    options: { warmup: 1 },
    engines: [
      { id: "ep9", label: "ep9 T0", role: "subject", version: "x" },
      { id: "qjs", label: "QuickJS", role: "gate", version: "y" },
    ],
    M: m,
    R: null,
  });
  assert.ok(
    md.includes("| richards | QuickJS | invalid | none | 0.0 | 1.010 | 1.010-1.010 | ep9 faster (gross) | net invalid (net time <= 0 in round(s) 1, 2, 3, 4, 5, 6, 7, 8, 9, 10), provisional |\n"),
    md,
  );
  assert.ok(md.includes("| richards | invalid | none | no gate verdict (provisional; the gross interval is above 0.6) |\n"), md);
});

test("analyze refuses missing samples", () => {
  const s = synth({ emptyMs: () => 1, workMs: () => 1 }).filter((x) => !(x.round === 3 && x.engine === "qjs" && x.workload === "k"));
  assert.throws(() => analyze(s, { subject: "ep9", comparators: ["qjs"], workloads: ["k"], warmup: 1 }), /k on qjs has 9 of 10 samples/);
});

test("labels: provisional run, void rows, canary readings", () => {
  const noisy = (e, r) => (e === "qjs" ? 30 * (1 + 0.04 * r) : 100); // IQR well above 5%
  const mk = (workMs) =>
    analyze(synth({ emptyMs: () => 1, workMs, workloads: ["richards"] }), { subject: "ep9", comparators: ["qjs"], workloads: ["richards"], warmup: 1 });

  const q = label(mk((e) => (e === "qjs" ? 30 : 100)), false).ratios[0];
  assert.equal(q.void, false);
  assert.deepEqual(q.status, ["valid"]);
  assert.equal(q.canary.threshold, 0.6);
  assert.equal(q.canary.side, "below"); // 0.3 < 0.6
  assert.equal(q.canary.gate, "fail");
  assert.equal(label(mk((e) => (e === "qjs" ? 80 : 100)), false).ratios[0].canary.gate, "pass");

  const p = label(mk((e) => (e === "qjs" ? 30 : 100)), true).ratios[0];
  assert.deepEqual(p.status, ["provisional"]);
  assert.match(p.canary.gate, /^no gate verdict \(provisional; the interval is below 0.6\)$/);

  const v = label(mk(noisy), false).ratios[0];
  assert.equal(v.void, true);
  assert.equal(v.status.length, 2);
  assert.match(v.status[0], /^void \(IQR \d+\.\d% > 5%\)$/);
  assert.equal(v.status[1], "provisional");
  assert.match(v.canary.gate, /^no gate verdict/);
});

test("checkRun refuses a wrong result or a failed run", () => {
  const ok = { status: 0, signal: null, error: null, stdout: "604229\n", stderr: "" };
  checkRun(ok, { engine: "qjs", workload: "t0-loop", expect: "604229" });
  checkRun({ ...ok, stdout: "604229\r\n" }, { engine: "qjs", workload: "t0-loop", expect: "604229" });
  assert.throws(() => checkRun({ ...ok, stdout: "604228\n" }, { engine: "qjs", workload: "t0-loop", expect: "604229" }), (e) => e instanceof Refusal && /printed "604228", want "604229"/.test(e.message));
  assert.throws(() => checkRun({ ...ok, status: 1, stderr: "TypeError: x" }, { engine: "ep9", workload: "t0-loop", expect: "604229" }), /t0-loop on ep9: exit 1\n {4}TypeError: x/);
  assert.throws(() => checkRun({ ...ok, status: null, signal: "SIGSEGV" }, { engine: "ep9", workload: "t0-loop", expect: "604229" }), /signal SIGSEGV/);
});

// ---- inputs -------------------------------------------------------------------------------

test("each T0 workload's expected output equals the file's own `want`", () => {
  for (const [name, w] of Object.entries(WORKLOADS)) {
    if (!w.file) continue;
    const m = readFileSync(path.join(ROOT, w.file), "utf8").match(/^const want = "?([^";]*)"?;$/m);
    assert.ok(m, `${w.file}: no const want`);
    assert.equal(w.expect, m[1], name);
  }
});

test("versions.txt, host files and options", () => {
  const v = parseVersions(
    "# host: darwin_arm64\nnode      v24.22.0 v8=13.6.233.17-node.57  /x/node\nquickjs   2026-06-04 (3d5e064e)  /x/src/qjs\n",
  );
  assert.equal(v.host, "darwin_arm64");
  assert.deepEqual(v.shells.node, { version: "v24.22.0 v8=13.6.233.17-node.57", path: "/x/node" });
  assert.deepEqual(v.shells.quickjs, { version: "2026-06-04 (3d5e064e)", path: "/x/src/qjs" });
  assert.equal(platformId("darwin", "arm64"), "darwin_arm64");
  assert.equal(platformId("win32", "x64"), "windows_x64");

  const t = parseToml('id = "x" # c\n# comment\nn = 1.5\nb = true\na = [1, 2.5]\n[t]\ns = "y"\n');
  assert.deepEqual(t, { id: "x", n: 1.5, b: true, a: [1, 2.5], t: { s: "y" } });
  assert.throws(() => parseToml("x = bare"), /unsupported TOML value/);

  const h = readHostFile(path.join(ROOT, "tools/bench/hosts/m5max-macos.toml"));
  assert.equal(h.id, "m5max-macos");
  assert.equal(h.cpu, "Apple M5 Max");
  assert.equal(h.cores, 18);
  assert.equal(h.gate_host, true);
  // Measured by the orchestrator on 2026-10-21 (fa451f6): the guard limit is 2.505 + 0.5.
  assert.equal(h.idle_baseline_load1, 2.505);
  assert.equal(loadCheck(3, h.idle_baseline_load1).pass, true);
  assert.equal(loadCheck(3.1, h.idle_baseline_load1).pass, false);
  // No baseline: the guard cannot pass.
  assert.equal(loadCheck(0, undefined).pass, false);

  assert.throws(() => parseArgs(["--ep9", "x", "--rounds", "9"]), /--rounds must be an integer >= 10/);
  assert.equal(parseArgs(["--ep9", "x"]).rounds, 10);
  assert.throws(() => parseArgs(["--rounds", "10"]), /--ep9 is required/);
  assert.throws(() => parseArgs(["--ep9", "x", "--engines", "qjs,node-jitless"]), /must include ep9/);
  assert.throws(() => parseArgs(["--ep9", "x", "--engines", "ep9,bun"]), /unknown engine bun/);
  assert.throws(() => parseArgs(["--ep9", "x", "--workloads", "nope"]), /unknown workload nope/);
  // The default is v8v7.mjs DEFAULT_HARNESS (stub until MATH1 sets base; V9-D14).
  assert.equal(parseArgs(["--ep9", "x"]).harness, DEFAULT_HARNESS);
  assert.ok(HARNESSES.includes(DEFAULT_HARNESS));
  assert.equal(parseArgs(["--ep9", "x", "--harness", "stub"]).harness, "stub");
  assert.equal(parseArgs(["--ep9", "x", "--harness", "base"]).harness, "base");
  assert.throws(() => parseArgs(["--ep9", "x", "--harness", "real"]), /--harness must be one of stub, base/);
});

test("load guard: settle, poll up to the wait limit, check after", async () => {
  const slept = [];
  const sleep = async (s) => void slept.push(s);
  const seq = (xs) => () => xs.shift();
  let g = await guardBefore({ idleBaseline: 1, settleS: 60, waitS: 120, pollS: 30 }, { load: seq([5, 4, 1.4]), sleep });
  assert.deepEqual(slept, [60, 30, 30]);
  assert.equal(g.pass, true);
  assert.equal(g.limit, 1.5);
  assert.equal(g.waited_s, 60);
  slept.length = 0;
  g = await guardBefore({ idleBaseline: 1, settleS: 60, waitS: 45, pollS: 30 }, { load: seq([5, 4, 3, 2]), sleep });
  assert.deepEqual(slept, [60, 30, 15]);
  assert.equal(g.pass, false);
  assert.equal(g.why, "load 3 > 1.5");
  slept.length = 0;
  g = await guardBefore({ idleBaseline: undefined, settleS: 60, waitS: 1800 }, { load: () => 0.1, sleep });
  assert.deepEqual(slept, [60]); // no baseline: waiting cannot help
  assert.equal(g.why, "no idle baseline in the host file");
  slept.length = 0;
  g = await guardAfter({ idleBaseline: 2, afterS: 120 }, { load: () => 2.5, sleep });
  assert.deepEqual(slept, [120]);
  assert.equal(g.pass, true);
});

test("ep9-bench JSON", () => {
  const p = parseBenchJson(
    JSON.stringify({
      schema: "ep9-bench/v1",
      counter: "time",
      counter_status: "ok",
      counter_note: null,
      runs: [1, 2, 3].map((i) => ({ instructions: 100 * i, wall_ns: 1e6 * i, max_rss_bytes: 10 * i })),
      summary: { instructions: { min: 100, median: 200, max: 300 } },
    }),
  );
  assert.equal(p.median, 200);
  assert.deepEqual(p.instructions, [100, 200, 300]);
  assert.equal(p.max_rss_bytes, 20);
  assert.throws(() => parseBenchJson('{"schema":"x"}'), /not ep9-bench\/v1/);
});

// ---- the CLI with fake engines ------------------------------------------------------------

function fakeSetup(t) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-cross-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fake = path.join(DIR, "testdata/fake-engine.sh");
  for (const n of ["ep9", "qjs", "node"]) symlinkSync(fake, path.join(dir, n));
  writeFileSync(path.join(dir, "versions.txt"), `# host: ${platformId()}\nquickjs   fake 1  ${dir}/qjs\nnode      fake 2  ${dir}/node\n`);
  const host = (idle, gate = true) => {
    const f = path.join(dir, `host-${idle}-${gate}.toml`);
    writeFileSync(
      f,
      `id = "test"\ncpu = "${cpuModel()}"\ncores = ${os.cpus().length}\nplatform = "${process.platform}-${process.arch}"\nos = "test"\ngate_host = ${gate}\nidle_baseline_load1 = ${idle}\n`,
    );
    return f;
  };
  const out = path.join(dir, "out.json");
  const md = path.join(dir, "out.md");
  const run = (args, env = {}) =>
    spawnSync(
      process.execPath,
      [
        RUN,
        "--ep9", path.join(dir, "ep9"),
        "--shells", path.join(dir, "versions.txt"),
        "--engines", "ep9,qjs,node-jitless",
        "--workloads", "t0-loop,t0-call,ci-v0",
        "--settle-s", "0", "--after-s", "0", "--wait-min", "0",
        "--out", out, "--md", md,
        ...args,
      ],
      { encoding: "utf8", env: { ...process.env, ...env } },
    );
  return { dir, host, run, out, md };
}

test("CLI: rotated rounds as planned, 10 rounds, JSON with every sample", (t) => {
  const f = fakeSetup(t);
  const log = path.join(f.dir, "calls.log");
  const r = f.run(["--host", f.host(1000)], { FAKE_LOG: log });
  assert.equal(r.status, 0, r.stderr);
  const res = JSON.parse(readFileSync(f.out, "utf8"));
  assert.equal(res.schema, "ep9-cross/v1");
  assert.equal(res.provisional, false, res.provisional_reasons.join("; "));
  assert.deepEqual(res.provisional_reasons, []);
  assert.equal(res.load.before.pass, true);
  assert.equal(res.load.after.pass, true);
  assert.equal(res.load.after_each_round.length, 11);
  // The output check runs every file on every engine first; then the rounds, in plan order.
  const calls = readFileSync(log, "utf8").trim().split("\n");
  const names = { ep9: "ep9", qjs: "qjs", "node-jitless": "node" };
  const check = 4 * 3;
  const plan = schedule(["ep9", "qjs", "node-jitless"], ["t0-loop", "t0-call", EMPTY], 11);
  assert.deepEqual(calls.slice(check), plan.map((p) => `${names[p.engine]} ${p.workload}`));
  assert.deepEqual(res.M.samples.map((s) => [s.round, s.workload, s.engine]), plan.map((p) => [p.round, p.workload, p.engine]));
  assert.equal(res.M.samples.filter((s) => s.warmup).length, 9);
  // ci-v0 is class R only: it is checked but not timed.
  assert.ok(!res.M.samples.some((s) => s.workload === "ci-v0"));
  assert.ok(calls.slice(0, check).includes("qjs ci-v0"));
  // Ratio rows: 2 workloads x 2 comparators, each with n = 10, an interval and a verdict.
  assert.equal(res.M.ratios.length, 4);
  for (const row of res.M.ratios) {
    assert.equal(row.n, 10);
    assert.deepEqual((row.net ?? row.gross).ranks, [2, 9]);
    assert.equal((row.net ?? row.gross).interval.length, 2);
    assert.ok(["ep9 faster", "ep9 slower", "marginal"].includes(row.verdict), row.verdict);
    // A noisy (void) row, or one with an invalid net ratio, is provisional even in a run that
    // passed the load guard. The fake engines' net times are noise around 0, so most rows are.
    if (row.void || row.net_invalid) assert.ok(row.status.includes("provisional"));
    else assert.deepEqual(row.status, ["valid"]);
  }
  assert.ok(readFileSync(f.md, "utf8").includes("Gate run: the load guard passed"));
  // Neither canary workload ran: the table says so.
  assert.match(readFileSync(f.md, "utf8"), /\| richards \| not in this run \| - \| no reading \|\n\| deltablue \| not in this run/);
  assert.equal(r.stdout, readFileSync(f.md, "utf8"));
});

test("CLI: load guard failure is provisional with --allow-busy, and 'host busy' without", (t) => {
  const f = fakeSetup(t);
  const busy = f.run(["--host", f.host(-100)]);
  assert.equal(busy.status, 3, busy.stderr);
  assert.match(busy.stderr, /host busy: load [\d.]+ > -99.5/);
  assert.ok(!existsSync(f.out));

  const r = f.run(["--host", f.host(-100), "--allow-busy"]);
  assert.equal(r.status, 0, r.stderr);
  const res = JSON.parse(readFileSync(f.out, "utf8"));
  assert.equal(res.provisional, true);
  assert.match(res.provisional_reasons.join("; "), /load guard failed before the run \(load [\d.]+ > -99.5\)/);
  assert.match(res.provisional_reasons.join("; "), /load guard failed after the run/);
  for (const row of res.M.ratios) assert.equal(row.status[0], "provisional");
  assert.match(readFileSync(f.md, "utf8"), /\*\*PROVISIONAL\*\*: load guard failed before the run/);
});

test("CLI: no host file, or not a gate host, is provisional", (t) => {
  const f = fakeSetup(t);
  let r = f.run([]);
  assert.equal(r.status, 0, r.stderr);
  let res = JSON.parse(readFileSync(f.out, "utf8"));
  assert.equal(res.provisional, true);
  assert.deepEqual(res.provisional_reasons, ["no host file (not a gate host)"]);
  assert.equal(res.load.before, null);
  r = f.run(["--host", f.host(1000, false)]);
  assert.equal(r.status, 0, r.stderr);
  res = JSON.parse(readFileSync(f.out, "utf8"));
  assert.deepEqual(res.provisional_reasons, ["host test is not a gate host"]);
});

test("CLI: a wrong result or a failed run is refused, and no report is written", (t) => {
  const f = fakeSetup(t);
  let r = f.run([], { FAKE_WRONG: "qjs:t0-call" });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /REFUSED: t0-call on qjs: printed "12345", want "577789"/);
  assert.ok(!existsSync(f.out) && !existsSync(f.md));
  r = f.run([], { FAKE_FAIL: "ep9:t0-loop" });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /REFUSED: t0-loop on ep9: exit 3\n {4}fake failure/);
  // The empty script must print nothing.
  r = f.run([], { FAKE_WRONG: "node:empty" });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /REFUSED: empty on node-jitless: printed "12345", want ""/);
});

test("CLI: class R counts every R workload on every engine", (t) => {
  const f = fakeSetup(t);
  const r = f.run(["--count", path.join(DIR, "testdata/fake-bench.sh")]);
  assert.equal(r.status, 0, r.stderr);
  const res = JSON.parse(readFileSync(f.out, "utf8"));
  assert.equal(res.R.runs, 5);
  assert.equal(res.R.counter, "time");
  assert.deepEqual(
    res.R.rows.map((x) => `${x.workload}@${x.engine}`),
    ["t0-loop", "t0-call", "ci-v0"].flatMap((w) => ["ep9", "qjs", "node-jitless"].map((e) => `${w}@${e}`)),
  );
  for (const row of res.R.rows) assert.ok(row.median > 1000);
  assert.equal(res.R.expect_stdout, true);
  assert.equal(res.R.ep9_hash_seed, "1");
  assert.match(renderMarkdown(res), /Class R: instructions retired, .*every run's output checked; ep9 with EP9_HASH_SEED=1/);
});

test("CLI: class R gives ep9-bench the expected output and ep9 (only) the hash seed", (t) => {
  const f = fakeSetup(t);
  const log = path.join(f.dir, "bench.log");
  const r = f.run(["--count", path.join(DIR, "testdata/fake-bench.sh"), "--hash-seed", "7"], { FAKE_BENCH_LOG: log, EP9_HASH_SEED: "99" });
  assert.equal(r.status, 0, r.stderr);
  const calls = readFileSync(log, "utf8").trim().split("\n");
  assert.equal(calls.length, 9);
  for (const c of calls) {
    assert.match(c, /expect=yes$/, c);
    // ep9 gets --hash-seed; the other engines get no EP9_HASH_SEED at all.
    assert.match(c, c.startsWith("ep9 ") ? /seed=7 / : /seed=unset /, c);
  }
});

test("CLI: class R refuses a run that is right in the output check but wrong when counted", (t) => {
  // Negative control: before W4-P4-PERFREP-EXPECT, countAll gave ep9-bench no --expect-stdout, so
  // this run exited 0 and reported a count for a wrong answer.
  const f = fakeSetup(t);
  const r = f.run(["--count", path.join(DIR, "testdata/fake-bench.sh")], { FAKE_WRONG_COUNTED: "ep9:t0-call" });
  assert.equal(r.status, 1, r.stderr);
  assert.match(r.stderr, /REFUSED: count t0-call on ep9: a counted run printed the wrong output \(ep9-bench exit 4\)\n {4}fake-bench: run 0 printed "12345", want "577789"/);
  assert.ok(!existsSync(f.out) && !existsSync(f.md));
});

// ---- v8v7 ---------------------------------------------------------------------------------

const haveV8v7 = existsSync(path.join(ROOT, "third_party/bench-v8v7/bench/bench-v8/richards.js"));

test("v8v7: the generated kernel is the same on two runs and names the pin", { skip: !haveV8v7 && "third_party/bench-v8v7 not synced" }, () => {
  const pin = readFileSync(path.join(ROOT, "third_party/bench-v8v7.commit"), "utf8").trim();
  const a = generate("richards");
  const b = generate("richards");
  assert.equal(a.text, b.text);
  assert.equal(a.pin, pin);
  assert.ok(a.text.includes(`boa-dev/data at ${pin}`));
  assert.equal(a.n, 200);
  assert.equal(a.expect, "Richards 200");
  // Line L of the kernel is line L + offset of the generated file.
  const src = readFileSync(path.join(ROOT, "third_party/bench-v8v7/bench/bench-v8/richards.js"), "utf8").split("\n");
  assert.equal(a.text.split("\n")[66 - 1 + a.offset], src[66 - 1]);
  assert.equal(generate("crypto", { n: 3 }).expect, "Encrypt 3\nDecrypt 3");
});

test("v8v7: benchmark names", () => {
  assert.deepEqual(benchmarkNames("x = [\n  new Benchmark('A', f),\n  new Benchmark(\"B\", g)];\n[new Benchmark('NavierStokes',"), ["A", "B", "NavierStokes"]);
});

test("v8v7: the base harness runs the pinned base.js unchanged and prints what the stub prints", { skip: !haveV8v7 && "third_party/bench-v8v7 not synced" }, () => {
  assert.deepEqual(HARNESSES, ["stub", "base"]);
  assert.throws(() => generate("richards", { harness: "real" }), /unknown harness real/);
  const stub = generate("richards", { harness: "stub" });
  const g = generate("richards", { harness: "base" });
  assert.equal(stub.harness, "stub");
  assert.equal(g.harness, "base");
  assert.equal(g.expect, stub.expect);
  assert.equal(g.n, stub.n);
  // base.js goes in unchanged, as one string literal of an indirect eval (its own strict script).
  const base = readFileSync(path.join(ROOT, "third_party/bench-v8v7/bench/bench-v8/base.js"), "utf8");
  assert.ok(g.text.includes(`(0, eval)(${JSON.stringify(base + "\nglobalThis.Benchmark = Benchmark;\nglobalThis.BenchmarkResult = BenchmarkResult;\nglobalThis.BenchmarkSuite = BenchmarkSuite;\n")});`));
  assert.ok(g.text.includes("BenchmarkSuite.RunSuites({"));
  assert.ok(!stub.text.includes("BenchmarkSuite.RunSuites({"));
  // Line L of the kernel is line L + offset of the generated file.
  const src = readFileSync(path.join(ROOT, "third_party/bench-v8v7/bench/bench-v8/richards.js"), "utf8").split("\n");
  assert.equal(g.text.split("\n")[66 - 1 + g.offset], src[66 - 1]);
  // The generated script runs (node here) and prints the expected output, nothing else.
  const dir = mkdtempSync(path.join(os.tmpdir(), "v8v7-base-"));
  try {
    const f = path.join(dir, "deltablue.js");
    const d = generate("deltablue", { harness: "base", n: 2 });
    writeFileSync(f, d.text);
    const r = spawnSync(process.execPath, [f], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout.trim(), d.expect);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("v8v7: an error in the evaluated base.js maps to its base.js line", { skip: !haveV8v7 && "third_party/bench-v8v7 not synced" }, () => {
  const g = generate("richards", { harness: "base", n: 1 });
  const base = readFileSync(path.join(ROOT, "third_party/bench-v8v7/bench/bench-v8/base.js"), "utf8").split("\n");
  const at = base.findIndex((l) => l.includes("Math.log(")) + 1;
  assert.ok(at > 0);
  const e = errorPlace(g, "/x/richards.js", { status: 1, stdout: "", stderr: `<eval>:${at}:12: Uncaught TypeError: undefined is not a function\n    at x\n` });
  assert.deepEqual(e, { message: "Uncaught TypeError: undefined is not a function", place: `base.js:${at}`, line: base[at - 1].trim() });
  // A kernel line: the file as the engine was given it.
  const k = errorPlace(g, "/x/richards.js", { status: 1, stdout: "", stderr: `/x/richards.js:${66 + g.offset}:3: Uncaught Error: boom\n` });
  assert.equal(k.place, "richards.js:66");
  // Another engine's form stays as it is; a driver ERROR line counts when stderr is empty.
  assert.deepEqual(errorPlace(g, "/x/richards.js", { status: 1, stdout: "", stderr: "TypeError: not a function\n" }), { message: "TypeError: not a function", place: null, line: null });
  assert.equal(errorPlace(g, "/x/r.js", { status: 0, stdout: "ERROR Richards boom\nRichards 1\n", stderr: "" }).message, "ERROR Richards boom");
});

// ---- Octane-7 (octane.mjs) ----------------------------------------------------------------

const haveOctane = existsSync(path.join(ROOT, "third_party/bench-octane/richards.js"));

test("octane: Octane-7 kernels, the same N as v8-v7, the pinned base.js and kernel unchanged", { skip: !haveOctane && "third_party/bench-octane not synced" }, () => {
  assert.deepEqual(Object.keys(OCTANE_KERNELS), ["richards", "deltablue", "crypto", "raytrace", "earley-boyer", "splay", "navier-stokes"]);
  const pin = readFileSync(path.join(ROOT, "third_party/bench-octane.commit"), "utf8").trim();
  const a = generateOctane("crypto");
  assert.equal(a.text, generateOctane("crypto").text);
  assert.equal(a.pin, pin);
  assert.ok(a.text.includes(`chromium/octane at ${pin}`));
  assert.equal(a.harness, OCTANE_HARNESS);
  assert.equal(a.n, 8);
  assert.equal(a.expect, "Encrypt 8\nDecrypt 8");
  for (const p of a.parts) {
    // Each part is the pinned file, unchanged, at its offset.
    const src = readFileSync(path.join(ROOT, "third_party/bench-octane", p.file), "utf8");
    assert.equal(a.text.split("\n").slice(p.offset, p.offset + p.lines).join("\n"), src.replace(/\n$/, ""), p.file);
  }
  assert.equal(a.parts[1].offset, a.offset);
  assert.throws(() => generateOctane("regexp"), /unknown Octane-7 kernel regexp/);
  assert.throws(() => generateOctane("richards", { n: 0 }), /bad --n/);
});

test("octane: the driver runs base.js's deterministic mode N times and refuses a bad score", { skip: !haveOctane && "third_party/bench-octane not synced" }, () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "octane-"));
  try {
    const g = generateOctane("splay", { n: 3 });
    const counted = g.text.replace(/^BenchmarkSuite\.RunSuites\(/m, "var __runs = 0, __run = SplayRun; Splay.benchmarks[0].run = function () { __runs++; __run(); };\nBenchmarkSuite.RunSuites(") + 'print("runs " + __runs);\n';
    const f = path.join(dir, "splay.js");
    writeFileSync(f, counted);
    let r = spawnSync(process.execPath, [f], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    // Exactly N run() calls: no warm-up, no "for 1 s".
    assert.equal(r.stdout.trim(), "Splay 3\nruns 3");
    // Negative control: a Math.log that gives NaN makes the score NaN, and the driver prints ERROR.
    writeFileSync(f, g.text.replace(PRINT_PRELUDE, `${PRINT_PRELUDE}Math.log = function () { return NaN; };\n`));
    r = spawnSync(process.execPath, [f], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /^ERROR Splay score NaN$/m);
    // An error inside run() goes through base.js's NotifyError: an ERROR line, so a refusal.
    writeFileSync(f, g.text.replace(/^BenchmarkSuite\.RunSuites\(/m, 'Splay.benchmarks[0].run = function () { throw new Error("boom"); };\nBenchmarkSuite.RunSuites('));
    r = spawnSync(process.execPath, [f], { encoding: "utf8" });
    assert.match(r.stdout, /^ERROR Splay Error: boom$/m);
    assert.notEqual(r.stdout.trim(), g.expect);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("octane: an error line in base.js or in the kernel maps back to that file", { skip: !haveOctane && "third_party/bench-octane not synced" }, () => {
  const g = generateOctane("richards", { n: 1 });
  const base = readFileSync(path.join(ROOT, "third_party/bench-octane/base.js"), "utf8").split("\n");
  const at = base.findIndex((l) => l.includes("Math.log(measurements[i].time)")) + 1;
  const e = errorPlace(g, "/w/r.js", { status: 1, stdout: "", stderr: `/w/r.js:${at + g.parts[0].offset}:15: Uncaught TypeError: undefined is not a function\n` });
  assert.equal(e.place, `base.js:${at}`);
  assert.equal(e.line, base[at - 1].trim());
  const k = errorPlace(g, "/w/r.js", { status: 1, stdout: "", stderr: `/w/r.js:${40 + g.offset}:1: Uncaught Error: x\n` });
  assert.equal(k.place, "richards.js:40");
  // The driver after the kernel is "generated".
  const d = errorPlace(g, "/w/r.js", { status: 1, stdout: "", stderr: `/w/r.js:${g.text.split("\n").length - 2}:1: Uncaught Error: x\n` });
  assert.match(d.place, /^generated:\d+$/);
});

// ---- workloads and sets -------------------------------------------------------------------

test("workloads: the 8 v8-v7 kernels, Octane-7 and the exit-2 sets", () => {
  for (const [name, w] of Object.entries(WORKLOADS)) {
    assert.equal([w.file, w.v8v7, w.octane].filter(Boolean).length, 1, name);
  }
  assert.deepEqual(V8V7_8.map((w) => WORKLOADS[w].v8v7), V8V7_8);
  assert.deepEqual(V8V7_7, ["richards", "deltablue", "crypto", "raytrace", "earley-boyer", "splay", "navier-stokes"]);
  assert.deepEqual(OCTANE_7, V8V7_7.map((w) => `octane-${w}`));
  for (const w of [...V8V7_8, ...OCTANE_7]) assert.deepEqual(WORKLOADS[w].classes, ["M", "R"], w);
  assert.deepEqual(SETS.exit2, [...V8V7_7, ...OCTANE_7]);
  assert.deepEqual(expandWorkloads(["t0-loop", "exit2", "richards"]), ["t0-loop", ...V8V7_7, ...OCTANE_7]);
  assert.deepEqual(parseArgs(["--ep9", "x", "--workloads", "ci-v0,octane-7"]).workloads, ["ci-v0", ...OCTANE_7]);
  assert.throws(() => parseArgs(["--ep9", "x", "--workloads", "richards,richards"]), /--workloads has a duplicate/);
  // The exit-2 readings read these sets.
  assert.deepEqual(READINGS.map((r) => [r.workloads, r.comparator, r.threshold]), [[OCTANE_7, "qjs", 1.0], [V8V7_7, "node-jitless", 0.75]]);
});

test("workloads: an Octane workload is the octane.mjs file and names its harness", { skip: (!haveOctane || !haveV8v7) && "bench checkouts not synced" }, () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "wl-"));
  try {
    const m = materialize(["octane-deltablue", "deltablue"], dir, { harness: "stub" });
    assert.equal(m["octane-deltablue"].harness, OCTANE_HARNESS);
    assert.equal(m["octane-deltablue"].expect, "DeltaBlue 100");
    assert.equal(readFileSync(m["octane-deltablue"].file, "utf8"), generateOctane("deltablue").text);
    assert.equal(m.deltablue.harness, "stub");
    assert.match(m["octane-deltablue"].source, /^third_party\/bench-octane deltablue\.js$/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ---- exit2.mjs ----------------------------------------------------------------------------

const EXIT2 = path.join(DIR, "exit2.mjs");
const fixture = () => JSON.parse(readFileSync(path.join(DIR, "testdata/cross-exit2.json"), "utf8"));

test("exit2: per-round geomeans, intervals and gate readings from the fixture", () => {
  const e = readExit2(fixture());
  const [oct, v8] = e.readings;
  // Octane-7 vs QuickJS: about 1.2 in every round: pass.
  assert.equal(oct.status, "valid");
  assert.equal(oct.net.n, 10);
  assert.deepEqual(oct.net.ranks, [2, 9]);
  assert.ok(oct.net.interval[0] >= 1.0 && Math.abs(oct.net.median - 1.2) < 0.01, JSON.stringify(oct.net));
  assert.equal(oct.gate, "pass");
  assert.equal(oct.kernels.length, 7);
  // The per-round geomean of round r is the geomean of the 7 kernels' ratios of round r.
  const rows = fixture().M.ratios.filter((r) => r.comparator === "qjs" && OCTANE_7.includes(r.workload));
  const r3 = Math.exp(rows.reduce((a, r) => a + Math.log(r.net.ratios[3]), 0) / rows.length);
  assert.ok(Math.abs(oct.net.series[3] - r3) < 1e-12);
  // v8-v7 vs node --jitless: about 0.75, and the interval contains 0.75: marginal.
  assert.equal(v8.status, "valid");
  assert.ok(v8.net.interval[0] < 0.75 && v8.net.interval[1] >= 0.75, JSON.stringify(v8.net.interval));
  assert.equal(v8.gate, "marginal");
  assert.equal(e.pass, false);
  assert.match(renderExit2(e), /\| Octane-7 geomean vs QuickJS \| 1 \| 1\.200 \| .* \| pass \|/);
});

test("exit2: pass, fail, and no gate verdict for a provisional run, a stub harness or a void row", () => {
  // Scale every node --jitless ratio: 0.75 x 1.1 passes, 0.75 x 0.9 fails.
  const scaled = (k) => {
    const j = fixture();
    for (const r of j.M.ratios.filter((x) => x.comparator === "node-jitless")) {
      r.net.ratios = r.net.ratios.map((x) => x * k);
      r.net.median *= k;
    }
    return j;
  };
  let e = readExit2(scaled(1.1));
  assert.deepEqual(e.readings.map((r) => r.gate), ["pass", "pass"]);
  assert.equal(e.pass, true);
  e = readExit2(scaled(0.9));
  assert.equal(e.readings[1].gate, "fail");

  let j = scaled(1.1);
  j.provisional = true;
  j.provisional_reasons = ["host x is not a gate host"];
  e = readExit2(j);
  assert.equal(e.pass, false);
  assert.equal(e.readings[0].status, "provisional");
  assert.match(e.readings[0].gate, /^no gate verdict \(the interval reads pass\)$/);
  assert.match(e.readings[0].reasons[0], /provisional run \(host x is not a gate host\)/);

  // V9-D14: the v8-v7 reading needs the real base.js.
  j = scaled(1.1);
  for (const w of j.workloads) if (w.harness === "base") w.harness = "stub";
  e = readExit2(j);
  assert.equal(e.readings[0].gate, "pass");
  assert.match(e.readings[1].gate, /^no gate verdict/);
  assert.match(e.readings[1].reasons.join(), /harness is not base for richards \(stub\)/);

  // A void kernel row (run.mjs labels it) gives no gate verdict.
  j = scaled(1.1);
  j.M.ratios.find((r) => r.workload === "octane-splay" && r.comparator === "qjs").status = ["void (IQR 6.0% > 5%)", "provisional"];
  e = readExit2(j);
  assert.match(e.readings[0].gate, /^no gate verdict/);
  assert.match(e.readings[0].reasons.join(), /row not valid: octane-splay/);

  // A net-invalid row: gross geomean shown, no gate verdict.
  j = scaled(1.1);
  const row = j.M.ratios.find((r) => r.workload === "crypto" && r.comparator === "node-jitless");
  row.net = null;
  row.net_invalid = "net time <= 0 in round(s) 3";
  row.status = ["net invalid (net time <= 0 in round(s) 3)", "provisional"];
  e = readExit2(j);
  assert.equal(e.readings[1].net, null);
  assert.ok(e.readings[1].gross);
  assert.match(e.readings[1].gate, /^no gate verdict \(the gross interval reads/);

  // A missing kernel: incomplete.
  j = scaled(1.1);
  j.M.ratios = j.M.ratios.filter((r) => r.workload !== "octane-raytrace");
  e = readExit2(j);
  assert.equal(e.readings[0].status, "incomplete");
  assert.match(e.readings[0].reasons[0], /no qjs row for octane-raytrace/);
});

test("exit2: fewer than 10 paired rounds, or series shorter than the run says, give no gate verdict", () => {
  const passing = () => {
    const j = fixture();
    for (const r of j.M.ratios.filter((x) => x.comparator === "node-jitless")) r.net.ratios = r.net.ratios.map((x) => x * 1.1);
    return j;
  };
  assert.equal(readExit2(passing()).pass, true);
  const cut = (j, n) => {
    for (const r of j.M.ratios) {
      r.net.ratios = r.net.ratios.slice(0, n);
      r.gross.ratios = r.gross.ratios.slice(0, n);
    }
    return j;
  };
  // Every series cut to 6 (verdict() still gives an interval at n = 6) while the run says 10.
  let e = readExit2(cut(passing(), 6));
  assert.equal(e.pass, false);
  for (const r of e.readings) {
    assert.match(r.gate, /^no gate verdict/);
    assert.match(r.reasons.join(), /ratio series not 10 long: .*\(6\/6\)/);
  }
  // The same 6 rounds, declared as 6: below the class M minimum.
  let j = cut(passing(), 6);
  j.M.rounds = 6;
  j.options.rounds = 6;
  e = readExit2(j);
  assert.equal(e.pass, false);
  assert.match(e.readings[0].reasons.join(), /the run has 6 paired rounds \(M\.rounds\); class M needs >= 10/);
  // One kernel's series one short.
  j = passing();
  const row = j.M.ratios.find((r) => r.workload === "octane-splay" && r.comparator === "qjs");
  row.net.ratios.pop();
  e = readExit2(j);
  assert.match(e.readings[0].gate, /^no gate verdict/);
  assert.match(e.readings[0].reasons.join(), /ratio series not 10 long: octane-splay \(9\/10\)/);
  assert.equal(e.readings[1].gate, "pass");
  // M.rounds and options.rounds disagree.
  j = passing();
  j.options.rounds = 12;
  e = readExit2(j);
  assert.equal(e.pass, false);
  assert.match(e.readings[1].reasons.join(), /M\.rounds \(10\) differs from options\.rounds \(12\)/);
});

test("exit2: CLI exit status and JSON", (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "exit2-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const out = path.join(dir, "e.json");
  let r = spawnSync(process.execPath, [EXIT2, path.join(DIR, "testdata/cross-exit2.json"), "--out", out], { encoding: "utf8" });
  assert.equal(r.status, 1, r.stderr);
  assert.match(r.stdout, /Exit 2: not passed\./);
  const e = JSON.parse(readFileSync(out, "utf8"));
  assert.equal(e.schema, "ep9-exit2/v1");
  assert.equal(e.commit, "0123456789abcdef0123456789abcdef01234567");
  const pass = fixture();
  for (const x of pass.M.ratios.filter((y) => y.comparator === "node-jitless")) x.net.ratios = x.net.ratios.map((v) => v * 1.1);
  const f = path.join(dir, "pass.json");
  writeFileSync(f, JSON.stringify(pass));
  r = spawnSync(process.execPath, [EXIT2, f], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /Exit 2: PASS/);
  writeFileSync(f, JSON.stringify({ schema: "ep9-bench/v1" }));
  r = spawnSync(process.execPath, [EXIT2, f], { encoding: "utf8" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /not an ep9-cross\/v1 run/);
  r = spawnSync(process.execPath, [EXIT2], { encoding: "utf8" });
  assert.equal(r.status, 2);
});

// ---- kernels.mjs --------------------------------------------------------------------------

const KERNELS_CLI = path.join(DIR, "kernels.mjs");

test("kernels: ids, options and blocker names", () => {
  assert.equal(KERNEL_IDS.length, 15);
  assert.deepEqual(KERNEL_IDS.filter((k) => k.startsWith("v8v7/")).map((k) => k.slice(5)), V8V7_8);
  assert.deepEqual(KERNEL_IDS.filter((k) => k.startsWith("octane/")).map((k) => `octane-${k.slice(7)}`), OCTANE_7);
  assert.throws(() => kernelsArgs([]), /--check ENGINE is required/);
  assert.throws(() => kernelsArgs(["--check", "x", "--not-yet", "v8v7/nope"]), /unknown kernel v8v7\/nope/);
  assert.throws(() => kernelsArgs(["--check", "x", "--harness", "real"]), /--harness must be one of/);
  assert.equal(kernelsArgs(["--check", "x"]).harness, DEFAULT_HARNESS);
  assert.equal(kernelsArgs(["--check", "x", "--out", "-"]).out, "-");
  // The d7cc797 blockers (lang-bench section 1.1), from ep9's messages.
  const ep9Math = ["abs", "floor", "pow", "max"];
  assert.equal(blockerOf("Uncaught Error: not implemented in ep9 v0: ** with a result that is not an exact integer: needs the correctly rounded pow core (H36, month 2)", "x", ep9Math), "pow core (Math.pow and ** with a non-integer result; H36)");
  assert.equal(blockerOf("Uncaught Error: not implemented in ep9 v0: RegExp literals", "var re0 = /^ba/;", ep9Math), "RegExp");
  assert.equal(blockerOf("Uncaught TypeError: undefined is not a function", "log += Math.log(measurements[i].time);", ep9Math), "Math.log");
  // Of the Math calls on the line, only the ones the engine lacks.
  assert.equal(blockerOf("Uncaught TypeError: undefined is not a function", "t = Math.floor(65536 * Math.random());", ep9Math), "Math.random");
  assert.equal(blockerOf("Uncaught TypeError: undefined is not a function", "t = Math.floor(65536 * Math.random());", null), "Math.floor, Math.random");
  assert.equal(blockerOf("Uncaught TypeError: x is not a function", "foo();", ep9Math), null);
});

test("kernels: --check runs all 15 on the oracle and the engine; JSON for m1-exit", { skip: (!haveOctane || !haveV8v7) && "bench checkouts not synced" }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "kernels-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fake = path.join(DIR, "testdata/fake-kernels-ep9.sh");
  const out = path.join(dir, "k.json");
  const run = (args, env = {}) =>
    spawnSync(process.execPath, [KERNELS_CLI, "--check", fake, "--harness", "stub", "--out", out, ...args], {
      encoding: "utf8",
      env: { ...process.env, FAKE_NODE: process.execPath, ...env },
    });

  // All 15 as node: exit 0.
  let r = run([]);
  assert.equal(r.status, 0, r.stderr + r.stdout);
  let j = JSON.parse(readFileSync(out, "utf8"));
  assert.equal(j.schema, "ep9-kernels/v1");
  assert.deepEqual(j.kernels.map((k) => k.id), KERNEL_IDS);
  for (const k of j.kernels) {
    assert.equal(k.status, OK, k.id);
    assert.equal(k.output, k.oracle_output);
    assert.equal(k.n, 1);
  }
  assert.deepEqual(j.summary, { total: 15, ok: 15, failing: [], failing_not_named: [], all_ok: true });
  assert.equal(j.harness.v8v7, "stub");
  assert.equal(j.harness.octane, OCTANE_HARNESS);
  assert.deepEqual(Object.keys(j.tree), ["crates", "tools/bench", "Cargo.lock", "rust-toolchain.toml"]);
  assert.ok(Array.isArray(j.dirty));
  assert.equal(j.ep9.version, "fake-ep9 0.0");
  assert.match(r.stdout, /15\/15 kernels run with node's output \(v8-v7 8\/8, Octane-7 7\/7\)\./);

  // ep9's error form in base.js, and a wrong output: exit 1, blocker named, place mapped.
  const at = generateKernel("octane/richards", { n: 1, harness: "stub" }).parts[0].offset + 203;
  r = run([], { FAKE_FAIL: `octane-richards:${at}`, FAKE_WRONG: "v8v7-richards" });
  assert.equal(r.status, 1, r.stderr);
  j = JSON.parse(readFileSync(out, "utf8"));
  const oct = j.kernels.find((k) => k.id === "octane/richards");
  assert.equal(oct.status, "fails");
  assert.equal(oct.place, "base.js:203");
  assert.equal(oct.blocker, "Math.log");
  const v8 = j.kernels.find((k) => k.id === "v8v7/richards");
  assert.equal(v8.status, "output differs");
  assert.match(v8.error, /printed "Richards 2", node printed "Richards 1"/);
  assert.deepEqual(j.summary.failing, ["v8v7/richards", "octane/richards"]);
  assert.match(r.stdout, /\| octane\/richards \| fails: Uncaught TypeError: undefined is not a function \| Math\.log \| base\.js:203 \| `log \+= Math\.log\(measurements\[i\]\.time\);` \|/);

  // A named NOT-YET list: the same failures exit 0, and stay in the JSON.
  r = run(["--not-yet", "v8v7/richards,octane/richards"], { FAKE_FAIL: `octane-richards:${at}`, FAKE_WRONG: "v8v7-richards" });
  assert.equal(r.status, 0, r.stderr);
  j = JSON.parse(readFileSync(out, "utf8"));
  assert.deepEqual(j.summary.failing_not_named, []);
  assert.equal(j.summary.all_ok, false);

  // The oracle must print the generator's output: no verdict (exit 3) otherwise.
  r = spawnSync(process.execPath, [KERNELS_CLI, "--check", fake, "--node", fake, "--out", out], {
    encoding: "utf8",
    env: { ...process.env, FAKE_NODE: process.execPath, FAKE_WRONG: "v8v7-richards" },
  });
  assert.equal(r.status, 3, r.stderr);
  assert.match(r.stderr, /no verdict: v8v7\/richards on the oracle .*printed "Richards 2"/);
});

test("kernels: result probes for regexp and navier-stokes, between the kernel and the driver", { skip: (!haveOctane || !haveV8v7) && "bench checkouts not synced" }, () => {
  assert.deepEqual(Object.keys(PROBES).sort(), ["navier-stokes", "regexp"]);
  for (const id of KERNEL_IDS) {
    for (const harness of HARNESSES) {
      const g = generateKernel(id, { n: 2, harness });
      const p = withProbe(g);
      if (!PROBES[g.kernel]) {
        // No probe: the file is the generator's.
        assert.equal(p.text, g.text, id);
        assert.equal(p.probe, null);
        continue;
      }
      assert.equal(p.probe, PROBES[g.kernel].what);
      // base.js and the kernel keep their lines; the probe comes after the kernel, before the driver.
      const lines = p.text.split("\n");
      for (const part of g.parts) assert.deepEqual(lines.slice(part.offset, part.offset + part.lines), g.text.split("\n").slice(part.offset, part.offset + part.lines), `${id} ${part.file}`);
      const end = lines.indexOf(`// ---- end of ${g.source} ----`);
      assert.equal(lines[end + 1], "// ---- kernels.mjs result probe (check runs only) ----", id);
      assert.ok(p.text.endsWith(g.text.slice(g.text.indexOf(`// ---- end of ${g.source} ----\n`) + `// ---- end of ${g.source} ----\n`.length)), id);
      assert.deepEqual(probeKeys(p), g.expect.split("\n").flatMap((l) => [1, 2].map((i) => `check ${l.replace(/ 2$/, "")} ${i}`)));
    }
  }
});

test("kernels: the probes catch a matcher that never matches and a solver that does no work", { skip: (!haveOctane || !haveV8v7) && "bench checkouts not synced" }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "kernels-probe-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fake = path.join(DIR, "testdata/fake-kernels-ep9.sh");
  const env = (fault) => ({ ...process.env, FAKE_NODE: process.execPath, FAKE_FAULT: fault });
  // The bug the probes close: without them, each fault still prints the harness's expected lines.
  for (const [id, fault] of [["v8v7/regexp", "exec-null"], ["v8v7/regexp", "replace-same"], ["octane/navier-stokes", "no-update"], ["v8v7/navier-stokes", "no-update"]]) {
    const g = generateKernel(id, { n: 1, harness: "base" });
    const name = id.replace("/", "-");
    const f = path.join(dir, `${name}.js`);
    writeFileSync(f, g.text);
    const r = spawnSync(fake, [f], { encoding: "utf8", env: env(`${name}:${fault}`) });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout.trim(), g.expect, `${id} ${fault}`);
  }
  const out = path.join(dir, "k.json");
  const check = (fault) => {
    const r = spawnSync(process.execPath, [KERNELS_CLI, "--check", fake, "--harness", "base", "--out", out], { encoding: "utf8", env: env(fault) });
    return { r, j: JSON.parse(readFileSync(out, "utf8")) };
  };
  let { r, j } = check("v8v7-regexp:exec-null,octane-navier-stokes:no-update,v8v7-navier-stokes:no-update");
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(j.summary.failing, ["v8v7/regexp", "v8v7/navier-stokes", "octane/navier-stokes"]);
  for (const id of j.summary.failing) {
    const k = j.kernels.find((x) => x.id === id);
    assert.equal(k.status, "output differs", id);
    assert.match(k.oracle_output, /^check \w+ 1 -?\d+$/m);
    assert.ok(k.probe);
  }
  // Every other kernel: output equal, no probe.
  for (const k of j.kernels.filter((x) => !j.summary.failing.includes(x.id))) {
    assert.equal(k.status, OK, k.id);
    assert.equal(k.probe, PROBES[k.kernel]?.what ?? null);
  }
  ({ r, j } = check("v8v7-regexp:replace-same"));
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(j.summary.failing, ["v8v7/regexp"]);
});

test("kernels: the probes catch NaN, infinities, undefined and 32-bit wraps in the grids, and typed regexp results", { skip: (!haveOctane || !haveV8v7) && "bench checkouts not synced" }, (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "kernels-probe-nf-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fake = path.join(DIR, "testdata/fake-kernels-ep9.sh");
  const env = (fault) => ({ ...process.env, FAKE_NODE: process.execPath, FAKE_FAULT: fault });
  const faults = [
    ...["v8v7/navier-stokes", "octane/navier-stokes"].flatMap((id) =>
      ["dens", "xvel", "yvel"].flatMap((grid) => ["nan", "inf", "ninf", "undef", "wrap"].map((bad) => [id, `${grid}-${bad}`]))),
    ["v8v7/regexp", "exec-numstr"],
  ];
  for (const [id, fault] of faults) {
    const g = generateKernel(id, { n: 1, harness: "base" });
    const name = id.replace("/", "-");
    // Without the probe, the regexp fault still prints the harness's lines. (Without it nothing
    // reads the grids: setup's display callback is a no-op, so a grid fault changes no cell.)
    let r;
    if (id.endsWith("/regexp")) {
      const plain = path.join(dir, `${name}.js`);
      writeFileSync(plain, g.text);
      r = spawnSync(fake, [plain], { encoding: "utf8", env: env(`${name}:${fault}`) });
      assert.equal(r.status, 0, `${id} ${fault}: ${r.stderr}`);
      assert.equal(r.stdout.trim(), g.expect, `${id} ${fault}`);
    }
    // With it, the probe line differs from node's (the --check comparison of one kernel).
    const probed = path.join(dir, "probed", `${name}.js`);
    mkdirSync(path.dirname(probed), { recursive: true });
    writeFileSync(probed, withProbe(g).text);
    const node = spawnSync(process.execPath, [probed], { encoding: "utf8" });
    assert.equal(node.status, 0, node.stderr);
    r = spawnSync(fake, [probed], { encoding: "utf8", env: env(`${name}:${fault}`) });
    assert.equal(r.status, 0, `${id} ${fault}: ${r.stderr}`);
    const probeLine = (s) => s.split("\n").filter((l) => l.startsWith("check "));
    assert.equal(probeLine(r.stdout).length, 1, `${id} ${fault}`);
    assert.notDeepEqual(probeLine(r.stdout), probeLine(node.stdout), `${id} ${fault}`);
    assert.equal(r.stdout.split("\n").filter((l) => !l.startsWith("check ")).join("\n").trim(), g.expect, `${id} ${fault}`);
  }
  // The gate: --check reports them as "output differs" and exits 1.
  const out = path.join(dir, "k.json");
  const r = spawnSync(process.execPath, [KERNELS_CLI, "--check", fake, "--harness", "base", "--out", out], {
    encoding: "utf8",
    env: env("v8v7-regexp:exec-numstr,v8v7-navier-stokes:dens-nan,octane-navier-stokes:xvel-inf"),
  });
  assert.equal(r.status, 1, r.stderr);
  const j = JSON.parse(readFileSync(out, "utf8"));
  assert.deepEqual(j.summary.failing, ["v8v7/regexp", "v8v7/navier-stokes", "octane/navier-stokes"]);
  for (const id of j.summary.failing) assert.equal(j.kernels.find((x) => x.id === id).status, "output differs", id);
});
