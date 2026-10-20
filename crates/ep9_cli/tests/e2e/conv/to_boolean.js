var vals = [0, -0, NaN, "", "0", "false", null, undefined, [], {}, Infinity, -1];
var out = [];
for (var i = 0; i < vals.length; i++) out.push(!!vals[i]);
print(out.join(" "));
