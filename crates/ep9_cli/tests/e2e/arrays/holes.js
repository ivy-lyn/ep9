var a = [1, , 3];
print(a.length, a[1], 1 in a, a.join("-"));
var b = [, , ,];
print(b.length, b.join("x"));
var c = [1, 2];
c[5] = 6;
print(c.length, c.join(","), 3 in c);
