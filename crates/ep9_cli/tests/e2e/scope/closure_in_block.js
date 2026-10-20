var fns = [];
for (var i = 0; i < 3; i++) { let j = i * 10; fns.push(function () { return j; }); }
print(fns[0](), fns[1](), fns[2]());
