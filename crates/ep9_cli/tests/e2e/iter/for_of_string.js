var out = [];
for (var ch of "a😀é") out.push(ch + ":" + ch.length);
print(out.join(" "));
print([..."xyz"].join("-"), [..."😀😀"].length);
