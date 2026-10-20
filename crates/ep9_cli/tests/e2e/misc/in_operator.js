var a = [10, 20];
print(0 in a, 2 in a, "length" in a, "push" in a);
var o = { k: undefined };
print("k" in o, o.k === undefined, "j" in o);
try { "x" in 5; } catch (e) { print(e instanceof TypeError); }
