#!/usr/bin/env node
// STARTUP-LAYOUT (route R:199, plan v13 section 6.2): the linker order file of the start-up path.
// The functions that `ep9 -e 0` runs go next to each other in the text section, so start-up
// touches fewer code pages (the start-up cost after the B0 and INTL W1 rows is mostly code-layout
// page faults: the route note).
//
//   node tools/bench/startup/order.mjs --cov-bin target/cov/release/ep9 --target <triple> \
//     [--out tools/bench/startup/<triple>.order] [--llvm-profdata PATH] [-- <ep9 args>]
//
// --cov-bin is an ep9 built with coverage counters, with the release profile of the binary the
// order file is for (the same toolchain, the same features, the same target):
//
//   CARGO_TARGET_DIR=target/cov RUSTFLAGS="-C instrument-coverage" \
//     cargo build --release --locked -p ep9_cli
//
// The script runs it once with the start-up arguments (default `-e 0`, EP9_HASH_SEED=1), merges
// the profile with llvm-profdata (the rustc sysroot's llvm-tools copy by default: its LLVM is the
// one of the toolchain), and writes one symbol per line: every function with a count above 0,
// most calls first, then by name. Function names in the profile are the symbol names (v0
// mangling; a local function's name has a "<file>;" prefix, which goes). A function that the
// release build inlines has no symbol there: the linker skips its line. The v0 names do not
// depend on RUSTFLAGS (the crate hashes come from cargo's -C metadata), so the names of the
// coverage build are the names of the release build.
//
// Format per target: Mach-O (ld64 `-order_file`) adds the leading "_" of a C symbol; ELF (lld
// `--symbol-ordering-file`) and COFF take the name as it is. The names depend on the target (the
// crate hashes do), so each target has its own file. The order file goes stale slowly: a renamed
// or new function has no line until the next run of this script; the linker ignores a line with
// no symbol.
//
// Exit status: 0 the file is written; 1 the run or llvm-profdata failed; 2 usage error.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** The default start-up command: ep9 evaluates `0` and exits. */
export const STARTUP_ARGS = ["-e", "0"];

/** Targets whose linker wants the Mach-O "_" prefix in an order file. */
const MACHO = /-apple-/;

class UsageError extends Error {}

export function parseArgs(argv) {
  const o = { covBin: null, target: null, out: null, llvmProfdata: null, args: STARTUP_ARGS };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new UsageError(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case "--cov-bin": o.covBin = path.resolve(val()); break;
      case "--target": o.target = val(); break;
      case "--out": o.out = val(); break;
      case "--llvm-profdata": o.llvmProfdata = val(); break;
      case "--": o.args = argv.slice(i + 1); i = argv.length; break;
      default: throw new UsageError(`unknown option ${a}`);
    }
  }
  if (!o.covBin || !o.target) throw new UsageError("--cov-bin and --target are required");
  return o;
}

/**
 * The functions of an `llvm-profdata show --all-functions` text: [{ name, count }], the
 * "<file>;" prefix of a local function's name taken off.
 */
export function parseShow(text) {
  const out = [];
  const re = /^ {2}(\S[^\n]*):\n {4}Hash: \S+\n {4}Counters: \d+\n {4}Function count: (\d+)$/gm;
  for (const m of text.matchAll(re)) {
    const name = m[1].includes(";") ? m[1].slice(m[1].indexOf(";") + 1) : m[1];
    out.push({ name, count: Number(m[2]) });
  }
  return out;
}

/**
 * The order file text: the functions with a count above 0, most calls first, then by name; one
 * symbol per line, with the Mach-O "_" prefix on an Apple target. A name seen twice (two local
 * functions of one name) gets one line, with the larger count.
 */
export function orderText(fns, target) {
  const best = new Map();
  for (const f of fns) if (f.count > 0 && !(best.get(f.name) >= f.count)) best.set(f.name, f.count);
  const prefix = MACHO.test(target) ? "_" : "";
  const names = [...best.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)).map(([n]) => `${prefix}${n}`);
  return names.length ? `${names.join("\n")}\n` : "";
}

/** llvm-profdata of the rustc sysroot (the llvm-tools component), else the one on the PATH. */
function profdataTool() {
  const r = spawnSync("rustc", ["--print", "sysroot"], { encoding: "utf8" });
  const host = spawnSync("rustc", ["-vV"], { encoding: "utf8" }).stdout?.match(/^host: (.*)$/m)?.[1];
  if (r.status === 0 && host) {
    const p = path.join(r.stdout.trim(), "lib", "rustlib", host, "bin", "llvm-profdata");
    if (spawnSync(p, ["--version"]).status === 0) return p;
  }
  return "llvm-profdata";
}

/** Runs the coverage build once and returns the order file text. */
export function generate(o) {
  const tmp = mkdtempSync(path.join(os.tmpdir(), "ep9-order-"));
  try {
    const env = { ...process.env, LLVM_PROFILE_FILE: path.join(tmp, "run-%p.profraw"), EP9_HASH_SEED: "1" };
    const run = spawnSync(o.covBin, o.args, { env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    if (run.status !== 0) throw new Error(`${o.covBin} ${o.args.join(" ")}: exit ${run.status}${run.error ? ` (${run.error.message})` : ""}: ${run.stderr.trim().split("\n").slice(-3).join(" / ")}`);
    const raw = readdirSync(tmp).filter((f) => f.endsWith(".profraw")).map((f) => path.join(tmp, f));
    if (!raw.length) throw new Error(`${o.covBin} wrote no profile: is it a -C instrument-coverage build?`);
    const tool = o.llvmProfdata ?? profdataTool();
    const merged = path.join(tmp, "run.profdata");
    const m = spawnSync(tool, ["merge", "-o", merged, ...raw], { encoding: "utf8" });
    if (m.status !== 0) throw new Error(`${tool} merge: ${m.error?.message ?? m.stderr.trim()}`);
    const s = spawnSync(tool, ["show", "--all-functions", merged], { encoding: "utf8", maxBuffer: 1 << 28 });
    if (s.status !== 0) throw new Error(`${tool} show: ${s.error?.message ?? s.stderr.trim()}`);
    const fns = parseShow(s.stdout);
    if (!fns.length) throw new Error(`${tool} show listed no function`);
    return { text: orderText(fns, o.target), functions: fns.length, ran: fns.filter((f) => f.count > 0).length };
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

function main() {
  let o;
  try {
    o = parseArgs(process.argv.slice(2));
  } catch (e) {
    if (!(e instanceof UsageError)) throw e;
    console.error(`order.mjs: ${e.message}`);
    process.exit(2);
  }
  let g;
  try {
    g = generate(o);
  } catch (e) {
    console.error(`order.mjs: ${e.message}`);
    process.exit(1);
  }
  if (o.out) writeFileSync(o.out, g.text);
  else process.stdout.write(g.text);
  console.error(`order.mjs: ${g.ran} of ${g.functions} functions ran (${o.args.join(" ")}); ${g.text.split("\n").filter(Boolean).length} lines${o.out ? ` in ${o.out}` : ""}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
