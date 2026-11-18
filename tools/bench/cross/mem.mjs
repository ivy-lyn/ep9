#!/usr/bin/env node
// G7 memory reader (docs/plan/02-architecture.md G7; class Mem of docs/plan/10-quality.md section
// 3; M2 exit 4 of docs/plan/09-milestones.md): peak RSS of each engine on each workload, >= 5 runs,
// the median, and both G7 caps: ep9 <= 1.2x node --jitless and <= 1.8x a pointer-compressed d8
// --jitless (jsvu, third_party/shells.toml [d8] pointer_compression = "true").
//
//   node tools/bench/cross/mem.mjs --ep9 target/release/ep9 --ep9-jitless <jitless ep9> \
//     --host tools/bench/hosts/m5max-macos.toml --out mem.json --md mem.md
//
//   --ep9 PATH          the ep9 binary of the jit preset build (required). Its --version build line
//                       must say "preset jit"
//   --ep9-jitless PATH  the jitless ep9 build (`cargo build --release -p ep9_cli
//                       --no-default-features`); its build line must say "preset jitless". The
//                       two-build rule (plan v12 V12-D6) needs both: a cap holds only when it holds
//                       for each ep9 build in the run
//   --engines LIST      default ep9,ep9-jitless,node-jitless,d8-jitless (ep9-jitless only with
//                       --ep9-jitless). Also known: qjs (a report column, no cap)
//   --shells FILE       reference shells (default third_party/shells/versions.txt)
//   --workloads LIST    default mem (workloads.mjs: the 7 v8-v7 kernels, no regexp)
//   --harness H         the v8v7 harness (default DEFAULT_HARNESS of v8v7.mjs)
//   --runs N            measured runs per engine and workload, >= 5 (class Mem; default 5)
//   --host FILE         tools/bench/hosts/<host>.toml. Without it the run is provisional
//   --allow-busy        if the load guard fails before the run, go on and label the run provisional
//   --settle-s N, --wait-min N, --after-s N   the load guard (defaults 60, 30, 120)
//   --timeout-s N       a run that takes longer fails (default 600)
//   --label TEXT        a name for the run
//   --out FILE          the JSON (schema ep9-mem/v1, every sample); --md FILE the Markdown
//
// Commit. commit.ep9 is the commit that every ep9 binary of the run names in its `ep9 --version`
// build line (stderr, P0-BUILDID), and only when each was built clean; else null, and m2-exit
// reads the run as OPEN. commit.runner is the checkout the reader ran in.
//
// Peak RSS. The OS reports the high-water resident set of the engine process when it exits:
// macOS `/usr/bin/time -l` "maximum resident set size" (bytes); Linux GNU `/usr/bin/time -f %M`
// (KiB; the `time` package on Debian and Ubuntu); Windows the PeakWorkingSetSize of the engine
// process (bytes), which a PowerShell wrapper reads with GetProcessMemoryInfo on the process
// handle after the process exits (WIN_PEAK_SCRIPT; plan v14 section 4.1 (5)). The engine's
// stdout and stderr pass through. EP9_MEM_TIME names another program of the same form (tests use
// a fake; on Windows another PowerShell). No other wrapper: valgrind or perf would add their own
// memory. On Windows (no process groups) a timeout or an interrupt ends the whole process tree
// of the wrapper, the engine too, with `taskkill /T /F` (winTreeKill; M9-P1 Codex r1 F2).
//
// Order. A round runs every workload on every engine once. The workload order rotates by one per
// round, and per workload the engine order rotates too (run.mjs `schedule`), so no engine always
// runs first. Every run must print the workload's expected output and exit 0, or the reader
// stops with "REFUSED" (exit 1) and writes no report.
//
// Statistics. Per engine and workload: the median of the runs (and min, max). Ratio per workload =
// median(ep9) / median(comparator); also the median, min and max of the per-round ratios. A cap
// holds when the ratio of every workload is at most the cap (the strict reading of "peak RSS on
// v8-v7"); the JSON gives the largest ratio and the geomean too. A provisional run (no host file,
// not a gate host, a failed load guard) is labelled so; class Mem may run on any host that passes
// the load guard (10-quality section 3), so a gate host is not needed for a valid reading.
//
// Exit status: 0 done; 1 refused or another error; 2 usage error; 3 host busy.
// Tests: node --test tools/bench/cross/mem.test.mjs

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync, writeSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleepMs } from "node:timers/promises";

