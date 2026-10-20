var o = {};
print(o + "", o.toString(), String_of({ toString: function () { return "custom"; } }));
function String_of(x) { return "" + x; }
print([1, [2, 3]] + "", [{}] + "");
