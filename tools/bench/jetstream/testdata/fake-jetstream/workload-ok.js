"use strict";
// A workload for the fake cli.js: it checks the parts of the realm host API that ep9-host.mjs
// gives, and that the realm runs as strict code (this file has the prologue).
// Outside the class: a class body is always strict code.
function thisOfPlainCall() {
  return this;
}
class Benchmark {
  runIteration(i) {
    return i + 1;
  }
  async check() {
    const strict = thisOfPlainCall() === undefined;
    if (!strict) throw new Error("not strict");
    const text = new ShellTextDecoder().decode(ShellPrefetchedResources[JetStream.preload.data]);
    if (text !== "hi!") throw new Error(`resource text ${text}`);
    if (readFile("./data.txt") !== "hi!") throw new Error("readFile");
    if (typeof performance.now() !== "number") throw new Error("performance.now");
    if (self !== globalThis) throw new Error("self");
    let threw = false;
    try { read("./data.txt", "binary"); } catch { threw = true; }
    if (!threw) throw new Error("a binary read must throw");
    const late = await new Promise((r) => setTimeout(() => r("late"), 5));
    if (late !== "late") throw new Error("setTimeout");
    console.log("workload ok line");
    printErr("workload err line");
  }
}
