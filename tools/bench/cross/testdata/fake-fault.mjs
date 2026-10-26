// Runs a generated kernel file on this node with a fault, for the kernels.mjs tests (negative
// controls of the result probes). Called by fake-kernels-ep9.sh when FAKE_FAULT names the file:
//   FAKE_FAULT=<name>:<fault>[,<name>:<fault>...]   <name> = the file name without .js
// Faults (each throws when it finds nothing to change, so a test cannot pass by accident):
//   exec-null     %RegExp.prototype%.exec returns null (a matcher that never matches)
//   replace-same  String.prototype.replace returns the string unchanged
//   no-update     runNavierStokes skips solver.update() (no solver work)
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
};
if (prelude[fault]) {
  text = `${prelude[fault]}\n${text}`;
} else if (fault === "no-update") {
  const re = /^(function runNavierStokes\(\)\n\{\n)    solver\.update\(\);$/m;
  if (!re.test(text)) throw new Error(`${file}: no solver.update() in runNavierStokes`);
  text = text.replace(re, "$1");
} else {
  throw new Error(`unknown fault ${JSON.stringify(fault)} for ${name}`);
}
vm.runInThisContext(text, { filename: file });
