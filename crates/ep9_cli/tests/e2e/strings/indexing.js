var s = "abcdef";
print(s[0], s[5], s[6], s["1"], s.length);
var t = "";
for (var i = s.length - 1; i >= 0; i--) t += s[i];
print(t);
print("é"[0], "😀"[0].length, "😀".length);
