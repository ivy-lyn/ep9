// node --test tools/bench/cross/mem.test.mjs (run.test.mjs imports this file, so the CI step
// `node --test tools/bench/cross/run.test.mjs` runs these tests too).
// Tests of mem.mjs (the G7 peak RSS reader) with fake engines (testdata/fake-engine.sh) and a
// fake time tool (FAKE_TIME_SH below): no shells and no ep9 build needed. Unix only (shell
// scripts), as run.test.mjs.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { cpuModel } from "../lib/host.mjs";
import { analyze, CAPS, checkMemRun, MIN_RUNS, parseArgs, parseGnuM, parseTimeL, renderMarkdown, rssTool, run, SCHEMA } from "./mem.mjs";
import { platformId, Refusal, schedule } from "./run.mjs";
import { SETS, V8V7_7, WORKLOADS } from "./workloads.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const MEM = path.join(DIR, "mem.mjs");
const FAKE = path.join(DIR, "testdata/fake-engine.sh");
// A fake /usr/bin/time (EP9_MEM_TIME). The test writes it into its temp dir.
const FAKE_TIME_SH = `#!/bin/sh
# Fake /usr/bin/time for mem.test.mjs. Forms: \`fake-time.sh -l -o FILE cmd...\` (macOS) and
# \`fake-time.sh -f %M -o FILE cmd...\` (GNU). It runs cmd, then writes the peak RSS to FILE in the
# form of the flag: FAKE_RSS_<name> bytes, where <name> is the base name of cmd (default
# 104857600). FAKE_NO_RSS=1 writes no RSS line.
mode=$1
shift
if [ "$mode" = "-f" ]; then shift; fi
[ "$1" = "-o" ] || exit 64
out=$2
shift 2
"$@"
st=$?
name=$(basename "$1" | tr -c 'A-Za-z0-9\\n' '_')
eval "rss=\\\${FAKE_RSS_$name:-104857600}"
if [ -n "$FAKE_NO_RSS" ]; then
  : > "$out"
elif [ "$mode" = "-l" ]; then
  printf '        0.10 real         0.05 user         0.01 sys\\n%20s  maximum resident set size\\n' "$rss" > "$out"
else
  echo $((rss / 1024)) > "$out"
fi
exit $st
`;
const skip = process.platform === "win32" && "shell-script fakes";

test("RSS tools: macOS time -l in bytes, GNU time %M in KiB", () => {
  const mac = "        0.27 real         0.20 user         0.02 sys\n            51462144  maximum resident set size\n                   0  average shared memory size\n";
  assert.equal(parseTimeL(mac), 51462144);
  assert.equal(parseTimeL("no rss here\n"), null);
  // GNU time prints "Command exited with non-zero status 1" before the value on a failure.
  assert.equal(parseGnuM("Command exited with non-zero status 1\n50256\n"), 50256 * 1024);
  assert.equal(parseGnuM(""), null);
  assert.equal(rssTool("darwin", {}).name, "macos-time-l-maximum-resident-set-size");
  assert.deepEqual(rssTool("linux", {}).args("f"), ["-f", "%M", "-o", "f"]);
  assert.equal(rssTool("linux", { EP9_MEM_TIME: "/x/time" }).program, "/x/time");
  assert.equal(rssTool("win32", {}), null);
});

test("options: the mem set is the 7 v8-v7 kernels; >= 5 runs; ep9-jitless needs its binary", () => {
  assert.deepEqual(SETS.mem, V8V7_7);
  const o = parseArgs(["--ep9", "x"]);
  assert.deepEqual(o.workloads, V8V7_7);
  assert.deepEqual(o.engines, ["ep9", "node-jitless", "d8-jitless"]);
  assert.equal(o.runs, MIN_RUNS);
  assert.deepEqual(parseArgs(["--ep9", "x", "--ep9-jitless", "y"]).engines, ["ep9", "ep9-jitless", "node-jitless", "d8-jitless"]);
  assert.throws(() => parseArgs(["--ep9", "x", "--runs", "4"]), /--runs must be an integer >= 5/);
  assert.throws(() => parseArgs(["--ep9", "x", "--engines", "ep9,ep9-jitless"]), /needs --ep9-jitless/);
  assert.throws(() => parseArgs(["--ep9", "x", "--engines", "node-jitless"]), /must include ep9/);
  assert.throws(() => parseArgs([]), /--ep9 is required/);
  assert.deepEqual(CAPS.map((c) => [c.comparator, c.cap]), [["node-jitless", 1.2], ["d8-jitless", 1.8]]);
});

/** Samples: rss[engine][workload] = list of values per round. */
function samples(rss) {
  const out = [];
  for (const [e, ws] of Object.entries(rss)) {
    for (const [w, xs] of Object.entries(ws)) xs.forEach((x, round) => out.push({ round, workload: w, engine: e, rss_bytes: x }));
  }
  return out;
}

