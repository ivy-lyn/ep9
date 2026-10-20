var fs = [];
for (let i = 0; i < 3; i++) { fs.push(() => i); i += 0; }
print(fs[0](), fs[1](), fs[2]());
var gs = [];
for (let i = 0; i < 3; i++) { gs.push(() => i); i++; }
print(gs.length, gs[0](), gs[1]());
