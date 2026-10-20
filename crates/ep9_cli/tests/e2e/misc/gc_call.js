var keep = [];
for (var i = 0; i < 100; i++) { keep.push({ i: i, s: "v" + i }); if (i % 25 === 0) gc(); }
gc();
var s = 0;
for (var j = 0; j < keep.length; j++) s += keep[j].i;
print(s, keep[99].s);
