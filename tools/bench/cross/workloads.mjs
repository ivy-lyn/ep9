// Workloads of the cross-engine runner (decision PERF-2) and the files it runs.
//
// Every engine runs the same text: PRINT_PRELUDE (print falls back to console.log) plus the
// workload. The empty-script baseline is the prelude alone, so net time = time(workload) -
// time(empty) removes engine start-up and the prelude.
//
// Classes (docs/plan/10-quality.md section 3): M = wall time in paired rounds; R = instructions
// retired (ep9-bench, 5 runs + 1 warm-up, median). ci-v0 is R only: about 7 ms on ep9, so its
// wall time is start-up.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

import { generate, PRINT_PRELUDE, ROOT } from "./v8v7.mjs";

const T0_DIR = "tools/bench/workloads";

/**
 * The table. `expect` is what the workload prints; each T0 file checks its own result against
 * the same value (`const want = ...`; run.test.mjs checks that the two agree). A v8v7 kernel's
 * expect comes from the generator.
 */
export const WORKLOADS = {
  "t0-loop": { file: `${T0_DIR}/t0-loop.js`, classes: ["M", "R"], expect: "604229" },
  "t0-branchy": { file: `${T0_DIR}/t0-branchy.js`, classes: ["M", "R"], expect: "1090221937" },
  "t0-call": { file: `${T0_DIR}/t0-call.js`, classes: ["M", "R"], expect: "577789" },
  "t0-prop": { file: `${T0_DIR}/t0-prop.js`, classes: ["M", "R"], expect: "354392" },
  "t0-gen": { file: `${T0_DIR}/t0-gen.js`, classes: ["M", "R"], expect: "351381" },
  "ci-v0": { file: `${T0_DIR}/ci-v0.js`, classes: ["R"], expect: "6765 37492500 2000 545003" },
  richards: { v8v7: "richards", classes: ["M", "R"] },
  deltablue: { v8v7: "deltablue", classes: ["M", "R"] },
};

/** Run 1 (week-4 plan section 11). deltablue joins in run 2, once Function.prototype.call lands. */
export const DEFAULT_WORKLOADS = ["t0-loop", "t0-branchy", "t0-call", "t0-prop", "ci-v0", "richards"];

/** The baseline "workload": the prelude only. */
export const EMPTY = "empty";

const sha256 = (text) => createHash("sha256").update(text).digest("hex");

/**
 * Writes the files of `names` (plus the empty script) into `dir`. Returns, per name, the file,
 * the expected output, the classes and where the text came from.
 */
export function materialize(names, dir, { root = ROOT } = {}) {
  const out = {};
  const put = (name, text, info) => {
    const file = path.join(dir, `${name}.js`);
    writeFileSync(file, text);
    out[name] = { name, file, sha256: sha256(text), ...info };
  };
  put(EMPTY, PRINT_PRELUDE, { classes: ["M", "R"], expect: "", source: "prelude only" });
  for (const name of names) {
    const w = WORKLOADS[name];
    if (!w) throw new Error(`unknown workload ${name} (known: ${Object.keys(WORKLOADS).join(", ")})`);
    if (w.v8v7) {
      const g = generate(w.v8v7, { root });
      put(name, g.text, {
        classes: w.classes,
        expect: g.expect,
        source: `third_party/bench-v8v7 ${g.source}`,
        pin: g.pin,
        blob: g.blob,
        n: g.n,
        generator: "tools/bench/cross/v8v7.mjs",
      });
    } else {
      put(name, PRINT_PRELUDE + readFileSync(path.join(root, w.file), "utf8"), {
        classes: w.classes,
        expect: w.expect,
        source: w.file,
      });
    }
  }
  return out;
}
