#!/usr/bin/env node
// Fake valgrind for .github/scripts/bench-set.test.mjs: `--tool=cachegrind` with the options that
// bench-set.mjs passes, then the program and its arguments. It runs the program (its stdout and
// stderr pass through; its exit status is ours), then writes a cachegrind log and out file in
// the valgrind 3.24 forms (S0 box files), with fixed counts.
//   --version                     prints "valgrind-fake"
//   FAKE_CG=JSON                  counts per "<program name>:<workload name>": { "ir": n, "bim": n }
//                                 (default ir = 1,000,000,000 + 1,000 x the workload name length;
//                                 bim = ir / 100)
//   FAKE_CG_NO_IR=<prog>:<w>      write a log with no "I refs" line
//   FAKE_CG_LOG=FILE              append "<program> <workload> seed=<EP9_HASH_SEED> helpers=<EP9_GC_HELPERS> branch=<yes|no>"
import { spawnSync } from "node:child_process";
import { appendFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const argv = process.argv.slice(2);
if (argv[0] === "--version") {
  console.log("valgrind-fake");
  process.exit(0);
}
const opt = {};
let i = 0;
for (; i < argv.length && argv[i].startsWith("--"); i++) {
  const [k, v] = argv[i].slice(2).split("=");
  opt[k] = v ?? "yes";
}
const [prog, ...args] = argv.slice(i);
const r = spawnSync(prog, args, { stdio: "inherit" });
const name = path.basename(prog);
const w = path.basename(args.at(-1) ?? "", ".js");
const key = `${name}:${w}`;
const table = process.env.FAKE_CG ? JSON.parse(process.env.FAKE_CG) : {};
const ir = table[key]?.ir ?? 1_000_000_000 + 1000 * w.length;
const bim = table[key]?.bim ?? Math.floor(ir / 100);
const branch = opt["branch-sim"] === "yes";
if (process.env.FAKE_CG_LOG) {
  appendFileSync(process.env.FAKE_CG_LOG, `${name} ${w} seed=${process.env.EP9_HASH_SEED ?? "unset"} helpers=${process.env.EP9_GC_HELPERS ?? "unset"} branch=${branch ? "yes" : "no"}\n`);
}
const n = (x) => x.toLocaleString("en-US");
// Branch counts: Bc = 10% of Ir, Bcm = 1% of Bc, Bi = 2% of Ir, Bim as given.
const bc = Math.floor(ir / 10);
const bcm = Math.floor(bc / 100);
const bi = Math.floor(ir / 50);
const log = ["==4201== Cachegrind, a high-precision tracing profiler", "==4201== Using Valgrind-3.24.0 (fake)", "==4201== "];
if (process.env.FAKE_CG_NO_IR !== key) log.push(`==4201== I refs:        ${n(ir)}`);
if (branch) {
  log.push("==4201== ");
  log.push(`==4201== Branches:      ${n(bc + bi)}  (${n(bc)} cond + ${n(bi)} ind)`);
  log.push(`==4201== Mispredicts:   ${n(bcm + bim)}  ( ${n(bcm)} cond +  ${n(bim)} ind)`);
  log.push("==4201== Mispred rate:           1.0% (        1.0%     +        1.0%   )");
}
if (opt["log-file"]) writeFileSync(opt["log-file"], `${log.join("\n")}\n`);
// The out file: 2 functions, one of them in 2 source files (bench-set.mjs sums them).
const hot = Math.floor(ir * 0.6);
const hot2 = Math.floor(ir * 0.1);
const cold = ir - hot - hot2;
const row = (line, x, share) =>
  branch ? `${line} ${x} ${Math.floor(bc * share)} ${Math.floor(bcm * share)} ${Math.floor(bi * share)} ${Math.floor(bim * share)}` : `${line} ${x}`;
const parts = [hot / ir, hot2 / ir];
const out = [
  `cmd: ${prog} ${args.join(" ")}`,
  `events: Ir${branch ? " Bc Bcm Bi Bim" : ""}`,
  "fl=src/interp/ops.rs",
  "fn=ep9_vm::interp::ops::hot",
  row(10, hot, parts[0]),
  "fl=src/interp/dsl.rs",
  "fn=ep9_vm::interp::ops::hot",
  row(20, hot2, parts[1]),
  "fl=src/main.rs",
  `fn=${w}::cold`,
];
// The rest goes to the cold row, so the rows sum to the summary line exactly.
const rest = (total, ...xs) => total - xs.reduce((a, b) => a + b, 0);
out.push(
  branch
    ? `30 ${cold} ${rest(bc, Math.floor(bc * parts[0]), Math.floor(bc * parts[1]))} ${rest(bcm, Math.floor(bcm * parts[0]), Math.floor(bcm * parts[1]))} ${rest(bi, Math.floor(bi * parts[0]), Math.floor(bi * parts[1]))} ${rest(bim, Math.floor(bim * parts[0]), Math.floor(bim * parts[1]))}`
    : `30 ${cold}`,
);
out.push(`summary: ${ir}${branch ? ` ${bc} ${bcm} ${bi} ${bim}` : ""}`);
if (opt["cachegrind-out-file"]) writeFileSync(opt["cachegrind-out-file"], `${out.join("\n")}\n`);
process.exit(r.status ?? 1);
