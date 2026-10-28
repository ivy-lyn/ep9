#!/usr/bin/env node
// The M1 exit-2 reader (docs/plan/09-milestones.md M1 exit 2; plan v9 section 9.4 item 2): from
// one ep9-cross/v1 run (run.mjs --workloads exit2 --harness base), the two speed readings
//
//   Octane-7 geomean vs QuickJS                     low end of the interval >= 1.0
//   v8-v7 7-kernel geomean (no regexp) vs node --jitless   low end of the interval >= 0.75
//
//   node tools/bench/cross/exit2.mjs run.json [--out exit2.json]
//
// Geomean and interval. run.mjs runs every workload once per round, so round r of every kernel
// is one paired sample. Per round, the geomean of the kernels' net ratios t(engine) / t(ep9) is
// one value; the reading is the median of these per-round values, with the order-statistic
// interval (n = 10: ranks 2 and 9) and the IQR void rule of lib/stats.mjs. The geomean of the
// kernels' median ratios (run.mjs's own geomean) is shown too.
//
// Gate verdict: "pass" when the interval's low end is >= the threshold, "fail" when its high end
// is below it, else "marginal". There is no gate verdict ("provisional") when: the run is
// provisional (no gate host, load guard; run.mjs); any of the kernels' rows is not "valid" (void,
// net invalid); the per-round geomeans are void; a v8-v7 kernel did not run with the real
// base.js (V9-D14: "exits 1-2 use the real base.js harness"); or the file has fewer than the
// class M minimum of 10 paired rounds (M.rounds, options.rounds and the length of every kernel's
// ratio series must all be the same number >= 10). A missing kernel or engine makes
// the reading "incomplete". Freshness (the commit against the tree) is m1-exit's check: this
// tool prints the commit and copies run.mjs's list of changed fresh paths (`dirty`, a boolean,
// and `dirty_paths`).
//
// Exit status: 0 both readings pass; 1 a reading does not pass; 2 usage error or not an
// ep9-cross/v1 file.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { geomean, verdict } from "../lib/stats.mjs";
import { MIN_ROUNDS } from "./run.mjs";
import { OCTANE_7, V8V7_7 } from "./workloads.mjs";

export const SCHEMA = "ep9-exit2/v1";

/** The two readings of M1 exit 2. */
export const READINGS = [
  { id: "octane7-vs-qjs", label: "Octane-7 geomean vs QuickJS", workloads: OCTANE_7, comparator: "qjs", threshold: 1.0, harness: "octane-base" },
  { id: "v8v7-7-vs-node-jitless", label: "v8-v7 7-kernel geomean vs node --jitless", workloads: V8V7_7, comparator: "node-jitless", threshold: 0.75, harness: "base" },
];

export class UsageError extends Error {}

/** Geomean per round of the rows' ratio lists (all of one length), or null. */
function perRound(lists) {
  if (!lists.length || lists.some((xs) => !xs || xs.length !== lists[0].length)) return null;
  return lists[0].map((_, r) => geomean(lists.map((xs) => xs[r])));
}

const gateSide = (interval, threshold) => (!interval ? null : interval[0] >= threshold ? "pass" : interval[1] < threshold ? "fail" : "marginal");

/** One reading from an ep9-cross/v1 result. */
export function reading(res, spec) {
  const out = { id: spec.id, label: spec.label, comparator: spec.comparator, threshold: spec.threshold, workloads: spec.workloads };
  const rows = spec.workloads.map((w) => res.M?.ratios?.find((r) => r.workload === w && r.comparator === spec.comparator));
  const missing = spec.workloads.filter((_, i) => !rows[i]);
  if (missing.length) {
    return { ...out, status: "incomplete", gate: "no reading", reasons: [`no ${spec.comparator} row for ${missing.join(", ")}`], net: null, gross: null, kernels: [] };
  }
  const reasons = [];
  if (res.provisional) reasons.push(`provisional run (${(res.provisional_reasons ?? []).join("; ") || "no reason given"})`);
  if (!res.host?.gate_host) reasons.push("not a gate host");
  const harness = Object.fromEntries((res.workloads ?? []).map((w) => [w.name, w.harness ?? null]));
  const wrongHarness = spec.workloads.filter((w) => harness[w] !== spec.harness);
  if (wrongHarness.length) reasons.push(`harness is not ${spec.harness} for ${wrongHarness.map((w) => `${w} (${harness[w] ?? "unknown"})`).join(", ")}`);
  // Class M: >= MIN_ROUNDS paired rounds, and every series as long as the run says (a cut or
  // hand-edited file must not get a verdict from fewer rounds: verdict() gives an interval at n = 6).
  const declared = res.M?.rounds;
  if (!(Number.isInteger(declared) && declared >= MIN_ROUNDS)) reasons.push(`the run has ${declared ?? "no"} paired rounds (M.rounds); class M needs >= ${MIN_ROUNDS}`);
  if (res.options?.rounds !== declared) reasons.push(`M.rounds (${declared}) differs from options.rounds (${res.options?.rounds})`);
  const cut = rows.filter((r) => !r.gross?.ratios || [r.net?.ratios, r.gross.ratios].some((xs) => xs && xs.length !== declared));
  if (cut.length) reasons.push(`ratio series not ${declared} long: ${cut.map((r) => `${r.workload} (${[r.net?.ratios?.length, r.gross?.ratios?.length].map((x) => x ?? "-").join("/")})`).join(", ")}`);
  const notValid = rows.filter((r) => !(r.status?.length === 1 && r.status[0] === "valid"));
  if (notValid.length && !res.provisional) reasons.push(`row not valid: ${notValid.map((r) => `${r.workload} (${(r.status ?? ["no status"]).join(", ")})`).join("; ")}`);
  const kernels = rows.map((r) => ({ workload: r.workload, metric: r.metric, median: (r.net ?? r.gross).median, interval: (r.net ?? r.gross).interval, status: r.status }));
  const netSeries = rows.every((r) => r.net) ? perRound(rows.map((r) => r.net.ratios)) : null;
  const grossSeries = perRound(rows.map((r) => r.gross?.ratios));
  if (!netSeries) reasons.push("net ratios invalid or unpaired in a row: the gross geomean is shown, not gated");
  const v = (series) => series && { ...verdict(series, spec.threshold), series };
  const net = v(netSeries);
  const gross = v(grossSeries);
  if (net?.void) reasons.push(`per-round geomeans void (IQR ${net.iqrPct.toFixed(1)}% > 5%)`);
  if (net && !net.interval) reasons.push(`no interval (n = ${net.n})`);
  const side = gateSide((net ?? gross)?.interval, spec.threshold);
  const medianGeomean = {
    net: rows.every((r) => r.net) ? geomean(rows.map((r) => r.net.median)) : null,
    gross: geomean(rows.map((r) => r.gross.median)),
  };
  return {
    ...out,
    status: reasons.length ? "provisional" : "valid",
    gate: reasons.length ? `no gate verdict (the ${net ? "" : "gross "}interval reads ${side ?? "nothing"})` : side,
    reasons,
    net,
    gross,
    geomean_of_medians: medianGeomean,
    kernels,
  };
}

