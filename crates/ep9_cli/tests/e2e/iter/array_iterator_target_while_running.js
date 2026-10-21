// While `next` runs user code, only that call holds the iterated object: here `valueOf` of
// its length allocates (a collection each time under --gc-stress), and no JS value reaches
// the object.
function grow() {
  var g = [];
  for (var k = 0; k < 20; k++) g.push({ k: k, s: "x" + k });
  return this.n;
}
function make(n) {
  var o = { length: { n: n, valueOf: grow } };
  for (var i = 0; i < n; i++) o[i] = "v" + i;
  return Array.prototype.values.call(o);
}
var it = make(3);
var r;
while (!(r = it.next()).done) print(r.value);
print([...make(4)].join(","));
for (var x of make(2)) print(x);
