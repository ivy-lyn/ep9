// Returning a promise from an async function takes the thenable job (2 extra ticks).
async function plain() { return 1; }
async function wrapped() { return Promise.resolve(2); }
wrapped().then(function (v) { print("wrapped", v); });
plain().then(function (v) { print("plain", v); });
Promise.resolve().then(function () { print("t1"); }).then(function () { print("t2"); }).then(function () { print("t3"); }).then(function () { print("t4"); });
