// Await reads `constructor` of a native promise once (PromiseResolve).
var reads = 0;
var p = Promise.resolve("value");
Object.defineProperty(p, "constructor", { get: function () { reads++; return Promise; } });
async function f() { var v = await p; print(v, reads); }
f();
print("reads now", reads);
