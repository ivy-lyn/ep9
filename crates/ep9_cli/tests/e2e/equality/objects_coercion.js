var o = { valueOf: function () { return 42; } };
var s = { toString: function () { return "str"; } };
print(o == 42, o == "42", o === 42, s == "str", s == "STR", [1] == 1, [1, 2] == "1,2");
print(null == 0, undefined == 0, null == false, ({}) == "[object Object]");
