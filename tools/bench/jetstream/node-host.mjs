// JSC-shell host API on node, so JetStream's cli.js runs under node (10-quality section 2).
//
// usage: node [V8 flags] node-host.mjs <jetstream-dir> [cli.js args...]
//        node node-host.mjs <jetstream-dir> --ep9-dump-benchmarks   (prints name, tags, files as JSON)
//
// cli.js takes the JSC path (no d8 Realm, no SpiderMonkey newGlobal): it calls runString("")
// for a fresh global per workload. Here each global comes from
// vm.createContext(vm.constants.DONT_CONTEXTIFY): an ordinary V8 global with no node
// interceptors, like a d8 Realm. JetStream code never sees node's `process` or `require`.
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const [dir, ...args] = process.argv.slice(2);
if (!dir) {
  console.error("usage: node-host.mjs <jetstream-dir> [cli.js args...]");
  process.exit(2);
}
process.chdir(dir);

const print = (...a) => process.stdout.write(a.join(" ") + "\n");
const printErr = (...a) => process.stderr.write(a.join(" ") + "\n");
const readFile = (p) => fs.readFileSync(p, "utf8");
// JSC: read(path, "binary") gives a Uint8Array; otherwise a string.
const read = (p, mode) => (mode === "binary" ? new Uint8Array(fs.readFileSync(p)) : readFile(p));

// The shell functions every global gets, as in the JSC shell.
function installShellApi(g, run) {
  g.print = print;
  g.printErr = printErr;
  g.readFile = readFile;
  g.read = read;
  g.load = (p) => run(readFile(p), p);
  g.loadString = (s) => run(s, "loadString");
  g.runString = runString;
  g.setTimeout = setTimeout;
  g.clearTimeout = clearTimeout;
  if (typeof globalThis.gc === "function") g.gc = globalThis.gc;
}

function runString(source) {
  const g = vm.createContext(vm.constants.DONT_CONTEXTIFY);
  const run = (s, filename) => vm.runInContext(s, g, { filename });
  installShellApi(g, run);
  g.console = console;
  g.performance = performance;
  if (source) run(source, "runString");
  return g;
}

// The driver runs in a fresh global too. In node's own global, Emscripten code (JetStream's
// wasm zlib) would see `process` and take its node path.
const main = runString("");

if (args[0] === "--ep9-dump-benchmarks") {
  // Load the driver without starting it, then list every benchmark (groups expanded).
  main.arguments = [];
  for (const f of ["./utils/shell-config.js", "./utils/params.js", "./JetStreamDriver.js"]) main.load(f);
  const describe = (b) => ({
    name: b.name,
    tags: [...b.tags].sort(),
    files: b.benchmarks ? [] : [...b.files],
    preload: b.benchmarks ? [] : b.preloadEntries.map(([, v]) => v),
    group: b.benchmarks ? b.benchmarks.map(describe) : undefined,
  });
  print(JSON.stringify(main.loadString("BENCHMARKS").map(describe), null, 1));
} else {
  main.arguments = args;
  main.load("./cli.js");
}
