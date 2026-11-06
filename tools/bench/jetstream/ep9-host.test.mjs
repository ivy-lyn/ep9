// Tests for ep9-host.mjs and the ep9 configs of run.mjs. The engine is testdata/fake-ep9.mjs (a
// V8 global with only print and gc, as the ep9 shell has) and the driver is
// testdata/fake-jetstream/cli.js, so no ep9 build and no JetStream checkout are needed. One test
// runs the real driver of third_party/bench-jetstream on the fake engine; it skips when the
// checkout is not synced.
// usage: node --test tools/bench/jetstream/ep9-host.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

import { bundle, hasStrictPrologue, MARK, parseRealmOutput, prelude, resourceTexts } from "./ep9-host.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../../..");
const fakeEp9 = path.join(here, "testdata/fake-ep9.mjs");
const fakeJs = path.join(here, "testdata/fake-jetstream");
const realJs = path.join(root, "third_party/bench-jetstream");

/** Runs `text` as ep9 would (only print and gc); returns the printed lines. */
function runLikeEp9(text) {
  const lines = [];
  const g = vm.createContext(vm.constants.DONT_CONTEXTIFY);
  g.print = (...a) => lines.push(a.map(String).join(" "));
  g.gc = () => {};
  vm.runInContext(text, g);
  return { g, lines };
}

test("strict prologue: after comments and white space only", () => {
  assert.equal(hasStrictPrologue('"use strict";\nx = 1'), true);
  assert.equal(hasStrictPrologue("// c\n/* d\n */\n  'use strict'\n"), true);
  assert.equal(hasStrictPrologue('x();\n"use strict";'), false);
  assert.equal(hasStrictPrologue('"use strict2";'), false);
  assert.equal(hasStrictPrologue("(function(){'use strict'})()"), false);
});

