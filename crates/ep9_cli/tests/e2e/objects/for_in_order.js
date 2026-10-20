var o = { b: 1, a: 2, 10: "x", 2: "y", c: 3 };
var ks = [];
for (var k in o) ks.push(k);
print(ks.join(","));
var p = { inherited: 1 };
function C() { this.own = 2; }
C.prototype = p;
var c = new C();
ks = [];
for (var k2 in c) ks.push(k2);
print(ks.join(","));
