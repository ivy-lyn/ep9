// Two async functions and a then-chain take turns, job by job.
var log = [];
async function a() { log.push("a0"); await 0; log.push("a1"); await 0; log.push("a2"); }
async function b() { log.push("b0"); await 0; log.push("b1"); await 0; log.push("b2"); }
a(); b();
Promise.resolve().then(function () { log.push("c1"); }).then(function () { log.push("c2"); }).then(function () { log.push("c3"); print(log.join(" ")); });
