var [a, , b = 7, ...rest] = [1, 2, undefined, 4, 5];
print(a, b, rest.join(","));
var x, y;
[x, y] = [10, 20];
[x, y] = [y, x];
print(x, y);
