#!/usr/bin/env node
// Measures a host's idle baseline for tools/bench/hosts/<host>.toml (docs/plan/10-quality.md
// section 3: "the idle baseline (median 1-minute load over 10 idle minutes)").
//
//   node tools/bench/cross/idle.mjs [--minutes 10] [--every-s 10]
//
// It samples the load of the host's load source (lib/host.mjs: the 1-minute load; on Windows a
// 60-s typeperf reading), and lists build or benchmark processes (cargo, rustc, cc, ld, ep9, qjs,
// d8, bs, valgrind; `ps` on Linux and macOS, `tasklist` on Windows) at each sample. A sample with such a process is busy: the tool then
// prints no baseline and exits 1. Run it with nothing else running; it cannot see other load.
// On success it prints the lines to put into the host file (the baseline and its load source).

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleepMs } from "node:timers/promises";

import { load1, loadSource } from "../lib/host.mjs";
import { median } from "../lib/stats.mjs";

const BUSY = /^(cargo|rustc|cc|clang|ld|ld64|lld|ep9|ep9-bench|qjs|d8|bs|valgrind|v8-[\d.]+)$/;

/**
 * The image names of a `tasklist /fo csv /nh` listing (one `"<image>","<pid>",...` line per
 * process), without the `.exe` suffix.
 */
export function tasklistNames(text) {
  return (text ?? "")
    .split(/\r?\n/)
    .map((l) => l.match(/^"([^"]*)"/)?.[1])
    .filter(Boolean)
    .map((n) => n.replace(/\.exe$/i, ""));
}

/** The running build or benchmark processes (`run` is injectable for tests). */
export function busyProcesses(platform = process.platform, run = spawnSync) {
  if (platform === "win32") {
    const r = run("tasklist", ["/fo", "csv", "/nh"], { encoding: "utf8", windowsHide: true });
    // Windows image names are not case-sensitive.
    return tasklistNames(r.stdout).map((n) => n.toLowerCase()).filter((n) => BUSY.test(n));
  }
  const r = run("ps", ["-Ao", "comm"], { encoding: "utf8" });
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
  console.log(`load_source = "${loadSource()}"`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
