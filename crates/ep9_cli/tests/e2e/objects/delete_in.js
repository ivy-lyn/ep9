var o = { a: 1, b: 2 };
print("a" in o, delete o.a, "a" in o, o.a, delete o.nothing, "b" in o);
print("toString" in o, "length" in [], 0 in [1], 1 in [1]);
var a = [1, 2, 3];
print(delete a[1], a[1], a.length, 1 in a);
