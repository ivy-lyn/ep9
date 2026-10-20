var a = ["x", "y", "z"];
var ks = [];
for (var k in a) ks.push(k + ":" + a[k] + ":" + typeof k);
print(ks.join(" "));
