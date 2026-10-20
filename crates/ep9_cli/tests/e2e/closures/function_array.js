var ops = [];
for (let n = 1; n <= 4; n++) ops.push(function (x) { return x * n; });
var r = [];
for (var i = 0; i < ops.length; i++) r.push(ops[i](10));
print(r.join(" "));
