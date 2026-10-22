// An async function that awaits another one: the inner one runs first, up to its await.
async function inner(n) { print("inner start", n); await null; print("inner end", n); return n * 2; }
async function outer() { print("outer start"); var v = await inner(1); print("outer got", v); var w = await inner(v); print("outer got", w); return w; }
outer().then(function (v) { print("result", v); });
print("after call");