import { guardAfter, guardBefore, hostInfo, load1, LOAD_MARGIN, readHostFile } from "../lib/host.mjs";
import { geomean, median } from "../lib/stats.mjs";
import { splitVersion } from "./kernels.mjs";
import { buildNote, cleanCommit, combineBuilds, ENGINES as CROSS_ENGINES, HostBusy, parseBuild, Refusal, resolveEngines, schedule, UsageError } from "./run.mjs";
import { DEFAULT_HARNESS, HARNESSES, ROOT } from "./v8v7.mjs";
import { expandWorkloads, materialize, V8V7_7, WORKLOADS } from "./workloads.mjs";

export const SCHEMA = "ep9-mem/v1";
/** Class Mem minimum (10-quality section 3). */
export const MIN_RUNS = 5;

/** The G7 caps (02-architecture G7): the comparator engine and the largest ep9 / comparator ratio. */
export const CAPS = [
  { comparator: "node-jitless", cap: 1.2, text: "<= 1.2x node 24 --jitless" },
  { comparator: "d8-jitless", cap: 1.8, text: "<= 1.8x a pointer-compressed d8 --jitless" },
];

/** Engines of this reader. An ep9 engine names its option and the preset its build line must say. */
export const ENGINES = {
  ep9: { label: "ep9 (jit preset)", ep9: "ep9", preset: "jit", role: "subject" },
  "ep9-jitless": { label: "ep9 (jitless build)", ep9: "ep9Jitless", preset: "jitless", role: "subject" },
  "node-jitless": { ...CROSS_ENGINES["node-jitless"], role: "cap" },
  "d8-jitless": { ...CROSS_ENGINES["d8-jitless"], label: "d8 --jitless (pointer-compressed)", role: "cap" },
  qjs: { ...CROSS_ENGINES.qjs, role: "context" },
};

export function parseArgs(argv) {
  const o = {
    ep9: null,
    ep9Jitless: null,
    engines: null,
    shells: path.join(ROOT, "third_party/shells/versions.txt"),
    workloads: ["mem"],
    harness: DEFAULT_HARNESS,
    runs: MIN_RUNS,
    host: null,
    allowBusy: false,
    settleS: 60,
    waitMin: 30,
    afterS: 120,
    pollS: 30,
    timeoutS: 600,
    label: null,
    out: null,
    md: null,
  };
  const num = (a, v) => {
    const n = Number(v);
    if (!Number.isFinite(n) || n < 0) throw new UsageError(`${a} needs a number >= 0, got ${v}`);
    return n;
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new UsageError(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case "--ep9": o.ep9 = path.resolve(val()); break;
      case "--ep9-jitless": o.ep9Jitless = path.resolve(val()); break;
      case "--engines": o.engines = val().split(","); break;
      case "--shells": o.shells = path.resolve(val()); break;
      case "--workloads": o.workloads = val().split(","); break;
      case "--harness": o.harness = val(); break;
      case "--runs": o.runs = num(a, val()); break;
      case "--host": o.host = path.resolve(val()); break;
      case "--allow-busy": o.allowBusy = true; break;
      case "--settle-s": o.settleS = num(a, val()); break;
      case "--wait-min": o.waitMin = num(a, val()); break;
      case "--after-s": o.afterS = num(a, val()); break;
      case "--poll-s": o.pollS = num(a, val()); break;
      case "--timeout-s": o.timeoutS = num(a, val()); break;
      case "--label": o.label = val(); break;
      case "--out": o.out = path.resolve(val()); break;
      case "--md": o.md = path.resolve(val()); break;
      default: throw new UsageError(`unknown option ${a}`);
    }
  }
  if (!o.ep9) throw new UsageError("--ep9 is required");
  o.engines ??= ["ep9", ...(o.ep9Jitless ? ["ep9-jitless"] : []), "node-jitless", "d8-jitless"];
  for (const e of o.engines) if (!ENGINES[e]) throw new UsageError(`unknown engine ${e} (known: ${Object.keys(ENGINES).join(", ")})`);
  if (new Set(o.engines).size !== o.engines.length) throw new UsageError("--engines has a duplicate");
  if (!o.engines.includes("ep9")) throw new UsageError("--engines must include ep9");
  if (o.engines.includes("ep9-jitless") && !o.ep9Jitless) throw new UsageError("engine ep9-jitless needs --ep9-jitless");
  if (!Number.isInteger(o.runs) || o.runs < MIN_RUNS) throw new UsageError(`--runs must be an integer >= ${MIN_RUNS} (class Mem)`);
  if (new Set(o.workloads).size !== o.workloads.length) throw new UsageError("--workloads has a duplicate");
  o.workloads = expandWorkloads(o.workloads);
  for (const w of o.workloads) if (!WORKLOADS[w]) throw new UsageError(`unknown workload ${w}`);
  if (!HARNESSES.includes(o.harness)) throw new UsageError(`--harness must be one of ${HARNESSES.join(", ")}`);
  return o;
}