test("analyze: medians, per-workload ratios, per-round ratios, and the caps per ep9 build", () => {
  const M = 1 << 20;
  const s = samples({
    ep9: { a: [110, 100, 120, 100, 100].map((x) => x * M), b: [300, 300, 300, 300, 300].map((x) => x * M) },
    "ep9-jitless": { a: [100, 100, 100, 100, 100].map((x) => x * M), b: [200, 200, 200, 200, 200].map((x) => x * M) },
    "node-jitless": { a: [100, 100, 100, 100, 100].map((x) => x * M), b: [200, 200, 200, 200, 200].map((x) => x * M) },
    "d8-jitless": { a: [50, 50, 50, 50, 50].map((x) => x * M), b: [100, 100, 100, 100, 100].map((x) => x * M) },
  });
  const r = analyze(s, { engines: ["ep9", "ep9-jitless", "node-jitless", "d8-jitless"], workloads: ["a", "b"] });
  const t = r.table.find((x) => x.workload === "a" && x.engine === "ep9");
  assert.deepEqual([t.n, t.median_bytes, t.min_bytes, t.max_bytes], [5, 100 * M, 100 * M, 120 * M]);
  const ra = r.ratios.find((x) => x.subject === "ep9" && x.comparator === "node-jitless" && x.workload === "a");
  assert.equal(ra.ratio, 1);
  assert.deepEqual([ra.round_ratios.n, ra.round_ratios.median, ra.round_ratios.max], [5, 1, 1.2]);
  const cap = (s, c) => r.caps.find((x) => x.subject === s && x.comparator === c);
  // ep9: b is 1.5x node (over 1.2) and 3.0x d8 (over 1.8). The jitless build is 1.0x and 2.0x.
  assert.deepEqual([cap("ep9", "node-jitless").holds, cap("ep9", "node-jitless").over, cap("ep9", "node-jitless").max_workload], [false, ["b"], "b"]);
  assert.equal(cap("ep9", "node-jitless").max_ratio, 1.5);
  assert.ok(Math.abs(cap("ep9", "node-jitless").geomean_ratio - Math.sqrt(1.5)) < 1e-12);
  assert.equal(cap("ep9-jitless", "node-jitless").holds, true);
  assert.deepEqual([cap("ep9-jitless", "d8-jitless").holds, cap("ep9-jitless", "d8-jitless").over], [false, ["a", "b"]]);
  assert.equal(r.g7.holds, false);
  // A cap passes at exactly the cap; without d8 in the run G7 has no reading.
  const ok = analyze(
    samples({ ep9: { a: [120, 120, 120, 120, 120] }, "node-jitless": { a: [100, 100, 100, 100, 100] } }),
    { engines: ["ep9", "node-jitless"], workloads: ["a"] },
  );
  assert.equal(ok.caps[0].holds, true);
  assert.equal(ok.g7.holds, null);
  assert.throws(() => analyze(samples({ ep9: { a: [1, null, 1, 1, 1] } }), { engines: ["ep9"], workloads: ["a"] }), /no full set/);
});

test("checkMemRun: a failed run, a wrong output or no RSS is refused", () => {
  const ok = { status: 0, stdout: "42\n", stderr: "", rss: 10 };
  checkMemRun(ok, { engine: "e", workload: "w", expect: "42" });
  assert.throws(() => checkMemRun({ ...ok, status: 1, stderr: "boom" }, { engine: "e", workload: "w", expect: "42" }), (e) => e instanceof Refusal && /exit 1/.test(e.message));
  assert.throws(() => checkMemRun({ ...ok, stdout: "41" }, { engine: "e", workload: "w", expect: "42" }), /printed "41"/);
  assert.throws(() => checkMemRun({ ...ok, rss: null }, { engine: "e", workload: "w", expect: "42" }), /no peak RSS/);
});

test("run: the output check, then rotated rounds; every sample in the JSON", async () => {
  const calls = [];
  const engines = ["ep9", "node-jitless", "d8-jitless"].map((id) => ({ id, label: id, role: "x", bin: `/bin/${id}`, args: [], version: "v" }));
  const rss = { ep9: 120, "node-jitless": 100, "d8-jitless": 80 };
  const exec = (e, file) => {
    const w = path.basename(file, ".js");
    calls.push(`${e.id} ${w}`);
    return { status: 0, stdout: `${WORKLOADS[w].expect}\n`, stderr: "", rss: rss[e.id] * (1 << 20), ms: 1 };
  };
  const o = parseArgs(["--ep9", "x", "--workloads", "t0-loop,t0-call"]);
  const res = await run(o, { exec, engines, tool: { name: "fake", unit: "bytes", program: "/x/time" }, log: () => {} });
  assert.equal(res.schema, SCHEMA);
  assert.equal(res.provisional, true);
  assert.deepEqual(res.provisional_reasons, ["no host file (the load guard cannot run)"]);
  const plan = schedule(o.engines, o.workloads, 5);
  assert.deepEqual(calls.slice(6), plan.map((p) => `${p.engine} ${p.workload}`));
  assert.equal(res.samples.length, 30);
  assert.deepEqual(res.caps.map((c) => [c.comparator, c.holds]), [["node-jitless", true], ["d8-jitless", true]]);
  assert.equal(res.g7.holds, true);
  assert.match(renderMarkdown(res), /G7 \(both caps, every ep9 build in the run\): holds \(provisional\)/);
});

