#!/usr/bin/env node
// perf CI v0 (ci.yml job perf-ci-v0; docs/plan/10-quality.md section 2): instruction counts of the
// T0 kernels, ci-v0 and the generated v8-v7 richards and deltablue, under one counter, as one JSON
// file plus a Markdown table. Tools: tools/bench/README.md "perf CI v0".
//
//   node .github/scripts/perf-ci-v0.mjs --ep9 target/release/ep9 --bench target/release/ep9-bench \
//     --out perf-ci-v0.json [--md FILE] [--counter cachegrind] [--runs 3] [--warmup 0] \
//     [--hash-seed 1] [--workloads t0-loop,...] [--jobs N] [--work DIR] [--timeout-s 1800]
//
//   workload  file it runs
//   --------  ------------
//   t0-loop, t0-branchy, t0-call, t0-prop, ci-v0
//             tools/bench/workloads/<name>.js as it is (BENCH(k) of the week-4 plan)
//   richards, deltablue
//             written by tools/bench/cross/v8v7.mjs from the pinned third_party/bench-v8v7
//             (bash scripts/sync-bench.sh bench-v8v7); it checks the pin
//
// Per workload:
//   1. check  one run of ep9 with no counter: exit 0 and stdout = the expected output of
//             tools/bench/cross/workloads.mjs (the T0 `want`; "<name> <N>" per v8-v7 benchmark),
//             the refusal rule of cross/run.mjs. A wrong answer gets no count.
//   2. count  ep9-bench --counter C --runs R --warmup W --expect-stdout <the expected output>; its
//             ep9-bench/v1 JSON is kept whole. Every run it makes must print the expected output
//             too, else it exits 4 and the workload is refused: a check run alone cannot vouch for
//             the runs that are counted.
// Every ep9 run gets EP9_HASH_SEED (--hash-seed; default 1). The string hash seed is random per
// process otherwise, and it changes how far shape-index probes go: t0-call moved by 3.7% between
// 3 cachegrind runs with random seeds, and by 609 instructions in 6.1e9 with EP9_HASH_SEED=1.
// Workloads are counted in parallel (--jobs, default: the CPU count). cachegrind counts the
// instructions of the program it runs, so a process next to it does not change the count.
// R = 3, W = 0 by default: with a fixed seed cachegrind gives the same count on every run (the
// spread column shows it), so warm-up runs and more runs add time, not information.
//
// Commit: GITHUB_SHA, else EP9_COMMIT, else `git rev-parse HEAD`, else null.
// Exit status: 0 every workload has a count; 1 a workload was refused or has no count (the JSON
// and the table are still written, with null counts); 2 usage error.

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { checkRun, parseBenchJson, Refusal, spawnRun } from "../../tools/bench/cross/run.mjs";
import { generate, ROOT } from "../../tools/bench/cross/v8v7.mjs";
import { WORKLOADS } from "../../tools/bench/cross/workloads.mjs";
import { hostInfo } from "../../tools/bench/lib/host.mjs";

export const SCHEMA = "ep9-perf-ci/v1";
export const DEFAULT_WORKLOADS = ["t0-loop", "t0-branchy", "t0-call", "t0-prop", "ci-v0", "richards", "deltablue"];

class UsageError extends Error {}

export function parseArgs(argv) {
  const o = {
    ep9: null,
    bench: null,
    out: null,
    md: null,
    counter: "cachegrind",
    runs: 3,
    warmup: 0,
    hashSeed: "1",
    workloads: DEFAULT_WORKLOADS,
    jobs: os.availableParallelism?.() ?? os.cpus().length,
    work: null,
    timeoutS: 1800,
  };
  const num = (a, v, min) => {
    const n = Number(v);
    if (!Number.isInteger(n) || n < min) throw new UsageError(`${a} needs an integer >= ${min}`);
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
      case "--bench": o.bench = path.resolve(val()); break;
      case "--out": o.out = val(); break;
      case "--md": o.md = val(); break;
      case "--counter": o.counter = val(); break;
      case "--runs": o.runs = num(a, val(), 1); break;
      case "--warmup": o.warmup = num(a, val(), 0); break;
      case "--hash-seed": o.hashSeed = val(); break;
      case "--workloads": o.workloads = val().split(",").filter(Boolean); break;
      case "--jobs": o.jobs = num(a, val(), 1); break;
      case "--work": o.work = path.resolve(val()); break;
      case "--timeout-s": o.timeoutS = num(a, val(), 1); break;
      default: throw new UsageError(`unknown option ${a}`);
    }
  }
  if (!o.ep9 || !o.bench || !o.out) throw new UsageError("--ep9, --bench and --out are required");
  for (const w of o.workloads) {
    if (!WORKLOADS[w]) throw new UsageError(`unknown workload ${w} (known: ${Object.keys(WORKLOADS).join(", ")})`);
  }
  return o;
}