/**
 * The peak RSS tool of this platform: { name, unit, program, command(reportFile, engineArgv) ->
 * the program's arguments, parse(text) -> bytes }.
 */
export function rssTool(platform = process.platform, env = process.env) {
  if (platform === "win32") {
    const program = env.EP9_MEM_TIME || "powershell";
    return { name: "windows-peak-working-set-size", unit: "bytes", program, command: winPeakCommand, parse: parseWinPeak };
  }
  const program = env.EP9_MEM_TIME || "/usr/bin/time";
  if (platform === "darwin") {
    const args = (f) => ["-l", "-o", f];
    return { name: "macos-time-l-maximum-resident-set-size", unit: "bytes", program, args, command: (f, argv) => [...args(f), ...argv], parse: parseTimeL };
  }
  if (platform === "linux") {
    const args = (f) => ["-f", "%M", "-o", f];
    return { name: "gnu-time-f-M", unit: "KiB", program, args, command: (f, argv) => [...args(f), ...argv], parse: parseGnuM };
  }
  return null;
}

/**
 * The PowerShell wrapper of the Windows tool: it starts the engine (`$Exe` with the command line
 * `$ArgLine`, UTF-8 in base64 so that PowerShell never reads a leading `-` of it as a parameter
 * name; stdio inherited), keeps its process handle, waits, reads PeakWorkingSetSize with
 * GetProcessMemoryInfo (the counters stay readable while a handle is open), writes
 * "PeakWorkingSetSize <bytes>" to `-Out` and exits with the engine's exit code.
 */
export const WIN_PEAK_SCRIPT = `param([string]$Out, [string]$Exe, [string]$ArgLine)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Ep9PeakRss {
  [StructLayout(LayoutKind.Sequential)]
  struct Counters {
    public uint cb; public uint PageFaultCount;
    public UIntPtr PeakWorkingSetSize; public UIntPtr WorkingSetSize;
    public UIntPtr QuotaPeakPagedPoolUsage; public UIntPtr QuotaPagedPoolUsage;
    public UIntPtr QuotaPeakNonPagedPoolUsage; public UIntPtr QuotaNonPagedPoolUsage;
    public UIntPtr PagefileUsage; public UIntPtr PeakPagefileUsage;
  }
  [DllImport("psapi.dll", SetLastError = true)]
  static extern bool GetProcessMemoryInfo(IntPtr process, out Counters counters, uint size);
  public static ulong Peak(IntPtr process) {
    Counters c;
    if (!GetProcessMemoryInfo(process, out c, (uint)Marshal.SizeOf(typeof(Counters)))) {
      throw new System.ComponentModel.Win32Exception();
    }
    return c.PeakWorkingSetSize.ToUInt64();
  }
}
'@
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.Arguments = if ($ArgLine) { [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($ArgLine)) } else { '' }
$psi.UseShellExecute = $false
$p = [System.Diagnostics.Process]::Start($psi)
$h = $p.Handle
$p.WaitForExit()
$peak = [Ep9PeakRss]::Peak($h)
[System.IO.File]::WriteAllText($Out, "PeakWorkingSetSize $peak\n")
exit $p.ExitCode
`;

/**
 * One argument in a Windows command line (the MSVCRT rules that CommandLineToArgvW and the
 * engines read): quoted when empty or when it holds a space, a tab or a quote; a quote gets a
 * backslash, and the backslashes before a quote or the closing quote are doubled.
 */
