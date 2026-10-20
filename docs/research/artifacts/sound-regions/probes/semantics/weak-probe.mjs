// B26: FinalizationRegistry callbacks never run inside a synchronous region; they run as later jobs.
// WeakRef.deref() may turn undefined between jobs. Run: node --expose-gc weak-probe.mjs
const log = [];
const fr = new FinalizationRegistry((h) => log.push('cleanup:' + h));
let wr;
(function () { const o = { v: 1 }; fr.register(o, 'o'); wr = new WeakRef(o); })();
log.push('region-start');
for (let i = 0; i < 5; i++) { globalThis.gc(); log.push('gc' + i + ':deref=' + typeof wr.deref()); }
log.push('region-end');
await new Promise((r) => setTimeout(r, 50));
globalThis.gc();
await new Promise((r) => setTimeout(r, 50));
log.push('later:deref=' + typeof wr.deref());
console.log('B26  | no user code runs inside a synchronous region (FR)       | ' + log.join(' '));
