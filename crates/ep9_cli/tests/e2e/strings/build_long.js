var s = "";
for (var i = 0; i < 1000; i++) s += "ab";
print(s.length, s[0], s[1999]);
var parts = [];
for (var j = 0; j < 100; j++) parts.push(j);
var joined = parts.join(",");
print(joined.length, joined[joined.length - 1]);
