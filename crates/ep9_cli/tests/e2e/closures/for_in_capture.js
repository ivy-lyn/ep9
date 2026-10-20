var o = { a: 1, b: 2, c: 3 };
var fs = [];
for (let k in o) fs.push(() => k + o[k]);
print(fs[0](), fs[1](), fs[2]());
