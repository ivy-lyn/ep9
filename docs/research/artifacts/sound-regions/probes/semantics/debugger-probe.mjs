// B28: a worker thread of the SAME program attaches to the main thread's inspector, pauses it at a
// `debugger` statement and rewrites a strict-mode local `let x = 1` (no eval, no with) to a string.
// So even PROVEN facts on locals need an "inspector attached" fuse.
import { Worker } from 'node:worker_threads';

const workerSrc = String.raw`
const { parentPort } = require('node:worker_threads');
const { Session } = require('node:inspector');
const s = new Session();
s.connectToMainThread();
s.on('Debugger.paused', ({ params }) => {
  const cf = params.callFrames[0];
  const scopes = cf.scopeChain.map((x) => x.type).join('/');
  s.post('Debugger.setVariableValue', { scopeNumber: 0, variableName: 'x', newValue: { value: 'str' }, callFrameId: cf.callFrameId }, (err) => {
    parentPort.postMessage({ log: 'paused=' + params.reason + ' scopes=' + scopes + ' setErr=' + (err ? err.message : 'none') });
    s.post('Debugger.resume');
  });
});
setInterval(() => {}, 1000);   // keep the worker alive (an inspector session does not)
s.post('Debugger.enable', () => parentPort.postMessage({ ready: true }));
`;
const w = new Worker(workerSrc, { eval: true });
const logs = [];
w.on('message', (m) => { if (m.log) logs.push(m.log); });
await new Promise((r) => w.once('message', r));   // keep the event loop free so inspector messages dispatch

function f() {
  let x = 1;            // strict (ESM), never reassigned in source, no eval, no with
  debugger;             // no-op without a debugger; pause point with one
  return [typeof x, x];
}
const r = f();
await new Promise((res) => setTimeout(res, 100));
console.log(`B28  | strict local never reassigned keeps its int value         | ${r} (${logs.join('; ')})`);
await w.terminate();
