var a = [1, 2, 3, 4, 5, 6];
for (var i = 0, j = a.length - 1; i < j; i++, j--) { var t = a[i]; a[i] = a[j]; a[j] = t; }
print(a.join(" "));
