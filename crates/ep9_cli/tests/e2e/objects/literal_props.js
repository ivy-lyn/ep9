var o = { a: 1, "b c": 2, 3: "three", d: { e: 4 } };
print(o.a, o["b c"], o[3], o["3"], o.d.e, o.missing);
o.a = 10; o.z = 26;
print(o.a, o.z);