function commitId() {
  const env = process.env.GITHUB_SHA || process.env.EP9_COMMIT;
  if (env) return env;
  const r = spawnSync("git", ["-C", ROOT, "rev-parse", "HEAD"], { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
}

const sha256 = (text) => createHash("sha256").update(text).digest("hex");

/** The file of one workload and what it must print; throws when v8v7.mjs cannot generate it. */
function prepare(name, work) {
  const w = WORKLOADS[name];
  if (!w.v8v7) {
    const file = path.join(ROOT, w.file);
    return { name, file: w.file, path: file, sha256: sha256(readFileSync(file)), expect: w.expect };
  }
  const g = generate(w.v8v7, { root: ROOT });
  const file = path.join(work, `${name}.js`);
  writeFileSync(file, g.text);
  return {
    name,
    file: `generated by tools/bench/cross/v8v7.mjs --kernel ${w.v8v7}`,
    path: file,
    sha256: sha256(g.text),
    expect: g.expect,
    source: `third_party/bench-v8v7 ${g.source}`,
    pin: g.pin,
    blob: g.blob,
    n: g.n,
  };
}

/** Runs ep9-bench on one workload; resolves to { bench, error, refused }. */
function count(o, w, commit, tmp) {
  const out = path.join(tmp, `${w.name}.bench.json`);
  const expect = path.join(tmp, `${w.name}.expect.txt`);
  writeFileSync(expect, w.expect);
  const args = ["--counter", o.counter, "--runs", String(o.runs), "--warmup", String(o.warmup), "--name", w.name, "--out", out, "--expect-stdout", expect, "--", o.ep9, w.path];
  const env = commit && !process.env.GITHUB_SHA ? { ...process.env, EP9_COMMIT: commit } : process.env;
  return new Promise((resolve) => {
    const child = spawn(o.bench, args, { env, stdio: ["ignore", "ignore", "pipe"], timeout: o.timeoutS * 1000 });
    let stderr = "";
    child.stderr.on("data", (d) => {
      stderr = (stderr + d).slice(-4000);
    });
    child.on("error", (e) => resolve({ bench: null, error: `${o.bench}: ${e.message}` }));
    child.on("close", (code, signal) => {
      let bench = null;
      try {
        bench = JSON.parse(readFileSync(out, "utf8"));
      } catch {}
      if (code === 0 && bench) return resolve({ bench, error: null });
      const tail = stderr.trim().split("\n").slice(-5).join(" / ");
      const how = signal ? `signal ${signal}` : `exit ${code}`;
      // Exit 3: no counter on this host; the JSON is written with null counts and says why.
      // Exit 4: a run printed something else than the expected output; no JSON.
      resolve({ bench, refused: code === 4, error: `ep9-bench ${how}${bench?.counter_note ? `: ${bench.counter_note}` : ""}${tail ? `: ${tail}` : ""}` });
    });
  });
}

/** Runs `fn` on every item, at most `jobs` at a time. */
async function pool(items, jobs, fn) {
  const out = new Array(items.length);
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const i = next++;
      out[i] = await fn(items[i]);
    }
  };
  await Promise.all(Array.from({ length: Math.min(jobs, items.length) }, worker));
  return out;
}

