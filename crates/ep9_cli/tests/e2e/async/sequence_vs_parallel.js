// Awaiting in sequence vs starting both first: the job order differs.
var log = [];
async function step(name) { log.push(name + " start"); await null; log.push(name + " end"); return name; }
async function seq() { await step("s1"); await step("s2"); }
async function par() { var x = step("p1"), y = step("p2"); await x; await y; }
seq(); par();
Promise.resolve().then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function () { print(log.join(", ")); });
