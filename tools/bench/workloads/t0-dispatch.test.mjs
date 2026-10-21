// node --test tools/bench/workloads/t0-dispatch.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import * as stats from "../lib/stats.mjs";
import { geomean, median, medianRanks, parseArgs, quantile, verdict } from "./t0-dispatch.mjs";

test("the statistics are the shared ones of tools/bench/lib/stats.mjs", () => {
  for (const f of ["median", "quantile", "medianRanks", "geomean"]) {
    assert.equal({ median, quantile, medianRanks, geomean }[f], stats[f], f);
  }
  // verdict keeps this runner's labels on top of the shared verdict.
  const xs = Array.from({ length: 15 }, (_, i) => 1.1 + i / 1000);
  assert.equal(verdict(xs).median, stats.verdict(xs, 1).median);
  assert.equal(stats.verdict(xs, 1).side, "above");
  assert.equal(verdict(xs).side, "B faster");
});

test("median ranks match the PERF-NOISE table", () => {
  assert.deepEqual(medianRanks(31), [10, 22]);
  assert.deepEqual(medianRanks(15), [4, 12]);
  assert.deepEqual(medianRanks(10), [2, 9]);
  assert.deepEqual(medianRanks(7), [1, 7]);
});

test("no 95% interval below 6 samples", () => {
  // [min, max] covers 1 - 2^(1-n): 93.75% at n = 5, 96.9% at n = 6.
  for (let n = 0; n <= 5; n++) assert.equal(medianRanks(n), null, `n = ${n}`);
  assert.deepEqual(medianRanks(6), [1, 6]);
  const v = verdict([1.5]);
  assert.equal(v.side, "insufficient data");
  assert.equal(v.interval, null);
  assert.equal(verdict([1.2, 1.3, 1.4, 1.5, 1.6]).side, "insufficient data");
  assert.equal(verdict([1.2, 1.3, 1.4, 1.5, 1.6, 1.7]).side, "B faster");
});

test("rounds below the class S minimum are rejected", () => {
  assert.throws(() => parseArgs(["--ep9", "x", "--rounds", "1"]), /--rounds must be >= 15/);
  assert.throws(() => parseArgs(["--ep9", "x", "--rounds", "14"]), /--rounds must be >= 15/);
  assert.equal(parseArgs(["--ep9", "x", "--rounds", "15"]).rounds, 15);
  // --cachegrind counts once and ignores --rounds.
  assert.equal(parseArgs(["--ep9", "x", "--cachegrind", "--rounds", "1"]).cachegrind, true);
});

test("verdict sides and void", () => {
  const up = Array.from({ length: 31 }, (_, i) => 1.1 + i * 0.001);
  assert.equal(verdict(up).side, "B faster");
  assert.equal(verdict(up).void, false);
  assert.deepEqual(verdict(up).interval, [up[9], up[21]]);
  const down = up.map((x) => 1 / x);
  assert.equal(verdict(down).side, "A faster");
  const mixed = Array.from({ length: 31 }, (_, i) => 0.9 + i * (0.2 / 30));
  const v = verdict(mixed);
  assert.equal(v.side, "tie");
  assert.equal(v.void, true, `IQR ${v.iqrPct}%`);
});

test("small helpers", () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  assert.equal(quantile([1, 2, 3, 4, 5], 0.25), 2);
  assert.ok(Math.abs(geomean([2, 8]) - 4) < 1e-12);
  assert.deepEqual(parseArgs(["--ep9", "x", "--rounds", "15"]).rounds, 15);
  assert.throws(() => parseArgs(["--configs", "a", "--ep9", "x"]));
});

test("two binaries get labels per binary", () => {
  const o = parseArgs(["--ep9", "a", "--ep9-b", "b", "--configs", "sibling,sibling"]);
  assert.deepEqual(
    o.runs.map((r) => [r.label, r.bin, r.dispatch]),
    [
      ["sibling@a", "a", "sibling"],
      ["sibling@b", "b", "sibling"],
    ],
  );
  assert.throws(() => parseArgs(["--ep9", "a", "--configs", "match,match"]));
});
