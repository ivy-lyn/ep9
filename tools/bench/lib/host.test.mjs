// node --test tools/bench/lib/host.test.mjs
// The load source of lib/host.mjs (plan v14 section 4.1 (5)): os.loadavg() on Linux and macOS, a
// 60-s typeperf reading on Windows (os.loadavg() is always 0 there), the guard on each, the host
// file's load_source, and the Windows busy list of cross/idle.mjs (tasklist). Fakes only: no
// typeperf or tasklist runs, so the tests run on any host.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { busyProcesses, tasklistNames } from "../cross/idle.mjs";
import {
  guardAfter,
  guardBefore,
  hostInfo,
  load1,
  loadCheck,
  loadSource,
  parseTypeperf,
  readHostFile,
  TYPEPERF_COUNTERS,
  TYPEPERF_SECONDS,
  typeperfLoad,
} from "./host.mjs";

// typeperf "\System\Processor Queue Length" "\Processor(_Total)\% Processor Time" -si 1 -sc 3
// (the CSV form of Windows Server 2025; the first busy sample is blank).
const TYPEPERF = [
  "",
  '"(PDH-CSV 4.0) (Coordinated Universal Time)(0)","\\\\runner\\System\\Processor Queue Length","\\\\runner\\Processor(_Total)\\% Processor Time"',
  '"11/16/2026 19:00:00.000","1.000000"," "',
  '"11/16/2026 19:00:01.000","2.000000","25.000000"',
  '"11/16/2026 19:00:02.000","0.000000","15.000000"',
  "Exiting, please wait...",
  "The command completed successfully.",
  "",
].join("\r\n");

test("load source: typeperf on Windows, loadavg elsewhere; EP9_LOAD_SOURCE wins", () => {
  assert.equal(loadSource("win32", {}), "typeperf");
  assert.equal(loadSource("linux", {}), "loadavg");
  assert.equal(loadSource("darwin", {}), "loadavg");
  assert.equal(loadSource("win32", { EP9_LOAD_SOURCE: "loadavg" }), "loadavg");
  assert.throws(() => loadSource("linux", { EP9_LOAD_SOURCE: "uptime" }), /not one of loadavg, typeperf/);
  assert.equal(hostInfo().load_source, loadSource());
});

test("typeperf: the means of the samples with both values, and the load in runnable threads", () => {
  const p = parseTypeperf(TYPEPERF);
  assert.deepEqual(p, { samples: 2, queue: 1, busy_pct: 20 });
  // 1 thread queued on average, plus 20% of 4 logical CPUs busy.
  assert.equal(typeperfLoad(p, 4), 1.8);
  assert.equal(parseTypeperf(""), null);
  assert.equal(parseTypeperf("Error: No valid counters.\r\n"), null);
});

test("load1 on Windows runs typeperf for 60 s with the 2 counters; no reading is null", () => {
  const calls = [];
  const run = (cmd, args, opts) => {
    calls.push([cmd, args, opts.timeout]);
    return { stdout: TYPEPERF, status: 0 };
  };
  assert.equal(load1({ platform: "win32", env: {}, run, cpus: 4 }), 1.8);
  assert.deepEqual(calls, [["typeperf", [...TYPEPERF_COUNTERS, "-si", "1", "-sc", String(TYPEPERF_SECONDS)], (TYPEPERF_SECONDS + 60) * 1000]]);
  assert.equal(TYPEPERF_SECONDS, 60);
  assert.equal(load1({ platform: "win32", env: {}, run: () => ({ stdout: "", status: 1 }), cpus: 4 }), null);
  // Not Windows: the 1-minute load, no typeperf.
  const none = () => assert.fail("typeperf must not run");
  assert.equal(typeof load1({ platform: "linux", env: {}, run: none }), "number");
  // EP9_LOAD_SOURCE=loadavg on Windows: the old reading (os.loadavg(), always 0 there).
  assert.equal(typeof load1({ platform: "win32", env: { EP9_LOAD_SOURCE: "loadavg" }, run: none }), "number");
});

test("guard: the same limit for each source; loadavg on Windows is no load guard; no reading fails", async () => {
  assert.deepEqual(loadCheck(1.8, 1.5, "typeperf", "win32"), { load1: 1.8, limit: 2, pass: true, why: null, source: "typeperf" });
  assert.equal(loadCheck(2.1, 1.5, "typeperf", "win32").pass, false);
  const c = loadCheck(0, 0, "loadavg", "win32");
  assert.equal(c.pass, false);
  assert.equal(c.limit, 0.5);
  assert.match(c.why, /^no load guard: the load source loadavg is always 0 on Windows/);
  assert.equal(loadCheck(0, 0, "loadavg", "linux").pass, true);
  assert.deepEqual(loadCheck(null, 1, "typeperf", "win32"), { load1: null, limit: 1.5, pass: false, why: "no load reading (typeperf)", source: "typeperf" });
  // A guard that cannot pass does not wait: loadavg on Windows.
  const slept = [];
  const sleep = async (s) => void slept.push(s);
  const g = await guardBefore({ idleBaseline: 0, settleS: 60, waitS: 1800, pollS: 30, source: "loadavg", platform: "win32" }, { load: () => 0, sleep });
  assert.deepEqual(slept, [60]);
  assert.equal(g.pass, false);
  assert.equal(g.source, "loadavg");
  // A typeperf guard polls like the loadavg guard.
  slept.length = 0;
  const loads = [3, 1.2];
  const h = await guardBefore({ idleBaseline: 1, settleS: 60, waitS: 120, pollS: 30, source: "typeperf", platform: "win32" }, { load: () => loads.shift(), sleep });
  assert.deepEqual(slept, [60, 30]);
  assert.equal(h.pass, true);
  assert.equal(h.source, "typeperf");
  const a = await guardAfter({ idleBaseline: 1, afterS: 120, source: "typeperf", platform: "win32" }, { load: () => 1.6, sleep });
  assert.equal(a.pass, false);
  assert.equal(a.why, "load 1.6 > 1.5");
});

test("host file: load_source must be a known source", (t) => {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-host-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const head = 'id = "gha"\ncpu = "x"\ncores = 4\nplatform = "win32-x64"\nos = "w"\ngate_host = false\nidle_baseline_load1 = 0.4\n';
  const file = (name, text) => {
    const f = path.join(dir, name);
    writeFileSync(f, text);
    return f;
  };
  assert.equal(readHostFile(file("a.toml", `${head}load_source = "typeperf"\n`)).load_source, "typeperf");
  assert.equal(readHostFile(file("b.toml", head)).load_source, undefined);
  assert.throws(() => readHostFile(file("c.toml", `${head}load_source = "uptime"\n`)), /load_source must be one of loadavg, typeperf/);
});

test("idle.mjs: the Windows busy list from tasklist", () => {
  const text = '"System Idle Process","0","Services","0","8 K"\r\n"cargo.exe","4120","Console","1","12,345 K"\r\n"RUSTC.EXE","4121","Console","1","1 K"\r\n"node.exe","9","Console","1","1 K"\r\n';
  assert.deepEqual(tasklistNames(text), ["System Idle Process", "cargo", "RUSTC", "node"]);
  const calls = [];
  const run = (cmd, args) => {
    calls.push([cmd, args]);
    return { stdout: text };
  };
  assert.deepEqual(busyProcesses("win32", run), ["cargo", "rustc"]);
  assert.deepEqual(calls, [["tasklist", ["/fo", "csv", "/nh"]]]);
  assert.deepEqual(busyProcesses("linux", () => ({ stdout: "COMM\n/usr/bin/cargo\nbash\nep9\n" })), ["cargo", "ep9"]);
});
