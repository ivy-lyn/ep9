// Promise.all over async functions keeps the input order.
async function delayed(v, n) { for (var i = 0; i < n; i++) await null; print("settled", v); return v; }
Promise.all([delayed("a", 3), delayed("b", 1), delayed("c", 2)]).then(function (vs) { print(vs.join(",")); });
