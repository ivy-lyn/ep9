var calls = [];
function k(v) { calls.push(v); return v; }
switch (3) { case k(1): case k(2): break; case k(3): calls.push("hit"); case k(4): calls.push("fall"); }
print(calls.join(","));
