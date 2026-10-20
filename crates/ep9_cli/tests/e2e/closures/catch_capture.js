var fs = [];
for (var i = 0; i < 3; i++) {
  try { throw i * 10; } catch (e) { fs.push(function () { return e; }); }
}
print(fs[0](), fs[1](), fs[2]());
