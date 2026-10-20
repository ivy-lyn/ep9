// B27: module-private bindings are a closed world for indirect eval / new Function / dynamic import.
import { pub, readSecret } from './scope-mod.mjs';
const url = new URL('./scope-mod.mjs', import.meta.url).href;
const r1 = (0, eval)('typeof secret');
const r2 = new Function('return typeof secret')();
const ns = await (0, eval)(`import(${JSON.stringify(url)})`);
const r3 = Object.keys(ns).join(',');
ns.pub.v = 'str';
console.log(`B27  | module-private binding unreachable by eval/new Function   | indirect eval: ${r1}; new Function: ${r2}; import() ns keys: ${r3}; exported pub.v now ${pub.v}; secret.v still ${readSecret()}`);
