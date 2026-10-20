function make() { var calls = []; return function () { calls.push(arguments.length); return calls.join(""); }; }
var m = make();
m(); m(1); m(1, 2);
print(m(1, 2, 3));
