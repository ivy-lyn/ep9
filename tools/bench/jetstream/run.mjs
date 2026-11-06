// Runs JetStream workloads one process at a time and prints per-workload median scores.
//
// usage: node tools/bench/jetstream/run.mjs --list <file> --reps <n> --out <json>
//             --config <name>=<engine>[:<flag>,<flag>...] [--config ...] [--iterations <n>]
//             [--ep9 <bin>]
//   engine: "node" (this node, through node-host.mjs), "node@<path>" (that node binary, through
//           node-host.mjs), "ep9" (the --ep9 binary, through ep9-host.mjs), "ep9@<path>" (that ep9
//           binary), or a path to a d8 binary (runs cli.js).
//   flags:  engine flags; a flag "cli:<arg>" goes to cli.js instead. For ep9 the flags go to the
//           ep9 binary, and a flag "host:<arg>" goes to ep9-host.mjs (e.g. host:--script-mode=strict).
//   example: --config full=node --config maxopt2=node:--max-opt=2
//            --config jitless=node:--jitless,cli:--no-prefetch   (no Wasm: see scripts/sync-bench.sh)
//            --config node-jitless=node@third_party/shells/.../bin/node:--jitless,cli:--no-prefetch
//            --config ep9=ep9@target/release/ep9               (the M2 list: 09-milestones M2 exit 7)
//   ep9: the JetStream driver runs on node and each workload realm on ep9 (ep9-host.mjs says how
//   and what differs from a JSC realm). The "ep9-host:" lines of its stderr go into "notes" of
//   each run. An ep9 config cannot take cli:--no-prefetch (the realm cannot read files).
//
// Order: for each rep, for each workload, every config runs back to back. The config order
// rotates per workload, so slow drift on a shared machine hits every config alike.
// Score per run = JetStream's workload score (geomean of First, Worst, Average; higher is better).
// Exit 1 if a workload has fewer than --reps scores in a config (a crash, no score): the JSON is
// still written, with "complete": false, the short pairs in "missing" and a null summary score.
// Tests: node --test tools/bench/jetstream/run.test.mjs
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
// EP9_JETSTREAM_DIR overrides the checkout; run.test.mjs points it at an empty dir.
const jsDir = process.env.EP9_JETSTREAM_DIR || path.join(root, "third_party/bench-jetstream");
const host = path.join(root, "tools/bench/jetstream/node-host.mjs");
const ep9Host = path.join(root, "tools/bench/jetstream/ep9-host.mjs");

const opts = { configs: [], reps: 3, list: null, out: null, iterations: null, ep9: null };
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
  const v = argv[++i];
  switch (argv[i - 1]) {
    case "--list": opts.list = v; break;
    case "--reps": opts.reps = Number(v); break;
    case "--out": opts.out = v; break;
    case "--iterations": opts.iterations = Number(v); break;
    case "--ep9": opts.ep9 = path.resolve(v); break;
    case "--config": {
      const eq = v.indexOf("=");
      const [name, spec] = [v.slice(0, eq), v.slice(eq + 1)];
      const [engine, flags = ""] = spec.split(/:(.*)/s);
      const all = flags ? flags.split(",") : [];
      opts.configs.push({
        name,
        engine,
        flags: all.filter((f) => !f.startsWith("cli:") && !f.startsWith("host:")),
        cliFlags: all.filter((f) => f.startsWith("cli:")).map((f) => f.slice(4)),
        hostFlags: all.filter((f) => f.startsWith("host:")).map((f) => f.slice(5)),
      });
      break;
    }
    default: throw new Error(`unknown argument ${argv[i - 1]}`);
  }
}
if (!opts.list || !opts.out || !opts.configs.length) {
  console.error("usage: run.mjs --list <file> --out <json> --config <name>=<engine>[:flags] [--reps n]");
  process.exit(2);
}
// Engine kinds: node-host.mjs on a node binary, ep9-host.mjs on an ep9 binary, or a d8 binary.
for (const cfg of opts.configs) {
  const [kind, bin] = cfg.engine.split(/@(.*)/s);
  if (cfg.engine === "node" || kind === "node") {
    cfg.kind = "node";
    cfg.bin = cfg.engine === "node" ? process.execPath : path.resolve(bin);
  } else if (cfg.engine === "ep9" || kind === "ep9") {
    cfg.kind = "ep9";
    cfg.bin = cfg.engine === "ep9" ? opts.ep9 : path.resolve(bin);
    if (!cfg.bin) {
      console.error(`config ${cfg.name}: engine "ep9" needs --ep9 <bin> (or use ep9@<path>)`);
      process.exit(2);
    }
    if (cfg.cliFlags.includes("--no-prefetch")) {
      console.error(`config ${cfg.name}: an ep9 realm cannot read files, so cli:--no-prefetch is not possible`);
      process.exit(2);
    }
  } else {
    cfg.kind = "d8";
    cfg.bin = cfg.engine;
  }
  if (cfg.kind !== "ep9" && cfg.hostFlags.length) {
    console.error(`config ${cfg.name}: host: flags are for ep9 configs only`);
    process.exit(2);
  }
  cfg.version = engineVersion(cfg);
}

