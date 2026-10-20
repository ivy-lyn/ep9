var a = [1, , 3];
var b = [...a];
print(b.length, 1 in b, b[1]);
function count() { return arguments.length; }
print(count(...a), count(...[], ...[1]), Math_max(...[3, 9, 2]));
function Math_max() { var m = -Infinity; for (var i = 0; i < arguments.length; i++) if (arguments[i] > m) m = arguments[i]; return m; }
