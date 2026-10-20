var o = {};
var key = { toString: function () { return "k"; } };
o[key] = 1;
o[true] = 2; o[null] = 3; o[undefined] = 4;
print(o.k, o["true"], o["null"], o["undefined"]);