test("bundle: strict when a script has the prologue (auto), and a note names the others", () => {
  const s = bundle(["a = 1;", '"use strict";\nlet b = 2;'], {}, "auto");
  assert.equal(s.strict, true);
  assert.ok(s.text.startsWith('"use strict";\n'));
  assert.equal(s.notes.length, 1);
  assert.match(s.notes[0], /realm runs as strict code .* 1 of 2 scripts have no "use strict" prologue: #0 "a = 1;"/);
  const l = bundle(["a = 1;", "b = 2"], {}, "auto");
  assert.equal(l.strict, false);
  assert.deepEqual(l.notes, []);
  // A forced mode: sloppy runs the strict script as sloppy code, and says so.
  const f = bundle(['"use strict"; c = 3;'], {}, "sloppy");
  assert.equal(f.strict, false);
  assert.match(f.notes[0], /sloppy code \(--script-mode sloppy\); 1 of 1 scripts have a "use strict" prologue/);
});

test("bundle: each script starts on a new line after a ;, so none runs on into the next", () => {
  // Without the separator, "f\n(2)" would call f. A line comment at a script end stays closed.
  const { g, lines } = runLikeEp9(bundle(["var f = () => 1 // end", "(print('two'))"], {}).text);
  assert.equal(typeof g.f, "function");
  assert.deepEqual(lines, ["two"]);
});

test("prelude: valid strict and sloppy code; the realm host API on a print-only engine", async () => {
  for (const head of ["", '"use strict";\n']) {
    const { g, lines } = runLikeEp9(head + prelude({ "./a.txt": "text A" }) + "\nconsole.log('x', 1); printErr('e');");
    assert.equal(lines[0], "x 1");
    assert.equal(lines[1], `${MARK}{"err":"e"}`);
    assert.equal(g.self, g);
    assert.equal(g.readFile("./a.txt"), "text A");
    assert.equal(g.read("./a.txt"), "text A");
    assert.throws(() => g.read("./a.txt", "binary"), /TypedArray/);
    assert.throws(() => g.readFile("./b.txt"), /no file \.\/b\.txt/);
    assert.throws(() => g.load("./a.txt"), /not available/);
    assert.throws(() => g.runString(""), /not available/);
    assert.equal(new g.ShellTextDecoder().decode(g.ShellPrefetchedResources["./a.txt"]), "text A");
    const t = g.performance.now();
    assert.equal(typeof t, "number");
    assert.ok(g.performance.now() >= t);
    g.top.currentResolve([1.5, 2]);
    assert.equal(lines.at(-1), `${MARK}{"resolve":[1.5,2]}`);
    g.top.currentReject(new TypeError("no"));
    assert.match(lines.at(-1), /^@@ep9-host@@ \{"reject":"TypeError: no","stack":/);
    // setTimeout runs its callback as a promise job, and clearTimeout cancels it.
    const got = [];
    g.setTimeout((x) => got.push(x), 50, "a");
    g.clearTimeout(g.setTimeout(() => got.push("b"), 0));
    await new Promise((r) => setImmediate(r));
    assert.deepEqual(got, ["a"]);
  }
});

test("realm output: marker lines apart from the workload's own lines", () => {
  const out = parseRealmOutput(`hello\n${MARK}{"err":"w"}\n${MARK}{"resolve":[3]}\n${MARK}{"resolve":[4]}\nbye\n`);
  assert.deepEqual(out.lines, ["hello", "bye"]);
  assert.deepEqual(out.errs, ["w"]);
  // The first outcome counts.
  assert.deepEqual(out.outcome, { resolve: [3] });
  assert.equal(parseRealmOutput("x\n").outcome, null);
  assert.deepEqual(resourceTexts({ a: new Int8Array([0x68, 0x69]), b: "s" }), { a: "hi", b: "s" });
});

/** ep9-host.mjs on the fake driver and the fake engine. */
function host(workload, env = {}, extra = []) {
  return spawnSync(
    process.execPath,
    [path.join(here, "ep9-host.mjs"), "--ep9", process.execPath, "--ep9-arg", fakeEp9, ...extra, fakeJs, `--test=${workload}`],
    { encoding: "utf8", env: { ...process.env, ...env } },
  );
}

test("host: a strict workload resolves; its output lines go to stdout and stderr", () => {
  const r = host("ok");
  assert.equal(r.status, 0, r.stderr);
  const json = r.stdout.split("\n").find((l) => l.startsWith('{"JetStream3.0"'));
  assert.equal(JSON.parse(json)["JetStream3.0"].tests.ok.metrics.Score.current[0], 6);
  assert.match(r.stdout, /^workload ok line$/m);
  assert.match(r.stderr, /^workload err line$/m);
  assert.match(r.stderr, /^ep9-host: realm 1: 4 scripts, strict, ep9 process \d+ ms, exit 0$/m);
  assert.match(r.stderr, /^ep9-host: realm 1: realm runs as strict code \(--script-mode auto\); 3 of 4 scripts/m);
});

test("host: a sloppy workload runs as sloppy code", () => {
  const r = host("sloppy");
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /"current":\[60\]/);
  assert.match(r.stderr, /4 scripts, sloppy/);
});

test("host: a rejected workload and a failed ep9 run fail the driver with the cause", () => {
  const rej = host("reject");
  assert.notEqual(rej.status, 0);
  assert.match(rej.stderr, /^ep9-host: realm 1: the workload rejected: RangeError: bad result 42$/m);
  const bad = host("ok", { FAKE_EP9_FAIL: "1" });
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /^ep9-host: realm 1: ep9 exit 1: \S+realm-1\.js:1:1: Uncaught TypeError: fake failure \| +at fake/m);
  assert.match(bad.stderr, /JetStream3 failed: Error: ep9 realm 1: ep9 exit 1/);
});

