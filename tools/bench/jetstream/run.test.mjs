// Tests for run.mjs with a fake engine (testdata/fake-d8.mjs). A failed run must make the
// comparison invalid and the exit code nonzero; it must not drop out of the median or the list.
// usage: node --test tools/bench/jetstream/run.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const fake = path.join(here, "testdata/fake-d8.mjs");
// run.mjs starts a d8-style engine in the JetStream checkout. The fake engine needs no checkout,
// so each run points EP9_JETSTREAM_DIR at its own empty temp dir.
const skip = false;

// Runs run.mjs on the workloads "easy" and "hard" with 3 reps. plans: config name -> --plan value.
function run(plans) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ep9-jetstream-run-"));
  try {
    const list = path.join(dir, "list.txt");
    const out = path.join(dir, "out.json");
    fs.writeFileSync(list, "easy\nhard\n");
    const configs = Object.entries(plans).flatMap(([name, plan]) => [
      "--config",
      `${name}=${process.execPath}:${fake},--tag=${name},--plan=${plan}`,
    ]);
    const p = spawnSync(
      process.execPath,
      [path.join(here, "run.mjs"), "--list", list, "--reps", "3", "--out", out, ...configs],
      { encoding: "utf8", env: { ...process.env, FAKE_STATE: dir, EP9_JETSTREAM_DIR: dir } },
    );
    return { status: p.status, stderr: p.stderr, out: JSON.parse(fs.readFileSync(out, "utf8")) };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test("all samples present: exit 0 and a ratio", { skip }, () => {
  const { status, out } = run({ base: "easy:100;hard:100", cand: "easy:200;hard:300/200/400" });
  assert.equal(status, 0);
  // easy 200/100, hard median(300, 200, 400)/100: geomean(2, 3).
  assert.ok(Math.abs(out.summary.cand.ratio_vs_first - Math.sqrt(6)) < 1e-9);
  assert.equal(out.complete, true);
  assert.deepEqual(out.missing, []);
  assert.equal(out.summary.cand.complete, true);
});

// Review finding: the candidate crashed on "hard" in all reps, and the runner reported
// ratio 2 from "easy" only and exited 0.
test("candidate fails every rep of a workload: invalid, exit 1", { skip }, () => {
  const { status, stderr, out } = run({ base: "easy:100;hard:100", cand: "easy:200;hard:x" });
  assert.equal(status, 1);
  assert.equal(out.complete, false);
  assert.deepEqual(out.missing, [{ name: "hard", config: "cand", n: 0, want: 3 }]);
  assert.equal(out.summary.cand.complete, false);
  assert.equal(out.summary.cand.ratio_vs_first, null);
  assert.equal(out.summary.cand.geomean_score, null);
  assert.equal(out.summary.base.complete, true);
  assert.match(stderr, /hard cand: 0 of 3/);
});

// A median from 1 of 3 samples looked complete ("workloads": 2) and exited 0.
test("candidate fails 2 of 3 reps: invalid, exit 1, failure details kept", { skip }, () => {
  const { status, out } = run({ base: "easy:100;hard:100", cand: "easy:200;hard:300/x/k" });
  assert.equal(status, 1);
  assert.equal(out.complete, false);
  assert.deepEqual(out.missing, [{ name: "hard", config: "cand", n: 1, want: 3 }]);
  assert.equal(out.summary.cand.ratio_vs_first, null);
  const failed = out.runs.filter((r) => r.error);
  assert.equal(failed.length, 2);
  // The cause is on stderr line 4; the old runner kept only lines 1-3.
  assert.equal(failed[0].status, 1);
  assert.match(failed[0].stderr, /FATAL: hard call 1 crashed/);
  assert.equal(failed[1].signal, "SIGKILL");
});

// The base fails a workload where the candidate is 10x slower: the old runner dropped it (ratio 1).
test("base fails a workload: invalid, exit 1", { skip }, () => {
  const { status, out } = run({ base: "easy:100;hard:x", cand: "easy:100;hard:10" });
  assert.equal(status, 1);
  assert.deepEqual(out.missing, [{ name: "hard", config: "base", n: 0, want: 3 }]);
  assert.equal(out.summary.base.complete, false);
  assert.equal(out.summary.cand.complete, false);
  assert.equal(out.summary.cand.ratio_vs_first, null);
});
