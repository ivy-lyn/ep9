// Two then-chains take turns: each step queues the next job behind the other chain's step.
var a = Promise.resolve("a");
var b = Promise.resolve("b");
a.then(function (v) { print(v, 1); }).then(function () { print("a", 2); }).then(function () { print("a", 3); });
b.then(function (v) { print(v, 1); }).then(function () { print("b", 2); }).then(function () { print("b", 3); });
print("sync done");
