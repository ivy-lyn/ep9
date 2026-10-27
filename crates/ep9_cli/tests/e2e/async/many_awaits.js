// Many awaits in one frame do not grow the native stack.
async function sum(n) { var s = 0; for (var i = 0; i < n; i++) s += await i; return s; }
sum(3000).then(function (v) { print("sum", v); });
