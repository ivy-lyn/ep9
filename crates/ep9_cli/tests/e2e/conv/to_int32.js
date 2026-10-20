var vals = [0, -0, 1.9, -1.9, 2147483648, 4294967296, 4294967297, -2147483649, 1e10, NaN, Infinity, "5.5", "x", true];
var out = [];
for (var i = 0; i < vals.length; i++) out.push(vals[i] | 0);
print(out.join(" "));
var u = [];
for (var j = 0; j < vals.length; j++) u.push(vals[j] >>> 0);
print(u.join(" "));