export function winQuote(arg) {
  if (arg !== "" && !/[\s"]/.test(arg)) return arg;
  return `"${arg.replace(/(\\*)"/g, '$1$1\\"').replace(/(\\+)$/, "$1$1")}"`;
}

let winScript = null;
/** The wrapper script file (written once per process into the temp dir, named by its hash). */
function winScriptFile() {
  if (!winScript) {
    const h = createHash("sha256").update(WIN_PEAK_SCRIPT).digest("hex").slice(0, 12);
    winScript = path.join(os.tmpdir(), `ep9-mem-peak-${h}.ps1`);
    writeFileSync(winScript, WIN_PEAK_SCRIPT);
  }
  return winScript;
}

/** The PowerShell arguments of one Windows run: the wrapper, the report file and the engine. */
export function winPeakCommand(report, [exe, ...args], script = winScriptFile()) {
  const line = Buffer.from(args.map(winQuote).join(" "), "utf8").toString("base64");
  return ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script, report, exe, line];
}

/** The Windows wrapper's report: "PeakWorkingSetSize <bytes>". */
export function parseWinPeak(text) {
  const m = (text ?? "").match(/^PeakWorkingSetSize (\d+)\s*$/m);
  return m ? Number(m[1]) : null;
}

/** macOS `time -l`: the "maximum resident set size" line, in bytes. */
export function parseTimeL(text) {
  const m = text.match(/^\s*(\d+)\s+maximum resident set size\s*$/m);
  return m ? Number(m[1]) : null;
}

/** GNU `time -f %M`: the last line that is one number, in KiB. */
export function parseGnuM(text) {
  const lines = text.trim().split("\n").map((l) => l.trim()).filter((l) => /^\d+$/.test(l));
  return lines.length ? Number(lines[lines.length - 1]) * 1024 : null;
}

// The process groups (POSIX) and process trees (Windows) of runGroup that are still live: pid ->
// the function that kills the group or tree. A group is not in the terminal's foreground group,
// so Ctrl-C (or a SIGTERM or SIGHUP to the runner) does not reach it, and on Windows the engine
// is a child of the PowerShell wrapper: while one is live, a handler kills every live one, then
// raises the signal again (M7-P0 Codex r2 C2-4; M9-P1 Codex r1 F2). The handlers go when none
// is live, so importers keep the default behaviour. A kill function returns null, or the text of
// a failed kill; a failed cleanup goes to stderr (M9-P1 Codex r2 F1). On POSIX, ESRCH (the
// group has already ended) is no failure.
const liveGroups = new Map();
const GROUP_SIGNALS = ["SIGINT", "SIGTERM", "SIGHUP"];
function killLiveGroups() {
  for (const [pid, kill] of liveGroups) {
    let failed = null;
    try {
      failed = kill();
    } catch (e) {
      if (e.code !== "ESRCH") failed = e.message;
    }
    if (failed) {
      try {
        writeSync(2, `mem.mjs: the cleanup of process ${pid} failed: ${failed}\n`);
      } catch {}
    }
  }
  liveGroups.clear();
}
const onGroupSignal = Object.fromEntries(
  GROUP_SIGNALS.map((sig) => [
    sig,
    () => {
      killLiveGroups();
      unhookGroups();
      process.kill(process.pid, sig);
    },
  ]),
);
function hookGroups() {
  for (const sig of GROUP_SIGNALS) process.on(sig, onGroupSignal[sig]);
  process.on("exit", killLiveGroups);
}
function unhookGroups() {
  for (const sig of GROUP_SIGNALS) process.off(sig, onGroupSignal[sig]);
  process.off("exit", killLiveGroups);
}

/**
 * The deadline for taskkill. spawnSync blocks the event loop until taskkill exits, so a stalled
 * taskkill gets SIGKILL at this deadline, and the run ends through the failed-kill path
 * (M9-P1 Codex r3).
 */
export const TASKKILL_TIMEOUT_S = 10;

/** The Windows command that ends `pid` and every process it started (the engine under the wrapper). */
export function winTreeKill(pid) {
  return ["taskkill", ["/PID", String(pid), "/T", "/F"]];
}

/**
 * Runs `program args` and kills it with everything it started when `timeoutS` ends or the
 * runner gets a SIGINT, SIGTERM or SIGHUP. POSIX: the process has its own process group, and
 * the whole group gets SIGKILL, so the engine under the RSS tool stops too, not only the tool
 * (M7-P0 Codex r1 BENCH10-2, r2 C2-4). Windows: `taskkill /T /F` on the wrapper's pid ends its
 * tree, the engine too (winTreeKill; M9-P1 Codex r1 F2). taskkill has a deadline of
 * TASKKILL_TIMEOUT_S (M9-P1 Codex r3). If taskkill cannot start, exits nonzero or passes its
 * deadline, the engine may still run and hold the pipes open, so no close may come: the wrapper
 * is killed, its stdout and stderr are destroyed, and the run ends at once with the error
 * "process tree kill failed (taskkill <code>); the engine may still run" (<code> is the exit
 * code, the spawn error code or "timed out after <N> s"), so the reading is void
 * (M9-P1 Codex r2 F1). Resolves to { status, signal, error, stdout, stderr } when the process
 * closes. `deps` (platform, spawn, spawnSync) is for tests.
 */
export function runGroup(program, args, timeoutS, deps = {}) {
  const platform = deps.platform ?? process.platform;
  const spawnFn = deps.spawn ?? spawn;
  const spawnSyncFn = deps.spawnSync ?? spawnSync;
  return new Promise((resolve) => {
    const group = platform !== "win32";
    // Hook before spawn: the child exists before spawn returns, and a signal handler (a JS
    // callback) runs only after this synchronous part, so it sees the pid (M7-P0 Codex r3 C3-2).
    if (!liveGroups.size) hookGroups();
    let child;
    try {
      child = spawnFn(program, args, { stdio: ["ignore", "pipe", "pipe"], detached: group });
    } catch (e) {
      if (!liveGroups.size) unhookGroups();
      throw e;
    }
    let killError = null;
    // Returns null, or the text of a failed tree kill (killLiveGroups reports it). It runs only
    // from the timer or a handler, after `finish` is set.
    const kill = () => {
      if (group) {
        process.kill(-child.pid, "SIGKILL");
        return null;
      }
      const [cmd, cmdArgs] = winTreeKill(child.pid);
      const r = spawnSyncFn(cmd, cmdArgs, { stdio: "ignore", windowsHide: true, timeout: TASKKILL_TIMEOUT_S * 1000, killSignal: "SIGKILL" });
      if (!r.error && r.status === 0) return null;
      // At the deadline spawnSync kills taskkill: error.code ETIMEDOUT, and the signal is set.
      const expired = r.error ? r.error.code === "ETIMEDOUT" : r.signal != null;
      const code = expired ? `timed out after ${TASKKILL_TIMEOUT_S} s` : r.error ? (r.error.code ?? r.error.message) : (r.status ?? r.signal);
      killError = `process tree kill failed (taskkill ${code}); the engine may still run`;
      try {
        child.kill("SIGKILL");
      } catch {}
      child.stdout.destroy();
      child.stderr.destroy();
      finish(null, null);
      return killError;
    };
    if (child.pid !== undefined) liveGroups.set(child.pid, kill);
    else if (!liveGroups.size) unhookGroups();
    let stdout = "";
    let stderr = "";
    let error = null;
    let timedOut = false;
    let done = false;
    const finish = (status, signal) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (liveGroups.delete(child.pid) && !liveGroups.size) unhookGroups();
      const timeout = `timed out after ${timeoutS} s`;
      const why = timedOut ? (killError ? `${timeout}; ${killError}` : `${timeout} (the process ${group ? "group" : "tree"} was killed)`) : (killError ?? error);
      resolve({ status, signal, error: why, stdout, stderr });
    };
    const timer = setTimeout(() => {
      timedOut = true;
      try {
        kill();
      } catch {}
    }, timeoutS * 1000);
    child.stdout.setEncoding("utf8").on("data", (d) => (stdout += d));
    child.stderr.setEncoding("utf8").on("data", (d) => (stderr += d));
    child.on("error", (e) => {
      error = e.message;
      if (child.pid === undefined) finish(null, null);
    });
    child.on("close", (status, signal) => finish(status, signal));
  });
}

