var base = { hello() { return "base hello"; } };
var obj = { __proto__: base, hello() { return "obj -> " + super.hello(); } };
print(obj.hello());
var o2 = { m() { return super.toString === Object.prototype.toString; } };
print(o2.m());
