// Global stores (StaGlobal) after the feedback vector exists (8 calls): a miss that records
// a field case stores through it (M2-P5-PLAN-D8), later calls take the hit path, and every
// other case takes the generic path.

// A `let` binding and a `var` property of the global object.
let lex = 0;
var gv = 0;
function setLex(v) { lex = v; }
function setVar(v) { gv = v; }
let log = [];
for (let i = 0; i < 20; i++) {
  setLex(i);
  setVar(i * 2);
  log.push(lex === i && gv === i * 2 ? "." : i);
}
print(lex, gv, globalThis.gv, "lex" in globalThis, log.join(""));

// A store in the TDZ throws; after the declaration it stores.
function setLate(v) { late = v; }
let tdz = "";
for (let i = 0; i < 12; i++) {
  try { setLate(i); } catch (e) { tdz = e.constructor.name; }
}
print(tdz);

// A `const`: TypeError every time.
const k = 1;
function setK(v) { k = v; }
let ke = 0;
for (let i = 0; i < 12; i++) {
  try { setK(i); } catch (e) { if (e instanceof TypeError) ke++; }
}
print(ke, k);

// A read-only global: ignored in sloppy code, TypeError in strict code.
function setNaN(v) { NaN = v; }
function setNaNStrict(v) { "use strict"; NaN = v; }
let ne = 0;
for (let i = 0; i < 12; i++) {
  setNaN(i);
  try { setNaNStrict(i); } catch (e) { if (e instanceof TypeError) ne++; }
}
print(NaN, ne);

// An accessor on the global object: the setter runs each time.
let seen = 0;
Object.defineProperty(globalThis, "acc", {
  get() { return seen; }, set(v) { seen += v; }, configurable: true,
});
function setAcc(v) { acc = v; }
for (let i = 1; i <= 12; i++) setAcc(i);
print(seen, acc);

// A sloppy store makes a new global; a strict store to a missing name throws.
function setFresh(v) { fresh = v; }
function setMissing(v) { "use strict"; missing = v; }
let me = 0;
for (let i = 0; i < 12; i++) {
  setFresh(i);
  try { setMissing(i); } catch (e) { if (e instanceof ReferenceError) me++; }
}
const fd = Object.getOwnPropertyDescriptor(globalThis, "fresh");
print(fresh, fd.writable, fd.enumerable, fd.configurable, me, "missing" in globalThis);

// The property becomes read-only after the case was recorded.
var w = 0;
function setW(v) { w = v; }
for (let i = 0; i < 12; i++) setW(i);
Object.defineProperty(globalThis, "w", { writable: false });
setW(100);
print(w);

// The property is deleted after the case was recorded, then made again by the store.
globalThis.d = 1;
function setD(v) { d = v; }
for (let i = 0; i < 12; i++) setD(i);
delete globalThis.d;
setD(5);
print(d, Object.getOwnPropertyDescriptor(globalThis, "d").configurable);

// An inherited data property: the store makes an own property of the global object.
Object.prototype.inh = "proto";
function setInh(v) { inh = v; }
for (let i = 0; i < 12; i++) setInh(i);
print(inh, Object.prototype.inh, Object.prototype.hasOwnProperty.call(globalThis, "inh"));
delete Object.prototype.inh;

// An inherited setter: it runs with the global object as `this`.
let ps = [];
Object.defineProperty(Object.prototype, "pset", {
  set(v) { ps.push(this === globalThis ? v : "other"); }, configurable: true,
});
function setP(v) { pset = v; }
for (let i = 0; i < 12; i++) setP(i);
print(ps.join(","), Object.prototype.hasOwnProperty.call(globalThis, "pset"));
delete Object.prototype.pset;

// A stored value that is an object survives collections (the record is a GC point).
var obj = null;
function setObj(v) { obj = v; }
for (let i = 0; i < 12; i++) setObj({ n: i, s: "v" + i });
gc();
print(obj.n, obj.s);

let late = "init";
for (let i = 0; i < 12; i++) setLate("x" + i);
print(late);