export async function run(o) {
  const commit = commitId();
  // Inherited by every ep9 run below: the check run, and ep9-bench (which passes it on).
  process.env.EP9_HASH_SEED = o.hashSeed;
  const tmp = mkdtempSync(path.join(os.tmpdir(), "ep9-perf-ci-"));
  const work = o.work ?? tmp;
  mkdirSync(work, { recursive: true });
  const ver = spawnSync(o.ep9, ["--version"], { encoding: "utf8" });
  const rows = [];
  try {
    for (const name of o.workloads) {
      let w;
      try {
        w = prepare(name, work);
      } catch (e) {
        rows.push({ name, status: "error", error: e.message });
        continue;
      }
      const r = spawnRun({ bin: o.ep9, args: [] }, w.path, o.timeoutS);
      try {
        checkRun(r, { engine: "ep9", workload: name, expect: w.expect });
        rows.push({ ...w, status: "ok" });
      } catch (e) {
        if (!(e instanceof Refusal)) throw e;
        // checkRun keeps the stderr tail (a stack trace); the first line is the error itself.
        const first = r.stderr.split("\n").find((l) => l.trim());
        rows.push({ ...w, status: "refused", error: `${e.message.split("\n")[0]}${first ? `: ${first.trim()}` : ""}` });
      }
    }
    const todo = rows.filter((r) => r.status === "ok");
    await pool(todo, o.jobs, async (r) => {
      const { bench, error, refused } = await count(o, r, commit, tmp);
      r.bench = bench;
      const s = bench ? parseBenchJson(JSON.stringify(bench)) : null;
      r.instructions = s && s.median !== null ? { min: s.min, median: s.median, max: s.max } : null;
      if (error || !r.instructions) {
        r.status = refused ? "refused" : "error";
        r.error = error ?? "ep9-bench gave no count";
      }
      process.stderr.write(`perf-ci-v0: ${r.name}: ${r.instructions?.median ?? "no count"}${r.error ? ` (${r.error})` : ""}\n`);
    });
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
  // One key order for every row; the local path of a generated file is left out.
  const workloads = rows.map(({ name, status, error, instructions, bench, path: _, ...file }) => ({
    name,
    status,
    error: error ?? null,
    instructions: instructions ?? null,
    ...file,
    bench: bench ?? null,
  }));
  const counted = workloads.find((r) => r.bench?.counter);
  return {
    schema: SCHEMA,
    commit,
    ep9: { path: path.relative(ROOT, o.ep9), version: ver.status === 0 ? ver.stdout.trim() : null },
    counter: counted?.bench.counter ?? null,
    target: counted?.bench.target ?? null,
    runs: o.runs,
    warmup: o.warmup,
    hash_seed: o.hashSeed,
    host: hostInfo(),
    complete: workloads.every((r) => r.status === "ok"),
    workloads,
  };
}

const fmt = (n) => (n === null || n === undefined ? "" : n.toLocaleString("en-US"));
const cell = (s) => String(s).replace(/\s*\n\s*/g, " / ").replaceAll("|", "\\|").slice(0, 160);

export function renderMarkdown(res) {
  const lines = [
    `### perf CI v0: instructions (${res.counter ?? "no counter"})`,
    "",
    `commit \`${res.commit ?? "unknown"}\`, target \`${res.target ?? "unknown"}\`, CPU ${res.host.cpu}, ` +
      `EP9_HASH_SEED=${res.hash_seed}, ${res.runs} runs + ${res.warmup} warm-up per workload; median, min, max`,
    "",
    "| workload | instructions | min | max | spread | status |",
    "|---|---:|---:|---:|---:|---|",
  ];
  for (const r of res.workloads) {
    const i = r.instructions;
    const spread = i ? `${(((i.max - i.min) / i.median) * 100).toFixed(4)}%` : "";
    lines.push(`| ${r.name} | ${fmt(i?.median)} | ${fmt(i?.min)} | ${fmt(i?.max)} | ${spread} | ${r.status}${r.error ? `: ${cell(r.error)}` : ""} |`);
  }
  if (!res.complete) lines.push("", "**Incomplete:** a workload has no count (see status).");
  return `${lines.join("\n")}\n`;
}

async function main() {
  let o;
  try {
    o = parseArgs(process.argv.slice(2));
  } catch (e) {
    if (!(e instanceof UsageError)) throw e;
    console.error(`perf-ci-v0: ${e.message}`);
    process.exit(2);
  }
  const res = await run(o);
  writeFileSync(o.out, `${JSON.stringify(res, null, 1)}\n`);
  const md = renderMarkdown(res);
  if (o.md) appendFileSync(o.md, md);
  process.stdout.write(md);
  process.exit(res.complete ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main().catch((e) => {
    console.error(e.stack ?? String(e));
    process.exit(1);
  });
}
