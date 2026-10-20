var o = { x: 1, y: 2, z: 3 };
var s = 0, ks = "";
for (var k in o) { s += o[k]; ks += k; }
print(s, ks);
for (var k2 in null) print("never");
for (var k3 in undefined) print("never");
print("done");
