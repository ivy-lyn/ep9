var words = ["a", "b", "a", "c", "b", "a"];
var counts = {};
for (var i = 0; i < words.length; i++) { var w = words[i]; counts[w] = (counts[w] || 0) + 1; }
var out = [];
for (var k in counts) out.push(k + "=" + counts[k]);
print(out.join(" "));
