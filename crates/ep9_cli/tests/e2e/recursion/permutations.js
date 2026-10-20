function perms(xs) {
  if (xs.length <= 1) return [xs.join("")];
  var out = [];
  for (var i = 0; i < xs.length; i++) {
    var rest = [];
    for (var j = 0; j < xs.length; j++) if (j !== i) rest.push(xs[j]);
    var ps = perms(rest);
    for (var k = 0; k < ps.length; k++) out.push(xs[i] + ps[k]);
  }
  return out;
}
var p = perms(["a", "b", "c", "d"]);
print(p.length, p.join(" "));
