var fl = [], fv = [];
for (let i = 0; i < 3; i++) fl.push(function () { return i; });
for (var j = 0; j < 3; j++) fv.push(function () { return j; });
print(fl[0](), fl[1](), fl[2](), fv[0](), fv[1](), fv[2]());
