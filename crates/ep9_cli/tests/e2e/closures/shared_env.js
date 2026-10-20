function pair() {
  var v = 0;
  return [function () { v++; return v; }, function () { return v * 10; }];
}
var p = pair();
p[0](); p[0]();
print(p[1](), p[0](), p[1]());
