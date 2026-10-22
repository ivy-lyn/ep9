// The executor runs at once; the handlers run after the script.
print("before");
var p = new Promise(function (resolve, reject) {
  print("in executor", typeof resolve, typeof reject);
  resolve(42);
  reject(new Error("ignored"));
  resolve(7);
});
p.then(function (v) { print("value", v); });
print("after");
