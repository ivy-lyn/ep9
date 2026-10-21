#!/usr/bin/env node
// Measures a host's idle baseline for tools/bench/hosts/<host>.toml (docs/plan/10-quality.md
// section 3: "the idle baseline (median 1-minute load over 10 idle minutes)").
//
//   node tools/bench/cross/idle.mjs [--minutes 10] [--every-s 10]
//
// It samples the 1-minute load, and lists build or benchmark processes (cargo, rustc, cc, ld,
// ep9, qjs, d8, bs, valgrind) at each sample. A sample with such a process is busy: the tool then
// prints no baseline and exits 1. Run it with nothing else running; it cannot see other load.
// On success it prints the line to put into the host file.

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleepMs } from "node:timers/promises";

import { load1 } from "../lib/host.mjs";
import { median } from "../lib/stats.mjs";

const BUSY = /^(cargo|rustc|cc|clang|ld|ld64|lld|ep9|ep9-bench|qjs|d8|bs|valgrind|v8-[\d.]+)$/;

export function busyProcesses() {
  if (process.platform === "win32") return [];
  const r = spawnSync("ps", ["-Ao", "comm"], { encoding: "utf8" });
  return (r.stdout || "")
    .split("\n")
    .slice(1)
    .map((l) => path.basename(l.trim()))
    .filter((n) => BUSY.test(n));
}

async function main() {
  const argv = process.argv.slice(2);
  let minutes = 10;
  let every = 10;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--minutes") minutes = Number(argv[++i]);
    else if (argv[i] === "--every-s") every = Number(argv[++i]);
    else {
      console.error(`idle.mjs: unknown option ${argv[i]}`);
      process.exit(2);
    }
  }
  const n = Math.max(1, Math.round((minutes * 60) / every));
  const samples = [];
  const busy = [];
  for (let i = 0; i < n; i++) {
    const b = busyProcesses();
    samples.push(load1());
    if (b.length) busy.push({ at: new Date().toISOString(), processes: [...new Set(b)] });
    process.stderr.write(`${new Date().toISOString()} load ${samples.at(-1)}${b.length ? ` BUSY: ${[...new Set(b)].join(" ")}` : ""}\n`);
    if (i + 1 < n) await sleepMs(every * 1000);
  }
  const med = median(samples);
  console.log(`# ${new Date().toISOString()}: ${n} samples every ${every} s; min ${Math.min(...samples)}, max ${Math.max(...samples)}`);
  console.log(`# samples = [${samples.join(", ")}]`);
  if (busy.length) {
    console.log(`# NOT IDLE: ${busy.length} sample(s) saw build or benchmark processes; no baseline.`);
    process.exit(1);
  }
  console.log(`idle_baseline_load1 = ${med}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
