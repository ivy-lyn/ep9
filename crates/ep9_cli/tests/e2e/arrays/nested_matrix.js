function mat(n) { var m = []; for (var i = 0; i < n; i++) { m.push([]); for (var j = 0; j < n; j++) m[i].push(i * n + j); } return m; }
function mul(a, b) { var n = a.length, r = mat(n); for (var i = 0; i < n; i++) for (var j = 0; j < n; j++) { var s = 0; for (var k = 0; k < n; k++) s += a[i][k] * b[k][j]; r[i][j] = s; } return r; }
var m = mat(3);
var p = mul(m, m);
print(p[0].join(" "), "/", p[1].join(" "), "/", p[2].join(" "));
