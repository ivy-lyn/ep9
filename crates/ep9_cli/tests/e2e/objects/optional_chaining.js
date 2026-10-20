var o = { a: { b: { c: 1 } }, f: function () { return "called"; } };
var n = null;
print(o?.a?.b?.c, n?.a, n?.a.b.c, o.x?.y, o.f?.(), o.g?.(), n?.[0]);