/** The engine's version line (ep9: its two --version lines), or null. */
function engineVersion(cfg) {
  try {
    const args = cfg.kind === "d8" ? ["-e", "print(version())"] : ["--version"];
    const out = execFileSync(cfg.bin, args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 30000 });
    return out.trim().split("\n").join("; ") || null;
  } catch {
    return null;
  }
}

const names = fs.readFileSync(opts.list, "utf8").split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));

const clip = (s, k = 1 << 15) => (s.length > 2 * k ? `${s.slice(0, k)}\n[... ${s.length - 2 * k} chars cut ...]\n${s.slice(-k)}` : s);

function runOnce(cfg, name) {
  const cli = [`--test=${name}`, "--dump-json-results", ...cfg.cliFlags];
  if (opts.iterations) cli.push(`--iteration-count=${opts.iterations}`);
  const [cmd, args, cwd] =
    cfg.kind === "node"
      ? [cfg.bin, [...cfg.flags, host, jsDir, ...cli], root]
      : cfg.kind === "ep9"
        ? [process.execPath, [ep9Host, "--ep9", cfg.bin, ...cfg.flags.flatMap((f) => ["--ep9-arg", f]), ...hostArgs(cfg), jsDir, ...cli], root]
        : [cfg.engine, [...cfg.flags, "cli.js", "--", ...cli], jsDir];
  const t0 = process.hrtime.bigint();
  const r = spawnSync(cmd, args, { cwd, encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "pipe"] });
  const stderr = String(r.stderr ?? "");
  const notes = cfg.kind === "ep9" ? hostNotes(stderr) : undefined;
  const extra = notes ? { notes } : {};
  if (r.error || r.status !== 0) {
    // Keep the exit status, the signal and the stderr text: a V8 "FATAL ERROR" line can be below line 3.
    const how = r.signal ? `signal ${r.signal}` : r.error ? r.error.message : `exit ${r.status}`;
    const error = `${how}: ${(stderr || r.error?.message || "").split("\n").slice(0, 3).join(" | ")}`;
    return { error, status: r.status, signal: r.signal, stderr: clip(stderr), ...extra, ...(notes && { cause: causeOf(notes) }) };
  }
  const stdout = r.stdout;
  const wall = Number(process.hrtime.bigint() - t0) / 1e9;
  const line = stdout.split("\n").find((l) => l.startsWith('{"JetStream3.0"'));
  if (!line) return { error: "no JSON result in output", ...extra };
  const tests = JSON.parse(line)["JetStream3.0"].tests;
  const score = tests[name]?.metrics?.Score?.current?.[0] ?? Object.values(tests)[0]?.metrics?.Score?.current?.[0];
  return Number.isFinite(score) ? { score, wall, ...extra } : { error: "no score", ...extra };
}

/** ep9-host.mjs flags of a config: "host:--script-mode=strict" -> --script-mode strict. */
function hostArgs(cfg) {
  return cfg.hostFlags.flatMap((f) => {
    const eq = f.indexOf("=");
    return eq < 0 ? [f] : [f.slice(0, eq), f.slice(eq + 1)];
  });
}

/** The "ep9-host:" lines of an ep9 run's stderr (ep9-host.mjs). */
function hostNotes(stderr) {
  return stderr.split("\n").filter((l) => l.startsWith("ep9-host: ")).map((l) => l.slice(10));
}