/** Runs one engine on one file under the RSS tool: { rss, ms, status, signal, error, stdout, stderr }. */
export async function measureRss(tool, engine, file, timeoutS) {
  const tmp = mkdtempSync(path.join(os.tmpdir(), "ep9-mem-"));
  try {
    const report = path.join(tmp, "time.txt");
    const t0 = process.hrtime.bigint();
    const argv = [engine.bin, ...engine.args, file];
    const r = await runGroup(tool.program, tool.command ? tool.command(report, argv) : [...tool.args(report), ...argv], timeoutS);
    const ms = Number(process.hrtime.bigint() - t0) / 1e6;
    let text = "";
    try {
      text = readFileSync(report, "utf8");
    } catch {}
    return { rss: tool.parse(text), ms, status: r.status, signal: r.signal, error: r.error, stdout: r.stdout, stderr: r.stderr };
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

/** Ep9 engines: --version, and the build line must name the preset of the engine. */
function resolveEp9(id, bin) {
  const v = spawnSync(bin, ["--version"], { encoding: "utf8" });
  if (v.error) throw new UsageError(`${id} ${bin}: ${v.error.message}`);
  const sv = splitVersion(v.stdout || "");
  const want = ENGINES[id].preset;
  if (!new RegExp(`preset ${want}\\b`).test(sv.engine_build ?? "")) {
    throw new UsageError(`${id} ${bin}: the build line is "${sv.engine_build}", want "preset ${want}"`);
  }
  // The freshness line of P0-BUILDID (`build`, null for an older binary).
  return { id, label: ENGINES[id].label, role: "subject", bin, args: [], ...sv, build: parseBuild(v.stderr) };
}

export function resolveAll(o) {
  const shellIds = o.engines.filter((e) => !ENGINES[e].ep9);
  const shells = shellIds.length ? resolveEngines(shellIds, { ep9: o.ep9, shellsFile: o.shells }) : [];
  return o.engines.map((id) => {
    if (ENGINES[id].ep9) return resolveEp9(id, o[ENGINES[id].ep9]);
    const s = shells.find((x) => x.id === id);
    return { ...s, label: ENGINES[id].label, role: ENGINES[id].role };
  });
}

/** Per engine and workload medians, per workload ratios, and the cap readings. */
export function analyze(samples, { engines, workloads }) {
  const by = {};
  for (const s of samples) ((by[s.workload] ??= {})[s.engine] ??= []).push(s);
  const table = [];
  for (const w of workloads) {
    for (const e of engines) {
      const xs = (by[w]?.[e] ?? []).map((s) => s.rss_bytes);
      if (!xs.length || xs.some((x) => typeof x !== "number")) throw new Error(`analyze: ${w} on ${e} has no full set of RSS samples`);
      table.push({ workload: w, engine: e, n: xs.length, median_bytes: median(xs), min_bytes: Math.min(...xs), max_bytes: Math.max(...xs) });
    }
  }
  const med = (w, e) => table.find((t) => t.workload === w && t.engine === e).median_bytes;
  const subjects = engines.filter((e) => ENGINES[e].role === "subject");
  const comparators = engines.filter((e) => ENGINES[e].role !== "subject");
  const ratios = [];
  for (const s of subjects) {
    for (const c of comparators) {
      for (const w of workloads) {
        const rounds = by[w][s].map((x) => {
          const y = by[w][c].find((z) => z.round === x.round);
          return y ? x.rss_bytes / y.rss_bytes : null;
        }).filter((x) => x !== null);
        ratios.push({
          subject: s,
          comparator: c,
          workload: w,
          ratio: med(w, s) / med(w, c),
          round_ratios: { n: rounds.length, median: rounds.length ? median(rounds) : null, min: Math.min(...rounds), max: Math.max(...rounds) },
        });
      }
    }
  }
  const caps = [];
  for (const cap of CAPS.filter((c) => engines.includes(c.comparator))) {
    for (const s of subjects) {
      const rows = ratios.filter((r) => r.subject === s && r.comparator === cap.comparator);
      const worst = rows.reduce((a, r) => (r.ratio > a.ratio ? r : a));
      caps.push({
        subject: s,
        comparator: cap.comparator,
        cap: cap.cap,
        text: cap.text,
        rule: "every workload",
        ratios: Object.fromEntries(rows.map((r) => [r.workload, r.ratio])),
        max_ratio: worst.ratio,
        max_workload: worst.workload,
        geomean_ratio: geomean(rows.map((r) => r.ratio)),
        holds: rows.every((r) => r.ratio <= cap.cap),
        over: rows.filter((r) => r.ratio > cap.cap).map((r) => r.workload),
      });
    }
  }
  // G7 has a reading only for the whole matrix (plan v12 section 3.4; M7-P0 Codex r1 BENCH10-1):
  // both caps, both ep9 builds (V12-D6) and the 7 v8-v7 kernels; then it holds when every cap
  // holds for both builds.
  const missing = [
    ...CAPS.filter((c) => !engines.includes(c.comparator)).map((c) => c.comparator),
    ...["ep9", "ep9-jitless"].filter((b) => !subjects.includes(b)),
    ...V8V7_7.filter((w) => !workloads.includes(w)),
  ];
  const g7 = {
    caps_in_run: CAPS.filter((c) => engines.includes(c.comparator)).map((c) => c.comparator),
    builds: subjects,
    missing,
    holds: missing.length ? null : caps.every((c) => c.holds),
  };
  return { table, ratios, caps, g7 };
}

function git(args) {
  const r = spawnSync("git", ["-C", ROOT, ...args], { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
}

/** Throws a Refusal unless the run exited 0, printed `expect` and gave a peak RSS. */
export function checkMemRun(r, { engine, workload, expect }) {
  const tail = (s) => s.trim().split("\n").slice(-5).join("\n    ");
  if (r.error || r.status !== 0) {
    const how = r.error ?? (r.signal ? `signal ${r.signal}` : `exit ${r.status}`);
    throw new Refusal(`${workload} on ${engine}: ${how}\n    ${tail(r.stderr)}`);
  }
  const out = r.stdout.replace(/\r\n/g, "\n").trim();
  if (out !== expect) throw new Refusal(`${workload} on ${engine}: printed ${JSON.stringify(out.slice(0, 200))}, want ${JSON.stringify(expect)}`);
  if (typeof r.rss !== "number" || !(r.rss > 0)) throw new Refusal(`${workload} on ${engine}: the RSS tool gave no peak RSS`);
}

/**
 * The whole reading. `deps` = { sleep(s), load(), log(text), exec(engine, file), tool } (injectable
 * for tests). Returns the ep9-mem/v1 result object.
 */
export async function run(o, deps = {}) {
  const sleep = deps.sleep ?? ((s) => sleepMs(s * 1000));
  const load = deps.load ?? load1;
  const log = deps.log ?? ((s) => process.stderr.write(`${s}\n`));
  const tool = deps.tool ?? rssTool();
  if (!tool) throw new UsageError(`no peak RSS tool for ${process.platform}`);
  const started = new Date().toISOString();
  const measured = hostInfo();
  const reasons = [];
  let hostFile = null;
  if (o.host) {
    hostFile = readHostFile(o.host);
    if (hostFile.cpu !== measured.cpu || hostFile.platform !== measured.platform) {
      throw new UsageError(`${o.host} is for "${hostFile.cpu}" (${hostFile.platform}); this host is "${measured.cpu}" (${measured.platform})`);
    }
    if (typeof hostFile.idle_baseline_load1 !== "number") reasons.push(`host file ${path.basename(o.host)} has no idle baseline`);
  } else {
    reasons.push("no host file (the load guard cannot run)");
  }
  const engines = deps.engines ?? resolveAll(o);
  const byId = Object.fromEntries(engines.map((e) => [e.id, e]));
  const exec = deps.exec ?? ((e, f) => measureRss(tool, e, f, o.timeoutS));

  const work = mkdtempSync(path.join(os.tmpdir(), "ep9-mem-work-"));
  try {
    const files = materialize(o.workloads, work, { harness: o.harness });
    const once = async (e, w) => {
      const r = await exec(byId[e], files[w].file);
      checkMemRun(r, { engine: e, workload: w, expect: files[w].expect });
      return r;
    };
    for (const w of o.workloads) for (const e of o.engines) await once(e, w);
    log(`output check: ${o.workloads.length} workloads x ${o.engines.length} engines ok`);

    let before = null;
    if (hostFile) {
      before = await guardBefore(
        { idleBaseline: hostFile.idle_baseline_load1, settleS: o.settleS, waitS: o.waitMin * 60, pollS: o.pollS },
        { load, sleep },
      );
      log(`load guard before: load ${before.load1}, limit ${before.limit}, ${before.pass ? "pass" : `FAIL (${before.why})`}`);
      if (!before.pass) {
        if (!o.allowBusy) {
          throw new HostBusy(before.limit === null ? `cannot run the load guard: ${before.why}` : `host busy: ${before.why} after waiting ${before.waited_s} s`);
        }
        if (before.limit !== null) reasons.push(`load guard failed before the run (${before.why})`);
      }
    }

    const plan = schedule(o.engines, o.workloads, o.runs);
    const samples = [];
    const roundLoad = [];
    for (const [seq, p] of plan.entries()) {
      const r = await once(p.engine, p.workload);
      samples.push({ seq, round: p.round, workload: p.workload, engine: p.engine, rss_bytes: r.rss, ms: Number(r.ms.toFixed(3)) });
      if (seq + 1 === plan.length || plan[seq + 1].round !== p.round) {
        roundLoad.push(load());
        log(`round ${p.round + 1}/${o.runs}: load ${roundLoad.at(-1)}`);
      }
    }

    let after = null;
    if (hostFile) {
      after = await guardAfter({ idleBaseline: hostFile.idle_baseline_load1, afterS: o.afterS }, { load, sleep });
      log(`load guard after: load ${after.load1}, limit ${after.limit}, ${after.pass ? "pass" : `FAIL (${after.why})`}`);
      if (!after.pass && after.limit !== null) reasons.push(`load guard failed after the run (${after.why})`);
    }

    const a = analyze(samples, { engines: o.engines, workloads: o.workloads });
    const binary = combineBuilds(engines.filter((e) => ENGINES[e.id]?.ep9).map((e) => e.build ?? null));
    return {
      schema: SCHEMA,
      label: o.label,
      started,
      finished: new Date().toISOString(),
      // The commit of the measured ep9 binaries (P0-BUILDID), not of this checkout: null when one
      // has no build line, was built dirty or names another commit (m2-exit: OPEN).
      commit: { ep9: cleanCommit(binary), binary, runner: git(["rev-parse", "HEAD"]), runner_dirty: (git(["status", "--porcelain", "--untracked-files=no"]) ?? "") !== "" },
      host: {
        file: o.host && path.relative(ROOT, o.host),
        id: hostFile?.id ?? null,
        gate_host: hostFile?.gate_host ?? false,
        idle_baseline_load1: hostFile?.idle_baseline_load1 ?? null,
        measured,
      },
      load: { margin: LOAD_MARGIN, before, after, after_each_round: roundLoad },
      provisional: reasons.length > 0,
      provisional_reasons: reasons,
      reader: { tool: tool.name, program: path.basename(tool.program), unit_of_tool: tool.unit, unit: "bytes" },
      runs: o.runs,
      harness: o.harness,
      engines: engines.map((e) => ({ id: e.id, label: e.label, role: e.role, version: e.version, engine_build: e.engine_build ?? null, build: e.build ?? null, bin: path.basename(e.bin), args: e.args })),
      workloads: Object.values(files).filter((f) => o.workloads.includes(f.name)).map(({ file, ...w }) => ({ ...w, file: path.basename(file) })),
      samples,
      ...a,
    };
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

// ---- Markdown -------------------------------------------------------------------------------

const mib = (b) => (b == null ? "-" : (b / (1 << 20)).toFixed(1));
const f3 = (x) => (x == null ? "-" : x.toFixed(3));

export function renderMarkdown(res) {
  const L = [];
  const eng = Object.fromEntries(res.engines.map((e) => [e.id, e]));
  const g = (c) => (!c ? "not checked" : c.limit === null ? `${c.load1} (no idle baseline)` : `${c.load1} (limit ${c.limit}: ${c.pass ? "pass" : "FAIL"})`);
  L.push(`Peak RSS ${res.label ? `"${res.label}" ` : ""}${res.started} to ${res.finished}; commit \`${(res.commit.ep9 ?? "unknown").slice(0, 12)}\`${buildNote(res.commit.binary)}.`);
  L.push("");
  L.push(`- Host: ${res.host.id ?? "no host file"}: ${res.host.measured.cpu}, ${res.host.measured.os} (${res.host.measured.platform}).`);
  L.push(`- Load: before ${g(res.load.before)}; after ${g(res.load.after)}; after each round ${Math.min(...res.load.after_each_round)}-${Math.max(...res.load.after_each_round)}.`);
  L.push(`- Engines: ${res.engines.map((e) => `${e.label} (${e.version}${e.engine_build ? `, ${e.engine_build}` : ""})`).join("; ")}.`);
  L.push(`- Reader: ${res.reader.tool}; ${res.runs} runs per engine and workload, rotated order; median. Ratio = median(ep9) / median(comparator).`);
  L.push(res.provisional ? `- **PROVISIONAL**: ${res.provisional_reasons.join("; ")}.` : "- Class Mem reading: the load guard passed before and after.");
  L.push("");
  L.push(`| workload | ${res.engines.map((e) => `${e.label} MiB`).join(" | ")} |`);
  L.push(`|---|${res.engines.map(() => "---").join("|")}|`);
  for (const w of [...new Set(res.table.map((t) => t.workload))]) {
    L.push(`| ${w} | ${res.engines.map((e) => {
      const t = res.table.find((x) => x.workload === w && x.engine === e.id);
      return `${mib(t.median_bytes)} [${mib(t.min_bytes)}-${mib(t.max_bytes)}]`;
    }).join(" | ")} |`);
  }
  L.push("");
  L.push("| cap | ep9 build | " + [...new Set(res.ratios.map((r) => r.workload))].join(" | ") + " | max | geomean | holds |");
  L.push(`|---|---|${[...new Set(res.ratios.map((r) => r.workload))].map(() => "---").join("|")}|---|---|---|`);
  for (const c of res.caps) {
    L.push(`| ${c.text} | ${eng[c.subject].label} | ${Object.values(c.ratios).map(f3).join(" | ")} | ${f3(c.max_ratio)} (${c.max_workload}) | ${f3(c.geomean_ratio)} | ${c.holds ? "yes" : `no (${c.over.join(", ")})`} |`);
  }
  L.push("");
  L.push(`G7 (both caps, both ep9 builds, the 7 v8-v7 kernels): ${res.g7.holds === null ? `no reading (missing: ${(res.g7.missing ?? []).join(", ")})` : res.g7.holds ? "holds" : "does not hold"}${res.provisional ? " (provisional)" : ""}.`);
  return `${L.join("\n")}\n`;
}

// ---- main -----------------------------------------------------------------------------------

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) {
    const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n");
    process.stdout.write(`${src.slice(1, src.findIndex((l, i) => i > 0 && !l.startsWith("//"))).map((l) => l.replace(/^\/\/ ?/, "")).join("\n")}\n`);
    return;
  }
  let o;
  let res;
  try {
    o = parseArgs(argv);
    res = await run(o);
  } catch (e) {
    if (e instanceof UsageError) {
      console.error(`mem.mjs: ${e.message}`);
      process.exit(2);
    }
    if (e instanceof HostBusy) {
      console.error(`mem.mjs: ${e.message}. Wait for a quiet window, or pass --allow-busy for a provisional run.`);
      process.exit(3);
    }
    if (e instanceof Refusal) {
      console.error(`REFUSED: ${e.message}\nmem.mjs: a run gave a wrong result; no report was written.`);
      process.exit(1);
    }
    throw e;
  }
  const md = renderMarkdown(res);
  if (o.out) writeFileSync(o.out, `${JSON.stringify(res, null, 1)}\n`);
  if (o.md) writeFileSync(o.md, md);
  process.stdout.write(md);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main().catch((e) => {
    console.error(e.stack ?? String(e));
    process.exit(1);
  });
}
