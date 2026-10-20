var [a, [b, c], { d }] = [1, [2, 3], { d: 4 }];
print(a, b, c, d);
var [x = 10, y = x * 2] = [];
print(x, y);
var [, , third] = "abc";
print(third);
function swap([p, q]) { return [q, p]; }
print(swap([1, 2]).join(","));
for (var [k, v] of [["a", 1], ["b", 2]]) print(k + v);
