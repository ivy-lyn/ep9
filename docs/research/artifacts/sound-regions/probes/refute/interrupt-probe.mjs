// Does a worker's inspector session run code on the main thread while the main thread is busy in a
// synchronous loop (no await, no debugger statement, no call out)? If yes, interrupt polls are effect points.
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
'use strict';
globalThis.box = { v: 1 };
const w = new Worker(src, { eval: true });
await new Promise((r) => w.once('message', r));
w.postMessage('go');
function region(b) {       // synchronous loop; reads b.v each iteration; no calls
  const t0 = Date.now();
  let seen = typeof b.v, changedAt = -1, i = 0;
  while (Date.now() - t0 < 1500) {
    i++;
    if (typeof b.v !== seen) { changedAt = Date.now() - t0; seen = typeof b.v; break; }
  }
  return { changedAt, final: typeof b.v, iters: i };
}
const r = region(globalThis.box);
console.log('INTERRUPT-PROBE', JSON.stringify(r));
await w.terminate();
