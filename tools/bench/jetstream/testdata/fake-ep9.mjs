// Fake ep9 for ep9-host.test.mjs: runs a script file in a fresh V8 global that has only print()
// and gc(), as the ep9 shell has (crates/ep9_cli/src/run.rs). Promise jobs run after the script.
//   usage: node fake-ep9.mjs <file.js>
//   FAKE_EP9_FAIL=1   print an ep9-style "Uncaught" report and exit 1 without running the file
//   FAKE_EP9_LOG=FILE append the file name of each run
import fs from "node:fs";
import vm from "node:vm";

const file = process.argv[process.argv.length - 1];
if (process.env.FAKE_EP9_LOG) fs.appendFileSync(process.env.FAKE_EP9_LOG, `${file}\n`);
const report = (e) => {
  process.stderr.write(`${file}:1:1: Uncaught ${e}\n    at fake (${file}:1:1)\n`);
  process.exit(1);
};
if (process.env.FAKE_EP9_FAIL) report("TypeError: fake failure");
const g = vm.createContext(vm.constants.DONT_CONTEXTIFY);
g.print = (...a) => process.stdout.write(`${a.map(String).join(" ")}\n`);
g.gc = () => {};
// ep9 has no Temporal clock in this fake: the prelude must fall back to Date.now().
delete g.Temporal;
try {
  vm.runInContext(fs.readFileSync(file, "utf8"), g, { filename: file });
} catch (e) {
  report(`${e?.name}: ${e?.message}`);
}
process.on("unhandledRejection", (e) => report(`${e?.name}: ${e?.message}`));
