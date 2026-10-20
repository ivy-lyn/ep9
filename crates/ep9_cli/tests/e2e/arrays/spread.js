var a = [1, 2], b = [3, 4];
var c = [0, ...a, ...b, 5];
print(c.length, c.join(","));
function sum3(x, y, z) { return x + y + z; }
print(sum3(...[1, 2, 3]), sum3(1, ...[2, 3]));
var copy = [...c];
copy[0] = 99;
print(c[0], copy[0]);
