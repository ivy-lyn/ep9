var out = [];
var n = 5;
do out.push(n); while (--n);
print(out.join(" "));
for (var i = 3; i--;) out.push("i" + i);
print(out.join(" "));
