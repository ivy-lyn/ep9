var o = {};
o.z = 1; o.a = 2; o[5] = 3; o.m = 4; o[0] = 5;
var ks = [];
for (var k in o) ks.push(k);
print(ks.join(" "));