test("host: --keep keeps the realm file; --no-prefetch is refused", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ep9-host-keep-"));
  try {
    const r = host("ok", {}, ["--keep", dir]);
    assert.equal(r.status, 0, r.stderr);
    const text = fs.readFileSync(path.join(dir, "realm-1.js"), "utf8");
    assert.ok(text.startsWith('"use strict";\n(function () {'));
    assert.match(text, /class Benchmark/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
  const n = spawnSync(process.execPath, [path.join(here, "ep9-host.mjs"), "--ep9", process.execPath, fakeJs, "--no-prefetch"], { encoding: "utf8" });
  assert.equal(n.status, 2);
  assert.match(n.stderr, /cannot read files/);
});

/** run.mjs with the fake driver; configs are --config values. */
function runner(configs, env = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ep9-jetstream-ep9-"));
  try {
    const list = path.join(dir, "list.txt");
    const out = path.join(dir, "out.json");
    fs.writeFileSync(list, "ok\nsloppy\nreject\n");
    const p = spawnSync(
      process.execPath,
      [path.join(here, "run.mjs"), "--list", list, "--reps", "1", "--out", out, ...configs.flatMap((c) => ["--config", c])],
      { encoding: "utf8", env: { ...process.env, ...env, EP9_JETSTREAM_DIR: fakeJs } },
    );
    return { status: p.status, stderr: p.stderr, out: fs.existsSync(out) ? JSON.parse(fs.readFileSync(out, "utf8")) : null };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test("run.mjs: ep9@<bin> runs ep9-host.mjs; notes, failures and causes go into the JSON", () => {
  const { status, out } = runner([`a=ep9@${process.execPath}:${fakeEp9}`, `b=ep9@${process.execPath}:${fakeEp9},host:--script-mode=sloppy`]);
  assert.equal(status, 1);
  assert.equal(out.configs[0].kind, "ep9");
  assert.deepEqual(out.configs[1].hostFlags, ["--script-mode=sloppy"]);
  assert.match(out.configs[0].version, /^v\d+/);
  const ok = out.runs.find((r) => r.name === "ok" && r.config === "a");
  assert.equal(ok.score, 6);
  assert.match(ok.notes.join("\n"), /realm 1: 4 scripts, strict/);
  // --script-mode sloppy reaches the host: the strict check of workload-ok fails.
  const forced = out.runs.find((r) => r.name === "ok" && r.config === "b");
  assert.match(forced.cause, /the workload rejected: Error: not strict/);
  const rej = out.failures.find((f) => f.name === "reject" && f.config === "a");
  assert.equal(rej.cause, "realm 1: the workload rejected: RangeError: bad result 42");
  assert.equal(out.complete, false);
  assert.deepEqual(
    out.missing.map((m) => `${m.name}/${m.config}`),
    ["ok/b", "reject/a", "reject/b"],
  );
});

test("run.mjs: engine ep9 needs --ep9; ep9 with cli:--no-prefetch and host: flags elsewhere are usage errors", () => {
  assert.equal(runner(["a=ep9"]).status, 2);
  assert.equal(runner([`a=ep9@${process.execPath}:cli:--no-prefetch`]).status, 2);
  assert.equal(runner([`a=${process.execPath}:host:--x`]).status, 2);
  // node@<path> is a node-host.mjs config on that binary.
  const r = runner([`n=node@${process.execPath}`]);
  assert.equal(r.out.configs[0].kind, "node");
});

test("real driver: one JetStream workload end to end on a print-only engine", { skip: !fs.existsSync(path.join(realJs, "cli.js")) && "third_party/bench-jetstream not synced" }, () => {
  const r = spawnSync(
    process.execPath,
    [path.join(here, "ep9-host.mjs"), "--ep9", process.execPath, "--ep9-arg", fakeEp9, realJs, "--test=hash-map", "--dump-json-results", "--iteration-count=5"],
    { encoding: "utf8" },
  );
  assert.equal(r.status, 0, r.stderr);
  const json = JSON.parse(r.stdout.split("\n").find((l) => l.startsWith('{"JetStream3.0"')));
  assert.ok(json["JetStream3.0"].tests["hash-map"].metrics.Score.current[0] > 0);
  assert.match(r.stderr, /ep9-host: realm 1: \d+ scripts, sloppy/);
});
