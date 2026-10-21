// for-in over a string visits the indices of its String wrapper (ToObject), then the
// inherited enumerable keys; the wrapper's own `length` hides an inherited one.
function keys(v) {
  var ks = [];
  for (var k in v) ks.push(k);
  return "[" + ks.join(",") + "]";
}
print(keys("ab"), keys(""), keys("a€c"));
var w = Object("ab");
w[5] = 1;
w.x = 2;
w[2] = 3;
print(keys(w));
Object.prototype.length = 1;
Object.prototype.y = 1;
print(keys("ab"), keys(w));
var s = "";
for (var i in "xyz") s += i + "=" + "xyz"[i] + ";";
print(s);
