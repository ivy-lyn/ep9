var o = { a: 1, b: { c: 2, d: [3] }, e: undefined };
var { a, b: { c }, e = "dflt", f = a + 10, a: renamed } = o;
print(a, c, e, f, renamed);
function g({ p = 1, q } = {}) { return p + ":" + q; }
print(g(), g({ q: 2 }), g({ p: 5, q: 6 }));
