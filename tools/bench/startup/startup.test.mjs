// node --test tools/bench/startup/startup.test.mjs
// STARTUP-LAYOUT (plan v13 section 6.2): the order-file generator (order.mjs), the order file
// of this directory, and the start-up budget row of .github/scripts/perf-ci-v0.mjs. No ep9 build
// needed: the generator's parts read a fixed llvm-profdata text.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { DEFAULT_WORKLOADS, parseArgs as perfArgs, renderMarkdown, STARTUP, STARTUP_BUDGETS, startupBudget } from "../../../.github/scripts/perf-ci-v0.mjs";
import { orderText, parseArgs, parseShow, STARTUP_ARGS } from "./order.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));

// The form of `llvm-profdata show --all-functions` (LLVM 21): a header, then 4 lines per function.
const SHOW = `Counters:
  _RNvCs1_3ep94main:
    Hash: 0x0000000000000001
    Counters: 3
    Function count: 1
  src/lib.rs;_RNvCs2_6ep9_vm4cold:
    Hash: 0x0000000000000002
    Counters: 1
    Function count: 0
  crates/a.rs;_RNvCs2_6ep9_vm3hot:
    Hash: 0x0000000000000003
    Counters: 2
    Function count: 40
  crates/b.rs;_RNvCs2_6ep9_vm3hot:
    Hash: 0x0000000000000004
    Counters: 2
    Function count: 7
  _RNvCs2_6ep9_vm4also:
    Hash: 0x0000000000000005
    Counters: 1
    Function count: 1
Instrumentation level: Front-end
Functions shown: 5
Total functions: 5
`;

test("order: the profile functions, the file prefix taken off", () => {
  assert.deepEqual(parseShow(SHOW), [
    { name: "_RNvCs1_3ep94main", count: 1 },
    { name: "_RNvCs2_6ep9_vm4cold", count: 0 },
    { name: "_RNvCs2_6ep9_vm3hot", count: 40 },
    { name: "_RNvCs2_6ep9_vm3hot", count: 7 },
    { name: "_RNvCs2_6ep9_vm4also", count: 1 },
  ]);
});

test("order: functions that ran, most calls first, then by name; the Mach-O prefix on Apple targets", () => {
  const fns = parseShow(SHOW);
  assert.equal(orderText(fns, "x86_64-unknown-linux-gnu"), "_RNvCs2_6ep9_vm3hot\n_RNvCs1_3ep94main\n_RNvCs2_6ep9_vm4also\n");
  assert.equal(orderText(fns, "aarch64-apple-darwin"), "__RNvCs2_6ep9_vm3hot\n__RNvCs1_3ep94main\n__RNvCs2_6ep9_vm4also\n");
  assert.equal(orderText([{ name: "x", count: 0 }], "aarch64-apple-darwin"), "");
});

test("order: options", () => {
  const o = parseArgs(["--cov-bin", "b", "--target", "aarch64-apple-darwin"]);
  assert.deepEqual(o.args, STARTUP_ARGS);
  assert.deepEqual(STARTUP_ARGS, ["-e", "0"]);
  assert.deepEqual(parseArgs(["--cov-bin", "b", "--target", "t", "--", "x.js"]).args, ["x.js"]);
  assert.throws(() => parseArgs(["--cov-bin", "b"]), /--cov-bin and --target are required/);
  assert.throws(() => parseArgs(["--cov-bin", "b", "--target", "t", "--bogus"]), /unknown option --bogus/);
});

test("order: the aarch64-apple-darwin order file is the generator's form", { skip: !existsSync(path.join(DIR, "aarch64-apple-darwin.order")) && "no order file" }, () => {
  const lines = readFileSync(path.join(DIR, "aarch64-apple-darwin.order"), "utf8").split("\n");
  assert.equal(lines.pop(), "", "one symbol per line, a newline at the end");
  assert.ok(lines.length > 100, `${lines.length} lines`);
  assert.equal(new Set(lines).size, lines.length, "no name twice");
  // Symbol names with the Mach-O "_" (v0 names, and the export names of the handlers, such as
  // "_ep9h__sib__exit__Exit"); no path (the file is in a public repository).
  for (const l of lines) assert.match(l, /^_[0-9A-Za-z_$.]+$/, l);
  assert.ok(lines.filter((l) => l.startsWith("__R")).length > lines.length / 2);
});

test("perf-ci-v0: the start-up row is a default workload with a budget per counter and target", () => {
  assert.equal(STARTUP, "startup");
  assert.equal(DEFAULT_WORKLOADS.at(-1), STARTUP);
  const o = perfArgs(["--ep9", "e", "--bench", "b", "--out", "o", "--workloads", "startup,t0-call"]);
  assert.deepEqual(o.workloads, ["startup", "t0-call"]);
  assert.throws(() => perfArgs(["--ep9", "e", "--bench", "b", "--out", "o", "--workloads", "start"]), /unknown workload start \(known: .*startup\)/);
  // CI's counter and target have a budget.
  const ci = STARTUP_BUDGETS.find((b) => b.counter === "cachegrind-i-refs" && b.target === "x86_64-unknown-linux-gnu");
  assert.ok(ci && ci.instructions > 0);
  assert.deepEqual(startupBudget("cachegrind-i-refs", "x86_64-unknown-linux-gnu", ci.instructions), { instructions: ci.instructions, source: ci.source, over: false });
  assert.equal(startupBudget("cachegrind-i-refs", "x86_64-unknown-linux-gnu", ci.instructions + 1).over, true);
  assert.equal(startupBudget("cachegrind-i-refs", "x86_64-unknown-linux-gnu", null).over, false);
  assert.equal(startupBudget("perf-stat-instructions-u", "x86_64-unknown-linux-gnu", 1), null);
});

test("perf-ci-v0: the table says within, OVER BUDGET or no budget on the start-up row", () => {
  const res = (budget, status = "ok") => ({
    counter: "cachegrind-i-refs",
    commit: "c",
    target: "x86_64-unknown-linux-gnu",
    host: { cpu: "cpu" },
    hash_seed: "1",
    runs: 3,
    warmup: 0,
    complete: true,
    workloads: [
      { name: "t0-call", status: "ok", instructions: { min: 9, median: 10, max: 11 } },
      { name: "startup", status, instructions: { min: 4, median: 4, max: 4 }, budget },
    ],
  });
  const row = (md) => md.split("\n").find((l) => l.startsWith("| startup |"));
  assert.equal(row(renderMarkdown(res({ instructions: 5, over: false }))), "| startup | 4 | 4 | 4 | 0.0000% | ok; budget 5: within |");
  assert.equal(row(renderMarkdown(res({ instructions: 3, over: true }))), "| startup | 4 | 4 | 4 | 0.0000% | ok; budget 3: OVER BUDGET |");
  assert.equal(row(renderMarkdown(res(null))), "| startup | 4 | 4 | 4 | 0.0000% | ok; no budget for this counter and target |");
  // The other rows have no budget text.
  assert.match(renderMarkdown(res(null)), /\| t0-call \| 10 \| 9 \| 11 \| 20\.0000% \| ok \|/);
});
