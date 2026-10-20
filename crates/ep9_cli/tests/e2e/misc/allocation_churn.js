var last;
for (var i = 0; i < 2000; i++) { last = { a: [i, i + 1], b: "s" + i }; }
print(last.a[1], last.b);
