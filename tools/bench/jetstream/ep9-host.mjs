// JetStream on ep9: the JSC-shell host API (10-quality section 2) with ep9 as the workload engine.
//
// usage: node ep9-host.mjs --ep9 <bin> [--ep9-arg <arg>]... [--script-mode auto|strict|sloppy]
//                          [--keep <dir>] <jetstream-dir> [cli.js args...]
//
// The ep9 shell has only print() and gc() (crates/ep9_cli/src/run.rs). It has no readFile, no
// load, no new global and no event loop. So the work splits like this:
//
//   node (this process)                          ep9 (one process per workload realm)
//   cli.js, params.js, JetStreamDriver.js   -->  the realm prelude (PRELUDE below), then every
//   unchanged, in a fresh V8 global, as           script that the driver gives to loadString(),
//   node-host.mjs runs them. runString("")        in order, as one script file. The workload's
//   returns an "ep9 realm" object.                own timing loop runs here (performance.now).
//                                            <--  top.currentResolve(results) comes back as a
//                                                 marker line on stdout.
//
// The score is JetStream's own score of the iteration times that the workload measured on ep9.
// The driver code that runs on node is not timed: it only builds the scripts and scores them.
//
// Differences from a real JSC realm (each one is written to stderr as an "ep9-host:" line, and
// run.mjs keeps those lines in its JSON):
//   - One script file per realm: the scripts are joined. A file-level "use strict" prologue only
//     works at the start of the file. --script-mode auto (the default) runs the realm as strict
//     code when any script has that prologue, else as sloppy code. A note names the scripts that
//     then run in the other mode.
//   - performance.now(): Temporal.Now.instant() (microsecond steps on macOS), else Date.now()
//     (1 ms steps). Not monotonic.
//   - setTimeout(f): f runs as a promise job; the delay is ignored. ep9 has no event loop.
//   - ShellPrefetchedResources: UTF-8 text, not bytes (ep9 has no TypedArray before M3).
//     ShellTextDecoder.decode gives the text back. A binary resource read throws.
//   - load(), readFile() of other files and runString() throw in the realm.
// The workloads of the M2 list need none of the missing parts (expectations/jetstream-m2.txt).
//
// The driver needs resource prefetch (the default): the realm cannot read files. Prefetch needs
// WebAssembly (the zlib polyfill), and the driver runs on this node, which has it.
//
// --keep <dir>: write each realm's script file there and keep it (to run it again by hand).
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

/** The prefix of the lines that the realm prelude prints for this host. */
export const MARK = "@@ep9-host@@ ";

/**
 * The prelude of each ep9 realm. `files` maps a path to its text (the prefetched resources).
 * It must be valid strict code and sloppy code. Its names stay inside the function.
 */
export function prelude(files) {
  return `(function () {
  const g = globalThis;
  const MARK = ${JSON.stringify(MARK)};
  const send = (o) => print(MARK + JSON.stringify(o));
  const text = (a) => a.map((x) => String(x)).join(" ");
  g.printErr = (...a) => send({ err: text(a) });
  const con = {
    log: (...a) => print(text(a)),
    info: (...a) => print(text(a)),
    debug: (...a) => send({ err: "Debug: " + text(a) }),
    warn: (...a) => send({ err: "Warn: " + text(a) }),
    error: (...a) => send({ err: "Error: " + text(a) }),
    assert: (c, m) => { if (!c) throw new Error("Assertion failed: " + m); },
    trace: () => print(String(new Error("trace").stack)),
  };
  g.console = con;
  let now;
  if (typeof Temporal === "object" && Temporal && Temporal.Now) {
    const t0 = Temporal.Now.instant().epochNanoseconds;
    now = () => Number(Temporal.Now.instant().epochNanoseconds - t0) / 1e6;
  } else {
    const t0 = Date.now();
    now = () => Date.now() - t0;
  }
  g.performance = { now, mark(name) { return { name }; }, measure() {}, timeOrigin: Date.now() };
  g.self = g;
  g.top = {
    currentResolve(r) { send({ resolve: Array.from(r, Number) }); },
    currentReject(e) { send({ reject: String(e), stack: e && e.stack ? String(e.stack) : "" }); },
  };
  let next = 1;
  const live = new Set();
  g.setTimeout = (f, ms, ...args) => {
    const id = next++;
    live.add(id);
    Promise.resolve().then(() => { if (live.delete(id)) f(...args); });
    return id;
  };
  g.clearTimeout = (id) => { live.delete(id); };
  const files = ${JSON.stringify(files)};
  const res = Object.create(null);
  for (const k of Object.keys(files)) res[k] = { ep9HostText: files[k] };
  g.ShellPrefetchedResources = res;
  g.ShellTextDecoder = class ShellTextDecoder {
    decode(x) {
      if (x && typeof x.ep9HostText === "string") return x.ep9HostText;
      throw new TypeError("ep9 host: ShellTextDecoder.decode takes a prefetched resource");
    }
  };
  const readText = (p) => {
    if (Object.prototype.hasOwnProperty.call(files, p)) return files[p];
    throw new Error("ep9 host: no file " + p + " in this realm (only prefetched resources)");
  };
  g.readFile = readText;
  g.read = (p, mode) => {
    if (mode === "binary") throw new TypeError("ep9 host: a binary read needs TypedArray (M3)");
    return readText(p);
  };
  g.load = (p) => { throw new Error("ep9 host: load(" + p + ") is not available in a realm"); };
  g.loadString = () => { throw new Error("ep9 host: loadString is not available in a realm"); };
  g.runString = () => { throw new Error("ep9 host: runString is not available in a realm"); };
})();
`;
}

