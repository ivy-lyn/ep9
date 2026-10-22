// An async function that throws after await rejects; later jobs still run.
async function bad() { await null; throw new Error("bad"); }
bad().then(null, function (e) { print("handled", e.message); });
Promise.resolve().then(function () { print("j1"); }).then(function () { print("j2"); }).then(function () { print("j3"); });