/** The cause of a failed ep9 run: its first note that is not a realm summary or a mode note. */
function causeOf(notes) {
  return notes.find((n) => !/^realm \d+: (\d+ scripts,|realm runs as)/.test(n)) ?? null;
}

function shortPath(p) {
  if (!p) return p;
  const rel = path.relative(root, p);
  return rel && !rel.startsWith("..") && !path.isAbsolute(rel) ? rel : path.basename(p);
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};
const geomean = (xs) => Math.exp(xs.reduce((a, x) => a + Math.log(x), 0) / xs.length);

const runs = [];
const started = new Date().toISOString();
const load = [os.loadavg()];
for (let rep = 0; rep < opts.reps; rep++) {
  names.forEach((name, k) => {
    const order = opts.configs.map((_, j) => opts.configs[(j + k + rep) % opts.configs.length]);
    for (const cfg of order) {
      const r = runOnce(cfg, name);
      runs.push({ rep, name, config: cfg.name, ...r });
      process.stderr.write(`rep ${rep} ${name} ${cfg.name}: ${r.error ?? r.score.toFixed(2)}\n`);
    }
  });
  load.push(os.loadavg());
}

// Medians per workload and config; ratios against the first config.
const base = opts.configs[0].name;
const table = names.map((name) => {
  const row = { name };
  for (const cfg of opts.configs) {
    const xs = runs.filter((r) => r.name === name && r.config === cfg.name && r.score).map((r) => r.score);
    row[cfg.name] = xs.length ? median(xs) : null;
    row[`${cfg.name}_n`] = xs.length;
  }
  return row;
});
// Each workload needs --reps scores in each config. A failed run must not drop out of a median or
// out of the list: then a config that crashes on hard workloads gets a better ratio.
const missing = table.flatMap((row) =>
  opts.configs
    .filter((cfg) => row[`${cfg.name}_n`] !== opts.reps)
    .map((cfg) => ({ name: row.name, config: cfg.name, n: row[`${cfg.name}_n`], want: opts.reps })),
);
// Each failed run with its cause (ep9: the first ep9-host note that is not a summary line).
const failures = runs
  .filter((r) => r.error)
  .map((r) => ({ name: r.name, config: r.config, rep: r.rep, error: r.error, cause: r.cause ?? null }));
const summary = {};
for (const cfg of opts.configs) {
  const ok = table.filter((r) => r[cfg.name] && r[base]);
  // A list score or a ratio from a part of the samples is not valid: it is null.
  const complete = !missing.some((m) => m.config === cfg.name || m.config === base);
  summary[cfg.name] = {
    complete,
    workloads: ok.length,
    geomean_score: complete && ok.length ? geomean(ok.map((r) => r[cfg.name])) : null,
    ratio_vs_first: complete && ok.length ? geomean(ok.map((r) => r[cfg.name] / r[base])) : null,
  };
}
const result = {
  started,
  finished: new Date().toISOString(),
  host: { platform: process.platform, arch: process.arch, cpus: os.cpus().length, node: process.version },
  loadavg_after_each_rep: load,
  jetstream_commit: fs.readFileSync(path.join(root, "third_party/bench-jetstream.commit"), "utf8").trim(),
  list: path.relative(root, path.resolve(opts.list)),
  reps: opts.reps,
  // A binary inside this checkout by its relative path (the JSON may go into docs/measurements).
  configs: opts.configs.map(({ bin, ...c }) => ({
    ...c,
    engine: c.kind === "d8" ? shortPath(c.engine) : c.engine.replace(/@(.*)$/s, (_, p) => `@${shortPath(path.resolve(p))}`),
    bin: shortPath(bin),
  })),
  complete: !missing.length,
  missing,
  failures,
  summary,
  table,
  runs,
};
fs.writeFileSync(opts.out, JSON.stringify(result, null, 1) + "\n");
console.log(JSON.stringify(summary, null, 1));
if (missing.length) {
  console.error(`INCOMPLETE: the comparison is not valid. Runs short of --reps ${opts.reps} (errors in "runs" of ${opts.out}):`);
  for (const m of missing) console.error(`  ${m.name} ${m.config}: ${m.n} of ${m.want}`);
  process.exitCode = 1;
}
