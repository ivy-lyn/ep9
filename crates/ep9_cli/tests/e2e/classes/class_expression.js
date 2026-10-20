var C = class Named { who() { return typeof Named; } };
print(new C().who(), typeof Named);
var D = class extends C { who2() { return "d"; } };
print(new D().who(), new D().who2());
