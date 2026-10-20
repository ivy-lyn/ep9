var vals = [undefined, null, true, false, "", " 42 ", "4x", [], [7], [1, 2], {}, "1e1000", "-0"];
var out = [];
for (var i = 0; i < vals.length; i++) out.push(+vals[i]);
print(out.join(" "));
print(1 / +"-0");