/** A temp dir with fake engines (symlinks to fake-engine.sh), a jit-preset fake ep9 and a shells file. */
function fakeSetup(t) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-mem-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const n of ["node", "d8", "ep9j"]) symlinkSync(FAKE, path.join(dir, n));
  const fakeTime = path.join(dir, "fake-time.sh");
  writeFileSync(fakeTime, FAKE_TIME_SH);
  chmodSync(fakeTime, 0o755);
  // fake-engine.sh says "preset jitless"; the jit-preset ep9 wraps it with its own build line.
  const ep9 = path.join(dir, "ep9");
  writeFileSync(ep9, `#!/bin/sh\nif [ "$1" = --version ]; then echo "fake ep9 0.0"; echo "features jit; preset jit"; exit 0; fi\nexec "${path.join(dir, "ep9j")}" "$@"\n`);
  chmodSync(ep9, 0o755);
  writeFileSync(path.join(dir, "versions.txt"), `# host: ${platformId()}\nnode v0 ${path.join(dir, "node")}\nd8 v0 ${path.join(dir, "d8")}\n`);
  const host = path.join(dir, "host.toml");
  writeFileSync(host, `id = "test"\ncpu = "${cpuModel()}"\ncores = ${os.cpus().length}\nplatform = "${process.platform}-${process.arch}"\nos = "test"\ngate_host = false\nidle_baseline_load1 = 1000\n`);
  const out = path.join(dir, "out.json");
  const md = path.join(dir, "out.md");
  const mem = (args, env = {}) =>
    spawnSync(
      process.execPath,
      [MEM, "--ep9", ep9, "--shells", path.join(dir, "versions.txt"), "--workloads", "t0-loop,t0-call", "--settle-s", "0", "--after-s", "0", "--wait-min", "0", "--out", out, "--md", md, ...args],
      { encoding: "utf8", env: { ...process.env, EP9_MEM_TIME: fakeTime, ...env } },
    );
  return { dir, host, out, md, mem };
}

test("CLI: a class Mem reading with the load guard; ratios and caps from the RSS tool", { skip }, (t) => {
  const f = fakeSetup(t);
  const r = f.mem(["--host", f.host], { FAKE_RSS_ep9: String(130 << 20), FAKE_RSS_node: String(100 << 20), FAKE_RSS_d8: String(60 << 20) });
  assert.equal(r.status, 0, r.stderr);
  const res = JSON.parse(readFileSync(f.out, "utf8"));
  assert.equal(res.provisional, false, res.provisional_reasons.join("; "));
  assert.equal(res.engines[0].engine_build, "features jit; preset jit");
  assert.equal(res.reader.unit, "bytes");
  assert.equal(res.samples.length, 5 * 2 * 3);
  const c = Object.fromEntries(res.caps.map((x) => [x.comparator, x]));
  // 130/100 = 1.3 > 1.2; 130/60 = 2.17 > 1.8.
  assert.equal(c["node-jitless"].holds, false);
  assert.ok(Math.abs(c["node-jitless"].max_ratio - 1.3) < 1e-12);
  assert.equal(c["d8-jitless"].holds, false);
  assert.equal(res.g7.holds, false);
  assert.equal(r.stdout, readFileSync(f.md, "utf8"));
  assert.match(r.stdout, /\| t0-loop \| 130\.0 \[130\.0-130\.0\] \| 100\.0/);
});

test("CLI: no RSS, a wrong output, a wrong build line and a busy host", { skip }, (t) => {
  const f = fakeSetup(t);
  const noRss = f.mem([], { FAKE_NO_RSS: "1" });
  assert.equal(noRss.status, 1);
  assert.match(noRss.stderr, /REFUSED: t0-loop on ep9: the RSS tool gave no peak RSS/);
  const wrong = f.mem([], { FAKE_WRONG: "d8:t0-call" });
  assert.equal(wrong.status, 1);
  assert.match(wrong.stderr, /REFUSED: t0-call on d8-jitless: printed "12345"/);
  // The jitless engine must be a jitless build: the jit-preset fake is refused.
  const pre = f.mem(["--ep9-jitless", path.join(f.dir, "ep9")]);
  assert.equal(pre.status, 2);
  assert.match(pre.stderr, /ep9-jitless .*want "preset jitless"/);
  const busy = path.join(f.dir, "busy.toml");
  writeFileSync(busy, readFileSync(f.host, "utf8").replace("idle_baseline_load1 = 1000", "idle_baseline_load1 = -1"));
  const b = f.mem(["--host", busy]);
  assert.equal(b.status, 3);
  const allowed = f.mem(["--host", busy, "--allow-busy"]);
  assert.equal(allowed.status, 0, allowed.stderr);
  assert.match(JSON.parse(readFileSync(f.out, "utf8")).provisional_reasons.join(";"), /load guard failed before the run/);
});
