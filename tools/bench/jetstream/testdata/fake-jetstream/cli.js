// A small stand-in for JetStream's cli.js (ep9-host.test.mjs). It makes one workload realm per
// run the way JetStreamDriver.js ShellScripts.run does: runString(""), then top, console, self,
// the prefetched resources (bytes), ShellTextDecoder and performance on the realm, then
// loadString() of each script, then it waits for top.currentResolve. It prints a JetStream JSON
// line whose score is the sum of the results.
const name = globalThis.arguments.find((a) => a.startsWith("--test=")).slice(7);
let resolve, reject;
const done = new Promise((a, b) => {
  resolve = a;
  reject = b;
});
const g = runString("");
g.console = Object.assign({}, console);
g.self = g;
g.top = { currentResolve: resolve, currentReject: reject };
g.ShellPrefetchedResources = { "./data.txt": new Int8Array([104, 105, 0x21]) };
// The real driver passes its TextDecoder polyfill here; the ep9 realm has its own.
g.ShellTextDecoder = class TextDecoderPolyfill {};
g.performance ??= performance;
const scripts = [
  "globalThis.JetStream = { preload: { data: './data.txt' } };",
  "performance.mark ??= function(name) { return { name }; };",
  readFile(`./workload-${name}.js`),
  `{
    const b = new Benchmark();
    const results = [];
    for (let i = 0; i < 3; i++) {
      const t0 = performance.now();
      results.push(b.runIteration(i));
      if (!(performance.now() >= t0)) throw new Error("clock goes back");
    }
    Promise.resolve(b.check).then((c) => c()).then(() => top.currentResolve(results), (e) => top.currentReject(e));
  }`,
];
for (const s of scripts) g.loadString(s);
done.then(
  (r) => print(JSON.stringify({ "JetStream3.0": { tests: { [name]: { metrics: { Score: { current: [r.reduce((a, x) => a + x, 0)] } } } } } })),
  (e) => {
    console.error("JetStream3 failed: " + e);
    throw e;
  },
);
