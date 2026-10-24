// finally blocks run with await inside; a finally that throws overrides the return value.
async function a() { try { return await "a"; } finally { await null; print("a finally"); } }
async function b() { try { return "b"; } finally { throw "b override"; } }
async function c() { try { throw "c"; } finally { print("c finally"); } }
a().then(function (v) { print("a ->", v); });
b().then(function (v) { print("b ->", v); }, function (e) { print("b rejected", e); });
c().then(null, function (e) { print("c rejected", e); });
