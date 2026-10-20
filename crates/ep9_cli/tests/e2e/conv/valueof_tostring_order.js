var log = [];
var o = { valueOf: function () { log.push("valueOf"); return 1; }, toString: function () { log.push("toString"); return "s"; } };
var a = o + 1;
var b = `${o}`;
var c = o * 2;
var d = o > 0;
print(a, b, c, d, log.join(","));
