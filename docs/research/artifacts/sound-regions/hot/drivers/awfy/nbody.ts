import { NBody } from '$EP9_RESEARCH_WORK/tsrs-type-oracle/corpus/awfy-ts/src/nbody.ts';
const b = new NBody();
const ms = Number(process.argv[2] ?? 1500);
const t0 = performance.now();
let n = 0;
while (performance.now() - t0 < ms) { if (!b.innerBenchmarkLoop(250000)) throw new Error('verify failed'); n++; }
console.log('NBody', n, 'iterations', (performance.now() - t0).toFixed(0), 'ms');
