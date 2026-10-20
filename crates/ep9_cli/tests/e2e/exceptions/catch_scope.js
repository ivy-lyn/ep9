var e = "outer";
try { throw "inner"; } catch (e) { print(e); var e2 = e; }
print(e, e2);
try { throw { a: 1, b: 2 }; } catch ({ a, b }) { print(a + b); }