/** True when `src` starts with a "use strict" directive (after comments and white space). */
export function hasStrictPrologue(src) {
  const rest = src.replace(/^(?:\s|\/\/[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/)*/, "");
  return /^(["'])use strict\1/.test(rest);
}

/**
 * The script file of one realm: the prelude, then each script, each on its own lines and after a
 * `;`, so no script runs on into the next one. Returns { text, strict, notes }.
 */
export function bundle(scripts, files, mode = "auto") {
  const flags = scripts.map(hasStrictPrologue);
  const any = flags.some(Boolean);
  const strict = mode === "strict" || (mode === "auto" && any);
  const notes = [];
  const other = flags.flatMap((f, i) => (f !== strict ? [i] : []));
  if (other.length && (any || strict)) {
    notes.push(
      `realm runs as ${strict ? "strict" : "sloppy"} code (--script-mode ${mode}); ` +
        `${other.length} of ${scripts.length} scripts have ${strict ? "no" : "a"} "use strict" prologue: ` +
        other.map((i) => `#${i} ${label(scripts[i])}`).join(", "),
    );
  }
  const head = strict ? '"use strict";\n' : "";
  const text = head + prelude(files) + scripts.map((s) => `;\n${s}\n`).join("");
  return { text, strict, notes };
}

/** A short name for a script: its first line that is not empty, cut to 60 characters. */
function label(s) {
  const line = s.split("\n").map((l) => l.trim()).find(Boolean) ?? "";
  return JSON.stringify(line.length > 60 ? `${line.slice(0, 57)}...` : line);
}

/** The text of each prefetched resource (bytes in UTF-8, as JetStream's TextDecoder reads them). */
export function resourceTexts(res) {
  const out = {};
  if (!res) return out;
  for (const [k, v] of Object.entries(res)) {
    out[k] = typeof v === "string" ? v : new TextDecoder().decode(v instanceof Uint8Array || ArrayBuffer.isView(v) ? v : new Uint8Array(v));
  }
  return out;
}

/**
 * Reads the stdout of an ep9 realm run. Returns { outcome, lines, errs }: outcome is
 * { resolve: [...] }, { reject, stack } or null; lines are the other stdout lines.
 */
export function parseRealmOutput(stdout) {
  let outcome = null;
  const lines = [];
  const errs = [];
  for (const line of stdout.replace(/\r\n/g, "\n").split("\n")) {
    if (!line.startsWith(MARK)) {
      lines.push(line);
      continue;
    }
    const o = JSON.parse(line.slice(MARK.length));
    if ("err" in o) errs.push(o.err);
    else if (!outcome) outcome = o;
  }
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  return { outcome, lines, errs };
}

function main() {
  const argv = process.argv.slice(2);
  const o = { ep9: null, ep9Args: [], mode: "auto", keep: null };
  let i = 0;
  for (; i < argv.length && argv[i].startsWith("--"); i++) {
    const v = argv[i + 1];
    switch (argv[i]) {
      case "--ep9": o.ep9 = v; i++; break;
      case "--ep9-arg": o.ep9Args.push(v); i++; break;
      case "--script-mode": o.mode = v; i++; break;
      case "--keep": o.keep = path.resolve(v); i++; break;
      default: usage(`unknown option ${argv[i]}`);
    }
  }
  const [dir, ...args] = argv.slice(i);
  if (!o.ep9 || !dir) usage("need --ep9 <bin> and <jetstream-dir>");
  if (!["auto", "strict", "sloppy"].includes(o.mode)) usage(`bad --script-mode ${o.mode}`);
  if (args.includes("--no-prefetch")) usage("--no-prefetch: an ep9 realm cannot read files; the driver must prefetch");
  o.ep9 = path.resolve(o.ep9);
  process.chdir(dir);
  if (o.keep) fs.mkdirSync(o.keep, { recursive: true });

  const print = (...a) => process.stdout.write(a.join(" ") + "\n");
  const printErr = (...a) => process.stderr.write(a.join(" ") + "\n");
  const note = (s) => process.stderr.write(`ep9-host: ${s}\n`);
  const readFile = (p) => fs.readFileSync(p, "utf8");
  const read = (p, mode) => (mode === "binary" ? new Uint8Array(fs.readFileSync(p)) : readFile(p));

  let realms = 0;
  // runString(""): a new workload realm on ep9. The driver sets top, console, self,
  // performance and the prefetched resources on it, then calls loadString for each script. The
  // ep9 run starts after the driver's synchronous run() returns (it then awaits top's promise).
  function ep9Realm() {
    const id = ++realms;
    const scripts = [];
    let queued = false;
    const realm = {
      loadString(s) {
        scripts.push(String(s));
        if (!queued) {
          queued = true;
          setTimeout(() => runRealm(id, realm, scripts), 0);
        }
      },
    };
    return realm;
  }

  function runRealm(id, realm, scripts) {
    const top = realm.top;
    const fail = (msg) => {
      note(`realm ${id}: ${msg}`);
      if (top?.currentReject) top.currentReject(new Error(`ep9 realm ${id}: ${msg}`));
    };
    const files = resourceTexts(realm.ShellPrefetchedResources);
    const b = bundle(scripts, files, o.mode);
    for (const n of b.notes) note(`realm ${id}: ${n}`);
    const tmp = o.keep ?? fs.mkdtempSync(path.join(os.tmpdir(), "ep9-jetstream-"));
    const file = path.join(tmp, `realm-${id}.js`);
    fs.writeFileSync(file, b.text);
    const t0 = process.hrtime.bigint();
    const r = spawnSync(o.ep9, [...o.ep9Args, file], { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "pipe"] });
    const ms = Number(process.hrtime.bigint() - t0) / 1e6;
    if (!o.keep) fs.rmSync(tmp, { recursive: true, force: true });
    let out;
    try {
      out = parseRealmOutput(r.stdout ?? "");
    } catch (e) {
      return fail(`bad marker line: ${e.message}`);
    }
    for (const l of out.lines) print(l);
    for (const e of out.errs) printErr(e);
    note(`realm ${id}: ${scripts.length} scripts, ${b.strict ? "strict" : "sloppy"}, ep9 process ${ms.toFixed(0)} ms, exit ${r.status ?? r.signal}`);
    if (r.error) return fail(`cannot run ${o.ep9}: ${r.error.message}`);
    if (r.status !== 0) {
      // The first lines name the cause: "<file>:<line>:<col>: Uncaught <Error>: <message>", then
      // the first stack lines.
      const head = String(r.stderr ?? "").trim().split("\n").slice(0, 3).join(" | ");
      if (o.keep) fs.writeFileSync(path.join(o.keep, `realm-${id}.stderr.txt`), r.stderr ?? "");
      return fail(`ep9 ${r.signal ? `signal ${r.signal}` : `exit ${r.status}`}: ${head}`);
    }
    if (!out.outcome) return fail("the realm gave no result (top.currentResolve was not called)");
    if (out.outcome.resolve) return top.currentResolve(out.outcome.resolve);
    note(`realm ${id}: the workload rejected: ${out.outcome.reject}`);
    const e = new Error(out.outcome.reject);
    e.stack = out.outcome.stack || e.stack;
    top.currentReject(e);
  }

  function installShellApi(g, run) {
    g.print = print;
    g.printErr = printErr;
    g.readFile = readFile;
    g.read = read;
    g.load = (p) => run(readFile(p), p);
    g.loadString = (s) => run(s, "loadString");
    g.runString = (source) => {
      if (source) throw new Error("ep9 host: runString with source is not supported");
      return ep9Realm();
    };
    g.setTimeout = setTimeout;
    g.clearTimeout = clearTimeout;
  }

  // The driver's own global, as in node-host.mjs: a plain V8 global with no node interceptors.
  const main = vm.createContext(vm.constants.DONT_CONTEXTIFY);
  installShellApi(main, (s, filename) => vm.runInContext(s, main, { filename }));
  main.console = console;
  main.performance = performance;
  main.arguments = args;
  main.load("./cli.js");
}

function usage(msg) {
  console.error(`ep9-host.mjs: ${msg}`);
  console.error("usage: node ep9-host.mjs --ep9 <bin> [--ep9-arg <arg>]... [--script-mode auto|strict|sloppy] [--keep <dir>] <jetstream-dir> [cli.js args...]");
  process.exit(2);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) main();