export function readExit2(res) {
  if (res?.schema !== "ep9-cross/v1") throw new UsageError(`not an ep9-cross/v1 run (schema ${JSON.stringify(res?.schema)})`);
  const readings = READINGS.map((s) => reading(res, s));
  return {
    schema: SCHEMA,
    source_schema: res.schema,
    commit: res.commit?.ep9 ?? null,
    // Uncommitted changes in the fresh paths when the run started (run.mjs, CX5-C5): null when
    // the run does not say; m1-exit item 2 gives OPEN unless it is false.
    dirty: Array.isArray(res.commit?.dirty) ? res.commit.dirty.length > 0 : null,
    dirty_paths: Array.isArray(res.commit?.dirty) ? res.commit.dirty : null,
    runner_commit: res.commit?.runner ?? null,
    started: res.started ?? null,
    host: { id: res.host?.id ?? null, gate_host: res.host?.gate_host ?? false, cpu: res.host?.measured?.cpu ?? null, platform: res.host?.measured?.platform ?? null },
    provisional: res.provisional ?? true,
    readings,
    pass: readings.every((r) => r.gate === "pass"),
  };
}

const f3 = (x) => (x == null ? "-" : x.toFixed(3));
const iv = (v) => (v?.interval ? `${f3(v.interval[0])}-${f3(v.interval[1])}` : "none");

export function renderMarkdown(e) {
  const L = [];
  const dirty = e.dirty === null ? " (the run does not say whether the tree had uncommitted changes)" : e.dirty ? ` + uncommitted changes in ${e.dirty_paths.join(", ")}` : "";
  L.push(`M1 exit 2 from the run of ep9 commit \`${(e.commit ?? "unknown").slice(0, 12)}\`${dirty} (${e.started ?? "?"}), host ${e.host.id ?? "no host file"}${e.host.gate_host ? " (gate host)" : ""}.`);
  L.push("");
  L.push("| reading | threshold | per-round geomean (net) | interval | IQR % | geomean of medians (net / gross) | gate |");
  L.push("|---|---|---|---|---|---|---|");
  for (const r of e.readings) {
    const v = r.net ?? r.gross;
    L.push(`| ${r.label} | ${r.threshold} | ${r.net ? f3(r.net.median) : r.gross ? `${f3(r.gross.median)} (gross)` : "-"} | ${iv(v)} | ${v ? v.iqrPct.toFixed(1) : "-"} | ${f3(r.geomean_of_medians?.net)} / ${f3(r.geomean_of_medians?.gross)} | ${r.gate} |`);
  }
  for (const r of e.readings) {
    if (!r.reasons.length) continue;
    L.push("");
    L.push(`${r.label}: ${r.reasons.join("; ")}.`);
  }
  L.push("");
  L.push(`Exit 2: ${e.pass ? "PASS (both readings pass; m1-exit checks that the commit is fresh)" : "not passed"}.`);
  return `${L.join("\n")}\n`;
}

export function parseArgs(argv) {
  const o = { input: null, out: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--out") {
      if (i + 1 >= argv.length) throw new UsageError("--out needs a value");
      o.out = path.resolve(argv[++i]);
    } else if (a.startsWith("-")) {
      throw new UsageError(`unknown option ${a}`);
    } else if (o.input) {
      throw new UsageError("one run.json only");
    } else {
      o.input = path.resolve(a);
    }
  }
  if (!o.input) throw new UsageError("the ep9-cross/v1 run.json is required");
  return o;
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) {
    // The usage is the comment at the top of this file.
    const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n");
    process.stdout.write(`${src.slice(1, src.findIndex((l, i) => i > 0 && !l.startsWith("//"))).map((l) => l.replace(/^\/\/ ?/, "")).join("\n")}\n`);
    return;
  }
  let e;
  try {
    const o = parseArgs(argv);
    let res;
    try {
      res = JSON.parse(readFileSync(o.input, "utf8"));
    } catch (err) {
      throw new UsageError(`${o.input}: ${err.message}`);
    }
    e = readExit2(res);
    if (o.out) writeFileSync(o.out, `${JSON.stringify(e, null, 1)}\n`);
  } catch (err) {
    if (err instanceof UsageError) {
      console.error(`exit2.mjs: ${err.message}`);
      process.exit(2);
    }
    throw err;
  }
  process.stdout.write(renderMarkdown(e));
  process.exit(e.pass ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
