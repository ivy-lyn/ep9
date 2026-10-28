// Runs a generated kernel file on this node with a fault, for the kernels.mjs tests (negative
// controls of the result probes). Called by fake-kernels-ep9.sh when FAKE_FAULT names the file:
//   FAKE_FAULT=<name>:<fault>[,<name>:<fault>...]   <name> = the file name without .js
// Faults (each throws when it finds nothing to change, so a test cannot pass by accident):
//   exec-null     %RegExp.prototype%.exec returns null (a matcher that never matches)
//   replace-same  String.prototype.replace returns the string unchanged
//   no-update     runNavierStokes skips solver.update() (no solver work)
//   <grid>-<bad>  the field's get<Grid>(x, y) returns <bad> in place of every value v with
//                 0 <= v * 1e9 < 1 (the cells a ToInt32 of v * 1e9 reads as 0; about 15,900 of the
//                 16,384 density cells and about 300 of each velocity grid after one frame).
//                 <grid>: dens, xvel, yvel = getDensity, getXVelocity, getYVelocity. <bad>: nan,
//                 inf, ninf (-Infinity), undef (undefined), wrap (v + 4.294967296, which is
//                 2^32 / 1e9: a ToInt32 of its value * 1e9 is still 0)
//   exec-numstr   %RegExp.prototype%.exec returns its digit-only strings as numbers
import { readFileSync } from "node:fs";
import path from "node:path";
import vm from "node:vm";

const file = process.argv[2];
const name = path.basename(file, ".js");
const fault = (process.env.FAKE_FAULT ?? "").split(",").map((x) => x.split(":")).find(([n]) => n === name)?.[1];
let text = readFileSync(file, "utf8");
const prelude = {
  "exec-null": "Object.getPrototypeOf(/a/).exec = function () { return null; };",
  "replace-same": "String.prototype.replace = function () { return String(this); };",
  "exec-numstr": [
    "(function () {",
    "  var R = Object.getPrototypeOf(/a/), exec = R.exec, hit = false;",
    "  R.exec = function (s) {",
    "    var r = exec.call(this, s);",
    "    // No RegExp here: test() would call this exec again.",
    "    if (r) for (var i = 0; i < r.length; i++) if (typeof r[i] === \"string\" && r[i] !== \"\" && !r[i].split(\"\").some(function (c) { return c < \"0\" || c > \"9\"; })) { r[i] = Number(r[i]); hit = true; }",
    "    return r;",
    "  };",
    "  process.on(\"exit\", function () { if (!hit) throw new Error(\"exec-numstr: no digit-only match\"); });",
    "})();",
  ].join("\n"),
};
const GRIDS = { dens: "dens", xvel: "u", yvel: "v" };
const BAD = { nan: "NaN", inf: "Infinity", ninf: "-Infinity", undef: "undefined", wrap: "__ff + 4.294967296" };
if (prelude[fault]) {
  text = `${prelude[fault]}\n${text}`;
} else if (fault === "no-update") {
  const re = /^(function runNavierStokes\(\)\n\{\n)    solver\.update\(\);$/m;
  if (!re.test(text)) throw new Error(`${file}: no solver.update() in runNavierStokes`);
  text = text.replace(re, "$1");
} else if (/^(dens|xvel|yvel)-(nan|inf|ninf|undef|wrap)$/.test(fault ?? "")) {
  const [grid, bad] = fault.split("-");
  const re = new RegExp(`return ${GRIDS[grid]}\\[\\(x \\+ 1\\) \\+ \\(y \\+ 1\\) \\* rowSize\\];`, "g");
  const n = text.match(re)?.length ?? 0;
  if (n !== 1) throw new Error(`${file}: ${n} get lines of the ${GRIDS[grid]} grid, want 1`);
  text = text.replace(re, (m) => `var __ff = ${m.slice(7, -1)}; if (!(__ff * 1e9 >= 0 && __ff * 1e9 < 1)) return __ff; globalThis.__ffHits = (globalThis.__ffHits || 0) + 1; return ${BAD[bad]};`);
  process.on("exit", () => {
    if (!globalThis.__ffHits) throw new Error(`${file}: ${fault} changed no cell`);
  });
} else {
  throw new Error(`unknown fault ${JSON.stringify(fault)} for ${name}`);
}
vm.runInThisContext(text, { filename: file });
