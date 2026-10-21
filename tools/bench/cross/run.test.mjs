// node --test tools/bench/cross/run.test.mjs
// Fake engines (testdata/fake-engine.sh, one symlink per engine) and synthetic samples: no shells
// and no ep9 build needed. The v8v7 tests skip when third_party/bench-v8v7 is not synced.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
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
import { benchmarkNames, generate } from "./v8v7.mjs";
import { EMPTY, WORKLOADS } from "./workloads.mjs";

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
  // Not measured yet: the guard cannot pass.
  assert.equal(h.idle_baseline_load1, undefined);
  assert.equal(loadCheck(1, h.idle_baseline_load1).pass, false);

  assert.throws(() => parseArgs(["--ep9", "x", "--rounds", "9"]), /--rounds must be an integer >= 10/);
  assert.equal(parseArgs(["--ep9", "x"]).rounds, 10);
  assert.throws(() => parseArgs(["--rounds", "10"]), /--ep9 is required/);
  assert.throws(() => parseArgs(["--ep9", "x", "--engines", "qjs,node-jitless"]), /must include ep9/);
  assert.throws(() => parseArgs(["--ep9", "x", "--engines", "ep9,bun"]), /unknown engine bun/);
  assert.throws(() => parseArgs(["--ep9", "x", "--workloads", "nope"]), /unknown workload nope/);
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
  assert.match(renderMarkdown(res), /Class R: instructions retired/);
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
