var a = new Array(3);
print(a.length, a[0], a.join("-"));
var b = Array(1, 2, 3);
print(b.length, b.join());
var c = new Array("x");
print(c.length, c[0]);
try { new Array(-1); } catch (e) { print(e instanceof RangeError); }
