function F() {}
var f = new F();
print(f instanceof F, f instanceof Object, 5 instanceof F, "s" instanceof Object, null instanceof F);
try { f instanceof 5; } catch (e) { print(e instanceof TypeError); }
