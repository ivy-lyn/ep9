var n = "dyn";
class C { [n + "Method"]() { return 1; } static [n]() { return 2; } ["a" + "b"]() { return 3; } }
print(new C().dynMethod(), C.dyn(), new C().ab());
