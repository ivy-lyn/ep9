var a = [];
a[1000] = "x";
print(a.length, a[999], a[1000]);
a.length = 10;
print(a.length, a[1000]);
