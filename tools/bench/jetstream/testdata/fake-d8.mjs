// Fake d8 for run.test.mjs. run.mjs calls: node fake-d8.mjs [flags...] cli.js -- --test=<name> ...
// Flag --plan=<name>:<r0>/<r1>/...;<name>:... gives the result of call 0, 1, ... of a workload
// (the last one repeats; a workload not in the plan scores 100):
//   a number: print that JetStream score; "x": write 5 stderr lines (the cause on line 4), exit 1;
//   "k": kill this process with SIGKILL.
// Flag --tag=<config> and env FAKE_STATE=<dir> count the calls per config and workload.
import fs from "node:fs";
import path from "node:path";

const argv = process.argv.slice(2);
const arg = (k) => argv.find((a) => a.startsWith(`--${k}=`))?.slice(k.length + 3);
const name = arg("test");
const plan = Object.fromEntries((arg("plan") ?? "").split(";").filter(Boolean).map((p) => p.split(":")));
const counter = path.join(process.env.FAKE_STATE, `${arg("tag")}-${name}.n`);
const n = fs.existsSync(counter) ? Number(fs.readFileSync(counter, "utf8")) : 0;
fs.writeFileSync(counter, String(n + 1));
const seq = (plan[name] ?? "100").split("/");
const r = seq[Math.min(n, seq.length - 1)];
if (r === "x") {
  process.stderr.write(`line 1\nline 2\nline 3\nFATAL: ${name} call ${n} crashed\nline 5\n`);
  process.exit(1);
} else if (r === "k") {
  process.kill(process.pid, "SIGKILL");
} else {
  console.log(JSON.stringify({ "JetStream3.0": { tests: { [name]: { metrics: { Score: { current: [Number(r)] } } } } } }));
}
