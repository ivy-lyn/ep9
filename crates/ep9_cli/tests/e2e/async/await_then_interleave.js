// An await of a native promise takes one job, as one `.then` step does.
async function f() { print("f1"); await null; print("f2"); await null; print("f3"); }
f();
Promise.resolve().then(function () { print("t1"); }).then(function () { print("t2"); }).then(function () { print("t3"); });
print("sync");
