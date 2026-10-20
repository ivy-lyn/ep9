var parts = [];
for (var i = 0; i < 300; i++) parts.push("p" + i);
var s = parts.join("");
print(s.length, s[0] + s[1], s[s.length - 1]);
