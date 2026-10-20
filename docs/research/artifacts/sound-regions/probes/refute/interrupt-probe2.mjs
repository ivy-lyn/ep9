// Same as interrupt-probe.mjs, but the loop has no calls at all (no Date.now): only a load, a compare and an add.
import { Worker } from 'node:worker_threads';
const src = String.raw`
const { parentPort } = require('node:worker_threads');
const { Session } = require('node:inspector');
const s = new Session();
s.connectToMainThread();
parentPort.on('message', () => {
  s.post('Runtime.evaluate', { expression: 'globalThis.box.v = "str"; 0' }, (e) => parentPort.postMessage({ done: e ? e.message : 'ok' }));
});
parentPort.postMessage({ ready: true });
setInterval(() => {}, 1000);
`;
globalThis.box = { v: 1 };
const w = new Worker(src, { eval: true });
await new Promise((r) => w.once('message', r));
w.postMessage('go');
function region(b) {
  let i = 0;
  for (; i < 2e9; i++) { if (typeof b.v !== 'number') break; }
  return { brokeAt: i, final: typeof b.v };
}
const r = region(globalThis.box);
console.log('INTERRUPT-PROBE2', JSON.stringify(r));
await w.terminate();
