var o = { valueOf: function () { return {}; }, toString: function () { return "7"; } };
print(o * 2, o + 1);
var bad = { valueOf: function () { return {}; }, toString: function () { return {}; } };
try { bad + 1; } catch (e) { print(e instanceof TypeError); }
